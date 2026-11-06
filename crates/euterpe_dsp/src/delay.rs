//! A delay line: a ring of `N` samples (a power of two) with integer,
//! linear and cubic taps.
//!
//! `N * 4` bytes of memory: a 65536-sample line (1.36 s at 48 kHz) is
//! 256 KB, which puts its graph in the Daisy's AXI SRAM arena
//! ([`LARGE_GRAPH_ARENA_BYTES`](euterpe_core::device::LARGE_GRAPH_ARENA_BYTES)).

/// `N` samples of history. Push one sample, then read taps `d` samples back
/// (`d = 0` is the sample just pushed).
pub struct DelayLine<const N: usize> {
    buf: [f32; N],
    /// Index of the newest sample.
    pos: usize,
}

impl<const N: usize> Default for DelayLine<N> {
    fn default() -> Self {
        const {
            assert!(
                N.is_power_of_two() && N >= 4,
                "N must be a power of two, at least 4"
            );
        };
        Self {
            buf: [0.0; N],
            pos: 0,
        }
    }
}

/// Index of the sample `d` behind `pos` in a ring of `n` (a power of two).
#[inline(always)]
const fn index(pos: usize, d: usize, n: usize) -> usize {
    pos.wrapping_sub(d) & (n - 1)
}

impl<const N: usize> DelayLine<N> {
    /// Longest delay a fractional tap reads, in samples.
    pub const MAX_DELAY: usize = N - 3;

    /// Add the newest sample.
    #[inline(always)]
    pub fn push(&mut self, x: f32) {
        self.pos = index(self.pos, N - 1, N);
        if let Some(slot) = self.buf.get_mut(self.pos) {
            *slot = x;
        }
    }

    /// The sample pushed `d` pushes ago. `d` wraps at `N`.
    #[inline(always)]
    #[must_use]
    pub fn tap(&self, d: usize) -> f32 {
        self.buf.get(index(self.pos, d, N)).copied().unwrap_or(0.0)
    }

    /// Integer part and fraction of a delay, clamped to `[lo, MAX_DELAY]`.
    #[inline(always)]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to [lo, MAX_DELAY] first"
    )]
    fn split(d: f32, lo: f32) -> (usize, f32) {
        #[expect(clippy::cast_precision_loss, reason = "N is far below 2^24")]
        let d = d.clamp(lo, Self::MAX_DELAY as f32);
        let i = d as usize;
        (i, d - i as f32)
    }

    /// A tap `d` samples back with linear interpolation. Clamped to
    /// `[0, MAX_DELAY]`.
    #[inline(always)]
    #[must_use]
    pub fn tap_linear(&self, d: f32) -> f32 {
        let (i, t) = Self::split(d, 0.0);
        let (a, b) = (self.tap(i), self.tap(i + 1));
        a + t * (b - a)
    }

    /// A tap `d` samples back with 4-point cubic Hermite interpolation:
    /// flatter in the treble than linear, for modulated delays. Clamped to
    /// `[1, MAX_DELAY]`.
    #[inline(always)]
    #[must_use]
    pub fn tap_cubic(&self, d: f32) -> f32 {
        let (i, t) = Self::split(d, 1.0);
        let (y0, y1, y2, y3) = (
            self.tap(i - 1),
            self.tap(i),
            self.tap(i + 1),
            self.tap(i + 2),
        );
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * t + c2) * t + c1) * t + y1
    }

    pub fn reset(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }
}

/// Proofs (`cargo kani -p euterpe_dsp`).
#[cfg(kani)]
mod proofs {
    use super::*;

    /// Every tap of every ring size is in bounds, from any position, for
    /// any delay.
    #[kani::proof]
    fn taps_stay_in_bounds() {
        let shift: u32 = kani::any();
        kani::assume((2..=20).contains(&shift));
        let n = 1_usize << shift;
        let pos: usize = kani::any();
        kani::assume(pos < n);
        let d: usize = kani::any();
        assert!(index(pos, d, n) < n);
    }

    /// After `k` more pushes, a sample is `k` back: `tap` reads the history
    /// in order, across the wrap.
    #[kani::proof]
    #[kani::unwind(10)]
    fn pushes_read_back_in_order() {
        let mut line = DelayLine::<8>::default();
        line.pos = kani::any();
        kani::assume(line.pos < 8);
        let k: usize = kani::any();
        kani::assume(k < 8);
        let x: f32 = kani::any();
        line.push(x);
        for i in 0..k {
            #[expect(clippy::cast_precision_loss, reason = "i < 8")]
            line.push(i as f32);
        }
        assert!(line.tap(k).to_bits() == x.to_bits());
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "integer taps are exact")]
mod tests {
    use super::*;

    fn ramp_line() -> DelayLine<16> {
        let mut line = DelayLine::<16>::default();
        for i in 0..40_u8 {
            line.push(f32::from(i));
        }
        line
    }

    #[test]
    fn integer_taps_read_back_exactly_across_the_wrap() {
        let line = ramp_line();
        for d in 0..16_u8 {
            assert_eq!(line.tap(usize::from(d)), f32::from(39 - d));
        }
    }

    #[test]
    fn interpolated_taps_are_exact_on_a_ramp() {
        // Linear and cubic interpolation both reproduce a straight line.
        let line = ramp_line();
        for d in [1.0, 1.25, 2.5, 7.75, 12.999] {
            assert!((line.tap_linear(d) - (39.0 - d)).abs() < 1e-4, "{d}");
            assert!((line.tap_cubic(d) - (39.0 - d)).abs() < 1e-4, "{d}");
        }
        assert_eq!(line.tap_linear(-5.0), 39.0, "clamped");
        assert_eq!(line.tap_linear(100.0), line.tap(DelayLine::<16>::MAX_DELAY));
    }

    #[test]
    fn cubic_is_flatter_than_linear_at_half_a_sample() {
        // A sine at a quarter of the sample rate, read half a sample late:
        // linear loses 3 dB, cubic much less.
        let mut line = DelayLine::<64>::default();
        let mut peak = (0.0_f32, 0.0_f32);
        for n in 0..64_u8 {
            line.push(euterpe_core::fastmath::sin_turns(
                f32::from(n) * 0.25 + 0.125,
            ));
            if n > 8 {
                peak.0 = peak.0.max(line.tap_linear(2.5).abs());
                peak.1 = peak.1.max(line.tap_cubic(2.5).abs());
            }
        }
        assert!(
            (peak.0 - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "{peak:?}"
        );
        assert!(peak.1 > 0.85, "{peak:?}");
    }
}
