use crate::Process;
use crate::params::{Knob, Toggle};
use crate::utils::{db_to_linear, time_constant};

/// Default limiter settings.
pub const DEFAULT_THRESHOLD_DB: f32 = -1.0;
pub const DEFAULT_RELEASE_MS: f32 = 100.0;
/// Fixed fast attack for brick-wall limiting.
const ATTACK_MS: f32 = 0.5;

pub struct LimiterParams {
    pub enabled: Toggle,
    pub threshold: Knob,
    pub release: Knob,
}

impl LimiterParams {
    pub fn new(sample_rate: u32) -> Self {
        let mut threshold = Knob::new("Threshold", DEFAULT_THRESHOLD_DB, -20.0, 0.0);
        let mut release = Knob::new("Release", DEFAULT_RELEASE_MS, 10.0, 500.0);
        threshold.init(sample_rate);
        release.init(sample_rate);
        Self {
            enabled: Toggle::new("Enabled", true),
            threshold,
            release,
        }
    }
}

impl Default for LimiterParams {
    fn default() -> Self {
        Self::new(48000)
    }
}

pub struct Limiter {
    params: LimiterParams,
    sample_rate: u32,
    /// Envelope follower state
    envelope: f32,
    /// Cached coefficients
    attack_coeff: f32,
    release_coeff: f32,
    threshold_linear: f32,
    /// Smoothed gain to avoid zipper noise
    gain: f32,
    gain_smooth_coeff: f32,
}

impl Limiter {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            params: LimiterParams::new(sample_rate),
            sample_rate,
            envelope: 0.0,
            attack_coeff: time_constant(ATTACK_MS, sample_rate),
            release_coeff: time_constant(DEFAULT_RELEASE_MS, sample_rate),
            threshold_linear: db_to_linear(DEFAULT_THRESHOLD_DB),
            gain: 1.0,
            gain_smooth_coeff: time_constant(2.0, sample_rate),
        }
    }

    pub fn params(&self) -> &LimiterParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut LimiterParams {
        &mut self.params
    }

    pub fn reset(&mut self) {
        self.envelope = 0.0;
        self.gain = 1.0;
    }

    fn update_coefficients(&mut self) {
        if self.params.threshold.take_dirty() {
            self.threshold_linear = db_to_linear(self.params.threshold.get());
        }
        if self.params.release.take_dirty() {
            self.release_coeff = time_constant(self.params.release.get(), self.sample_rate);
        }
    }
}

impl Process for Limiter {
    #[inline]
    fn process(&mut self, sample: f32) -> f32 {
        self.update_coefficients();

        if !self.params.enabled.get() {
            return sample;
        }

        // Peak detection
        let input_level = sample.abs();

        // Envelope follower (fast attack, variable release)
        if input_level > self.envelope {
            self.envelope =
                self.attack_coeff * self.envelope + (1.0 - self.attack_coeff) * input_level;
        } else {
            self.envelope =
                self.release_coeff * self.envelope + (1.0 - self.release_coeff) * input_level;
        }

        // Compute target gain (brick-wall: infinite ratio)
        let target_gain = if self.envelope > self.threshold_linear {
            self.threshold_linear / self.envelope
        } else {
            1.0
        };

        // Smooth gain changes
        self.gain =
            self.gain_smooth_coeff * self.gain + (1.0 - self.gain_smooth_coeff) * target_gain;

        sample * self.gain
    }
}
