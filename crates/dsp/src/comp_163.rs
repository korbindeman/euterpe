//! dbx 163-style compressor emulation.
//!
//! Key characteristics:
//! - Single compression control (threshold and ratio linked)
//! - Soft-knee: ratio gradually increases from 1:1 to infinity:1 over ~30dB
//! - RMS level detection (not peak)
//! - Feed-forward topology: envelope detector sees input signal
//! - Fixed release rate: 120 dB/second
//! - Program-dependent attack: faster for transients

use crate::Process;
use crate::params::{Knob, Toggle};
use crate::utils::{db_to_linear, linear_to_db, time_constant};

#[allow(unused_imports)]
use micromath::F32Ext;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default compression amount (0.0 = no compression, 1.0 = full compression).
pub const DEFAULT_COMPRESSION: f32 = 0.5;
/// Default output gain in dB.
pub const DEFAULT_OUTPUT_DB: f32 = 0.0;

// Soft knee width in dB - the 163 has a very wide knee (~30-40dB)
const KNEE_WIDTH_DB: f32 = 30.0;

// Fixed release rate: 120 dB/second
// This is a rate, not a time constant - we'll compute the per-sample decrement
const RELEASE_RATE_DB_PER_SEC: f32 = 120.0;

// RMS averaging time constant (roughly 10-20ms for musical response)
const RMS_TIME_MS: f32 = 15.0;

// Minimum attack time for transients
const ATTACK_MIN_MS: f32 = 0.1;
// Maximum attack time for sustained signals
const ATTACK_MAX_MS: f32 = 20.0;

// Output gain range
const OUTPUT_MIN_DB: f32 = -20.0;
const OUTPUT_MAX_DB: f32 = 20.0;

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

pub struct Comp163Params {
    pub enabled: Toggle,
    /// Compression amount (0.0 to 1.0) - controls threshold and effective ratio
    pub compression: Knob,
    /// Output gain (makeup) in dB
    pub output: Knob,
}

impl Comp163Params {
    pub fn new() -> Self {
        Self {
            enabled: Toggle::new("Enabled", true),
            compression: Knob::new("Compression", DEFAULT_COMPRESSION, 0.0, 1.0),
            output: Knob::new("Output", DEFAULT_OUTPUT_DB, OUTPUT_MIN_DB, OUTPUT_MAX_DB),
        }
    }
}

impl Default for Comp163Params {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Compressor
// ---------------------------------------------------------------------------

pub struct Comp163 {
    params: Comp163Params,
    #[allow(dead_code)]
    sample_rate: u32,

    // RMS envelope state (squared values for RMS calculation)
    rms_squared: f32,
    rms_coeff: f32,

    // Gain reduction state (in dB, for fixed release rate)
    gain_reduction_db: f32,

    // Release rate per sample (in dB)
    release_rate_per_sample: f32,

    // Attack coefficients (min/max for program-dependent)
    attack_coeff_fast: f32,
    attack_coeff_slow: f32,

    // Cached output gain
    output_gain: f32,
}

impl Comp163 {
    pub fn new(sample_rate: u32) -> Self {
        let release_rate_per_sample = RELEASE_RATE_DB_PER_SEC / sample_rate as f32;

        Self {
            params: Comp163Params::new(),
            sample_rate,
            rms_squared: 0.0,
            rms_coeff: time_constant(RMS_TIME_MS, sample_rate),
            gain_reduction_db: 0.0,
            release_rate_per_sample,
            attack_coeff_fast: time_constant(ATTACK_MIN_MS, sample_rate),
            attack_coeff_slow: time_constant(ATTACK_MAX_MS, sample_rate),
            output_gain: db_to_linear(DEFAULT_OUTPUT_DB),
        }
    }

    pub fn params(&self) -> &Comp163Params {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut Comp163Params {
        &mut self.params
    }

    fn update_coefficients(&mut self) {
        // Compression knob doesn't need caching - read directly in process
        let _ = self.params.compression.take_dirty();

        if self.params.output.take_dirty() {
            self.output_gain = db_to_linear(self.params.output.get());
        }
    }

    /// Compute the threshold based on compression knob position.
    /// At 0.0: threshold is very high (no compression)
    /// At 1.0: threshold is very low (heavy compression)
    #[inline]
    fn get_threshold_db(&self, compression: f32) -> f32 {
        // Map compression 0..1 to threshold 0..-60 dB
        -compression * 60.0
    }

    /// Compute gain reduction in dB using soft-knee characteristic.
    /// The 163's soft knee gradually increases ratio from 1:1 to infinity:1.
    #[inline]
    fn compute_target_gain_reduction(&self, level_db: f32, compression: f32) -> f32 {
        let threshold_db = self.get_threshold_db(compression);
        let half_knee = KNEE_WIDTH_DB / 2.0;

        // Below the knee start - no compression
        if level_db <= threshold_db - half_knee {
            return 0.0;
        }

        // The 163's "Over Easy" curve: within the knee region, compression
        // gradually increases. Above the knee, it approaches infinity:1
        // (limiting behavior).

        let over_threshold = level_db - threshold_db;

        if over_threshold < half_knee {
            // In the soft knee region: quadratic curve
            // Ratio smoothly increases from 1:1 at knee start
            let x = over_threshold + half_knee; // 0 to KNEE_WIDTH_DB
            let normalized = x / KNEE_WIDTH_DB; // 0 to 1
            // Quadratic gain reduction
            normalized * normalized * over_threshold.max(0.0)
        } else {
            // Above the knee: approach infinity:1 (limiting)
            // Output level stays roughly constant at threshold + half_knee
            let knee_end_level = threshold_db + half_knee;
            level_db - knee_end_level
        }
    }

    /// Program-dependent attack: faster attack for transients (large level jumps).
    #[inline]
    fn get_attack_coeff(&self, level_db: f32, current_envelope_db: f32) -> f32 {
        let level_diff = (level_db - current_envelope_db).max(0.0);

        // Large transients (>12dB jump) get fast attack
        // Small changes get slow attack
        let transient_amount = (level_diff / 12.0).min(1.0);

        // Interpolate between slow and fast attack
        let slow = self.attack_coeff_slow;
        let fast = self.attack_coeff_fast;
        slow * (1.0 - transient_amount) + fast * transient_amount
    }
}

impl Process for Comp163 {
    fn process(&mut self, sample: f32) -> f32 {
        self.update_coefficients();

        if !self.params.enabled.get() {
            return sample;
        }

        let compression = self.params.compression.get();

        // No compression when knob is at 0
        if compression <= 0.0 {
            return sample * self.output_gain;
        }

        // 1. RMS level detection (feed-forward: detect input)
        let squared = sample * sample;
        self.rms_squared = self.rms_coeff * self.rms_squared + (1.0 - self.rms_coeff) * squared;
        let rms_level = self.rms_squared.sqrt();
        let level_db = linear_to_db(rms_level);

        // 2. Compute target gain reduction
        let target_gr_db = self.compute_target_gain_reduction(level_db, compression);

        // 3. Apply program-dependent attack / fixed release rate
        if target_gr_db > self.gain_reduction_db {
            // Attack: use program-dependent coefficient
            let current_envelope_db = linear_to_db(self.rms_squared.sqrt());
            let attack_coeff = self.get_attack_coeff(level_db, current_envelope_db);
            self.gain_reduction_db =
                attack_coeff * self.gain_reduction_db + (1.0 - attack_coeff) * target_gr_db;
        } else {
            // Release: fixed rate of 120 dB/second
            self.gain_reduction_db =
                (self.gain_reduction_db - self.release_rate_per_sample).max(target_gr_db);
        }

        // 4. Apply gain reduction
        let gain = db_to_linear(-self.gain_reduction_db);
        let compressed = sample * gain;

        // 5. Apply output gain
        compressed * self.output_gain
    }
}
