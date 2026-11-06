//! `pedal.eq`: a bass shelf at 120 Hz, a mid bell from 250 Hz to 5 kHz and
//! a treble shelf at 4 kHz, each ±15 dB, then a level. Three [`Svf`]s in
//! series, their coefficients recomputed once per block from the smoothed
//! knobs.

use euterpe_core::params::Taper;
use euterpe_core::{
    Block, Ctx, Device, ParamKind, ParamSet, Params, Processor, Smoother, fastmath,
};
use euterpe_dsp::{Svf, SvfCoeffs};

/// Shelf corner frequencies, in Hz.
pub const BASS_HZ: f32 = 120.0;
pub const TREBLE_HZ: f32 = 4_000.0;
/// Q of the shelves (Butterworth: no bump at the corner) and of the bell.
pub const SHELF_Q: f32 = core::f32::consts::FRAC_1_SQRT_2;
pub const MID_Q: f32 = 0.8;

#[derive(Params, Clone, Debug, PartialEq)]
pub struct EqParams {
    #[param(id = "bass", label = "Bass", kind = ParamKind::float(-15.0, 15.0, 0.0).unit("dB").smooth_ms(20.0))]
    pub bass_db: f32,
    #[param(id = "mid", label = "Mid", kind = ParamKind::float(-15.0, 15.0, 0.0).unit("dB").smooth_ms(20.0))]
    pub mid_db: f32,
    #[param(
        id = "mid_freq",
        label = "Mid Freq",
        kind = ParamKind::float(250.0, 5_000.0, 800.0).unit("Hz").taper(Taper::Exponential).smooth_ms(50.0)
    )]
    pub mid_hz: f32,
    #[param(id = "treble", label = "Treble", kind = ParamKind::float(-15.0, 15.0, 0.0).unit("dB").smooth_ms(20.0))]
    pub treble_db: f32,
    #[param(id = "level", label = "Level", kind = ParamKind::float(-24.0, 12.0, 0.0).unit("dB").smooth_ms(20.0))]
    pub level_db: f32,
}

#[derive(Default)]
pub struct Eq {
    bands: [Svf; 3],
    bass: Smoother,
    mid: Smoother,
    mid_hz: Smoother,
    treble: Smoother,
    level: Smoother,
    dt: f32,
}

impl Processor for Eq {
    type Params = EqParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.bass.prepare(ctx, EqParams::smooth_ms("bass"));
        self.mid.prepare(ctx, EqParams::smooth_ms("mid"));
        self.mid_hz.prepare(ctx, EqParams::smooth_ms("mid_freq"));
        self.treble.prepare(ctx, EqParams::smooth_ms("treble"));
        self.level.prepare(ctx, EqParams::smooth_ms("level"));
        self.dt = ctx.dt();
    }

    fn reset(&mut self) {
        self.bands.iter_mut().for_each(Svf::reset);
        self.bass.reset();
        self.mid.reset();
        self.mid_hz.reset();
        self.treble.reset();
        self.level.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &EqParams) {
        let dt = self.dt;
        let coeffs = [
            SvfCoeffs::low_shelf(BASS_HZ * dt, SHELF_Q, self.bass.block(p.bass_db).value),
            SvfCoeffs::bell(
                self.mid_hz.block(p.mid_hz).value * dt,
                MID_Q,
                self.mid.block(p.mid_db).value,
            ),
            SvfCoeffs::high_shelf(
                TREBLE_HZ * dt,
                SHELF_Q,
                self.treble.block(p.treble_db).value,
            ),
        ];
        let mut level = self.level.block(fastmath::db_to_gain(p.level_db));
        for s in buf.iter_mut() {
            let mut x = *s;
            for (band, c) in self.bands.iter_mut().zip(&coeffs) {
                x = band.tick(c, x);
            }
            *s = x * level.tick();
        }
    }
}

/// `pedal.eq`
pub struct EqDevice;

impl Device for EqDevice {
    const NAME: &'static str = "pedal.eq";
    const DESCRIPTION: &'static str = "3-band EQ: bass shelf (120 Hz), sweepable mid bell (250 Hz - 5 kHz), treble shelf (4 kHz).";
    type Graph = Eq;
}

#[cfg(test)]
mod tests {
    use super::*;
    use euterpe_offline::analyze::{self, Setup};
    use euterpe_offline::check::{CheckOptions, check_processor};

    type C64 = (f64, f64);

    fn mul(a: C64, b: C64) -> C64 {
        (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
    }

    /// `|num(s) / den(s)|^2` at `s = j w`, coefficients from `s^0` up.
    fn mag2(num: [f64; 3], den: [f64; 3], w: f64) -> f64 {
        let at = |c: [f64; 3]| {
            let s = (0.0, w);
            let s2 = mul(s, s);
            (c[0] + c[2] * s2.0, c[1] * w + c[2] * s2.1)
        };
        let (n, d) = (at(num), at(den));
        (n.0 * n.0 + n.1 * n.1) / (d.0 * d.0 + d.1 * d.1)
    }

    /// The RBJ cookbook's analog prototypes, through the prewarped bilinear
    /// transform: an independent closed form for the whole EQ, in dB.
    fn closed_form_db(p: &EqParams, freq: f64) -> f64 {
        let warp = |fc: f64| {
            (core::f64::consts::PI * freq / 48e3).tan() / (core::f64::consts::PI * fc / 48e3).tan()
        };
        let shelf_q = f64::from(SHELF_Q);
        let bass = {
            let a = 10.0_f64.powf(f64::from(p.bass_db) / 40.0);
            let sa = a.sqrt();
            mag2(
                [a * a, a * sa / shelf_q, a],
                [1.0, sa / shelf_q, a],
                warp(f64::from(BASS_HZ)),
            )
        };
        let mid = {
            let a = 10.0_f64.powf(f64::from(p.mid_db) / 40.0);
            let q = f64::from(MID_Q);
            mag2(
                [1.0, a / q, 1.0],
                [1.0, 1.0 / (a * q), 1.0],
                warp(f64::from(p.mid_hz)),
            )
        };
        let treble = {
            let a = 10.0_f64.powf(f64::from(p.treble_db) / 40.0);
            let sa = a.sqrt();
            mag2(
                [a, a * sa / shelf_q, a * a],
                [a, sa / shelf_q, 1.0],
                warp(f64::from(TREBLE_HZ)),
            )
        };
        10.0 * (bass * mid * treble).log10() + f64::from(p.level_db)
    }

    #[test]
    fn passes_invariants() {
        check_processor::<Eq>(&CheckOptions::default()).unwrap();
    }

    #[test]
    fn flat_at_noon() {
        for r in analyze::response::<Eq>(
            &EqParams::default(),
            &Setup::default(),
            &analyze::THIRD_OCTAVES,
            -20.0,
        ) {
            assert!(r.gain_db.abs() < 1e-3, "{} Hz: {} dB", r.freq, r.gain_db);
        }
    }

    #[test]
    fn response_matches_the_closed_form() {
        let settings = [
            EqParams {
                bass_db: 9.0,
                mid_db: -6.0,
                mid_hz: 1_200.0,
                treble_db: 4.0,
                level_db: -3.0,
            },
            EqParams {
                bass_db: -15.0,
                mid_db: 15.0,
                mid_hz: 300.0,
                treble_db: -15.0,
                level_db: 0.0,
            },
        ];
        for p in &settings {
            for r in analyze::response::<Eq>(p, &Setup::default(), &analyze::THIRD_OCTAVES, -20.0) {
                let want = closed_form_db(p, r.freq);
                assert!(
                    (r.gain_db - want).abs() < 0.02,
                    "{} Hz: {} vs {want} dB",
                    r.freq,
                    r.gain_db
                );
            }
        }
    }
}
