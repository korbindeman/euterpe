#![no_std]

pub use micromath::F32Ext;

pub mod biquad;
pub mod cab_sim;
pub mod comp_1176;
pub mod comp_163;
pub mod compressor;
pub mod eq;
pub mod gate;
pub mod limiter;
pub mod params;
pub mod saturator;
pub mod utils;

// ---------------------------------------------------------------------------
// Process trait — every DSP block implements this
// ---------------------------------------------------------------------------

pub trait Process {
    fn process(&mut self, sample: f32) -> f32;
}
