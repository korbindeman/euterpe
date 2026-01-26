use crate::Process;
use crate::params::{Knob, Toggle};
use crate::utils::{db_to_linear, linear_to_db, time_constant};

/// Default compressor settings.
pub const DEFAULT_THRESHOLD_DB: f32 = -20.0;
pub const DEFAULT_RATIO: f32 = 4.0;
pub const DEFAULT_ATTACK_MS: f32 = 15.0;
pub const DEFAULT_RELEASE_MS: f32 = 100.0;

pub struct CompressorParams {
    pub enabled: Toggle,
    pub threshold: Knob,
    pub ratio: Knob,
    pub attack: Knob,
    pub release: Knob,
}

impl CompressorParams {
    pub fn new() -> Self {
        Self {
            enabled: Toggle::new("Enabled", true),
            threshold: Knob::new("Threshold", DEFAULT_THRESHOLD_DB, -60.0, 0.0),
            ratio: Knob::new("Ratio", DEFAULT_RATIO, 1.0, 20.0),
            attack: Knob::new("Attack", DEFAULT_ATTACK_MS, 0.1, 100.0),
            release: Knob::new("Release", DEFAULT_RELEASE_MS, 10.0, 1000.0),
        }
    }
}

impl Default for CompressorParams {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Compressor {
    params: CompressorParams,
    sample_rate: u32,
    // Envelope follower state
    envelope: f32,
    // Cached coefficients
    attack_coeff: f32,
    release_coeff: f32,
    threshold_linear: f32,
    knee_width: f32, // in dB, for soft knee
}

impl Compressor {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            params: CompressorParams::new(),
            sample_rate,
            envelope: 0.0,
            attack_coeff: time_constant(DEFAULT_ATTACK_MS, sample_rate),
            release_coeff: time_constant(DEFAULT_RELEASE_MS, sample_rate),
            threshold_linear: db_to_linear(DEFAULT_THRESHOLD_DB),
            knee_width: 6.0,
        }
    }

    pub fn params(&self) -> &CompressorParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut CompressorParams {
        &mut self.params
    }

    fn update_coefficients(&mut self) {
        if self.params.threshold.take_dirty() {
            self.threshold_linear = db_to_linear(self.params.threshold.get());
        }
        if self.params.attack.take_dirty() {
            self.attack_coeff = time_constant(self.params.attack.get(), self.sample_rate);
        }
        if self.params.release.take_dirty() {
            self.release_coeff = time_constant(self.params.release.get(), self.sample_rate);
        }
        // ratio is read directly, no caching needed
        let _ = self.params.ratio.take_dirty();
    }

    /// Compute the gain reduction (in dB) for a given input level (in dB),
    /// using soft-knee compression around the threshold.
    fn compute_gain_db(&self, level_db: f32) -> f32 {
        let threshold_db = linear_to_db(self.threshold_linear);
        let width = self.knee_width;
        let half_width = width / 2.0;
        let ratio = self.params.ratio.get();

        if level_db <= threshold_db - half_width {
            // Below the knee — no gain reduction
            0.0
        } else if level_db >= threshold_db + half_width {
            // Above the knee — full compression
            -((level_db - threshold_db) * (1.0 - 1.0 / ratio))
        } else {
            // Inside the soft knee region
            let x = level_db - threshold_db + half_width;
            -(((1.0 - 1.0 / ratio) * x * x) / (2.0 * width))
        }
    }
}

impl Process for Compressor {
    fn process(&mut self, sample: f32) -> f32 {
        self.update_coefficients();

        if !self.params.enabled.get() {
            return sample;
        }

        // 1. Peak detection — absolute value of input
        let input_level = sample.abs();

        // 2. Envelope follower (attack / release smoothing)
        if input_level > self.envelope {
            self.envelope =
                self.attack_coeff * self.envelope + (1.0 - self.attack_coeff) * input_level;
        } else {
            self.envelope =
                self.release_coeff * self.envelope + (1.0 - self.release_coeff) * input_level;
        }

        // 3. Compute gain reduction in dB domain
        let level_db = linear_to_db(self.envelope);
        let gain_db = self.compute_gain_db(level_db);

        // 4. Convert gain reduction to linear and apply
        let gain = db_to_linear(gain_db);
        sample * gain
    }
}
