//! Block convolution with an impulse response: cabinet sims, fixed EQ curves.
//!
//! Uniformly partitioned overlap-save with one partition per
//! [`BLOCK`]: the IR is cut into `P` pieces of 32 taps, each kept as a
//! 64-point spectrum. Every block does the same work (one forward FFT, `P`
//! complex multiply-adds over 33 bins, one inverse FFT), so there are no
//! spikes, and the output of a block depends on that block's input: **zero
//! latency**. The cost grows linearly with the IR length; at 2048 taps the
//! multiply-adds dominate.

use euterpe_core::{BLOCK, Block, Ctx, Processor};

use crate::fft::{self, BINS, Complex, N};

const _: () = assert!(N == 2 * BLOCK);

/// Convolution with an IR of up to `P * 32` taps.
///
/// Memory is three spectra per partition, `3 * P * 33 * 8` bytes: 51 KB
/// for `P = 64` (2048 taps, 43 ms at 48 kHz). For long IRs,
/// [`SplitConvolver`](crate::SplitConvolver) is cheaper.
///
/// The spectra are stored per bin (`[bin][partition]`), so the hot loop
/// sums one bin over all partitions with its accumulator in registers and
/// both operands contiguous. The input ring is mirrored (each spectrum is
/// written at `pos` and `pos + P`), so the `P` most recent inputs, oldest
/// first, are always one contiguous run: one multiply-add loop of a
/// compile-time length per bin.
pub struct Convolver<const P: usize> {
    /// Partition spectra, scaled by `1/32` to undo [`fft::irfft_x32`], in
    /// reverse partition order: `ir[k][P - 1 - p]` is bin `k` of partition `p`.
    ir: [[Complex; P]; BINS],
    /// Input spectra, a mirrored ring over blocks with the newest at `pos`
    /// and again at `pos + P` (see [`ring_window`]).
    fdl: [[[Complex; P]; 2]; BINS],
    pos: usize,
    /// The previous input block, then the current one.
    window: [f32; N],
}

impl<const P: usize> Default for Convolver<P> {
    fn default() -> Self {
        const { assert!(P >= 1) };
        Self {
            ir: [[Complex::ZERO; P]; BINS],
            fdl: [[[Complex::ZERO; P]; 2]; BINS],
            pos: 0,
            window: [0.0; N],
        }
    }
}

/// The ring slot after `pos`, in `0..p`.
#[inline(always)]
pub(crate) const fn next_pos(pos: usize, p: usize) -> usize {
    if pos + 1 < p { pos + 1 } else { 0 }
}

/// Write `x` as the newest entry of a mirrored ring of `P` at `pos`, and
/// return the window of all `P` entries, oldest first, newest last. With the
/// IR stored reversed (partition `P - 1 - j` at `j`), window entry `j`
/// pairs with exactly the partition of its age.
#[inline(always)]
pub(crate) fn ring_push<const P: usize>(
    ring: &mut [[Complex; P]; 2],
    pos: usize,
    x: Complex,
) -> Option<&[Complex; P]> {
    let flat = ring.as_flattened_mut();
    if let Some(s) = flat.get_mut(pos) {
        *s = x;
    }
    if let Some(s) = flat.get_mut(pos + P) {
        *s = x;
    }
    let (start, end) = ring_window(pos, P);
    flat.get(start..end).and_then(|w| w.try_into().ok())
}

/// The mirrored ring's window, `[pos + 1, pos + 1 + p)`, for the newest
/// entry at `pos`.
#[inline(always)]
pub(crate) const fn ring_window(pos: usize, p: usize) -> (usize, usize) {
    (pos + 1, pos + 1 + p)
}

/// `sum a[i] * b[i]` over complex arrays.
///
/// Each product term goes straight into its accumulator (`vmla`/`vmls` on
/// the M7), rather than forming the complex product and then adding it.
#[inline(always)]
pub(crate) fn dot<const P: usize>(a: &[Complex; P], b: &[Complex; P]) -> Complex {
    let (mut re, mut im) = (0.0, 0.0);
    for (&x, &h) in a.iter().zip(b) {
        re += x.re * h.re;
        re -= x.im * h.im;
        im += x.re * h.im;
        im += x.im * h.re;
    }
    Complex::new(re, im)
}

impl<const P: usize> Convolver<P> {
    /// Longest IR this convolver holds, in taps.
    pub const MAX_TAPS: usize = P * BLOCK;

    /// A convolver loaded with `ir`.
    #[must_use]
    pub fn with_ir(ir: &[f32]) -> Self {
        let mut c = Self::default();
        c.set_ir(ir);
        c
    }

    /// Load an impulse response. Taps past [`Self::MAX_TAPS`] are dropped.
    /// Not real-time: it runs `P` FFTs. Clears the signal state.
    pub fn set_ir(&mut self, ir: &[f32]) {
        let mut taps = ir.chunks(BLOCK);
        for p in 0..P {
            let mut x = [0.0; N];
            if let Some(chunk) = taps.next() {
                for (x, &t) in x.iter_mut().zip(chunk) {
                    *x = t;
                }
            }
            let mut spectrum = [Complex::ZERO; BINS];
            fft::rfft(&x, &mut spectrum);
            for (row, c) in self.ir.iter_mut().zip(spectrum) {
                row[P - 1 - p] = c.scale(1.0 / 32.0);
            }
        }
        self.reset();
    }

    pub fn reset(&mut self) {
        // `fill` zeroes in place; assigning an array literal can build it on
        // the stack first.
        self.fdl
            .as_flattened_mut()
            .as_flattened_mut()
            .fill(Complex::ZERO);
        self.pos = 0;
        self.window.fill(0.0);
    }

    /// Convolve one block in place.
    #[inline(always)]
    pub fn process(&mut self, buf: &mut Block) {
        // Slide the window: [previous block, this block].
        self.window.copy_within(BLOCK.., 0);
        if let Some(newest) = self.window.last_chunk_mut::<BLOCK>() {
            *newest = *buf;
        }
        let mut spectrum = [Complex::ZERO; BINS];
        fft::rfft(&self.window, &mut spectrum);

        // Into the ring, at `pos`.
        let pos = next_pos(self.pos, P);
        self.pos = pos;

        // Partition p meets the input from p blocks ago: the ring's window,
        // oldest first, against the reversed IR.
        let mut acc = [Complex::ZERO; BINS];
        for ((a, (ring, h)), x) in acc
            .iter_mut()
            .zip(self.fdl.iter_mut().zip(&self.ir))
            .zip(spectrum)
        {
            if let Some(window) = ring_push(ring, pos, x) {
                *a = dot(window, h);
            }
        }

        // Overlap-save: the second half is the linear convolution.
        let mut y = [0.0; N];
        fft::irfft_x32(&acc, &mut y);
        if let Some(valid) = y.last_chunk::<BLOCK>() {
            *buf = *valid;
        }
    }
}

impl<const P: usize> Processor for Convolver<P> {
    type Params = ();

    fn prepare(&mut self, _: &Ctx) {
        self.reset();
    }

    fn reset(&mut self) {
        Convolver::reset(self);
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, (): &()) {
        Convolver::process(self, buf);
    }
}

/// Proofs of the ring arithmetic (`cargo kani -p euterpe_dsp`).
#[cfg(kani)]
mod proofs {
    use super::*;

    /// `pos` never leaves the ring, from any valid position.
    #[kani::proof]
    fn ring_position_stays_in_range() {
        let p: usize = kani::any();
        kani::assume((1..=1024).contains(&p));
        let pos: usize = kani::any();
        kani::assume(pos < p);
        assert!(next_pos(pos, p) < p);
    }

    /// For every ring size up to 1024 and every position, both writes and
    /// the whole window are inside the mirrored ring of `2 * p`, so
    /// `ring_push` always returns the window.
    #[kani::proof]
    fn ring_window_is_in_bounds() {
        let p: usize = kani::any();
        kani::assume((1..=1024).contains(&p));
        let pos: usize = kani::any();
        kani::assume(pos < p);
        let (start, end) = ring_window(pos, p);
        assert!(pos + p < 2 * p, "the mirror write");
        assert!(end - start == p && end <= 2 * p);
    }

    /// After any sequence of pushes into an 8-slot mirrored ring, window
    /// entry `j` holds the input from `P - 1 - j` pushes ago: the oldest
    /// first, the newest last, every one exactly once.
    #[kani::proof]
    #[kani::unwind(12)]
    fn ring_window_holds_the_last_p_inputs_in_order() {
        const P: usize = 8;
        let mut ring = [[Complex::ZERO; P]; 2];
        let mut pos = 0;
        let pushes: usize = kani::any();
        kani::assume((P..=P + 2).contains(&pushes));
        let mut last = None;
        for n in 0..pushes {
            pos = next_pos(pos, P);
            #[expect(clippy::cast_precision_loss, reason = "n < 16")]
            let x = Complex::new(n as f32, 0.0);
            last = ring_push(&mut ring, pos, x).copied();
        }
        let window = last.expect("the window is always in bounds");
        for (j, w) in window.iter().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "small")]
            let want = (pushes - P + j) as f32;
            assert!(w.re == want);
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "silence in, exact silence out")]
mod tests {
    extern crate std;
    use std::vec;
    use std::vec::Vec;

    use super::*;
    use euterpe_offline::check::{CheckOptions, check_processor};
    use euterpe_offline::{RenderOptions, render};

    fn noise(len: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    fn direct(x: &[f32], h: &[f32]) -> Vec<f64> {
        (0..x.len())
            .map(|n| {
                h.iter()
                    .enumerate()
                    .filter(|&(k, _)| k <= n)
                    .map(|(k, &c)| f64::from(c) * f64::from(x[n - k]))
                    .sum()
            })
            .collect()
    }

    fn run<const P: usize>(x: &[f32], h: &[f32]) -> Vec<f32> {
        let mut c = Convolver::<P>::with_ir(h);
        let mut out = x.to_vec();
        for block in out.chunks_exact_mut(BLOCK) {
            c.process(block.try_into().unwrap());
        }
        out
    }

    #[test]
    fn matches_direct_convolution() {
        let x = noise(BLOCK * 40, 1);
        // Lengths that end mid-partition, exactly on one, and use them all.
        for len in [1_u32, 31, 32, 100, 256] {
            let h = noise(len as usize, 7 + len);
            let got = run::<8>(&x, &h);
            let want = direct(&x, &h);
            let err = got
                .iter()
                .zip(&want)
                .map(|(&g, &w)| (f64::from(g) - w).abs())
                .fold(0.0, f64::max);
            assert!(err < 1e-4, "len {len}: {err}");
        }
    }

    #[test]
    fn identity_ir_passes_through_with_zero_latency() {
        let x = noise(BLOCK * 10, 3);
        let got = run::<4>(&x, &[1.0]);
        for (a, b) in x.iter().zip(&got) {
            assert!((a - b).abs() < 1e-6);
        }
        assert_eq!(Convolver::<4>::default().latency(), 0);
    }

    #[test]
    fn long_ir_is_truncated_to_capacity() {
        let h = noise(Convolver::<2>::MAX_TAPS + 50, 9);
        let x = noise(BLOCK * 8, 4);
        let got = run::<2>(&x, &h);
        let want = direct(&x, &h[..Convolver::<2>::MAX_TAPS]);
        for (g, w) in got.iter().zip(&want) {
            assert!((f64::from(*g) - w).abs() < 1e-4);
        }
    }

    #[test]
    fn passes_invariants() {
        #[derive(Default)]
        struct Cab(Convolver<4>);
        impl Processor for Cab {
            type Params = ();
            fn prepare(&mut self, ctx: &Ctx) {
                if self.0.ir[0][3] == Complex::ZERO {
                    self.0.set_ir(&noise(100, 5));
                }
                self.0.prepare(ctx);
            }
            fn reset(&mut self) {
                Processor::reset(&mut self.0);
            }
            fn process(&mut self, buf: &mut Block, p: &()) {
                Processor::process(&mut self.0, buf, p);
            }
        }
        check_processor::<Cab>(&CheckOptions::default()).unwrap();
        let out = render(
            &mut Cab::default(),
            &(),
            &vec![0.0; 960],
            &RenderOptions::new(48_000),
        );
        assert!(out.iter().all(|&s| s == 0.0));
    }
}
