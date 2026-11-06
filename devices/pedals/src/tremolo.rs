//! `pedal.tremolo`: the signal times `1 - depth * (1 - lfo) / 2`, an LFO
//! scaled to swing the gain between `1 - depth` and 1.

use euterpe_core::params::Taper;
use euterpe_core::{Block, Ctx, Device, ParamKind, ParamSet, Params, Processor, Smoother};
use euterpe_dsp::{Phasor, Shape};

#[derive(Params, Clone, Debug, PartialEq)]
pub struct TremoloParams {
    #[param(
        id = "rate",
        label = "Rate",
        kind = ParamKind::float(0.5, 15.0, 5.0).unit("Hz").taper(Taper::Exponential).smooth_ms(100.0)
    )]
    pub rate_hz: f32,
    #[param(id = "depth", label = "Depth", kind = ParamKind::float(0.0, 1.0, 0.6).smooth_ms(20.0))]
    pub depth: f32,
    #[param(id = "shape", label = "Shape", kind = ParamKind::choice(Shape::LABELS, 0))]
    pub shape: u8,
}

#[derive(Default)]
pub struct Tremolo {
    lfo: Phasor,
    rate: Smoother,
    depth: Smoother,
    dt: f32,
}

impl Processor for Tremolo {
    type Params = TremoloParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.rate.prepare(ctx, TremoloParams::smooth_ms("rate"));
        self.depth.prepare(ctx, TremoloParams::smooth_ms("depth"));
        self.dt = ctx.dt();
    }

    fn reset(&mut self) {
        self.lfo.reset();
        self.rate.reset();
        self.depth.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &TremoloParams) {
        let shape = Shape::from_index(p.shape);
        let mut inc = self.rate.block(p.rate_hz);
        inc.value *= self.dt;
        inc.step *= self.dt;
        let mut depth = self.depth.block(p.depth.clamp(0.0, 1.0));
        for s in buf.iter_mut() {
            let lfo = shape.at(self.lfo.tick(inc.tick()));
            *s *= 1.0 - depth.tick() * 0.5 * (1.0 - lfo);
        }
    }
}

/// `pedal.tremolo`
pub struct TremoloDevice;

impl Device for TremoloDevice {
    const NAME: &'static str = "pedal.tremolo";
    const DESCRIPTION: &'static str =
        "Tremolo: an LFO (sine, triangle, square, saw) on the gain, with rate and depth.";
    type Graph = Tremolo;
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "depth 0 is exactly unity")]
mod tests {
    extern crate std;
    use std::vec;
    use std::vec::Vec;

    use super::*;
    use euterpe_offline::check::{CheckOptions, check_processor};
    use euterpe_offline::{RenderOptions, render};

    fn run(p: &TremoloParams, input: &[f32]) -> Vec<f32> {
        render(
            &mut Tremolo::default(),
            p,
            input,
            &RenderOptions::new(48_000),
        )
    }

    #[test]
    fn passes_invariants() {
        check_processor::<Tremolo>(&CheckOptions::default()).unwrap();
    }

    #[test]
    fn depth_zero_is_exactly_unity() {
        let p = TremoloParams {
            depth: 0.0,
            ..TremoloParams::default()
        };
        let x: Vec<f32> = (0..4800_u16)
            .map(|i| f32::from(i % 97) / 97.0 - 0.5)
            .collect();
        assert_eq!(run(&p, &x), x);
    }

    /// On DC, the output is the gain curve itself: compare it with the
    /// closed form `1 - d (1 - sin(2 pi r t)) / 2`.
    #[test]
    fn sine_gain_follows_the_closed_form() {
        let p = TremoloParams {
            rate_hz: 4.0,
            depth: 0.8,
            shape: 0,
        };
        let out = run(&p, &vec![1.0; 48_000]);
        for (n, &y) in out.iter().enumerate() {
            let t = n as f64 / 48_000.0;
            let want = 1.0 - 0.8 * (1.0 - (core::f64::consts::TAU * 4.0 * t).sin()) / 2.0;
            assert!(
                (f64::from(y) - want).abs() < 1e-4,
                "sample {n}: {y} vs {want}"
            );
        }
        let lo = out.iter().copied().fold(1.0, f32::min);
        assert!((lo - 0.2).abs() < 1e-4, "swings down to 1 - depth: {lo}");
    }

    #[test]
    fn square_at_full_depth_gates() {
        let p = TremoloParams {
            rate_hz: 2.0,
            depth: 1.0,
            shape: 2,
        };
        let out = run(&p, &vec![1.0; 24_000]);
        // Quarter and three-quarter period: fully on, fully off.
        assert!((out[6_000] - 1.0).abs() < 1e-6);
        assert!(out[18_000].abs() < 1e-6);
    }
}
