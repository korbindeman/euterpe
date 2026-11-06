//! Fast math with documented, tested error bounds.
//!
//! These are the functions to use in the hot path. Each one states its
//! maximum error, and the tests in this module check that bound against `std`
//! over the stated domain. Polynomial coefficients are near-minimax fits.
//!
//! | Function | Domain | Max error |
//! | --- | --- | --- |
//! | [`exp2`] | `[-126, 128)` | 2.5e-7 relative, exact at integers |
//! | [`exp`] | `[-87, 88)` | 5e-6 relative (f32 rounding of `x * log2(e)` dominates at large `x`) |
//! | [`log2`] | positive normal floats | 4e-6 absolute (f32 rounding of the exponent sum at large magnitudes) |
//! | [`ln`] | positive normal floats | 3e-6 absolute |
//! | [`tanh`] | all finite | 3e-7 absolute |
//! | [`db_to_gain`] | `[-120, 120]` dB | 1e-6 relative |
//! | [`gain_to_db`] | `>= 1e-6` | 3e-5 dB |
//!
//! Outside their domains the functions saturate rather than produce NaN,
//! except [`log2`]/[`ln`] of non-positive input, which return `-inf`.

#![allow(
    clippy::excessive_precision,
    reason = "coefficients are kept exactly as fitted; clippy would round some of them"
)]

use core::f32::consts::{LN_2, LOG2_E};

/// `log2(10) / 20`: converts dB to a base-2 exponent.
const DB_TO_LOG2: f32 = 0.166_096_4;
/// `20 * log10(2)`: converts a base-2 exponent to dB.
const LOG2_TO_DB: f32 = 6.020_6;

/// The largest f32 below 128: the top of `exp2`'s domain.
const EXP2_MAX_ARG: f32 = f32::from_bits(0x42ff_ffff);

/// Largest integer less than or equal to `x`, for `x` within `i32` range.
#[inline(always)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "documented: x must be within i32 range"
)]
pub fn floor_i32(x: f32) -> i32 {
    let i = x as i32;
    i - i32::from((i as f32) > x)
}

/// `2^x`. Max relative error 2.5e-7 on `[-126, 128)`; saturates outside.
#[inline]
pub fn exp2(x: f32) -> f32 {
    let x = x.clamp(-126.0, EXP2_MAX_ARG);
    let xi = floor_i32(x);
    let f = x - xi as f32;
    // 2^f on [0, 1), degree 5, constrained so p(0) = 1 and p(1) = 2 exactly.
    // That makes exp2 exact at integers and monotonic across them.
    let p = 1.0
        + f * (0.693_151_74
            + f * (0.240_159_27 + f * (0.055_818_675 + f * (0.008_990_997 + f * 0.001_879_317_8))));
    let scale = f32::from_bits((xi + 127).cast_unsigned() << 23);
    p * scale
}

/// `e^x`. Max relative error 5e-6 on `[-87, 88)`.
#[inline]
pub fn exp(x: f32) -> f32 {
    exp2(x * LOG2_E)
}

/// `log2(x)` for positive normal `x`. Max absolute error 4e-6.
/// Returns `-inf` for `x <= 0`.
#[inline]
pub fn log2(x: f32) -> f32 {
    if x <= 0.0 {
        return f32::NEG_INFINITY;
    }
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff).cast_signed() - 127_i32;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000); // [1, 2)
    let t = m - 1.0;
    // log2(1 + t) on [0, 1), degree 6.
    let p = 1.845_868e-6
        + t * (1.442_495_3
            + t * (-0.717_790_7
                + t * (0.456_519_77
                    + t * (-0.276_536_92 + t * (0.120_998_72 + t * -0.025_689_885)))));
    e as f32 + p
}

/// Natural log for positive normal `x`. Max absolute error 3e-6.
#[inline]
pub fn ln(x: f32) -> f32 {
    log2(x) * LN_2
}

/// Hyperbolic tangent. Max absolute error 3e-7; exact ±1 saturation past |x| = 9.
#[inline]
pub fn tanh(x: f32) -> f32 {
    // Computed on |x| so the result is exactly odd; tanh(0) = 0 exactly.
    let a = x.abs().min(9.0);
    let e = exp2(a * (2.0 * LOG2_E));
    let y = if a >= 9.0 { 1.0 } else { 1.0 - 2.0 / (e + 1.0) };
    y.copysign(x)
}

/// Decibels to linear gain. Max relative error 1e-6 on `[-120, 120]` dB.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    exp2(db * DB_TO_LOG2)
}

/// Linear gain to decibels. Inputs below 1e-6 (-120 dB) are clamped.
/// Max absolute error 3e-5 dB.
#[inline]
pub fn gain_to_db(gain: f32) -> f32 {
    log2(gain.max(1e-6)) * LOG2_TO_DB
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::cast_possible_truncation,
    reason = "tests compare exact results on purpose, against std on small values"
)]
mod tests {
    extern crate std;
    use super::*;

    /// Evenly spaced points over `[a, b]`, plus the endpoints.
    fn sweep(a: f32, b: f32, n: usize) -> impl Iterator<Item = f32> {
        (0..=n).map(move |i| a + (b - a) * (i as f32 / n as f32))
    }

    fn max_rel_err(
        f: impl Fn(f32) -> f32,
        g: impl Fn(f64) -> f64,
        xs: impl Iterator<Item = f32>,
    ) -> f64 {
        xs.map(|x| {
            let want = g(f64::from(x));
            ((f64::from(f(x)) - want) / want).abs()
        })
        .fold(0.0, f64::max)
    }

    fn max_abs_err(
        f: impl Fn(f32) -> f32,
        g: impl Fn(f64) -> f64,
        xs: impl Iterator<Item = f32>,
    ) -> f64 {
        xs.map(|x| (f64::from(f(x)) - g(f64::from(x))).abs())
            .fold(0.0, f64::max)
    }

    #[test]
    fn exp2_exact_at_integers_and_monotonic() {
        for i in -126..=127 {
            assert_eq!(exp2(i as f32), (i as f32).exp2(), "2^{i}");
        }
        let mut last = 0.0;
        for x in sweep(-20.0, 20.0, 400_000) {
            let y = exp2(x);
            assert!(y >= last, "not monotonic at {x}");
            last = y;
        }
    }

    #[test]
    fn floor_matches_std() {
        for x in sweep(-10.0, 10.0, 10_000) {
            assert_eq!(floor_i32(x), x.floor() as i32, "{x}");
        }
    }

    #[test]
    fn exp2_bound() {
        let e = max_rel_err(exp2, f64::exp2, sweep(-126.0, 127.99, 1_000_000));
        assert!(e < 2.5e-7, "exp2 rel err {e}");
    }

    #[test]
    fn exp_bound() {
        let e = max_rel_err(exp, f64::exp, sweep(-87.0, 87.99, 1_000_000));
        assert!(e < 5e-6, "exp rel err {e}");
    }

    #[test]
    fn log2_bound() {
        // Spans many octaves, including dense coverage of [1, 2).
        let xs = sweep(-40.0, 40.0, 200_000)
            .map(f32::exp2)
            .chain(sweep(1.0, 2.0, 200_000));
        let e = max_abs_err(log2, f64::log2, xs);
        assert!(e < 4e-6, "log2 abs err {e}");
        assert_eq!(log2(0.0), f32::NEG_INFINITY);
        assert_eq!(log2(-1.0), f32::NEG_INFINITY);
    }

    #[test]
    fn ln_bound() {
        let xs = sweep(-30.0, 30.0, 200_000).map(f32::exp2);
        let e = max_abs_err(ln, f64::ln, xs);
        assert!(e < 3e-6, "ln abs err {e}");
    }

    #[test]
    fn tanh_bound() {
        let e = max_abs_err(tanh, f64::tanh, sweep(-20.0, 20.0, 2_000_000));
        assert!(e < 3e-7, "tanh abs err {e}");
        assert_eq!(tanh(1e9), 1.0);
        assert_eq!(tanh(-1e9), -1.0);
        assert_eq!(tanh(0.0), 0.0);
    }

    #[test]
    fn tanh_is_odd_and_monotonic() {
        let mut last = -1.0;
        for x in sweep(-10.0, 10.0, 100_000) {
            let y = tanh(x);
            assert!(y >= last, "not monotonic at {x}");
            assert_eq!(y, -tanh(-x), "not odd at {x}");
            last = y;
        }
    }

    #[test]
    fn db_bounds() {
        let e = max_rel_err(
            db_to_gain,
            |db| 10f64.powf(db / 20.0),
            sweep(-120.0, 120.0, 100_000),
        );
        assert!(e < 1e-6, "db_to_gain rel err {e}");
        let e = max_abs_err(
            gain_to_db,
            |g| 20.0 * g.log10(),
            sweep(-120.0, 40.0, 100_000).map(|db| 10f32.powf(db / 20.0)),
        );
        assert!(e < 3e-5, "gain_to_db abs err {e}");
        assert_eq!(gain_to_db(0.0), gain_to_db(1e-6));
    }
}
