//! Generic Daisy Seed runtime helpers.
//!
//! This crate provides re-exports and helper types for building
//! audio applications on the Daisy Seed hardware.

#![no_std]

// Re-export dependencies for convenience
pub use cortex_m;
pub use cortex_m_rt;
pub use daisy;
pub use panic_halt;
pub use stm32h7xx_hal as hal;

pub use daisy::audio;
pub use hal::pac;

/// Trait for audio processors that run on Daisy hardware.
///
/// Implementors process stereo frames (left, right) and return processed output.
pub trait AudioProcessor {
    /// Process a stereo input frame and return a stereo output frame.
    fn process(&mut self, input: (f32, f32)) -> (f32, f32);
}
