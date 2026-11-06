//! 1176-style FET compressor emulation.
//!
//! Key characteristics:
//! - Feedback topology: envelope detector sees output signal
//! - Ultra-fast attack: 20µs to 800µs
//! - Release: 50ms to 1100ms
//! - Fixed ratio detents: 4:1, 8:1, 12:1, 20:1
//! - Hard knee compression
//! - Program-dependent timing: louder signals = faster response
//! - Input drive / output level model (no threshold knob)

use crate::Process;
use crate::params::{Knob, Toggle};
use crate::utils::{db_to_linear, linear_to_db, time_constant};

#[allow(unused_imports)]
use micromath::F32Ext;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default input gain in dB.
pub const DEFAULT_INPUT_DB: f32 = 0.0;
/// Default output gain in dB.
pub const DEFAULT_OUTPUT_DB: f32 = 0.0;
/// Default attack time in ms.
pub const DEFAULT_ATTACK_MS: f32 = 0.4;
/// Default release time in ms.
pub const DEFAULT_RELEASE_MS: f32 = 300.0;

// Attack range: 20µs to 800µs
const ATTACK_MIN_MS: f32 = 0.02;
const ATTACK_MAX_MS: f32 = 0.8;

// Release range: 50ms to 1100ms
const RELEASE_MIN_MS: f32 = 50.0;
const RELEASE_MAX_MS: f32 = 1100.0;

// Input/output gain range
const GAIN_MIN_DB: f32 = -20.0;
const GAIN_MAX_DB: f32 = 40.0;

// Internal threshold for compression (fixed, drive into it with input gain)
const THRESHOLD_DB: f32 = -20.0;

// ---------------------------------------------------------------------------
// Ratio enum
// ---------------------------------------------------------------------------

/// Fixed ratio settings for 1176-style compressor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ratio1176 {
    /// 4:1 ratio - gentle compression
    #[default]
    R4,
    /// 8:1 ratio - moderate compression
    R8,
    /// 12:1 ratio - heavy compression
    R12,
    /// 20:1 ratio - limiting
    R20,
}

impl Ratio1176 {
    /// Get the numeric ratio value.
    pub fn value(self) -> f32 {
        match self {
            Ratio1176::R4 => 4.0,
            Ratio1176::R8 => 8.0,
            Ratio1176::R12 => 12.0,
            Ratio1176::R20 => 20.0,
        }
    }

    /// Cycle to the next ratio setting.
    pub fn next(self) -> Self {
        match self {
            Ratio1176::R4 => Ratio1176::R8,
            Ratio1176::R8 => Ratio1176::R12,
            Ratio1176::R12 => Ratio1176::R20,
            Ratio1176::R20 => Ratio1176::R4,
        }
    }

    /// Get ratio from index (0-3).
    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Ratio1176::R4,
            1 => Ratio1176::R8,
            2 => Ratio1176::R12,
            _ => Ratio1176::R20,
        }
    }

    /// Get index (0-3) from ratio.
    pub fn to_index(self) -> usize {
        match self {
            Ratio1176::R4 => 0,
            Ratio1176::R8 => 1,
            Ratio1176::R12 => 2,
            Ratio1176::R20 => 3,
        }
    }
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

pub struct Comp1176Params {
    pub enabled: Toggle,
    /// Input gain (drive) in dB - controls how hard signal hits the compressor
    pub input: Knob,
    /// Output gain (makeup) in dB
    pub output: Knob,
    /// Attack time (0.02ms to 0.8ms)
    pub attack: Knob,
    /// Release time (50ms to 1100ms)
    pub release: Knob,
    /// Compression ratio
    ratio: Ratio1176,
    ratio_dirty: bool,
}

impl Comp1176Params {
    pub fn new() -> Self {
        Self {
            enabled: Toggle::new("Enabled", true),
            input: Knob::new("Input", DEFAULT_INPUT_DB, GAIN_MIN_DB, GAIN_MAX_DB),
            output: Knob::new("Output", DEFAULT_OUTPUT_DB, GAIN_MIN_DB, GAIN_MAX_DB),
            attack: Knob::new("Attack", DEFAULT_ATTACK_MS, ATTACK_MIN_MS, ATTACK_MAX_MS),
            release: Knob::new(
                "Release",
                DEFAULT_RELEASE_MS,
                RELEASE_MIN_MS,
                RELEASE_MAX_MS,
            ),
            ratio: Ratio1176::default(),
            ratio_dirty: true,
        }
    }

    pub fn ratio(&self) -> Ratio1176 {
        self.ratio
    }

    pub fn set_ratio(&mut self, ratio: Ratio1176) {
        if self.ratio != ratio {
            self.ratio = ratio;
            self.ratio_dirty = true;
        }
    }

    pub fn take_ratio_dirty(&mut self) -> bool {
        let was_dirty = self.ratio_dirty;
        self.ratio_dirty = false;
        was_dirty
    }
}

impl Default for Comp1176Params {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Compressor
// ---------------------------------------------------------------------------

pub struct Comp1176 {
    params: Comp1176Params,
    sample_rate: u32,

    // Envelope follower state (follows output due to feedback topology)
    envelope: f32,

    // Cached coefficients
    attack_coeff: f32,
    release_coeff: f32,
    input_gain: f32,
    output_gain: f32,
    ratio: f32,
}

impl Comp1176 {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            params: Comp1176Params::new(),
            sample_rate,
            envelope: 0.0,
            attack_coeff: time_constant(DEFAULT_ATTACK_MS, sample_rate),
            release_coeff: time_constant(DEFAULT_RELEASE_MS, sample_rate),
            input_gain: db_to_linear(DEFAULT_INPUT_DB),
            output_gain: db_to_linear(DEFAULT_OUTPUT_DB),
            ratio: Ratio1176::default().value(),
        }
    }

    pub fn params(&self) -> &Comp1176Params {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut Comp1176Params {
        &mut self.params
    }

    pub fn reset(&mut self) {
        self.envelope = 0.0;
    }

    fn update_coefficients(&mut self) {
        if self.params.input.take_dirty() {
            self.input_gain = db_to_linear(self.params.input.get());
        }
        if self.params.output.take_dirty() {
            self.output_gain = db_to_linear(self.params.output.get());
        }
        if self.params.attack.take_dirty() {
            self.attack_coeff = time_constant(self.params.attack.get(), self.sample_rate);
        }
        if self.params.release.take_dirty() {
            self.release_coeff = time_constant(self.params.release.get(), self.sample_rate);
        }
        if self.params.take_ratio_dirty() {
            self.ratio = self.params.ratio().value();
        }
    }

    /// Compute gain reduction in dB for a given level in dB.
    /// Uses hard knee at fixed threshold.
    #[inline]
    fn compute_gain_db(&self, level_db: f32) -> f32 {
        if level_db <= THRESHOLD_DB {
            // Below threshold - no compression
            0.0
        } else {
            // Hard knee compression
            let over_db = level_db - THRESHOLD_DB;
            let compressed_over = over_db / self.ratio;
            compressed_over - over_db
        }
    }

    /// Program-dependent timing: modulate attack/release based on signal level.
    /// Louder signals get faster response (more aggressive coefficients).
    #[inline]
    fn program_dependent_coeffs(&self, level_db: f32) -> (f32, f32) {
        // Calculate how far above threshold we are (0.0 to ~1.0 for typical signals)
        #[allow(
            clippy::manual_clamp,
            reason = "max/min maps NaN to 0; clamp would pass it through"
        )]
        let over_threshold = ((level_db - THRESHOLD_DB) / 20.0).max(0.0).min(1.0);

        // Speed up attack/release for louder signals
        // At max level: attack ~2x faster, release ~1.5x faster
        let attack_mod = 1.0 - over_threshold * 0.5;
        let release_mod = 1.0 - over_threshold * 0.3;

        // Modulate coefficients (smaller coeff = faster response)
        let attack = self.attack_coeff * attack_mod;
        let release = self.release_coeff * release_mod;

        (attack, release)
    }
}

impl Process for Comp1176 {
    fn process(&mut self, sample: f32) -> f32 {
        self.update_coefficients();

        if !self.params.enabled.get() {
            return sample;
        }

        // 1. Apply input gain (drive into compressor)
        let driven = sample * self.input_gain;

        // 2. Feedback topology: use envelope from previous output to compute gain
        //    This creates the characteristic 1176 response where the compressor
        //    "chases" the signal.
        let envelope_db = linear_to_db(self.envelope);
        let gain_db = self.compute_gain_db(envelope_db);
        let gain = db_to_linear(gain_db);

        // 3. Apply compression
        let compressed = driven * gain;

        // 4. Update envelope follower with compressed output (feedback topology)
        let output_level = compressed.abs();
        let (attack_coeff, release_coeff) = self.program_dependent_coeffs(envelope_db);

        if output_level > self.envelope {
            self.envelope = attack_coeff * self.envelope + (1.0 - attack_coeff) * output_level;
        } else {
            self.envelope = release_coeff * self.envelope + (1.0 - release_coeff) * output_level;
        }

        // 5. Apply output gain (makeup)
        compressed * self.output_gain
    }
}
