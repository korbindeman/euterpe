#![no_std]

pub use micromath::F32Ext;

pub mod params;
pub mod processors;
pub mod utils;

// Re-export processors at crate root for convenience
pub use processors::biquad;
pub use processors::cab_sim;
pub use processors::chorus;
pub use processors::comp_163;
pub use processors::comp_1176;
pub use processors::compressor;
pub use processors::eq;
pub use processors::gate;
pub use processors::limiter;
pub use processors::saturator;

// ---------------------------------------------------------------------------
// Process trait — every DSP block implements this
// ---------------------------------------------------------------------------

pub trait Process {
    fn process(&mut self, sample: f32) -> f32;
}
