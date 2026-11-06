//! A 64-point real FFT for the block [`Convolver`](crate::Convolver).
//!
//! 64 points is two [`BLOCK`](euterpe_core::BLOCK)s: the size uniformly
//! partitioned overlap-save needs. The real transform runs as a 32-point
//! complex FFT on the even/odd samples packed as `re`/`im`, followed by a
//! split step. Twiddles are computed at compile time, and every loop runs
//! over iterators, so there is no index to bounds-check.

use core::ops::{Add, Mul, Sub};

/// Points in the real transform.
pub const N: usize = 64;
/// Bins of the real transform: DC to Nyquist.
pub const BINS: usize = N / 2 + 1;
/// Points of the complex FFT inside it.
const M: usize = N / 2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    pub const ZERO: Self = Self { re: 0.0, im: 0.0 };

    #[inline(always)]
    #[must_use]
    pub const fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    #[inline(always)]
    #[must_use]
    pub fn conj(self) -> Self {
        Self::new(self.re, -self.im)
    }

    #[inline(always)]
    #[must_use]
    pub fn scale(self, s: f32) -> Self {
        Self::new(self.re * s, self.im * s)
    }

    /// `self * i`.
    #[inline(always)]
    #[must_use]
    fn mul_i(self) -> Self {
        Self::new(-self.im, self.re)
    }
}

impl Add for Complex {
    type Output = Self;
    #[inline(always)]
    fn add(self, o: Self) -> Self {
        Self::new(self.re + o.re, self.im + o.im)
    }
}

impl Sub for Complex {
    type Output = Self;
    #[inline(always)]
    fn sub(self, o: Self) -> Self {
        Self::new(self.re - o.re, self.im - o.im)
    }
}

impl Mul for Complex {
    type Output = Self;
    #[inline(always)]
    fn mul(self, o: Self) -> Self {
        Self::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }
}

// ---------------------------------------------------------------------------
// Compile-time tables
// ---------------------------------------------------------------------------

/// `sin` and `cos` of `x` in f64 by Taylor series, for tables only.
const fn sin_cos(x: f64) -> (f64, f64) {
    // Reduce to [-pi, pi], where 30 terms are far past f64 precision.
    let tau = core::f64::consts::TAU;
    let mut x = x;
    while x > core::f64::consts::PI {
        x -= tau;
    }
    while x < -core::f64::consts::PI {
        x += tau;
    }
    let (mut s, mut c) = (0.0_f64, 0.0_f64);
    let (mut ts, mut tc) = (x, 1.0_f64);
    let mut k = 0_usize;
    while k < 30 {
        s += ts;
        c += tc;
        let n = (2 * k + 2) as f64;
        ts *= -x * x / (n * (n + 1.0_f64));
        tc *= -x * x / ((n - 1.0_f64) * n);
        k += 1;
    }
    (s, c)
}

/// `e^(-2 pi i k / n)` for `k` in `0..LEN`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "tables are rounded to f32 on purpose"
)]
const fn twiddles<const LEN: usize>(n: usize) -> [Complex; LEN] {
    let mut t = [Complex::ZERO; LEN];
    let mut k = 0;
    while k < LEN {
        let (s, c) = sin_cos(-core::f64::consts::TAU * k as f64 / n as f64);
        t[k] = Complex::new(c as f32, s as f32);
        k += 1;
    }
    t
}

/// Twiddles of the 32-point complex FFT: `e^(-2 pi i k / 32)`, `k < 16`.
static FFT_TW: [Complex; M / 2] = twiddles::<{ M / 2 }>(M);
/// Twiddles of the real split step: `e^(-2 pi i k / 64)`, `k <= 32`.
static SPLIT_TW: [Complex; BINS] = twiddles::<BINS>(N);

const fn bit_reverse() -> [u8; M] {
    let mut t = [0u8; M];
    let mut i = 0;
    while i < M {
        let mut r = 0_usize;
        let mut b = 0_usize;
        while b < 5 {
            r |= ((i >> b) & 1) << (4 - b);
            b += 1;
        }
        #[expect(clippy::cast_possible_truncation, reason = "r < 32")]
        {
            t[i] = r as u8;
        }
        i += 1;
    }
    t
}

static BIT_REVERSE: [u8; M] = bit_reverse();

// ---------------------------------------------------------------------------
// Transforms
// ---------------------------------------------------------------------------

/// In-place 32-point complex FFT, radix 2, decimation in time. `inverse`
/// conjugates the twiddles and does not scale.
#[inline]
fn fft32(z: &mut [Complex; M], inverse: bool) {
    let src = *z;
    for (dst, &r) in z.iter_mut().zip(&BIT_REVERSE) {
        *dst = src.get(usize::from(r)).copied().unwrap_or_default();
    }
    let mut len = 2;
    while len <= M {
        let step = M / len;
        for chunk in z.chunks_exact_mut(len) {
            let (lo, hi) = chunk.split_at_mut(len / 2);
            for ((a, b), &w) in lo.iter_mut().zip(hi).zip(FFT_TW.iter().step_by(step)) {
                let w = if inverse { w.conj() } else { w };
                let t = b.mul(w);
                *b = a.sub(t);
                *a = a.add(t);
            }
        }
        len *= 2;
    }
}

/// Forward real FFT: `X[k] = sum x[n] e^(-2 pi i n k / 64)`, `k = 0..=32`.
#[inline]
pub fn rfft(x: &[f32; N], out: &mut [Complex; BINS]) {
    let mut z = [Complex::ZERO; M];
    for (z, &[re, im]) in z.iter_mut().zip(x.as_chunks::<2>().0) {
        *z = Complex::new(re, im);
    }
    fft32(&mut z, false);
    // Z[k] for k = 0..=32, with Z[32] = Z[0].
    let mut ext = [Complex::ZERO; BINS];
    ext[..M].copy_from_slice(&z);
    ext[M] = z[0];
    for ((x, (&zk, &zmk)), &w) in out
        .iter_mut()
        .zip(ext.iter().zip(ext.iter().rev()))
        .zip(&SPLIT_TW)
    {
        let zmk = zmk.conj();
        let even = zk.add(zmk).scale(0.5);
        // (Z[k] - conj Z[M-k]) / 2i
        let odd = zk.sub(zmk).mul_i().scale(-0.5);
        *x = even.add(w.mul(odd));
    }
}

/// Inverse real FFT, **scaled by 32**: `out = 32 * irfft(x)`. Callers fold
/// the `1/32` into a coefficient they already multiply by.
#[inline]
pub fn irfft_x32(x: &[Complex; BINS], out: &mut [f32; N]) {
    let mut z = [Complex::ZERO; M];
    for ((z, (&xk, &xmk)), &w) in z
        .iter_mut()
        .zip(x.iter().zip(x.iter().rev()))
        .zip(&SPLIT_TW)
    {
        let xmk = xmk.conj();
        let even = xk.add(xmk).scale(0.5);
        let odd = xk.sub(xmk).scale(0.5).mul(w.conj());
        *z = even.add(odd.mul_i());
    }
    fft32(&mut z, true);
    for (&z, pair) in z.iter().zip(out.as_chunks_mut::<2>().0) {
        *pair = [z.re, z.im];
    }
}

/// Proofs (`cargo kani -p euterpe_dsp`).
#[cfg(kani)]
mod proofs {
    use super::*;

    /// The bit-reversal table is a permutation of 0..32 (an involution), so
    /// `fft32`'s `get` never falls back and no sample is lost or doubled.
    #[kani::proof]
    fn bit_reverse_is_an_involution() {
        let i: usize = kani::any();
        kani::assume(i < M);
        let r = usize::from(BIT_REVERSE[i]);
        assert!(r < M);
        assert!(usize::from(BIT_REVERSE[r]) == i);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    fn input() -> [f32; N] {
        core::array::from_fn(|i| ((i * 29 % 61) as f32 / 30.0) - 1.0)
    }

    #[test]
    fn tables_are_accurate() {
        for (k, w) in SPLIT_TW.iter().enumerate() {
            let a = -core::f64::consts::TAU * k as f64 / 64.0;
            assert!((f64::from(w.re) - a.cos()).abs() < 1e-7);
            assert!((f64::from(w.im) - a.sin()).abs() < 1e-7);
        }
        assert_eq!(BIT_REVERSE[1], 16);
        assert_eq!(BIT_REVERSE[6], 12);
    }

    #[test]
    fn rfft_matches_direct_dft() {
        let x = input();
        let mut got = [Complex::ZERO; BINS];
        rfft(&x, &mut got);
        for (k, g) in got.iter().enumerate() {
            let (mut re, mut im) = (0.0, 0.0);
            for (n, &v) in x.iter().enumerate() {
                let a = -core::f64::consts::TAU * (n * k) as f64 / 64.0;
                re += f64::from(v) * a.cos();
                im += f64::from(v) * a.sin();
            }
            assert!((f64::from(g.re) - re).abs() < 1e-4, "k={k}");
            assert!((f64::from(g.im) - im).abs() < 1e-4, "k={k}");
        }
    }

    #[test]
    fn irfft_inverts_rfft() {
        let x = input();
        let mut spec = [Complex::ZERO; BINS];
        rfft(&x, &mut spec);
        let mut back = [0.0; N];
        irfft_x32(&spec, &mut back);
        for (a, b) in x.iter().zip(&back) {
            assert!((a - b / 32.0).abs() < 1e-5);
        }
    }
}
