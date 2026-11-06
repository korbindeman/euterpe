//! Control-rate parameter smoothing.
//!
//! A [`Smoother`] is advanced once per block and hands out a [`Ramp`]: a start
//! value and a per-sample increment. Inside the sample loop a ramp costs one
//! add, and nothing is recomputed per sample.

use crate::processor::{BLOCK, Ctx};

/// A linear ramp across one block. Call [`Ramp::tick`] once per sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    pub value: f32,
    pub step: f32,
}

impl Ramp {
    pub const fn constant(value: f32) -> Self {
        Self { value, step: 0.0 }
    }

    /// The value for this sample, then advance.
    #[inline(always)]
    pub fn tick(&mut self) -> f32 {
        let v = self.value;
        self.value += self.step;
        v
    }

    /// True if the value does not change across the block. Processors can use
    /// this to take a cheaper path.
    #[inline]
    pub fn is_constant(&self) -> bool {
        self.step == 0.0
    }
}

/// Moves linearly toward a target over a fixed time, in whole blocks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smoother {
    current: f32,
    target: f32,
    /// Per-sample increment of the ramp in progress.
    increment: f32,
    /// Samples left in the ramp in progress.
    remaining: u32,
    /// Ramp length in samples, set by [`Smoother::prepare`].
    length: u32,
    /// False until the first target arrives; that target is taken without a ramp.
    primed: bool,
}

impl Default for Smoother {
    fn default() -> Self {
        Self::new()
    }
}

impl Smoother {
    /// A smoother that jumps straight to the first target it is given.
    pub const fn new() -> Self {
        Self {
            current: 0.0,
            target: 0.0,
            increment: 0.0,
            remaining: 0,
            length: 0,
            primed: false,
        }
    }

    /// Set the ramp time. 0 ms jumps straight to new targets.
    pub fn prepare(&mut self, ctx: &Ctx, ms: f32) {
        self.length = ctx.ms_to_samples(ms).max(0.0) as u32;
    }

    /// Forget the current value: the next target is taken without a ramp.
    /// Call this from a processor's `reset`.
    pub fn reset(&mut self) {
        self.primed = false;
        self.remaining = 0;
        self.increment = 0.0;
    }

    /// Jump to `value` with no ramp.
    pub fn snap(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.remaining = 0;
        self.increment = 0.0;
        self.primed = true;
    }

    /// The value at the start of the next block.
    pub fn current(&self) -> f32 {
        self.current
    }

    /// True while a ramp is in progress.
    pub fn is_ramping(&self) -> bool {
        self.remaining > 0
    }

    /// Advance one block toward `target` and return the ramp for that block.
    ///
    /// The first target after [`Smoother::new`] or [`Smoother::reset`] is
    /// taken immediately. A new target after that restarts the ramp from the current value. The final block
    /// of a ramp lands exactly on the target.
    #[inline]
    pub fn block(&mut self, target: f32) -> Ramp {
        if !self.primed {
            self.snap(target);
        } else if target != self.target {
            self.target = target;
            if self.length == 0 {
                self.snap(target);
            } else {
                self.remaining = self.length;
                self.increment = (target - self.current) / self.length as f32;
            }
        }

        if self.remaining == 0 {
            return Ramp::constant(self.current);
        }

        let start = self.current;
        let end = if self.remaining as usize <= BLOCK {
            self.remaining = 0;
            self.target
        } else {
            self.remaining -= BLOCK as u32;
            start + self.increment * BLOCK as f32
        };
        self.current = end;
        Ramp {
            value: start,
            step: (end - start) / BLOCK as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut Smoother, target: f32) -> [f32; BLOCK] {
        let mut r = s.block(target);
        core::array::from_fn(|_| r.tick())
    }

    #[test]
    fn reaches_target_exactly() {
        let ctx = Ctx::new(48_000.0);
        let mut s = Smoother::new();
        s.snap(0.0);
        s.prepare(&ctx, 10.0); // 480 samples = 15 blocks
        let mut blocks = 0;
        while {
            run(&mut s, 1.0);
            blocks += 1;
            s.is_ramping()
        } {}
        assert_eq!(blocks, 15);
        assert_eq!(s.current(), 1.0);
        assert!(run(&mut s, 1.0).iter().all(|&v| v == 1.0));
    }

    #[test]
    fn ramp_is_monotonic_and_continuous() {
        let ctx = Ctx::new(48_000.0);
        let mut s = Smoother::new();
        s.snap(0.0);
        s.prepare(&ctx, 5.0);
        let mut last = 0.0;
        for _ in 0..20 {
            for v in run(&mut s, 2.0) {
                assert!(v >= last);
                assert!(v - last < 0.01);
                last = v;
            }
        }
    }

    #[test]
    fn retarget_mid_ramp() {
        let ctx = Ctx::new(48_000.0);
        let mut s = Smoother::new();
        s.snap(0.0);
        s.prepare(&ctx, 10.0);
        run(&mut s, 1.0);
        let mid = s.current();
        let first = run(&mut s, -1.0)[0];
        assert_eq!(
            first, mid,
            "a new target must continue from the current value"
        );
        for _ in 0..20 {
            run(&mut s, -1.0);
        }
        assert_eq!(s.current(), -1.0);
    }

    #[test]
    fn first_target_after_reset_is_immediate() {
        let mut s = Smoother::new();
        s.prepare(&Ctx::new(48_000.0), 50.0);
        assert!(run(&mut s, 0.7).iter().all(|&v| v == 0.7));
        run(&mut s, 0.2); // starts a ramp
        s.reset();
        assert!(run(&mut s, -0.3).iter().all(|&v| v == -0.3));
    }

    #[test]
    fn zero_time_snaps() {
        let mut s = Smoother::new();
        s.snap(0.0);
        s.prepare(&Ctx::new(48_000.0), 0.0);
        assert!(run(&mut s, 3.0).iter().all(|&v| v == 3.0));
    }
}
