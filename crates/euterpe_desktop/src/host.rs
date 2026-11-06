//! The audio callback: picks the input source, runs the device in 32-sample
//! blocks, and reports meters and load to the UI. Nothing here locks or
//! allocates on the audio thread except `try_lock` on hand-over slots.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use desktop_audio::AudioProcessor;
use euterpe_core::fastmath;

use crate::runner::{BlockAdapter, Runner};

/// Where the device's input comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The interface's first input channel.
    Interface,
    Sine,
    Noise,
    /// A loaded WAV file, looped.
    File,
}

impl Source {
    pub const ALL: [Source; 4] = [Source::Interface, Source::Sine, Source::Noise, Source::File];

    pub fn label(self) -> &'static str {
        match self {
            Source::Interface => "Interface input",
            Source::Sine => "Sine",
            Source::Noise => "Noise",
            Source::File => "WAV file (loop)",
        }
    }

    fn from_u32(x: u32) -> Self {
        Source::ALL
            .get(x as usize)
            .copied()
            .unwrap_or(Source::Interface)
    }
}

/// A float in an atomic, for UI ↔ audio values.
#[derive(Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
    /// Raise to `v` if larger. For non-negative floats the bit patterns sort
    /// like the values, so an integer max works.
    pub fn raise(&self, v: f32) {
        self.0.fetch_max(v.max(0.0).to_bits(), Ordering::Relaxed);
    }
    /// Read and reset to zero.
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

/// State shared by the UI and the audio thread.
pub struct Shared {
    /// A new runner for the audio thread to pick up.
    pub pending: Mutex<Option<Box<dyn Runner>>>,
    /// The runner it replaced, dropped by the UI thread rather than on the
    /// audio thread.
    pub retired: Mutex<Option<Box<dyn Runner>>>,
    /// A newly loaded file for the `File` source.
    pub pending_file: Mutex<Option<Arc<Vec<f32>>>>,
    source: AtomicU32,
    pub sine_hz: AtomicF32,
    pub source_level_db: AtomicF32,
    pub output_db: AtomicF32,
    pub bypass: AtomicBool,
    /// Peaks since the UI last read them.
    pub in_peak: AtomicF32,
    pub out_peak: AtomicF32,
    /// Time spent processing, as a fraction of the time the audio covers.
    pub load: AtomicF32,
    pub load_peak: AtomicF32,
    pub blocks: AtomicU64,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            pending: Mutex::new(None),
            retired: Mutex::new(None),
            pending_file: Mutex::new(None),
            source: AtomicU32::new(0),
            sine_hz: AtomicF32::new(220.0),
            source_level_db: AtomicF32::new(-12.0),
            output_db: AtomicF32::new(0.0),
            bypass: AtomicBool::new(false),
            in_peak: AtomicF32::default(),
            out_peak: AtomicF32::default(),
            load: AtomicF32::default(),
            load_peak: AtomicF32::default(),
            blocks: AtomicU64::new(0),
        }
    }
}

impl Shared {
    pub fn source(&self) -> Source {
        Source::from_u32(self.source.load(Ordering::Relaxed))
    }

    pub fn set_source(&self, s: Source) {
        self.source.store(s as u32, Ordering::Relaxed);
    }
}

/// The audio-thread side.
pub struct Host {
    shared: Arc<Shared>,
    runner: Option<Box<dyn Runner>>,
    adapter: BlockAdapter,
    sample_rate: f32,
    phase: f32,
    noise: u32,
    file: Option<Arc<Vec<f32>>>,
    file_pos: usize,
}

impl Host {
    pub fn new(shared: Arc<Shared>, sample_rate: f32) -> Self {
        Self {
            shared,
            runner: None,
            adapter: BlockAdapter::default(),
            sample_rate,
            phase: 0.0,
            noise: 0x1234_5678,
            file: None,
            file_pos: 0,
        }
    }

    fn pick_up_handovers(&mut self) {
        if let Ok(mut pending) = self.shared.pending.try_lock()
            && let Some(new) = pending.take()
        {
            let old = self.runner.replace(new);
            self.adapter = BlockAdapter::default();
            if let Ok(mut retired) = self.shared.retired.try_lock() {
                *retired = old;
            }
            // If the UI holds `retired` right now, `old` drops here: rare,
            // and only when switching devices.
        }
        if let Ok(mut file) = self.shared.pending_file.try_lock()
            && let Some(f) = file.take()
        {
            self.file = Some(f);
            self.file_pos = 0;
        }
    }

    fn source_sample(&mut self, source: Source, interface: f32, amp: f32) -> f32 {
        match source {
            Source::Interface => interface,
            Source::Sine => {
                let hz = self.shared.sine_hz.get();
                self.phase = (self.phase + hz / self.sample_rate).fract();
                (std::f32::consts::TAU * self.phase).sin() * amp
            }
            Source::Noise => {
                let mut x = self.noise;
                x ^= x << 13_u32;
                x ^= x >> 17_u32;
                x ^= x << 5_u32;
                self.noise = x;
                ((x >> 8_u32) as f32 / 8_388_608.0 - 1.0) * amp
            }
            Source::File => match &self.file {
                Some(f) if !f.is_empty() => {
                    let s = f[self.file_pos % f.len()];
                    self.file_pos = (self.file_pos + 1) % f.len();
                    s
                }
                _ => 0.0,
            },
        }
    }
}

impl AudioProcessor for Host {
    fn process_buffer(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]]) {
        let start = Instant::now();
        self.pick_up_handovers();

        let source = self.shared.source();
        let amp = fastmath::db_to_gain(self.shared.source_level_db.get());
        let out_gain = fastmath::db_to_gain(self.shared.output_db.get());
        let bypass = self.shared.bypass.load(Ordering::Relaxed);
        let (mut in_peak, mut out_peak) = (0.0_f32, 0.0_f32);

        for (frame_in, frame_out) in input.iter().zip(output.iter_mut()) {
            let x = self.source_sample(source, frame_in[0], amp);
            in_peak = in_peak.max(x.abs());
            let y = match (&mut self.runner, bypass) {
                (Some(runner), false) => self.adapter.tick(x, |block| {
                    runner.process(block);
                    self.shared.blocks.fetch_add(1, Ordering::Relaxed);
                }),
                _ => x,
            };
            let y = y * out_gain;
            out_peak = out_peak.max(y.abs());
            *frame_out = [y, y];
        }

        self.shared.in_peak.raise(in_peak);
        self.shared.out_peak.raise(out_peak);
        let covered = input.len() as f32 / self.sample_rate;
        if covered > 0.0 {
            let load = start.elapsed().as_secs_f32() / covered;
            self.shared.load.set(load);
            self.shared.load_peak.raise(load);
        }
    }
}

#[cfg(test)]
mod tests {
    use euterpe_core::Block;

    use super::*;

    struct Halve;
    impl Runner for Halve {
        fn process(&mut self, block: &mut Block) {
            block.iter_mut().for_each(|s| *s *= 0.5);
        }
        fn latency(&self) -> usize {
            0
        }
    }

    fn run(host: &mut Host, input: &[f32]) -> Vec<f32> {
        let frames: Vec<[f32; 2]> = input.iter().map(|&x| [x, 0.0]).collect();
        let mut out = vec![[0.0; 2]; frames.len()];
        host.process_buffer(&frames, &mut out);
        out.iter().map(|f| f[0]).collect()
    }

    #[test]
    fn runs_the_device_on_the_interface_input_with_one_block_of_latency() {
        let shared = Arc::new(Shared::default());
        *shared.pending.lock().unwrap() = Some(Box::new(Halve));
        let mut host = Host::new(Arc::clone(&shared), 48_000.0);
        let out = run(&mut host, &[1.0; 96]);
        assert!(out[..32].iter().all(|&y| y == 0.0));
        assert!(out[32..].iter().all(|&y| (y - 0.5).abs() < 1e-6));
        assert_eq!(shared.blocks.load(Ordering::Relaxed), 3);
        assert!((shared.in_peak.take() - 1.0).abs() < 1e-6);
        assert!((shared.out_peak.take() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn bypass_passes_input_straight_through() {
        let shared = Arc::new(Shared::default());
        *shared.pending.lock().unwrap() = Some(Box::new(Halve));
        shared.bypass.store(true, Ordering::Relaxed);
        let mut host = Host::new(Arc::clone(&shared), 48_000.0);
        assert!(
            run(&mut host, &[0.25; 40])
                .iter()
                .all(|&y| (y - 0.25).abs() < 1e-6)
        );
    }

    #[test]
    fn sine_source_has_the_requested_level() {
        let shared = Arc::new(Shared::default());
        shared.set_source(Source::Sine);
        shared.source_level_db.set(-6.0);
        let mut host = Host::new(Arc::clone(&shared), 48_000.0);
        let out = run(&mut host, &vec![0.0; 4_800]);
        let peak = out.iter().fold(0.0_f32, |m, &y| m.max(y.abs()));
        assert!((peak - fastmath::db_to_gain(-6.0)).abs() < 1e-3, "{peak}");
    }

    #[test]
    fn switching_runners_retires_the_old_one() {
        let shared = Arc::new(Shared::default());
        *shared.pending.lock().unwrap() = Some(Box::new(Halve));
        let mut host = Host::new(Arc::clone(&shared), 48_000.0);
        run(&mut host, &[0.0; 32]);
        *shared.pending.lock().unwrap() = Some(Box::new(Halve));
        run(&mut host, &[0.0; 32]);
        assert!(shared.retired.lock().unwrap().is_some());
    }
}
