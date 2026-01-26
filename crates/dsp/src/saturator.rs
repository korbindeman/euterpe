use crate::Process;
use crate::params::{Knob, Toggle};
use crate::utils::{db_to_linear, tanhf};

/// Default saturator drive in dB.
pub const DEFAULT_DRIVE_DB: f32 = 12.0;
/// Default sample rate for smoother initialization.
const DEFAULT_SAMPLE_RATE: u32 = 48000;

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
