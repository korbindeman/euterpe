//! `pedal.fuzz`: a lot of gain into an asymmetric clipper.
//!
//! The clipper is `tanh(g x + bias) - tanh(bias)`: the bias makes it clip
//! harder on one side (even harmonics, the fuzz "spit"), and subtracting
//! `tanh(bias)` keeps silence silent. It runs at 4x, so the harmonics of
//! 40 dB of gain don't fold back into the audible band. After it, a DC
//! blocker removes the offset the asymmetry leaves on a signal, then a
//! lowpass tone control and the output level run at the device rate.

use euterpe_core::params::Taper;
use euterpe_core::{
    Block, Ctx, Device, ParamKind, ParamSet, Params, Processor, Smoother, fastmath,
};
use euterpe_dsp::{DcBlocker, Oversampled4, Svf, SvfCoeffs, dc_blocker_coeff};

#[derive(Params, Clone, Debug, PartialEq)]
pub struct FuzzParams {
    #[param(id = "fuzz", label = "Fuzz", kind = ParamKind::float(0.0, 48.0, 30.0).unit("dB").smooth_ms(20.0))]
    pub fuzz_db: f32,
    #[param(id = "bias", label = "Bias", kind = ParamKind::float(0.0, 0.8, 0.3).smooth_ms(50.0))]
    pub bias: f32,
    #[param(
        id = "tone",
        label = "Tone",
        kind = ParamKind::float(500.0, 12_000.0, 3_000.0).unit("Hz").taper(Taper::Exponential).smooth_ms(20.0)
    )]
    pub tone_hz: f32,
    #[param(id = "level", label = "Level", kind = ParamKind::float(-36.0, 6.0, -12.0).unit("dB").smooth_ms(20.0))]
    pub level_db: f32,
}

/// The oversampled part: gain, the biased clipper and the DC blocker.
#[derive(Default)]
pub struct Clip {
    gain: Smoother,
    bias: Smoother,
    dc: DcBlocker,
    dc_coeff: f32,
}

/// Reads only `fuzz` and `bias`: sharing [`FuzzParams`] lets the fuzz hand
/// its params straight through the oversampler.
impl Processor for Clip {
    type Params = FuzzParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.gain.prepare(ctx, FuzzParams::smooth_ms("fuzz"));
        self.bias.prepare(ctx, FuzzParams::smooth_ms("bias"));
        self.dc_coeff = dc_blocker_coeff(10.0 * ctx.dt());
    }

    fn reset(&mut self) {
        self.gain.reset();
        self.bias.reset();
        self.dc.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &FuzzParams) {
        let mut gain = self.gain.block(fastmath::db_to_gain(p.fuzz_db));
        let mut bias = self.bias.block(p.bias);
        for s in buf.iter_mut() {
            let b = bias.tick();
            let y = fastmath::tanh(*s * gain.tick() + b) - fastmath::tanh(b);
            *s = self.dc.tick(self.dc_coeff, y);
        }
    }
}

/// The fuzz: [`Clip`] at 4x, then tone and level.
#[derive(Default)]
pub struct Fuzz {
    clip: Oversampled4<Clip>,
    tone: Svf,
    tone_hz: Smoother,
    level: Smoother,
    dt: f32,
}

impl Processor for Fuzz {
    type Params = FuzzParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.clip.prepare(ctx);
        self.tone_hz.prepare(ctx, FuzzParams::smooth_ms("tone"));
        self.level.prepare(ctx, FuzzParams::smooth_ms("level"));
        self.dt = ctx.dt();
    }

    fn reset(&mut self) {
        self.clip.reset();
        self.tone.reset();
        self.tone_hz.reset();
        self.level.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &FuzzParams) {
        self.clip.process(buf, p);
        let tone = SvfCoeffs::lowpass(self.tone_hz.block(p.tone_hz).value * self.dt, 0.6);
        let mut level = self.level.block(fastmath::db_to_gain(p.level_db));
        for s in buf.iter_mut() {
            *s = self.tone.tick(&tone, *s) * level.tick();
        }
    }

    fn latency(&self) -> usize {
        self.clip.latency()
    }
}

/// `pedal.fuzz`
pub struct FuzzDevice;

impl Device for FuzzDevice {
    const NAME: &'static str = "pedal.fuzz";
    const DESCRIPTION: &'static str =
        "Asymmetric tanh fuzz at 4x oversampling, with bias, a lowpass tone and level.";
    type Graph = Fuzz;
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "silence in, exact silence out")]
mod tests {
    extern crate std;
    use std::vec;

    use super::*;
    use euterpe_offline::analyze::{self, Setup};
    use euterpe_offline::check::{CheckOptions, check_processor};
    use euterpe_offline::{RenderOptions, render};

    #[test]
    fn passes_invariants() {
        check_processor::<Fuzz>(&CheckOptions::default()).unwrap();
        let out = render(
            &mut Fuzz::default(),
            &FuzzParams::default(),
            &vec![0.0; 4800],
            &RenderOptions::new(48_000),
        );
        assert!(out.iter().all(|&s| s == 0.0));
    }

    /// The bias is what makes even harmonics: none without it (tanh is odd),
    /// a strong second with it.
    #[test]
    fn bias_adds_even_harmonics() {
        let setup = Setup::default();
        let mut p = FuzzParams {
            fuzz_db: 12.0,
            bias: 0.0,
            tone_hz: 12_000.0,
            level_db: 0.0,
        };
        let odd = &analyze::thd::<Fuzz>(&p, &setup, 500.0, &[-20.0])[0];
        p.bias = 0.5;
        let even = &analyze::thd::<Fuzz>(&p, &setup, 500.0, &[-20.0])[0];
        let h2 = |t: &analyze::ThdPoint| t.harmonics_db[0].unwrap();
        assert!(h2(odd) < -90.0, "symmetric: H2 {} dB", h2(odd));
        assert!(h2(even) > -30.0, "biased: H2 {} dB", h2(even));
        // 12 dB into tanh at -20 dBFS is gentle: H3 matches the Taylor
        // series, (g a)^2 / 12 for amplitude a = 0.1, g = 4.
        let h3 = odd.harmonics_db[1].unwrap();
        let want = 20.0 * (0.4_f64 * 0.4 / 12.0).log10();
        assert!((h3 - want).abs() < 0.5, "H3 {h3} vs {want} dB");
    }

    /// Measured: at the default 30 dB, aliasing is at the noise floor for
    /// guitar fundamentals; without oversampling it is -56 dB at 1 kHz and
    /// -24 dB at 3 kHz. At the full 48 dB the clipper makes a near-square
    /// wave, whose harmonics around 192 kHz still fold back into the band
    /// (-36 dB at 3 kHz, vs -15 dB at 1x).
    #[test]
    fn oversampling_keeps_aliasing_down() {
        let setup = Setup::default();
        let p = FuzzParams {
            tone_hz: 12_000.0,
            ..FuzzParams::default()
        };
        for a in analyze::aliasing::<Fuzz>(&p, &setup, &[1_000.0, 3_000.0], -12.0) {
            assert!(
                a.audible_alias_db < -100.0,
                "{} Hz: {} dB",
                a.freq,
                a.audible_alias_db
            );
        }
        let p = FuzzParams { fuzz_db: 48.0, ..p };
        let a = &analyze::aliasing::<Fuzz>(&p, &setup, &[3_000.0], -12.0)[0];
        assert!(a.audible_alias_db < -33.0, "{} dB", a.audible_alias_db);
    }

    #[test]
    fn tone_is_a_lowpass() {
        let p = FuzzParams {
            fuzz_db: 0.0,
            bias: 0.0,
            tone_hz: 1_000.0,
            level_db: 0.0,
        };
        // At -40 dBFS and no gain the clipper is linear (tanh x ~ x).
        let r = analyze::response::<Fuzz>(&p, &Setup::default(), &[100.0, 1_000.0, 8_000.0], -40.0);
        // The DC blocker (10 Hz) and the Q 0.6 lowpass each take ~0.04 dB here.
        assert!(r[0].gain_db.abs() < 0.1, "{:?}", r[0]);
        assert!(r[2].gain_db < -30.0, "{:?}", r[2]);
    }
}
