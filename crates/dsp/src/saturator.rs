#[allow(unused_imports)]
use micromath::F32Ext;

use crate::Process;
use crate::params::{Knob, Toggle};

/// Default saturator drive in dB.
pub const DEFAULT_DRIVE_DB: f32 = 12.0;
/// Default sample rate for smoother initialization.
const DEFAULT_SAMPLE_RATE: u32 = 48000;

/// Compute tanh(x) using micromath's exp.
/// tanh(x) = (e^(2x) - 1) / (e^(2x) + 1)
fn tanhf(x: f32) -> f32 {
    let e2x = (2.0 * x).exp();
    (e2x - 1.0) / (e2x + 1.0)
}

fn db_to_linear(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

pub struct SaturatorParams {
    pub enabled: Toggle,
    pub drive: Knob,
}

impl SaturatorParams {
    pub fn new(sample_rate: u32) -> Self {
        let mut drive = Knob::new("Drive", DEFAULT_DRIVE_DB, 0.0, 24.0);
        drive.init(sample_rate);
        Self {
            enabled: Toggle::new("Enabled", true),
            drive,
        }
    }
}

impl Default for SaturatorParams {
    fn default() -> Self {
        Self::new(DEFAULT_SAMPLE_RATE)
    }
}

pub struct Saturator {
    params: SaturatorParams,
    output_comp: f32,
}

impl Saturator {
    pub fn new() -> Self {
        Self::with_sample_rate(DEFAULT_SAMPLE_RATE)
    }

    pub fn with_sample_rate(sample_rate: u32) -> Self {
        let output_comp = 1.0 / tanhf(1.0);
        Self {
            params: SaturatorParams::new(sample_rate),
            output_comp,
        }
    }

    pub fn params(&self) -> &SaturatorParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut SaturatorParams {
        &mut self.params
    }
}

impl Default for Saturator {
    fn default() -> Self {
        Self::new()
    }
}

impl Process for Saturator {
    fn process(&mut self, sample: f32) -> f32 {
        if !self.params.enabled.get() {
            return sample;
        }

        let drive = db_to_linear(self.params.drive.get_smoothed());
        let driven = sample * drive;
        let clipped = tanhf(driven);
        clipped * self.output_comp
    }
}
