//! The [`Processor`] trait and the block it operates on.

use crate::params::ParamSet;

/// Samples per block. Project-wide and fixed: changing it is a recompile.
///
/// 32 samples at 48 kHz is 0.67 ms.
pub const BLOCK: usize = 32;

/// One block of mono audio, processed in place.
pub type Block = [f32; BLOCK];

/// The context a processor runs in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ctx {
    /// The rate this processor runs at, in Hz. Inside an oversampling wrapper
    /// this is already multiplied by the oversampling factor.
    pub sample_rate: f32,
    /// Oversampling factor relative to the device rate (1 = none).
    pub oversampling: u32,
}

impl Ctx {
    pub const fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            oversampling: 1,
        }
    }

    /// Sample period in seconds.
    #[inline]
    pub fn dt(&self) -> f32 {
        1.0 / self.sample_rate
    }

    /// Convert a duration in milliseconds to samples at this rate.
    #[inline]
    pub fn ms_to_samples(&self, ms: f32) -> f32 {
        ms * 0.001 * self.sample_rate
    }

    /// The context for a processor running `factor` times faster than this one.
    #[must_use]
    pub const fn oversampled(&self, factor: u32) -> Self {
        Self {
            sample_rate: self.sample_rate * factor as f32,
            oversampling: self.oversampling * factor,
        }
    }
}

/// A unit of DSP work, processing one [`Block`] at a time, in place.
///
/// Rules for implementations (see the performance rules in the architecture spec):
/// no allocation, no panics and no dynamic dispatch inside [`Processor::process`].
pub trait Processor {
    /// The parameters this processor reads. Use `()` for none.
    type Params: ParamSet;

    /// Called once before processing, and again whenever the context changes.
    /// Compute rate-dependent coefficients here.
    fn prepare(&mut self, ctx: &Ctx);

    /// Clear all signal state (filter memory, delay lines). Keeps loaded assets.
    fn reset(&mut self);

    /// Process one block in place.
    ///
    /// `params` holds the current targets. Processors smooth them themselves
    /// with a [`crate::Smoother`] where clicks would be audible.
    fn process(&mut self, buf: &mut Block, params: &Self::Params);

    /// Latency in samples at this processor's rate.
    fn latency(&self) -> usize {
        0
    }
}

/// A processor that is naturally per-sample (feedback loops, simple filters).
///
/// Wrap it in [`PerSample`] to use it as a [`Processor`]. The compiler inlines
/// [`SampleProcessor::tick`] into the block loop.
pub trait SampleProcessor {
    type Params: ParamSet;

    fn prepare(&mut self, ctx: &Ctx);
    fn reset(&mut self);

    /// Called once per block before the ticks. Do control-rate work here
    /// (coefficient updates, starting smoothing ramps).
    #[inline]
    fn begin_block(&mut self, _params: &Self::Params) {}

    /// Process one sample.
    fn tick(&mut self, x: f32, params: &Self::Params) -> f32;

    fn latency(&self) -> usize {
        0
    }
}

/// Adapts a [`SampleProcessor`] into a [`Processor`].
#[derive(Clone, Debug, Default)]
pub struct PerSample<P>(pub P);

impl<P: SampleProcessor> Processor for PerSample<P> {
    type Params = P::Params;

    fn prepare(&mut self, ctx: &Ctx) {
        self.0.prepare(ctx);
    }

    fn reset(&mut self) {
        self.0.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, params: &Self::Params) {
        self.0.begin_block(params);
        for s in buf.iter_mut() {
            *s = self.0.tick(*s, params);
        }
    }

    fn latency(&self) -> usize {
        self.0.latency()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact, bit-identical results on purpose"
)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Doubler;

    impl SampleProcessor for Doubler {
        type Params = ();
        fn prepare(&mut self, _: &Ctx) {}
        fn reset(&mut self) {}
        fn tick(&mut self, x: f32, (): &()) -> f32 {
            x * 2.0
        }
    }

    #[test]
    fn per_sample_runs_every_sample() {
        let mut p = PerSample(Doubler);
        p.prepare(&Ctx::new(48_000.0));
        let mut buf: Block = core::array::from_fn(|i| i as f32);
        p.process(&mut buf, &());
        for (i, s) in buf.iter().enumerate() {
            assert_eq!(*s, i as f32 * 2.0);
        }
    }

    #[test]
    fn oversampled_ctx() {
        let ctx = Ctx::new(48_000.0).oversampled(4);
        assert_eq!(ctx.sample_rate, 192_000.0);
        assert_eq!(ctx.oversampling, 4);
    }
}
