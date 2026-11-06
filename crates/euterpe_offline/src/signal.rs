//! Deterministic test signals.

use std::f64::consts::TAU;

/// A unit impulse at sample 0, followed by silence.
pub fn impulse(len: usize, amplitude: f32) -> Vec<f32> {
    let mut v = vec![0.0; len];
    if let Some(first) = v.first_mut() {
        *first = amplitude;
    }
    v
}

/// A sine at `freq` Hz.
pub fn sine(freq: f64, amplitude: f32, len: usize, sample_rate: u32) -> Vec<f32> {
    let w = TAU * freq / sample_rate as f64;
    (0..len)
        .map(|i| (w * i as f64).sin() as f32 * amplitude)
        .collect()
}

/// An exponential (log) sine sweep from `f0` to `f1` Hz, as used for
/// impulse-response and harmonic-distortion measurement (Farina's method).
pub fn log_sweep(f0: f64, f1: f64, amplitude: f32, len: usize, sample_rate: u32) -> Vec<f32> {
    let t_total = len as f64 / sample_rate as f64;
    let k = (f1 / f0).ln();
    let l = t_total / k;
    (0..len)
        .map(|i| {
            let t = i as f64 / sample_rate as f64;
            let phase = TAU * f0 * l * ((t / l).exp() - 1.0);
            phase.sin() as f32 * amplitude
        })
        .collect()
}

/// Uniform white noise in `-amplitude..amplitude` from a fixed seed.
pub fn noise(amplitude: f32, len: usize, seed: u64) -> Vec<f32> {
    // xorshift64*: fast, deterministic, good enough for test signals.
    let mut state = seed.max(1);
    (0..len)
        .map(|_| {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            let r = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
            let unit = (r >> 40) as f32 / (1u64 << 24) as f32; // [0, 1)
            (unit * 2.0 - 1.0) * amplitude
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_has_expected_period() {
        let s = sine(1000.0, 1.0, 48, 48_000);
        assert!(s[0].abs() < 1e-6);
        assert!((s[12] - 1.0).abs() < 1e-6); // quarter period
    }

    #[test]
    fn sweep_starts_at_zero_and_stays_bounded() {
        let s = log_sweep(20.0, 20_000.0, 0.5, 48_000, 48_000);
        assert_eq!(s[0], 0.0);
        assert!(s.iter().all(|x| x.abs() <= 0.5));
    }

    #[test]
    fn noise_is_deterministic_and_bounded() {
        let a = noise(0.8, 10_000, 7);
        assert_eq!(a, noise(0.8, 10_000, 7));
        assert_ne!(a, noise(0.8, 10_000, 8));
        assert!(a.iter().all(|x| x.abs() <= 0.8));
        let mean: f32 = a.iter().sum::<f32>() / a.len() as f32;
        assert!(mean.abs() < 0.02);
    }
}
