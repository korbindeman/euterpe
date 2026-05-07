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

// ---------------------------------------------------------------------------
// Delay line
// ---------------------------------------------------------------------------

/// A fixed-size circular delay buffer with linear interpolation.
///
/// The size N must be a power of 2 for efficient modulo operations.
pub struct DelayLine<const N: usize> {
    buffer: [f32; N],
    write_pos: usize,
    mask: usize,
}

impl<const N: usize> DelayLine<N> {
    /// Create a new delay line. N must be a power of 2.
    pub fn new() -> Self {
        debug_assert!(N.is_power_of_two(), "DelayLine size must be a power of 2");
        Self {
            buffer: [0.0; N],
            write_pos: 0,
            mask: N - 1,
        }
    }

    /// Write a sample to the delay line and advance the write position.
    #[inline]
    pub fn write(&mut self, sample: f32) {
        self.buffer[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) & self.mask;
    }

    /// Read from the delay line with linear interpolation.
    /// `delay_samples` is the number of samples to look back from the current write position.
    #[inline]
    pub fn read(&self, delay_samples: f32) -> f32 {
        let read_pos = self.write_pos as f32 - delay_samples;
        let read_pos = if read_pos < 0.0 {
            read_pos + N as f32
        } else {
            read_pos
        };

        let idx0 = read_pos as usize & self.mask;
        let idx1 = (idx0 + 1) & self.mask;
        let frac = read_pos - read_pos.floor();

        self.buffer[idx0] * (1.0 - frac) + self.buffer[idx1] * frac
    }

    /// Read from a fixed integer delay (no interpolation).
    #[inline]
    pub fn read_fixed(&self, delay_samples: usize) -> f32 {
        let idx = (self.write_pos.wrapping_sub(delay_samples)) & self.mask;
        self.buffer[idx]
    }

    /// Clear the delay buffer.
    pub fn clear(&mut self) {
        self.buffer = [0.0; N];
    }
}

impl<const N: usize> Default for DelayLine<N> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// LFO (Low Frequency Oscillator)
// ---------------------------------------------------------------------------

/// A simple sine-wave LFO with configurable rate.
pub struct Lfo {
    phase: f32,
    phase_inc: f32,
    sample_rate: f32,
}

impl Lfo {
    /// Create a new LFO with the given rate in Hz.
    pub fn new(rate_hz: f32, sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        Self {
            phase: 0.0,
            phase_inc: rate_hz / sr,
            sample_rate: sr,
        }
    }

    /// Set the LFO rate in Hz.
    #[inline]
    pub fn set_rate(&mut self, rate_hz: f32) {
        self.phase_inc = rate_hz / self.sample_rate;
    }

    /// Get the current LFO value (sine wave, range -1 to 1) and advance phase.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        let value = (self.phase * core::f32::consts::TAU).sin();
        self.phase += self.phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        value
    }

    /// Reset the LFO phase to zero.
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
}
