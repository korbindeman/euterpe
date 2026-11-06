//! Small recurrent networks for black-box amp and pedal models.
//!
//! [`Gru`] is a single-layer GRU with one input, `H` hidden units and a
//! linear output, the architecture of the `GuitarML` captures and of Wright et
//! al. (`DAFx` 2019): `y = x + w_out · h + b_out` when the model is trained
//! with a skip connection (`SKIP`), else `y = w_out · h + b_out`.
//!
//! The gate equations are those of `PyTorch`, so trained weights load unchanged:
//!
//! ```text
//! r = σ(w_ir x + b_ir + W_hr h + b_hr)
//! z = σ(w_iz x + b_iz + W_hz h + b_hz)
//! n = tanh(w_in x + b_in + r ⊙ (W_hn h + b_hn))
//! h = (1 − z) ⊙ n + z ⊙ h
//! ```
//!
//! Weights live in a `static` named by a [`GruModel`], never in a field, so
//! a zeroed `Gru` is valid (see `Device::GRAPH_INIT`). Each unit's weights
//! are contiguous ([`GruUnit`]): a step streams through them once.

use core::marker::PhantomData;

/// One hidden unit's weights, kept together so a step reads them as one
/// contiguous stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GruUnit<const H: usize> {
    /// `w_h[j]` = the r, z and n weights from `h[j]` into this unit.
    pub w_h: [[f32; 3]; H],
    /// The r, z and n input weights.
    pub w_i: [f32; 3],
    /// `b_ir + b_hr` (they are only ever added together).
    pub b_r: f32,
    /// `b_iz + b_hz`.
    pub b_z: f32,
    /// `b_in`, outside the reset product.
    pub b_in: f32,
    /// `b_hn`, inside the reset product.
    pub b_hn: f32,
    /// This unit's output weight.
    pub w_out: f32,
}

/// A GRU's trained weights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GruWeights<const H: usize> {
    pub units: [GruUnit<H>; H],
    /// Output bias.
    pub b_out: f32,
}

impl<const H: usize> GruWeights<H> {
    /// Deterministic pseudo-random weights in `[-scale, scale]`, for tests
    /// and benches: a GRU's cost does not depend on its weights.
    #[must_use]
    pub const fn test_pattern(seed: u32, scale: f32) -> Self {
        let mut s = seed;
        macro_rules! next {
            () => {{
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                #[expect(clippy::cast_precision_loss, reason = "24-bit value, exact in f32")]
                let v = ((s >> 8_u32) as f32 / 8_388_608.0 - 1.0) * scale;
                v
            }};
        }
        let unit = GruUnit {
            w_h: [[0.0; 3]; H],
            w_i: [0.0; 3],
            b_r: 0.0,
            b_z: 0.0,
            b_in: 0.0,
            b_hn: 0.0,
            w_out: 0.0,
        };
        let mut w = Self {
            units: [unit; H],
            b_out: 0.0,
        };
        let mut i = 0;
        while i < H {
            let u = &mut w.units[i];
            let mut j = 0;
            while j < H {
                u.w_h[j] = [next!(), next!(), next!()];
                j += 1;
            }
            u.w_i = [next!(), next!(), next!()];
            u.b_r = next!();
            u.b_z = next!();
            u.b_in = next!();
            u.b_hn = next!();
            u.w_out = next!();
            i += 1;
        }
        w.b_out = next!();
        w
    }
}

/// A trained GRU: the weights and whether the output adds the input back.
pub trait GruModel<const H: usize> {
    const WEIGHTS: &'static GruWeights<H>;
    /// The model was trained with a skip connection: `y = x + net(x)`.
    const SKIP: bool;
}

/// `tanh(x)` as a 13/6 rational function, without `exp2`: about half the
/// instructions of `fastmath::tanh` at a similar error (tested below). The
/// coefficients are Eigen's `generic_fast_tanh_float`.
#[inline(always)]
#[expect(clippy::excessive_precision, reason = "coefficients kept as published")]
pub fn tanh_rational(x: f32) -> f32 {
    const CLAMP: f32 = 7.905_311;
    // Constant bounds: no run-time check, so no panic path.
    let x = x.clamp(-CLAMP, CLAMP);
    let x2 = x * x;
    let mut p = -2.760_768_5e-16_f32;
    p = p * x2 + 2.000_187_9e-13;
    p = p * x2 + -8.604_671_5e-11;
    p = p * x2 + 5.122_297e-8;
    p = p * x2 + 1.485_722_4e-5;
    p = p * x2 + 6.372_619_3e-4;
    p = p * x2 + 4.893_524_6e-3;
    let q = ((1.198_258_4e-6_f32 * x2 + 1.185_347e-4) * x2 + 2.268_434_6e-3) * x2 + 4.893_525e-3;
    x * p / q
}

/// `1 / (1 + e^-x)`.
#[inline(always)]
fn sigmoid(x: f32) -> f32 {
    0.5_f32 * tanh_rational(0.5 * x) + 0.5
}

/// A single-layer GRU run once per sample.
pub struct Gru<M: GruModel<H>, const H: usize> {
    h: [f32; H],
    _model: PhantomData<M>,
}

impl<M: GruModel<H>, const H: usize> Default for Gru<M, H> {
    fn default() -> Self {
        Self {
            h: [0.0; H],
            _model: PhantomData,
        }
    }
}

impl<M: GruModel<H>, const H: usize> Gru<M, H> {
    /// Clear the hidden state.
    pub fn reset(&mut self) {
        self.h = [0.0; H];
    }

    /// One sample in, one out.
    #[inline(always)]
    pub fn tick(&mut self, x: f32) -> f32 {
        let h = &self.h;
        let mut next = [0.0_f32; H];
        let mut y = M::WEIGHTS.b_out;
        for ((u, out), &prev) in M::WEIGHTS.units.iter().zip(&mut next).zip(h) {
            // Three independent sums, so the multiply-adds overlap.
            let mut r = u.w_i[0] * x + u.b_r;
            let mut z = u.w_i[1] * x + u.b_z;
            let mut n = u.b_hn;
            for (w, &hj) in u.w_h.iter().zip(h) {
                r += w[0] * hj;
                z += w[1] * hj;
                n += w[2] * hj;
            }
            let r = sigmoid(r);
            let z = sigmoid(z);
            let n = tanh_rational(r * n + u.w_i[2] * x + u.b_in);
            let hi = n + z * (prev - n);
            *out = hi;
            y += u.w_out * hi;
        }
        self.h = next;
        if M::SKIP { y + x } else { y }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact values are deliberate here")]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    /// The GRU equations in f64, written out as `PyTorch` defines them.
    fn reference<const H: usize>(w: &GruWeights<H>, skip: bool, input: &[f32]) -> Vec<f64> {
        let sig = |v: f64| 1.0 / (1.0 + (-v).exp());
        let mut h = [0.0_f64; H];
        input
            .iter()
            .map(|&x| {
                let x = f64::from(x);
                let mut next = [0.0_f64; H];
                for (i, u) in w.units.iter().enumerate() {
                    let dot =
                        |g: usize| -> f64 { (0..H).map(|j| f64::from(u.w_h[j][g]) * h[j]).sum() };
                    let r = sig(f64::from(u.w_i[0]) * x + f64::from(u.b_r) + dot(0));
                    let z = sig(f64::from(u.w_i[1]) * x + f64::from(u.b_z) + dot(1));
                    let n = (f64::from(u.w_i[2]) * x
                        + f64::from(u.b_in)
                        + r * (dot(2) + f64::from(u.b_hn)))
                    .tanh();
                    next[i] = (1.0 - z) * n + z * h[i];
                }
                h = next;
                let y: f64 = (0..H)
                    .map(|i| f64::from(w.units[i].w_out) * h[i])
                    .sum::<f64>()
                    + f64::from(w.b_out);
                if skip { y + x } else { y }
            })
            .collect()
    }

    #[test]
    fn tanh_rational_is_accurate() {
        let mut worst = 0.0_f64;
        for k in -200_000..=200_000 {
            #[expect(clippy::cast_precision_loss, reason = "small k")]
            let x = k as f32 * 1e-4;
            let err = (f64::from(tanh_rational(x)) - f64::from(x).tanh()).abs();
            worst = worst.max(err);
            assert!(tanh_rational(x).abs() <= 1.0, "{x}");
        }
        assert!(worst < 1e-6, "max error {worst}");
        assert_eq!(tanh_rational(100.0), tanh_rational(8.0));
        assert_eq!(tanh_rational(0.0), 0.0);
    }

    struct Test8;
    impl GruModel<8> for Test8 {
        const WEIGHTS: &'static GruWeights<8> = &GruWeights::test_pattern(1, 0.6);
        const SKIP: bool = true;
    }

    #[test]
    fn matches_the_reference_equations() {
        let input: Vec<f32> = (0..4800)
            .map(|n| {
                #[allow(clippy::cast_precision_loss, reason = "small n")]
                let t = n as f32 / 48_000.0;
                0.8 * f32::sin(core::f32::consts::TAU * 110.0 * t)
            })
            .collect();
        let want = reference(Test8::WEIGHTS, true, &input);
        let mut gru = Gru::<Test8, 8>::default();
        let mut err = 0.0_f64;
        for (&x, &w) in input.iter().zip(&want) {
            err = err.max((f64::from(gru.tick(x)) - w).abs());
        }
        assert!(err < 1e-5, "max error {err}");
        gru.reset();
        assert_eq!(gru.h, [0.0; 8]);
    }
}
