use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use desktop_audio::AudioProcessor;
use preamp::SAMPLE_RATE;
use preamp::channel::Channel;
use preamp::dsp::Process;

/// Pending IR to be loaded into a channel.
/// The UI thread sets this, and the audio thread takes it.
#[derive(Default)]
pub struct PendingIr {
    pub ir: Option<Vec<f32>>,
}

/// Shared state between UI and audio threads.
pub struct SharedState {
    pub ch1: Mutex<Channel>,
    pub ch2: Mutex<Channel>,
    pub mono_mode: AtomicBool,
    // Pending IRs are separate so the UI can set them without locking the channel
    pub ch1_pending_ir: Mutex<PendingIr>,
    pub ch2_pending_ir: Mutex<PendingIr>,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            ch1: Mutex::new(Channel::new(SAMPLE_RATE)),
            ch2: Mutex::new(Channel::new(SAMPLE_RATE)),
            mono_mode: AtomicBool::new(true),
            ch1_pending_ir: Mutex::new(PendingIr::default()),
            ch2_pending_ir: Mutex::new(PendingIr::default()),
        }
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}

struct DcBlocker {
    prev_input: [f32; 2],
    prev_output: [f32; 2],
    pole: f32,
}

impl DcBlocker {
    const CUTOFF_HZ: f32 = 20.0;

    fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate as f32;
        let pole = (-2.0 * std::f32::consts::PI * Self::CUTOFF_HZ / sample_rate).exp();
        Self {
            prev_input: [0.0; 2],
            prev_output: [0.0; 2],
            pole,
        }
    }

    fn process_frame(&mut self, input: [f32; 2]) -> [f32; 2] {
        let mut output = [0.0; 2];
        for ch in 0..2 {
            let current = input[ch];
            let y = current - self.prev_input[ch] + self.pole * self.prev_output[ch];
            self.prev_input[ch] = current;
            self.prev_output[ch] = y;
            output[ch] = y;
        }
        output
    }
}

struct InputTransientSuppressor {
    prev_input: [f32; 2],
    gain: f32,
    mute_samples_remaining: usize,
    mute_samples: usize,
    fade_in_step: f32,
}

impl InputTransientSuppressor {
    const POP_DELTA_THRESHOLD: f32 = 0.45;
    const POP_LEVEL_THRESHOLD: f32 = 0.25;
    const POP_CLIP_THRESHOLD: f32 = 0.9;
    const MUTE_MS: f32 = 35.0;
    const FADE_IN_MS: f32 = 80.0;

    fn new(sample_rate: u32) -> Self {
        let fade_in_samples = ((sample_rate as f32 * Self::FADE_IN_MS) / 1000.0).max(1.0);
        Self {
            prev_input: [0.0; 2],
            gain: 1.0,
            mute_samples_remaining: 0,
            mute_samples: ((sample_rate as f32 * Self::MUTE_MS) / 1000.0).round() as usize,
            fade_in_step: 1.0 / fade_in_samples,
        }
    }

    fn process_frame(&mut self, input: [f32; 2]) -> [f32; 2] {
        if self.is_plug_transient(input) {
            self.mute_samples_remaining = self.mute_samples;
            self.gain = 0.0;
        }

        self.prev_input = input;

        if self.mute_samples_remaining > 0 {
            self.mute_samples_remaining -= 1;
            return [0.0; 2];
        }

        if self.gain < 1.0 {
            self.gain = (self.gain + self.fade_in_step).min(1.0);
        }

        [input[0] * self.gain, input[1] * self.gain]
    }

    fn is_plug_transient(&self, input: [f32; 2]) -> bool {
        input
            .iter()
            .zip(self.prev_input.iter())
            .any(|(&current, &previous)| {
                let level = current.abs();
                let delta = (current - previous).abs();
                level >= Self::POP_CLIP_THRESHOLD
                    || (level >= Self::POP_LEVEL_THRESHOLD && delta >= Self::POP_DELTA_THRESHOLD)
            })
    }
}

/// Audio processor that wraps the shared preamp state.
pub struct PreampProcessor {
    state: Arc<SharedState>,
    dc_blocker: DcBlocker,
    transient_suppressor: InputTransientSuppressor,
}

impl PreampProcessor {
    pub fn new(state: Arc<SharedState>) -> Self {
        Self {
            state,
            dc_blocker: DcBlocker::new(SAMPLE_RATE),
            transient_suppressor: InputTransientSuppressor::new(SAMPLE_RATE),
        }
    }
}

impl AudioProcessor for PreampProcessor {
    fn process_buffer(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]]) {
        // Check for pending IRs (separate locks, quick check)
        if let Ok(mut pending) = self.state.ch1_pending_ir.try_lock()
            && let Some(ir) = pending.ir.take()
            && let Ok(mut ch1) = self.state.ch1.try_lock()
        {
            ch1.set_ir(&ir);
        }
        if let Ok(mut pending) = self.state.ch2_pending_ir.try_lock()
            && let Some(ir) = pending.ir.take()
            && let Ok(mut ch2) = self.state.ch2.try_lock()
        {
            ch2.set_ir(&ir);
        }

        let mono = self.state.mono_mode.load(Ordering::Relaxed);

        // Lock channels for the entire buffer.
        // Use lock() not try_lock() - we'd rather wait briefly for the UI
        // than drop to passthrough and cause clicking.
        let mut ch1_guard = self.state.ch1.lock().unwrap();
        let mut ch2_guard = self.state.ch2.lock().unwrap();

        for (i, &input_frame) in input.iter().enumerate() {
            let dc_blocked_input = self.dc_blocker.process_frame(input_frame);
            let filtered_input = self.transient_suppressor.process_frame(dc_blocked_input);
            let out_l = ch1_guard.process(filtered_input[0]);
            let out_r = ch2_guard.process(filtered_input[1]);

            output[i] = if mono {
                let mixed = (out_l + out_r) / 2.0;
                [mixed, mixed]
            } else {
                [out_l, out_r]
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DcBlocker, InputTransientSuppressor};

    #[test]
    fn dc_blocker_removes_constant_offset() {
        let mut blocker = DcBlocker::new(48_000);
        let mut last = [0.0, 0.0];

        for _ in 0..4_096 {
            last = blocker.process_frame([0.25, 0.0]);
        }

        assert!(last[0].abs() < 0.001);
    }

    #[test]
    fn dc_blocker_preserves_ac_signal() {
        let mut blocker = DcBlocker::new(48_000);
        let mut peak = 0.0f32;

        for i in 0..512 {
            let input = if i % 2 == 0 { 0.1 } else { -0.1 };
            let output = blocker.process_frame([input, 0.0]);
            peak = peak.max(output[0].abs());
        }

        assert!(peak > 0.05);
    }

    #[test]
    fn suppresses_large_input_step() {
        let mut suppressor = InputTransientSuppressor::new(48_000);

        for _ in 0..16 {
            let frame = suppressor.process_frame([0.01, 0.0]);
            assert!(frame[0] > 0.0);
        }

        let muted = suppressor.process_frame([0.9, 0.0]);
        assert_eq!(muted, [0.0, 0.0]);
    }

    #[test]
    fn keeps_normal_signal_unchanged() {
        let mut suppressor = InputTransientSuppressor::new(48_000);

        let mut last = [0.0, 0.0];
        for sample in [0.01, 0.03, 0.06, 0.09, 0.12] {
            last = suppressor.process_frame([sample, 0.0]);
        }

        assert!(last[0] > 0.0);
        assert!((last[0] - 0.12).abs() < 1e-6);
    }
}
