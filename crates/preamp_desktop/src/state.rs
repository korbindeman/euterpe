use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use desktop_audio::AudioProcessor;
use preamp::channel::Channel;
use preamp::dsp::Process;
use preamp::{IR_MAX_LENGTH, SAMPLE_RATE};

/// Pending IR to be loaded into a channel.
/// The UI thread sets this, and the audio thread takes it.
pub struct PendingIr {
    pub ir: Option<Vec<f32>>,
}

impl Default for PendingIr {
    fn default() -> Self {
        Self { ir: None }
    }
}

/// Shared state between UI and audio threads.
pub struct SharedState {
    pub ch1: Mutex<Channel<IR_MAX_LENGTH>>,
    pub ch2: Mutex<Channel<IR_MAX_LENGTH>>,
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

/// Audio processor that wraps the shared preamp state.
pub struct PreampProcessor {
    state: Arc<SharedState>,
}

impl PreampProcessor {
    pub fn new(state: Arc<SharedState>) -> Self {
        Self { state }
    }
}

impl AudioProcessor for PreampProcessor {
    fn process_buffer(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]]) {
        // Check for pending IRs (separate locks, quick check)
        if let Ok(mut pending) = self.state.ch1_pending_ir.try_lock() {
            if let Some(ir) = pending.ir.take() {
                if let Ok(mut ch1) = self.state.ch1.try_lock() {
                    ch1.set_ir(&ir);
                }
            }
        }
        if let Ok(mut pending) = self.state.ch2_pending_ir.try_lock() {
            if let Some(ir) = pending.ir.take() {
                if let Ok(mut ch2) = self.state.ch2.try_lock() {
                    ch2.set_ir(&ir);
                }
            }
        }

        let mono = self.state.mono_mode.load(Ordering::Relaxed);

        // Lock channels for the entire buffer.
        // Use lock() not try_lock() - we'd rather wait briefly for the UI
        // than drop to passthrough and cause clicking.
        let mut ch1_guard = self.state.ch1.lock().unwrap();
        let mut ch2_guard = self.state.ch2.lock().unwrap();

        for (i, &input_frame) in input.iter().enumerate() {
            let out_l = ch1_guard.process(input_frame[0]);
            let out_r = ch2_guard.process(input_frame[1]);

            output[i] = if mono {
                let mixed = (out_l + out_r) / 2.0;
                [mixed, mixed]
            } else {
                [out_l, out_r]
            };
        }
    }
}
