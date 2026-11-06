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
/// Memory is two spectra per partition, `2 * P * 33 * 8` bytes: 34 KB for
/// `P = 64` (2048 taps, 43 ms at 48 kHz).
///
/// Both spectra are stored per bin (`[bin][partition]`), so the hot loop
/// sums one bin over all partitions with its accumulator in registers and
/// both operands contiguous.
pub struct Convolver<const P: usize> {
    /// Partition spectra, scaled by `1/32` to undo [`fft::irfft_x32`], in
    /// reverse partition order: `ir[k][P - 1 - p]` is bin `k` of partition `p`.
    ir: [[Complex; P]; BINS],
    /// Input spectra, a ring over blocks with the newest at `pos`.
    fdl: [[Complex; P]; BINS],
    pos: usize,
    /// The previous input block, then the current one.
    window: [f32; N],
}

impl<const P: usize> Default for Convolver<P> {
    fn default() -> Self {
        const { assert!(P >= 1) };
        Self {
            ir: [[Complex::ZERO; P]; BINS],
            fdl: [[Complex::ZERO; P]; BINS],
            pos: 0,
            window: [0.0; N],
        }
    }
}

/// The ring slot after `pos`, in `0..p`.
#[inline(always)]
const fn next_pos(pos: usize, p: usize) -> usize {
    if pos + 1 < p { pos + 1 } else { 0 }
}

/// Where the ring and the reversed IR split into the two forward runs that
/// pair partition `k` with the input from `k` blocks ago: `fdl[..len]` with
/// `ir[split..]`, then `fdl[len..]` with `ir[..split]`.
#[inline(always)]
const fn runs(pos: usize, p: usize) -> (usize, usize) {
    (pos + 1, p - 1 - pos)
}

/// `sum a[i] * b[i]` over complex slices.
///
/// Each product term goes straight into its accumulator (`vmla`/`vmls` on
/// the M7), rather than forming the complex product and then adding it.
#[inline(always)]
fn dot(a: &[Complex], b: &[Complex]) -> Complex {
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
        self.fdl = [[Complex::ZERO; P]; BINS];
        self.pos = 0;
        self.window = [0.0; N];
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

        // Partition p meets the input from p blocks ago, `fdl[(pos - p) mod
        // P]`. With the IR stored reversed, that is two forward runs:
        // fdl[..=pos] with ir[P-1-pos..], and fdl[pos+1..] with ir[..P-1-pos].
        let (new_len, split) = runs(pos, P);
        let mut acc = [Complex::ZERO; BINS];
        for ((a, (row, h)), x) in acc
            .iter_mut()
            .zip(self.fdl.iter_mut().zip(&self.ir))
            .zip(spectrum)
        {
            if let Some(slot) = row.get_mut(pos) {
                *slot = x;
            }
            let (Some((f_new, f_old)), Some((h_old, h_new))) =
                (row.split_at_checked(new_len), h.split_at_checked(split))
            else {
                continue;
            };
            *a = dot(f_new, h_new) + dot(f_old, h_old);
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

    /// For every ring size up to 1024 and every position, both splits
    /// exist (so `process` never takes its fallback), the paired runs have
    /// equal lengths, and they cover the ring exactly once.
    #[kani::proof]
    fn runs_split_cleanly() {
        let p: usize = kani::any();
        kani::assume((1..=1024).contains(&p));
        let pos: usize = kani::any();
        kani::assume(pos < p);
        let (len, split) = runs(pos, p);
        assert!(len <= p && split <= p);
        assert!(len == p - split, "fdl[..len] pairs with ir[split..]");
        assert!(p - len == split, "fdl[len..] pairs with ir[..split]");
    }

    /// Walking the two runs the way `process` does pairs ring slot `q` with
    /// partition `(pos - q) mod P`: each partition meets the input from
    /// exactly that many blocks ago. Checked for every slot of an 8-slot ring.
    #[kani::proof]
    #[kani::unwind(10)]
    fn runs_pair_each_partition_with_its_delay() {
        const P: usize = 8;
        let pos: usize = kani::any();
        kani::assume(pos < P);
        let (len, split) = runs(pos, P);
        let mut seen = [false; P];
        for q in 0..P {
            // Index into the reversed IR that `process` multiplies slot q by.
            let h = if q < len { split + q } else { q - len };
            let partition = P - 1 - h;
            assert!(partition == (pos + P - q) % P);
            assert!(!seen[partition]);
            seen[partition] = true;
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
