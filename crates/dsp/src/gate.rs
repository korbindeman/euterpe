#[allow(unused_imports)]
use micromath::F32Ext;

use crate::Process;
use crate::params::{Knob, Toggle};

/// Default noise gate settings.
pub const DEFAULT_THRESHOLD_DB: f32 = -70.0;
pub const DEFAULT_ATTACK_MS: f32 = 1.0;
pub const DEFAULT_RELEASE_MS: f32 = 50.0;
/// Hysteresis in dB (gate opens at threshold, closes at threshold - hysteresis)
pub const DEFAULT_HYSTERESIS_DB: f32 = 3.0;

pub struct NoiseGateParams {
    pub enabled: Toggle,
    pub threshold: Knob,
}

impl NoiseGateParams {
    pub fn new() -> Self {
        Self {
            enabled: Toggle::new("Enabled", true),
            threshold: Knob::new("Threshold", DEFAULT_THRESHOLD_DB, -80.0, 0.0),
        }
    }
}

impl Default for NoiseGateParams {
    fn default() -> Self {
        Self::new()
    }
}

pub struct NoiseGate {
    params: NoiseGateParams,
    #[allow(dead_code)]
    sample_rate: u32,
    /// Envelope follower for input level detection
    envelope: f32,
    attack_coeff: f32,
    release_coeff: f32,
    /// Smoothed gain (0.0 to 1.0) applied to output
    gain: f32,
    /// Gain smoothing coefficient (for smooth open/close)
    gain_attack_coeff: f32,
    gain_release_coeff: f32,
    // Cached from params
    threshold_linear: f32,
    hysteresis_linear: f32,
    /// Gate state (true = open, false = closed)
    is_open: bool,
}

fn time_constant(ms: f32, sample_rate: u32) -> f32 {
    (-1.0 / (ms * 0.001 * sample_rate as f32)).exp()
}

fn db_to_linear(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

impl NoiseGate {
    pub fn new(sample_rate: u32) -> Self {
        let threshold_linear = db_to_linear(DEFAULT_THRESHOLD_DB);
        let hysteresis_linear = db_to_linear(DEFAULT_THRESHOLD_DB - DEFAULT_HYSTERESIS_DB);

        Self {
            params: NoiseGateParams::new(),
            sample_rate,
            envelope: 0.0,
            attack_coeff: time_constant(DEFAULT_ATTACK_MS, sample_rate),
            release_coeff: time_constant(DEFAULT_RELEASE_MS, sample_rate),
            gain: 1.0,
            // Smooth gain transitions: ~5ms attack, ~20ms release
            gain_attack_coeff: time_constant(5.0, sample_rate),
            gain_release_coeff: time_constant(20.0, sample_rate),
            threshold_linear,
            hysteresis_linear,
            is_open: true,
        }
    }

    pub fn params(&self) -> &NoiseGateParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut NoiseGateParams {
        &mut self.params
    }

    fn update_coefficients(&mut self) {
        if self.params.threshold.take_dirty() {
            let threshold_db = self.params.threshold.get();
            self.threshold_linear = db_to_linear(threshold_db);
            self.hysteresis_linear = db_to_linear(threshold_db - DEFAULT_HYSTERESIS_DB);
        }
    }
}

impl Process for NoiseGate {
    #[inline]
    fn process(&mut self, sample: f32) -> f32 {
        self.update_coefficients();

        if !self.params.enabled.get() {
            return sample;
        }

        let input_level = sample.abs();

        // Envelope follower for level detection
        if input_level > self.envelope {
            self.envelope =
                self.attack_coeff * self.envelope + (1.0 - self.attack_coeff) * input_level;
        } else {
            self.envelope =
                self.release_coeff * self.envelope + (1.0 - self.release_coeff) * input_level;
        }

        // Hysteresis: open at threshold, close at threshold - hysteresis
        let target_gain = if self.is_open {
            if self.envelope < self.hysteresis_linear {
                self.is_open = false;
                0.0
            } else {
                1.0
            }
        } else {
            if self.envelope > self.threshold_linear {
                self.is_open = true;
                1.0
            } else {
                0.0
            }
        };

        // Smooth gain transitions
        if target_gain > self.gain {
            // Opening - use attack (fast)
            self.gain =
                self.gain_attack_coeff * self.gain + (1.0 - self.gain_attack_coeff) * target_gain;
        } else {
            // Closing - use release (slower)
            self.gain =
                self.gain_release_coeff * self.gain + (1.0 - self.gain_release_coeff) * target_gain;
        }

        sample * self.gain
    }
}
