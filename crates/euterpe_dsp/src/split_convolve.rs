//! Two-stage zero-latency convolution for long IRs.
//!
//! [`Convolver`] does one 33-bin multiply-add per 32 taps per block, so a
//! 2048-tap cab spends nearly all its time there. [`SplitConvolver`] keeps
//! that only for the head of the IR and puts the tail on bigger
//! partitions:
//!
//! - **Head**: taps `0..512` on a [`Convolver<16>`], 32-tap partitions, so
//!   the output of a block still depends on that block's input.
//! - **Tail**: taps `512..` in 256-tap partitions, each a 512-point
//!   spectrum, one 257-bin multiply-add per 256 taps per *frame* (8
//!   blocks): an eighth of the head's cost per tap.
//!
//! The tail's input frame is complete at the end of a frame, and its output
//! is first needed 512 samples (two frames) after the frame started, so
//! its work is spread over the next frame's eight blocks:
//!
//! | block of the frame | tail work |
//! | --- | --- |
//! | 0 | forward FFT of the last two frames |
//! | 1 to 6 | a sixth of the bins: multiply-add over every partition |
//! | 7 | inverse FFT, which becomes the next frame's output |
//!
//! Every block costs the same from one frame to the next (no spikes beyond
//! this fixed pattern), and the result is the same convolution as
//! [`Convolver`] up to f32 rounding.

use euterpe_core::{BLOCK, Block, Ctx, Processor};

use crate::convolve::{Convolver, dot, next_pos, ring_window};
use crate::fft::{Complex, n512};

/// Head partitions: the head covers twice the tail partition, which is the
/// time the tail needs to compute.
const HEAD_PARTS: usize = 16;
/// Taps on the head.
pub const HEAD_TAPS: usize = HEAD_PARTS * BLOCK;
/// Taps per tail partition.
const TAIL_PART: usize = n512::N / 2;
/// Blocks per tail frame.
const FRAME: usize = TAIL_PART / BLOCK;
/// Bins of a tail spectrum.
const TB: usize = n512::BINS;
/// Blocks of each frame that do the tail's multiply-adds.
const MAC_SLOTS: usize = FRAME - 2;

const _: () = assert!(HEAD_TAPS == 2 * TAIL_PART && FRAME == 8);

/// The bins tail slot `slot` (`0..MAC_SLOTS`) multiplies.
#[inline(always)]
const fn bin_range(slot: usize) -> (usize, usize) {
    (slot * TB / MAC_SLOTS, (slot + 1) * TB / MAC_SLOTS)
}

/// Convolution with an IR of up to `512 + T * 256` taps; `T = 6` holds
/// 2048 (43 ms at 48 kHz).
///
/// Memory: the head's 12.7 KB, plus `3 * T * 257 * 8` bytes for the tail
/// spectra (the input ring is mirrored) and 5 KB of frame buffers
/// (54.7 KB at `T = 6`).
pub struct SplitConvolver<const T: usize> {
    head: Convolver<HEAD_PARTS>,
    /// Tail partition spectra, scaled by `1/256` to undo
    /// [`n512::irfft_scaled`], in reverse order: `tail_ir[k][T - 1 - q]` is
    /// bin `k` of tail partition `q`.
    tail_ir: [[Complex; T]; TB],
    /// Input frame spectra, a mirrored ring with the newest at `pos` and
    /// `pos + T`, as in [`Convolver`].
    fdl: [[[Complex; T]; 2]; TB],
    pos: usize,
    /// The previous frame, then the current one, filled block by block.
    window: [f32; n512::N],
    /// The new frame's spectrum during block 0, then the sum the
    /// multiply-add blocks build.
    spectrum: [Complex; TB],
    /// The tail's output for this frame. Block 7 replaces it with the
    /// next frame's, after reading its own part.
    ready: [f32; TAIL_PART],
    /// Which block of the frame is next.
    slot: usize,
}

impl<const T: usize> Default for SplitConvolver<T> {
    fn default() -> Self {
        Self {
            head: Convolver::default(),
            tail_ir: [[Complex::ZERO; T]; TB],
            fdl: [[[Complex::ZERO; T]; 2]; TB],
            pos: 0,
            window: [0.0; n512::N],
            spectrum: [Complex::ZERO; TB],
            ready: [0.0; TAIL_PART],
            slot: 0,
        }
    }
}

impl<const T: usize> SplitConvolver<T> {
    /// Longest IR this convolver holds, in taps.
    pub const MAX_TAPS: usize = HEAD_TAPS + T * TAIL_PART;

    /// A convolver loaded with `ir`.
    #[must_use]
    pub fn with_ir(ir: &[f32]) -> Self {
        let mut c = Self::default();
        c.set_ir(ir);
        c
    }

    /// Load an impulse response. Taps past [`Self::MAX_TAPS`] are dropped.
    /// Not real-time: it runs `16 + T` FFTs. Clears the signal state.
    pub fn set_ir(&mut self, ir: &[f32]) {
        let (head, tail) = ir.split_at(ir.len().min(HEAD_TAPS));
        self.head.set_ir(head);
        let mut taps = tail.chunks(TAIL_PART);
        for q in 0..T {
            let mut x = [0.0; n512::N];
            if let Some(chunk) = taps.next() {
                for (x, &t) in x.iter_mut().zip(chunk) {
                    *x = t;
                }
            }
            let mut spectrum = [Complex::ZERO; TB];
            n512::rfft(&x, &mut spectrum);
            for (row, c) in self.tail_ir.iter_mut().zip(spectrum) {
                row[T - 1 - q] = c.scale(1.0 / 256.0);
            }
        }
        self.reset();
    }

    pub fn reset(&mut self) {
        self.head.reset();
        // `fill` zeroes in place, as in `Convolver::reset`.
        self.fdl
            .as_flattened_mut()
            .as_flattened_mut()
            .fill(Complex::ZERO);
        self.pos = 0;
        self.window.fill(0.0);
        self.spectrum.fill(Complex::ZERO);
        self.ready.fill(0.0);
        self.slot = 0;
    }

    /// Block 0 of a frame: the spectrum of the last two frames goes into
    /// the ring, and the window slides by a frame.
    #[inline(always)]
    fn tail_forward(&mut self) {
        n512::rfft(&self.window, &mut self.spectrum);
        self.window.copy_within(TAIL_PART.., 0);
        let pos = next_pos(self.pos, T);
        self.pos = pos;
        for (ring, &x) in self.fdl.iter_mut().zip(&self.spectrum) {
            let flat = ring.as_flattened_mut();
            if let Some(s) = flat.get_mut(pos) {
                *s = x;
            }
            if let Some(s) = flat.get_mut(pos + T) {
                *s = x;
            }
        }
    }

    /// Blocks 1 to 6: partition `q` meets the frame from `q` frames ago,
    /// over this block's share of the bins: the ring's window, oldest
    /// first, against the reversed IR.
    #[inline(always)]
    fn tail_mac(&mut self, slot: usize) {
        let (start, end) = bin_range(slot);
        let (w_start, w_end) = ring_window(self.pos, T);
        let (Some(acc), Some(fdl), Some(ir)) = (
            self.spectrum.get_mut(start..end),
            self.fdl.get(start..end),
            self.tail_ir.get(start..end),
        ) else {
            return;
        };
        for (a, (ring, h)) in acc.iter_mut().zip(fdl.iter().zip(ir)) {
            if let Some(window) = ring
                .as_flattened()
                .get(w_start..w_end)
                .and_then(|w| <&[Complex; T]>::try_from(w).ok())
            {
                *a = dot(window, h);
            }
        }
    }

    /// Block 7: the next frame's tail output.
    #[inline(always)]
    fn tail_inverse(&mut self) {
        let mut y = [0.0; n512::N];
        n512::irfft_scaled(&self.spectrum, &mut y);
        if let Some(valid) = y.last_chunk::<TAIL_PART>() {
            self.ready = *valid;
        }
    }

    /// Convolve one block in place.
    #[inline(never)]
    pub fn process(&mut self, buf: &mut Block) {
        let slot = self.slot;
        if slot == 0 {
            self.tail_forward();
        } else if slot <= MAC_SLOTS {
            self.tail_mac(slot - 1);
        }

        // This block's input into the tail's window.
        let at = TAIL_PART + slot * BLOCK;
        if let Some(dst) = self
            .window
            .get_mut(at..at + BLOCK)
            .and_then(|d| <&mut Block>::try_from(d).ok())
        {
            *dst = *buf;
        }

        self.head.process(buf);
        if let Some(tail) = self
            .ready
            .get(slot * BLOCK..(slot + 1) * BLOCK)
            .and_then(|t| <&Block>::try_from(t).ok())
        {
            for (y, &t) in buf.iter_mut().zip(tail) {
                *y += t;
            }
        }

        if slot == FRAME - 1 {
            self.tail_inverse();
            self.slot = 0;
        } else {
            self.slot = slot + 1;
        }
    }
}

impl<const T: usize> Processor for SplitConvolver<T> {
    type Params = ();

    fn prepare(&mut self, _: &Ctx) {
        self.reset();
    }

    fn reset(&mut self) {
        SplitConvolver::reset(self);
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, (): &()) {
        SplitConvolver::process(self, buf);
    }
}

/// Proofs of the schedule (`cargo kani -p euterpe_dsp`).
#[cfg(kani)]
mod proofs {
    use super::*;

    /// The multiply-add blocks' bin ranges are in bounds, in order, and
    /// together cover every bin exactly once.
    #[kani::proof]
    fn bin_ranges_partition_the_bins() {
        let s: usize = kani::any();
        kani::assume(s < MAC_SLOTS);
        let (a, b) = bin_range(s);
        assert!(a <= b && b <= TB);
        if s == 0 {
            assert!(a == 0);
        } else {
            assert!(bin_range(s - 1).1 == a, "no gap or overlap");
        }
        if s == MAC_SLOTS - 1 {
            assert!(b == TB);
        }
    }

    /// Every block of a frame writes its input inside the window and reads
    /// its output inside the ready buffer.
    #[kani::proof]
    fn frame_slots_stay_in_bounds() {
        let slot: usize = kani::any();
        kani::assume(slot < FRAME);
        assert!(TAIL_PART + (slot + 1) * BLOCK <= n512::N);
        assert!((slot + 1) * BLOCK <= TAIL_PART);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;

    use super::*;
    use euterpe_offline::check::{CheckOptions, check_processor};

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
                    .take(n + 1)
                    .enumerate()
                    .map(|(k, &c)| f64::from(c) * f64::from(x[n - k]))
                    .sum()
            })
            .collect()
    }

    fn run<const T: usize>(c: &mut SplitConvolver<T>, x: &[f32]) -> Vec<f32> {
        let mut out = x.to_vec();
        for block in out.chunks_exact_mut(BLOCK) {
            c.process(block.try_into().unwrap());
        }
        out
    }

    fn max_err(got: &[f32], want: &[f64]) -> f64 {
        got.iter()
            .zip(want)
            .map(|(&g, &w)| (f64::from(g) - w).abs())
            .fold(0.0, f64::max)
    }

    #[test]
    fn matches_direct_convolution() {
        let x = noise(BLOCK * 160, 1);
        // Head only, ending mid tail partition, exactly full, and past full.
        for seed in [100_u32, 512, 513, 700, 1024, 1536, 1600] {
            let len = seed as usize;
            let h = noise(len, 7 + seed);
            let mut c = SplitConvolver::<4>::with_ir(&h);
            let got = run(&mut c, &x);
            let keep = len.min(SplitConvolver::<4>::MAX_TAPS);
            let err = max_err(&got, &direct(&x, &h[..keep]));
            assert!(err < 2e-4, "len {len}: {err}");
        }
    }

    #[test]
    fn a_tail_tap_lands_on_its_sample() {
        // Single taps at the head/tail boundary and deep in the tail: an
        // impulse in comes out exactly that many samples later.
        for tap in [511_usize, 512, 767, 768, 1000, 2047] {
            let mut h = std::vec![0.0; tap + 1];
            h[tap] = 1.0;
            let mut c = SplitConvolver::<6>::with_ir(&h);
            for start in [0_usize, 37, 300] {
                c.reset();
                let mut x = std::vec![0.0; 3072];
                x[start] = 1.0;
                let y = run(&mut c, &x);
                let (peak, _) = y
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                    .unwrap();
                assert_eq!(peak, start + tap, "tap {tap}, impulse at {start}");
                assert!((y[peak] - 1.0).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn agrees_with_the_uniform_convolver_on_2048_taps() {
        let h = noise(2048, 11);
        let x = noise(BLOCK * 300, 2);
        let mut split = SplitConvolver::<6>::with_ir(&h);
        let mut uniform = Convolver::<64>::with_ir(&h);
        let got = run(&mut split, &x);
        let mut want = x.clone();
        for block in want.chunks_exact_mut(BLOCK) {
            uniform.process(block.try_into().unwrap());
        }
        let err = got
            .iter()
            .zip(&want)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(err < 1e-3, "{err}");
        assert_eq!(SplitConvolver::<6>::MAX_TAPS, 2048);
    }

    #[test]
    fn passes_invariants() {
        #[derive(Default)]
        struct Cab(SplitConvolver<2>);
        impl Processor for Cab {
            type Params = ();
            fn prepare(&mut self, ctx: &Ctx) {
                if self.0.tail_ir[0][0] == Complex::ZERO {
                    let h: Vec<f32> = noise(900, 5).iter().map(|t| t * 0.02).collect();
                    self.0.set_ir(&h);
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
    }
}
