//! `pedal.delay`: a feedback delay, up to 1 s.
//!
//! `y = x + mix * w`, where `w` is the line read `time` back, and the line
//! takes `x + feedback * lowpass(w)`. The dry signal stays at unity. The
//! delay time is smoothed slowly and read with linear interpolation, so
//! turning the knob glides the pitch like a tape delay instead of clicking.
//! The lowpass darkens each repeat; at its top setting (16 kHz) it is all
//! but transparent.

use euterpe_core::params::Taper;
use euterpe_core::place::Init;
use euterpe_core::{Block, Ctx, Device, ParamKind, ParamSet, Params, Processor, Smoother};
use euterpe_dsp::{DelayLine, OnePole, one_pole_coeff};

/// 2^16 samples: 1.36 s at 48 kHz.
const LINE: usize = 1 << 16;

#[derive(Params, Clone, Debug, PartialEq)]
pub struct DelayParams {
    #[param(id = "time", label = "Time", kind = ParamKind::float(20.0, 1_000.0, 350.0).unit("ms").smooth_ms(200.0))]
    pub time_ms: f32,
    #[param(id = "feedback", label = "Feedback", kind = ParamKind::float(0.0, 0.95, 0.4).smooth_ms(20.0))]
    pub feedback: f32,
    #[param(id = "mix", label = "Mix", kind = ParamKind::float(0.0, 1.0, 0.35).smooth_ms(20.0))]
    pub mix: f32,
    #[param(
        id = "tone",
        label = "Tone",
        kind = ParamKind::float(1_000.0, 16_000.0, 6_000.0).unit("Hz").taper(Taper::Exponential).smooth_ms(20.0)
    )]
    pub tone_hz: f32,
}

#[derive(Default)]
pub struct Delay {
    line: DelayLine<LINE>,
    tone: OnePole,
    time: Smoother,
    feedback: Smoother,
    mix: Smoother,
    tone_hz: Smoother,
    /// Samples per millisecond.
    per_ms: f32,
    dt: f32,
}

impl Processor for Delay {
    type Params = DelayParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.time.prepare(ctx, DelayParams::smooth_ms("time"));
        self.feedback
            .prepare(ctx, DelayParams::smooth_ms("feedback"));
        self.mix.prepare(ctx, DelayParams::smooth_ms("mix"));
        self.tone_hz.prepare(ctx, DelayParams::smooth_ms("tone"));
        self.per_ms = ctx.ms_to_samples(1.0);
        self.dt = ctx.dt();
    }

    fn reset(&mut self) {
        self.line.reset();
        self.tone.reset();
        self.time.reset();
        self.feedback.reset();
        self.mix.reset();
        self.tone_hz.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &DelayParams) {
        let mut time = self.time.block(p.time_ms);
        time.value *= self.per_ms;
        time.step *= self.per_ms;
        let mut feedback = self.feedback.block(p.feedback.clamp(0.0, 0.95));
        let mut mix = self.mix.block(p.mix);
        let tone = one_pole_coeff(self.tone_hz.block(p.tone_hz).value * self.dt);
        for s in buf.iter_mut() {
            let x = *s;
            // Read before this sample is pushed: `time - 1` back from the
            // newest is `time` back from this one.
            let w = self.line.tap_linear(time.tick() - 1.0);
            self.line
                .push(x + feedback.tick() * self.tone.lowpass(tone, w));
            *s = x + mix.tick() * w;
        }
    }
}

/// `pedal.delay`
pub struct DelayDevice;

impl Device for DelayDevice {
    const NAME: &'static str = "pedal.delay";
    const DESCRIPTION: &'static str =
        "Feedback delay up to 1 s with a darkening tone in the repeats; the time glides like tape.";
    type Graph = Delay;
    // SAFETY: every field of `Delay` defaults to zero: the line's samples
    // and position, the filter state, the smoothers (`Smoother::new` is all
    // zeros) and the rates, which `prepare` sets. `check_init` tests it.
    const GRAPH_INIT: Init = unsafe { Init::zeroed() };
    const PANEL: &'static [euterpe_core::panel::Binding] = &[
        euterpe_core::panel::pot(0, "time"),
        euterpe_core::panel::pot(1, "feedback"),
        euterpe_core::panel::pot(2, "mix"),
        euterpe_core::panel::pot(3, "tone"),
    ];
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

    #[test]
    fn passes_invariants() {
        check_processor::<Delay>(&CheckOptions::default()).unwrap();
        euterpe_offline::check::check_init::<DelayDevice>(&CheckOptions::default()).unwrap();
    }

    /// An impulse comes back every `time`, each repeat `feedback` times the
    /// last. The tone filter and the interpolation smear a repeat over a few
    /// samples but keep its sum (both have unity gain at DC), so each
    /// repeat's sum is exact: `mix * feedback^(k-1)`.
    #[test]
    fn repeats_land_on_time_and_decay_by_feedback() {
        let p = DelayParams {
            time_ms: 100.0,
            feedback: 0.5,
            mix: 0.8,
            tone_hz: 3_000.0,
        };
        let mut x = vec![0.0; 48_000];
        x[0] = 1.0;
        let out = render(&mut Delay::default(), &p, &x, &RenderOptions::new(48_000));
        assert_eq!(out[0], 1.0, "dry at unity");
        assert!(
            out[1..4_800].iter().all(|&s| s == 0.0),
            "nothing before the first repeat"
        );
        let sums: Vec<f64> = (1..6)
            .map(|k| {
                out[k * 4_800 - 100..k * 4_800 + 300]
                    .iter()
                    .map(|&s| f64::from(s))
                    .sum()
            })
            .collect();
        for (k, sum) in sums.iter().enumerate() {
            let want = 0.8 * 0.5_f64.powi(i32::try_from(k).unwrap());
            assert!(
                (sum - want).abs() < 1e-4,
                "repeat {}: {sum} vs {want}",
                k + 1
            );
        }
        // The first repeat is the impulse itself, a whole number of samples
        // late: no filtering yet.
        assert!((out[4_800] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn longest_time_fits_the_line() {
        let p = DelayParams {
            time_ms: 1_000.0,
            feedback: 0.0,
            mix: 1.0,
            ..DelayParams::default()
        };
        let mut x = vec![0.0; 50_000];
        x[0] = 1.0;
        let out = render(&mut Delay::default(), &p, &x, &RenderOptions::new(48_000));
        assert!((out[48_000] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn silence_stays_silent() {
        let out = render(
            &mut Delay::default(),
            &DelayParams::default(),
            &vec![0.0; 9_600],
            &RenderOptions::new(48_000),
        );
        assert!(out.iter().all(|&s| s == 0.0));
    }
}
