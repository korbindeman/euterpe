//! Shared DSP utilities.

#[allow(unused_imports)]
use micromath::F32Ext;

// ---------------------------------------------------------------------------
// dB / Linear conversion
// ---------------------------------------------------------------------------

/// Convert dB to linear amplitude.
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

/// Convert linear amplitude to dB. Clamps to floor to avoid log(0).
#[inline]
pub fn linear_to_db(lin: f32) -> f32 {
    const FLOOR: f32 = 1e-12;
    20.0 * lin.max(FLOOR).log10()
}

// ---------------------------------------------------------------------------
// Time constants
// ---------------------------------------------------------------------------

/// Convert milliseconds to a one-pole filter coefficient.
/// Used for envelope followers, smoothing, etc.
#[inline]
pub fn time_constant(ms: f32, sample_rate: u32) -> f32 {
    (-1.0 / (ms * 0.001 * sample_rate as f32)).exp()
}

// ---------------------------------------------------------------------------
// Envelope follower
// ---------------------------------------------------------------------------

/// Simple peak envelope follower with separate attack/release.
pub struct EnvelopeFollower {
    pub envelope: f32,
    attack_coeff: f32,
    release_coeff: f32,
}

impl EnvelopeFollower {
    pub fn new(attack_ms: f32, release_ms: f32, sample_rate: u32) -> Self {
        Self {
            envelope: 0.0,
            attack_coeff: time_constant(attack_ms, sample_rate),
            release_coeff: time_constant(release_ms, sample_rate),
        }
    }

    pub fn set_attack(&mut self, ms: f32, sample_rate: u32) {
        self.attack_coeff = time_constant(ms, sample_rate);
    }

    pub fn set_release(&mut self, ms: f32, sample_rate: u32) {
        self.release_coeff = time_constant(ms, sample_rate);
    }

    /// Process a sample and return the updated envelope value.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let level = input.abs();
        let coeff = if level > self.envelope {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.envelope = coeff * self.envelope + (1.0 - coeff) * level;
        self.envelope
    }
}

// ---------------------------------------------------------------------------
// Waveshapers
// ---------------------------------------------------------------------------

/// Compute tanh(x) using exp.
#[inline]
pub fn tanhf(x: f32) -> f32 {
    let e2x = (2.0 * x).exp();
    (e2x - 1.0) / (e2x + 1.0)
}
