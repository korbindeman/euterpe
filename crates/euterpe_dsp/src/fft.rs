//! Real FFTs for the convolvers: 64 points ([`rfft`], two
//! [`BLOCK`](euterpe_core::BLOCK)s, for [`Convolver`](crate::Convolver))
//! and 512 points ([`n512`], for the tail of
//! [`SplitConvolver`](crate::SplitConvolver)).
//!
//! Each real transform runs as a complex FFT of half the size on the
//! even/odd samples packed as `re`/`im`, followed by a split step. Twiddles
//! are computed at compile time, and every loop runs over iterators, so
//! there is no index to bounds-check.

use core::ops::{Add, Mul, Sub};

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

/// Every radix-2 stage's twiddles for a `LEN + 1`-point complex FFT,
/// concatenated (see `FFT_TW` in each size's module); conjugated if `conj`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "tables are rounded to f32 on purpose"
)]
const fn stage_twiddles<const LEN: usize>(conj: bool) -> [Complex; LEN] {
    let mut t = [Complex::ZERO; LEN];
    let mut h = 1;
    while h <= LEN {
        let mut j = 0;
        while j < h {
            let (s, c) = sin_cos(-core::f64::consts::TAU * j as f64 / (2 * h) as f64);
            let s = if conj { -s } else { s };
            t[h - 1 + j] = Complex::new(c as f32, s as f32);
            j += 1;
        }
        h *= 2;
    }
    t
}

/// One radix-2 decimation-in-time stage over `z`, of half-size `tw.len()`
/// with twiddles `tw`. Shared by every size and both directions, so there is
/// one copy of the butterfly loop in flash.
#[inline(never)]
fn stage(z: &mut [Complex], tw: &[Complex]) {
    let h = tw.len();
    let Some(n) = h.checked_mul(2).filter(|&n| n > 0) else {
        return;
    };
    for chunk in z.chunks_exact_mut(n) {
        let (lo, hi) = chunk.split_at_mut(h);
        for ((a, b), &w) in lo.iter_mut().zip(hi).zip(tw) {
            let t = b.mul(w);
            *b = a.sub(t);
            *a = a.add(t);
        }
    }
}

/// A real FFT of `$n` points (`$m = $n / 2`, `$bits = log2($m)`).
macro_rules! real_fft {
    ($n:literal, $m:literal, $bits:literal) => {
        use core::ops::{Add, Mul, Sub};

        use super::{Complex, stage_twiddles, twiddles};

        /// Points in the real transform.
        pub const N: usize = $n;
        /// Bins of the real transform: DC to Nyquist.
        pub const BINS: usize = N / 2 + 1;
        /// Points of the complex FFT inside it.
        const M: usize = $m;
        const _: () = assert!(M * 2 == N && 1 << $bits == M && M <= 256);

        /// Twiddles of every radix-2 stage, concatenated: the stage of
        /// half-size `h` uses `e^(-2 pi i j / 2h)` for `j < h`, at offset
        /// `h - 1`. Stored contiguously, so each stage zips a slice instead
        /// of striding through one table.
        static FFT_TW: [Complex; M - 1] = stage_twiddles::<{ M - 1 }>(false);
        /// The same, conjugated, for the inverse.
        static IFFT_TW: [Complex; M - 1] = stage_twiddles::<{ M - 1 }>(true);
        /// Twiddles of the real split step: `e^(-2 pi i k / N)`, `k <= M`.
        static SPLIT_TW: [Complex; BINS] = twiddles::<BINS>(N);

        const fn bit_reverse() -> [u8; M] {
            let mut t = [0u8; M];
            let mut i = 0;
            while i < M {
                let mut r = 0_usize;
                let mut b = 0_usize;
                while b < $bits {
                    r |= ((i >> b) & 1) << ($bits - 1 - b);
                    b += 1;
                }
                #[expect(clippy::cast_possible_truncation, reason = "r < M <= 256")]
                {
                    t[i] = r as u8;
                }
                i += 1;
            }
            t
        }

        static BIT_REVERSE: [u8; M] = bit_reverse();

        /// In-place complex FFT of `M` points, radix 2, decimation in time,
        /// with `table`'s twiddles: [`FFT_TW`] forward, [`IFFT_TW`] for the
        /// inverse (`sign` -1 and 1), which does not scale.
        ///
        /// Out of line, as are the real transforms: a call is nothing next
        /// to a transform, and inlined or per-direction copies cost
        /// kilobytes of flash each.
        #[inline(never)]
        fn fft(z: &mut [Complex; M], table: &[Complex; M - 1], sign: f32) {
            // Bit reversal is an involution: swap each pair once. The mask
            // keeps the index provably in range, so there is no bounds check.
            for i in 0..M {
                let r = usize::from(BIT_REVERSE[i]) & (M - 1);
                if i < r {
                    z.swap(i, r);
                }
            }
            // Half-size 1: the twiddle is 1.
            for pair in z.as_chunks_mut::<2>().0 {
                let [a, b] = *pair;
                *pair = [a.add(b), a.sub(b)];
            }
            // Half-size 2: twiddles 1 and -i (i for the inverse).
            for quad in z.as_chunks_mut::<4>().0 {
                let [a0, a1, b0, b1] = *quad;
                let t1 = b1.mul_i().scale(sign);
                *quad = [a0.add(b0), a1.add(t1), a0.sub(b0), a1.sub(t1)];
            }
            let mut h = 4;
            while h < M {
                if let Some(tw) = table.get(h - 1..2 * h - 1) {
                    super::stage(z, tw);
                }
                h *= 2;
            }
        }

        /// Forward real FFT: `X[k] = sum x[n] e^(-2 pi i n k / N)`, `k = 0..=N/2`.
        #[inline(never)]
        pub fn rfft(x: &[f32; N], out: &mut [Complex; BINS]) {
            let mut z = [Complex::ZERO; M];
            for (z, &[re, im]) in z.iter_mut().zip(x.as_chunks::<2>().0) {
                *z = Complex::new(re, im);
            }
            fft(&mut z, &FFT_TW, -1.0);
            // Z[k] and Z[M - k] for k = 0..=M, with Z[M] = Z[0].
            let z0 = z[0];
            let up = z.iter().chain(core::iter::once(&z0));
            let down = core::iter::once(&z0)
                .chain(z.iter().skip(1).rev())
                .chain(core::iter::once(&z0));
            for ((x, (&zk, &zmk)), &w) in out.iter_mut().zip(up.zip(down)).zip(&SPLIT_TW) {
                let zmk = zmk.conj();
                let even = zk.add(zmk).scale(0.5);
                // (Z[k] - conj Z[M-k]) / 2i
                let odd = zk.sub(zmk).mul_i().scale(-0.5);
                *x = even.add(w.mul(odd));
            }
        }

        /// Inverse real FFT, **scaled by `N / 2`**: `out = N/2 * irfft(x)`.
        /// Callers fold the `2 / N` into a coefficient they already multiply by.
        #[inline(never)]
        pub fn irfft_scaled(x: &[Complex; BINS], out: &mut [f32; N]) {
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
            fft(&mut z, &IFFT_TW, 1.0);
            for (&z, pair) in z.iter().zip(out.as_chunks_mut::<2>().0) {
                *pair = [z.re, z.im];
            }
        }

        /// Proofs (`cargo kani -p euterpe_dsp`).
        #[cfg(kani)]
        mod proofs {
            use super::*;

            /// The bit-reversal table is a permutation of `0..M` (an
            /// involution), so `fft`'s `get` never falls back and no sample
            /// is lost or doubled.
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
                    let a = -core::f64::consts::TAU * k as f64 / N as f64;
                    assert!((f64::from(w.re) - a.cos()).abs() < 1e-7);
                    assert!((f64::from(w.im) - a.sin()).abs() < 1e-7);
                }
                for i in 0..M {
                    assert_eq!(usize::from(BIT_REVERSE[usize::from(BIT_REVERSE[i])]), i);
                }
                let mut h = 1;
                while h < M {
                    for j in 0..h {
                        let a = -core::f64::consts::TAU * j as f64 / (2 * h) as f64;
                        let w = FFT_TW[h - 1 + j];
                        assert!((f64::from(w.re) - a.cos()).abs() < 1e-7);
                        assert!((f64::from(w.im) - a.sin()).abs() < 1e-7);
                        assert_eq!(IFFT_TW[h - 1 + j], w.conj());
                    }
                    h *= 2;
                }
                assert_eq!(usize::from(BIT_REVERSE[1]), M / 2);
            }

            #[test]
            fn rfft_matches_direct_dft() {
                let x = input();
                let mut got = [Complex::ZERO; BINS];
                rfft(&x, &mut got);
                let tol = 2e-6 * N as f64;
                for (k, g) in got.iter().enumerate() {
                    let (mut re, mut im) = (0.0, 0.0);
                    for (n, &v) in x.iter().enumerate() {
                        let a = -core::f64::consts::TAU * ((n * k) % N) as f64 / N as f64;
                        re += f64::from(v) * a.cos();
                        im += f64::from(v) * a.sin();
                    }
                    assert!((f64::from(g.re) - re).abs() < tol, "k={k}");
                    assert!((f64::from(g.im) - im).abs() < tol, "k={k}");
                }
            }

            #[test]
            fn irfft_inverts_rfft() {
                let x = input();
                let mut spec = [Complex::ZERO; BINS];
                rfft(&x, &mut spec);
                let mut back = [0.0; N];
                irfft_scaled(&spec, &mut back);
                for (a, b) in x.iter().zip(&back) {
                    assert!((a - b / M as f32).abs() < 1e-5);
                }
            }
        }
    };
}

/// The 64-point transform, two blocks: [`Convolver`](crate::Convolver).
pub mod n64 {
    real_fft!(64, 32, 5);
}

/// The 512-point transform: the tail of
/// [`SplitConvolver`](crate::SplitConvolver).
pub mod n512 {
    real_fft!(512, 256, 8);
}

pub use n64::{BINS, N, rfft};

/// The 64-point inverse, scaled by 32. See [`n64::irfft_scaled`].
#[inline]
pub fn irfft_x32(x: &[Complex; BINS], out: &mut [f32; N]) {
    n64::irfft_scaled(x, out);
}
