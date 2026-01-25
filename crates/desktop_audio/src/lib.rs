//! Generic desktop audio runtime using cpal.
//!
//! Provides an `AudioProcessor` trait and an `AudioEngine` that handles
//! device setup, ring buffering, and stream management.

mod engine;
mod ir_loader;

#[cfg(feature = "egui")]
mod ui;

pub use engine::{AudioEngine, AudioEngineConfig, list_input_devices, list_output_devices};
pub use ir_loader::{IrError, load_ir};

#[cfg(feature = "egui")]
pub use ui::{KnobUi, ToggleUi};

/// Trait for audio processors that can be used with the audio engine.
///
/// Implementors process buffers of stereo frames.
pub trait AudioProcessor: Send + 'static {
    /// Process a buffer of stereo input frames, writing results to output buffer.
    /// Input and output slices have the same length.
    fn process_buffer(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]]);
}
