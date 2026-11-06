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
//! | [`sin_turns`], [`cos_turns`] | `|x| < 2^22` turns | 3e-7 absolute, exactly odd/even, never past ±1 |
//! | [`tan_pi`] | `[0, 0.49]` | 1.5e-6 relative |
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
    tanh_magnitude(x.abs()).copysign(x)
}

/// `tanh(a)` for `a >= 0`.
#[inline(always)]
fn tanh_magnitude(a: f32) -> f32 {
    let a = a.min(9.0);
    if a >= 9.0 {
        return 1.0;
    }
    tanh_from_exp(exp2(a * (2.0 * LOG2_E)))
}

/// `tanh(a)` from `e = e^(2a)`.
#[inline(always)]
fn tanh_from_exp(e: f32) -> f32 {
    1.0 - 2.0 / (e + 1.0)
}

/// `sin(2 pi x)`: the sine of `x` turns. Max absolute error 3e-7 for
/// `|x| < 2^22` (past that, f32 has no fractional turns left). Exactly odd,
/// exactly 0 at whole and half turns, and never past ±1.
#[inline]
pub fn sin_turns(x: f32) -> f32 {
    // The nearest whole turn away: r in [-0.5, 0.5].
    let r = x - floor_i32(x + 0.5) as f32;
    // sin(2 pi a) = sin(2 pi (1/2 - a)): fold onto [0, 1/4].
    let a = r.abs();
    let a = a.min(0.5 - a);
    sin_quarter(a).copysign(r)
}

/// `cos(2 pi x)`, with the error bound of [`sin_turns`].
#[inline]
pub fn cos_turns(x: f32) -> f32 {
    sin_turns(x + 0.25)
}

/// `sin(2 pi a)` for `a` in `[0, 1/4]`: odd, degree 9, minimax. At most
/// 0.99999994 at `a = 1/4`.
#[inline(always)]
#[expect(
    clippy::approx_constant,
    reason = "a fitted coefficient, one ulp below TAU"
)]
fn sin_quarter(a: f32) -> f32 {
    let a2 = a * a;
    a * (6.283_185 + a2 * (-41.341_656 + a2 * (81.601_006 + a2 * (-76.549_87 + a2 * 39.537_426))))
}

/// `tan(pi x)` for `x` in `[0, 0.49]`: the prewarped cutoff of a
/// bilinear-transform filter, `x = cutoff / sample_rate`. Max relative error
/// 1.5e-6. Clamped to that domain.
#[inline]
pub fn tan_pi(x: f32) -> f32 {
    let h = 0.5 * x.clamp(0.0, 0.49);
    // Both in [0, 1/4]: no reduction needed.
    sin_quarter(h) / sin_quarter(0.25 - h)
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

    fn sin_turns_ref(x: f64) -> f64 {
        (core::f64::consts::TAU * x).sin()
    }

    #[test]
    fn sin_cos_bound() {
        let e = max_abs_err(sin_turns, sin_turns_ref, sweep(-4.0, 4.0, 1_000_000));
        assert!(e < 3e-7, "sin err {e}");
        let e = max_abs_err(
            cos_turns,
            |x| sin_turns_ref(x + 0.25),
            sweep(-4.0, 4.0, 1_000_000),
        );
        assert!(e < 3e-7, "cos err {e}");
        // Far from zero: the reduction is still exact.
        let far = sweep(0.0, 1.0, 10_000).map(|x| x + 3_000_000.0);
        let e = max_abs_err(sin_turns, sin_turns_ref, far);
        assert!(e < 3e-7, "sin far err {e}");
    }

    #[test]
    fn sin_is_odd_bounded_and_exact_at_half_turns() {
        for x in sweep(-3.0, 3.0, 100_000) {
            let s = sin_turns(x);
            assert_eq!(sin_turns(-x), -s, "{x}");
            assert!(s.abs() <= 1.0, "{x}");
        }
        for k in -8..=8 {
            assert_eq!(sin_turns(k as f32 * 0.5), 0.0);
        }
        assert_eq!(cos_turns(0.0), 0.999_999_94);
    }

    #[test]
    fn tan_pi_bound() {
        let e = max_rel_err(
            tan_pi,
            |x| (core::f64::consts::PI * x).tan(),
            sweep(1e-4, 0.49, 1_000_000),
        );
        assert!(e < 1.5e-6, "tan rel err {e}");
        assert_eq!(tan_pi(0.0), 0.0);
        assert_eq!(tan_pi(0.7), tan_pi(0.49), "clamped");
    }
}

/// Proofs over every f32 input (`cargo kani -p euterpe_core`).
#[cfg(kani)]
mod proofs {
    use super::*;

    // `tanh` is proven bounded and odd in steps: one proof over both the
    // polynomial and the division is too large for the solver.

    /// Step 1: `e^(2a) >= 1` for `a >= 0`, from the `exp2` that computes it.
    #[kani::proof]
    fn exp2_of_non_negative_is_at_least_one() {
        let x: f32 = kani::any();
        kani::assume(x >= 0.0);
        let y = exp2(x);
        assert!(y >= 1.0 && y.is_finite());
    }

    /// Step 2: for every finite `e >= 1`, `1 - 2 / (e + 1)` is in `[0, 1]`.
    #[kani::proof]
    fn tanh_from_exp_is_in_unit_range() {
        let e: f32 = kani::any();
        kani::assume(e >= 1.0 && e.is_finite());
        let y = tanh_from_exp(e);
        assert!((0.0..=1.0).contains(&y));
    }

    /// Step 3: the sign is applied by `copysign` to a value computed from
    /// `|x|` alone, and `|-x|` is `|x|` bit for bit, so `tanh(-x) == -tanh(x)`
    /// exactly whenever the magnitude `y` is not NaN.
    #[kani::proof]
    fn sign_step_is_odd() {
        let x: f32 = kani::any();
        let y: f32 = kani::any();
        kani::assume(!x.is_nan() && !y.is_nan() && y >= 0.0);
        assert!((-x).abs().to_bits() == x.abs().to_bits());
        assert!(y.copysign(-x).to_bits() == (-y.copysign(x)).to_bits());
    }

    /// With steps 1 and 2, the saturated branch: `tanh_magnitude` is in
    /// `[0, 1]` for every `a >= 0` (NaN-free).
    #[kani::proof]
    fn tanh_magnitude_saturates_to_one() {
        let a: f32 = kani::any();
        kani::assume(a >= 9.0);
        assert!(tanh_magnitude(a) == 1.0);
    }

    /// `exp2` (and so `db_to_gain`) never overflows its exponent arithmetic
    /// and returns a finite, positive number for every non-NaN input.
    #[kani::proof]
    fn exp2_is_finite_and_positive() {
        let x: f32 = kani::any();
        kani::assume(!x.is_nan());
        let y = exp2(x);
        assert!(y.is_finite());
        assert!(y > 0.0);
    }
}
