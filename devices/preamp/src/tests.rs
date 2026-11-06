//! The port, checked two ways on real guitar DI:
//!
//! 1. Against [`reference`]: each legacy algorithm transcribed in f64 with
//!    exact math. This is the tight check (better than -85 dB per stage):
//!    it says the port does what the legacy code meant to do.
//! 2. Against the legacy code itself, loosely. The legacy `dsp` crate is
//!    `no_std` and gets its math from micromath, whose `log10` is off by up
//!    to 1 dB (it reads -10 dB as -10.8 dB) and whose `cos` is too coarse
//!    for low-frequency biquads. So the legacy compressors' thresholds and
//!    the legacy EQ's low bands are audibly off from their own design, and
//!    the port does not reproduce those errors. These tests pin how far
//!    apart the two are.
//!
//! Residuals are relative to the reference's (or legacy's) RMS.

#![allow(
    clippy::float_cmp,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "test code: exact comparisons and casts between f32, f64 and indices are deliberate"
)]

extern crate std;

use std::vec::Vec;

use dsp::Process;
use euterpe_offline::analyze::{NullResult, null};
use euterpe_offline::check::{CheckOptions, check_init, check_processor};
use euterpe_offline::{RenderOptions, render};

use super::*;
use crate::stages::{EQ_FREQS, Fet, FetParams, Standard, StandardParams, Vca, VcaParams};

/// Guitar DI from the committed fixtures, 2 s at 0.2 peak (below the plug
/// pop detector), so the dynamics stages have something to do.
fn guitar() -> Vec<f32> {
    let fixtures = euterpe_fixtures::load_all().expect("committed fixtures load");
    let f = fixtures
        .iter()
        .find(|f| f.sample_rate == 48_000 && f.di.len() >= 96_000)
        .expect("a 2 s, 48 kHz fixture");
    let peak = f.di[..96_000].iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    f.di[..96_000].iter().map(|&s| s * 0.2 / peak).collect()
}

fn legacy(mut p: impl Process, x: &[f32]) -> Vec<f32> {
    x.iter().map(|&s| p.process(s)).collect()
}

fn ours<P: Processor + Default>(p: &P::Params, x: &[f32]) -> Vec<f32> {
    render(&mut P::default(), p, x, &RenderOptions::new(48_000))
}

fn vs(new: &[f32], want: &[f64]) -> NullResult {
    let want: Vec<f32> = want.iter().map(|&v| v as f32).collect();
    null(new, &want, false)
}

/// The tight check and the loose one, for one stage.
fn check(name: &str, new: &[f32], reference: &[f64], old: &[f32], max_db: f64, legacy_db: f64) {
    let r = vs(new, reference);
    assert!(r.residual_rms_db < max_db, "{name} vs reference: {r:?}");
    let l = null(new, old, false);
    assert!(l.residual_rms_db < legacy_db, "{name} vs legacy: {l:?}");
}

/// The legacy algorithms in f64, with exact math.
mod reference {
    extern crate std;
    use std::vec::Vec;

    const SR: f64 = 48_000.0;

    pub fn db_to_lin(db: f64) -> f64 {
        10.0_f64.powf(db / 20.0)
    }
    pub fn lin_to_db(x: f64) -> f64 {
        20.0 * x.max(1e-12).log10()
    }
    pub fn tc(ms: f64) -> f64 {
        (-1.0 / (ms * 0.001 * SR)).exp()
    }
    fn follow(state: &mut f64, target: f64, attack: f64, release: f64) -> f64 {
        let c = if target > *state { attack } else { release };
        *state = c * *state + (1.0 - c) * target;
        *state
    }

    pub fn gate(x: &[f32], threshold_db: f64) -> Vec<f64> {
        let (open_above, close_below) = (db_to_lin(threshold_db), db_to_lin(threshold_db - 3.0));
        let (mut env, mut gain, mut open) = (0.0, 1.0, true);
        x.iter()
            .map(|&s| {
                let s = f64::from(s);
                let e = follow(&mut env, s.abs(), tc(1.0), tc(50.0));
                open = if open {
                    e >= close_below
                } else {
                    e > open_above
                };
                s * follow(&mut gain, f64::from(u8::from(open)), tc(5.0), tc(20.0))
            })
            .collect()
    }

    pub fn standard(x: &[f32], thr: f64, ratio: f64, attack: f64, release: f64) -> Vec<f64> {
        let mut env = 0.0;
        x.iter()
            .map(|&s| {
                let s = f64::from(s);
                let level = lin_to_db(follow(&mut env, s.abs(), tc(attack), tc(release)));
                let (w, slope) = (6.0, 1.0 - 1.0 / ratio);
                let g = if level <= thr - w / 2.0 {
                    0.0
                } else if level >= thr + w / 2.0 {
                    -(level - thr) * slope
                } else {
                    let v = level - thr + w / 2.0;
                    -slope * v * v / (2.0 * w)
                };
                s * db_to_lin(g)
            })
            .collect()
    }

    pub fn fet(x: &[f32], input_db: f64, ratio: f64, attack: f64, release: f64) -> Vec<f64> {
        let mut env = 0.0_f64;
        x.iter()
            .map(|&s| {
                let driven = f64::from(s) * db_to_lin(input_db);
                let env_db = lin_to_db(env);
                let over = env_db + 20.0;
                let g = if over <= 0.0 {
                    0.0
                } else {
                    over / ratio - over
                };
                let y = driven * db_to_lin(g);
                let hard = (over / 20.0).clamp(0.0, 1.0);
                let (a, r) = (
                    tc(attack) * (1.0 - 0.5 * hard),
                    tc(release) * (1.0 - 0.3 * hard),
                );
                follow(&mut env, y.abs(), a, r);
                y
            })
            .collect()
    }

    pub fn vca(x: &[f32], compression: f64) -> Vec<f64> {
        let (mut ms, mut gr) = (0.0_f64, 0.0_f64);
        let thr = -compression * 60.0;
        x.iter()
            .map(|&s| {
                let s = f64::from(s);
                ms = tc(15.0) * ms + (1.0 - tc(15.0)) * s * s;
                let level = lin_to_db(ms.sqrt());
                let over = level - thr;
                let target = if over <= -15.0 {
                    0.0
                } else if over < 15.0 {
                    let n = (over + 15.0) / 30.0;
                    n * n * over.max(0.0)
                } else {
                    over - 15.0
                };
                gr = if target > gr {
                    tc(20.0) * gr + (1.0 - tc(20.0)) * target
                } else {
                    (gr - 120.0 / SR).max(target)
                };
                s * db_to_lin(-gr)
            })
            .collect()
    }

    /// RBJ cookbook biquad, direct form I, as the legacy `Biquad`.
    pub struct Biquad([f64; 5], [f64; 4]);

    impl Biquad {
        fn new(b: [f64; 3], a: [f64; 3]) -> Self {
            Self(
                [
                    b[0] / a[0],
                    b[1] / a[0],
                    b[2] / a[0],
                    a[1] / a[0],
                    a[2] / a[0],
                ],
                [0.0; 4],
            )
        }
        pub fn shelf(low: bool, f: f64, db: f64) -> Self {
            let a = 10.0_f64.powf(db / 40.0);
            let w = core::f64::consts::TAU * f / SR;
            let (c, sa) = (
                w.cos(),
                2.0 * a.sqrt() * w.sin() / core::f64::consts::SQRT_2,
            );
            let k = if low { 1.0 } else { -1.0 };
            Self::new(
                [
                    a * ((a + 1.0) - k * (a - 1.0) * c + sa),
                    k * 2.0 * a * ((a - 1.0) - k * (a + 1.0) * c),
                    a * ((a + 1.0) - k * (a - 1.0) * c - sa),
                ],
                [
                    (a + 1.0) + k * (a - 1.0) * c + sa,
                    -k * 2.0 * ((a - 1.0) + k * (a + 1.0) * c),
                    (a + 1.0) + k * (a - 1.0) * c - sa,
                ],
            )
        }
        pub fn peak(f: f64, db: f64, q: f64) -> Self {
            let a = 10.0_f64.powf(db / 40.0);
            let w = core::f64::consts::TAU * f / SR;
            let alpha = w.sin() / (2.0 * q);
            Self::new(
                [1.0 + alpha * a, -2.0 * w.cos(), 1.0 - alpha * a],
                [1.0 + alpha / a, -2.0 * w.cos(), 1.0 - alpha / a],
            )
        }
        pub fn tick(&mut self, x: f64) -> f64 {
            let [b0, b1, b2, a1, a2] = self.0;
            let [x1, x2, y1, y2] = self.1;
            let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
            self.1 = [x, x1, y, y1];
            y
        }
    }

    pub fn eq(x: &[f32], freqs: [f32; 4], gains: [f64; 3]) -> Vec<f64> {
        let f = freqs.map(f64::from);
        let mut bands = [
            Biquad::shelf(true, f[0], gains[0]),
            Biquad::peak(f[1], gains[1], 1.0),
            Biquad::shelf(false, f[2], gains[2]),
            Biquad::shelf(false, f[3], 6.0),
        ];
        x.iter()
            .map(|&s| bands.iter_mut().fold(f64::from(s), |v, b| b.tick(v)))
            .collect()
    }

    pub fn saturator(x: &[f32], drive_db: f64) -> Vec<f64> {
        x.iter()
            .map(|&s| (f64::from(s) * db_to_lin(drive_db)).tanh() / 1.0_f64.tanh())
            .collect()
    }

    pub fn chorus(x: &[f32], rate: f64, depth: f64, mix: f64) -> Vec<f64> {
        let mut phase = 0.0_f64;
        (0..x.len())
            .map(|n| {
                let lfo = (core::f64::consts::TAU * phase).sin();
                phase += rate / SR;
                if phase >= 1.0 {
                    phase -= 1.0;
                }
                let d = 0.007 * SR + lfo * 0.003 * SR * depth;
                // The legacy line reads `d` back from its write position,
                // which is one past the sample just written.
                let t = (n + 1) as f64 - d;
                let i = t.floor();
                let at = |k: f64| {
                    if k >= 0.0 {
                        f64::from(x[k as usize])
                    } else {
                        0.0
                    }
                };
                let wet = at(i) * (1.0 - (t - i)) + at(i + 1.0) * (t - i);
                f64::from(x[n]) * (1.0 - mix) + wet * mix
            })
            .collect()
    }

    pub fn limiter(x: &[f32], thr_db: f64, release: f64) -> Vec<f64> {
        let thr = db_to_lin(thr_db);
        let (mut env, mut gain) = (0.0, 1.0);
        x.iter()
            .map(|&s| {
                let s = f64::from(s);
                let e = follow(&mut env, s.abs(), tc(0.5), tc(release));
                let target = if e > thr { thr / e } else { 1.0 };
                s * follow(&mut gain, target, tc(2.0), tc(2.0))
            })
            .collect()
    }

    /// DC blocker and pop suppressor; assumes no pops (the test signals
    /// have none).
    pub fn conditioner(x: &[f32]) -> Vec<f64> {
        let pole = (-core::f64::consts::TAU * 20.0 / SR).exp();
        let (mut x1, mut y1) = (0.0, 0.0);
        x.iter()
            .map(|&s| {
                let s = f64::from(s);
                let y = s - x1 + pole * y1;
                x1 = s;
                y1 = y;
                y
            })
            .collect()
    }

    pub fn convolve(x: &[f64], h: &[f32]) -> Vec<f64> {
        (0..x.len())
            .map(|n| {
                h.iter()
                    .take(n + 1)
                    .enumerate()
                    .map(|(k, &c)| f64::from(c) * x[n - k])
                    .sum()
            })
            .collect()
    }

    pub fn to_f32(x: &[f64]) -> Vec<f32> {
        x.iter().map(|&v| v as f32).collect()
    }
}

#[test]
fn passes_invariants() {
    check_processor::<Preamp>(&CheckOptions::default()).unwrap();
    check_init::<PreampDevice>(&CheckOptions::default()).unwrap();
}

#[test]
fn schema_ids_match_the_legacy_model() {
    let ids: Vec<&str> = PreampParams::SCHEMA.iter().map(|d| d.id).collect();
    for legacy_id in [
        "mode",
        "channel.gain",
        "gate.enabled",
        "gate.threshold",
        "compressor.type",
        "compressor.enabled",
        "compressor.standard.threshold",
        "compressor.standard.ratio",
        "compressor.1176.input",
        "compressor.1176.ratio",
        "compressor.163.compression",
        "eq.low.enabled",
        "eq.low.gain",
        "eq.air.enabled",
        "saturator.drive",
        "cab.enabled",
        "chorus.mix",
        "limiter.threshold",
        "limiter.release",
    ] {
        assert!(ids.contains(&legacy_id), "missing {legacy_id}");
    }
    let p = PreampParams::default();
    assert_eq!(p.compressor.kind, 2, "the 163 is the default, as before");
    assert!(p.cab.enabled && !p.saturator.enabled && !p.chorus.enabled);
}

#[test]
fn gate_matches() {
    let mut x = guitar();
    // A quiet second half, so the gate closes and opens.
    x.iter_mut().skip(48_000).for_each(|s| *s *= 0.0005);
    let mut old = dsp::gate::NoiseGate::new(48_000);
    old.params_mut().threshold.set(-50.0);
    let p = GateParams {
        enabled: true,
        threshold_db: -50.0,
    };
    let new = ours::<Gate>(&p, &x);
    check(
        "gate",
        &new,
        &reference::gate(&x, -50.0),
        &legacy(old, &x),
        -100.0,
        -90.0,
    );
}

#[test]
fn compressors_match() {
    let x = guitar();

    let mut old = dsp::compressor::Compressor::new(48_000);
    old.params_mut().threshold.set(-30.0);
    old.params_mut().ratio.set(6.0);
    old.params_mut().attack.set(5.0);
    let p = StandardParams {
        enabled: true,
        threshold_db: -30.0,
        ratio: 6.0,
        attack_ms: 5.0,
        release_ms: 100.0,
    };
    let want = reference::standard(&x, -30.0, 6.0, 5.0, 100.0);
    check(
        "standard",
        &ours::<Standard>(&p, &x),
        &want,
        &legacy(old, &x),
        -90.0,
        -20.0,
    );

    let mut old = dsp::comp_1176::Comp1176::new(48_000);
    old.params_mut().input.set(20.0);
    old.params_mut().set_ratio(dsp::comp_1176::Ratio1176::R8);
    let p = FetParams {
        input_db: 20.0,
        ratio: 1,
        ..Default::default()
    };
    let want = reference::fet(&x, 20.0, 8.0, 0.4, 300.0);
    check(
        "1176",
        &ours::<Fet>(&p, &x),
        &want,
        &legacy(old, &x),
        -95.0,
        -15.0,
    );

    let mut old = dsp::comp_163::Comp163::new(48_000);
    old.params_mut().compression.set(0.7);
    let p = VcaParams {
        compression: 0.7,
        ..Default::default()
    };
    let want = reference::vca(&x, 0.7);
    check(
        "163",
        &ours::<Vca>(&p, &x),
        &want,
        &legacy(old, &x),
        -95.0,
        -15.0,
    );
}

#[test]
fn eq_matches_in_both_modes() {
    let x = guitar();
    for mode in [0_u8, 1] {
        let [lo, mid, hi, air] = EQ_FREQS[usize::from(mode)];
        let mut old = dsp::eq::ThreeBandEq::new(lo, mid, hi, air, 6.0, 1.0, 48_000.0);
        old.params_mut().low_gain.set(-4.0);
        old.params_mut().high_gain.set(9.0);
        let mut p = EqParams::default();
        p.low.gain_db = -4.0;
        p.high.gain_db = 9.0;
        let mut eq = Eq::default();
        eq.mode = mode;
        let new = render(&mut eq, &p, &x, &RenderOptions::new(48_000));
        let want = reference::eq(&x, EQ_FREQS[usize::from(mode)], [-4.0, 6.0, 9.0]);
        check("eq", &new, &want, &legacy(old, &x), -110.0, -15.0);
    }
}

#[test]
fn saturator_chorus_and_limiter_match() {
    let x = guitar();

    let mut old = dsp::saturator::Saturator::new();
    old.params_mut().enabled.set(true);
    let p = SaturatorParams {
        enabled: true,
        drive_db: 12.0,
    };
    let new = ours::<Saturator>(&p, &x);
    check(
        "saturator",
        &new,
        &reference::saturator(&x, 12.0),
        &legacy(old, &x),
        -100.0,
        -55.0,
    );

    let mut old = dsp::chorus::Chorus::new();
    old.params_mut().enabled.set(true);
    let p = ChorusParams {
        enabled: true,
        ..Default::default()
    };
    let new = ours::<Chorus>(&p, &x);
    check(
        "chorus",
        &new,
        &reference::chorus(&x, 0.5, 0.3, 0.3),
        &legacy(old, &x),
        -95.0,
        -50.0,
    );

    let loud: Vec<f32> = x.iter().map(|&s| s * 4.0).collect();
    let mut old = dsp::limiter::Limiter::new(48_000);
    old.params_mut().threshold.set(-6.0);
    let p = LimiterParams {
        enabled: true,
        threshold_db: -6.0,
        release_ms: 100.0,
    };
    let new = ours::<Limiter>(&p, &loud);
    let want = reference::limiter(&loud, -6.0, 100.0);
    check("limiter", &new, &want, &legacy(old, &loud), -90.0, -60.0);
}

/// The whole default channel (DC blocker, gate, the 163, EQ boosts, cab,
/// limiter) against the reference stages in series, and against the legacy
/// `Channel`, 256 samples later for the legacy cab's latency.
#[test]
fn channel_matches() {
    let x = guitar();
    let defaults = PreampParams::default();
    let new = ours::<Preamp>(&defaults, &x);

    let s = reference::conditioner(&x);
    let s = reference::gate(&reference::to_f32(&s), -70.0);
    let s = reference::vca(&reference::to_f32(&s), 0.5);
    let s = reference::eq(&reference::to_f32(&s), EQ_FREQS[0], [6.0, 6.0, 6.0]);
    let s = reference::convolve(&s, &euterpe_irs::DELUXE_REVERB);
    let want = reference::limiter(&reference::to_f32(&s), -1.0, 100.0);
    let r = vs(&new, &want);
    assert!(r.residual_rms_db < -80.0, "vs reference: {r:?}");

    let mut old = preamp::channel::Channel::new(48_000);
    old.set_ir(&euterpe_irs::DELUXE_REVERB);
    let old = legacy(old, &x);
    let l = null(&new[..new.len() - 256], &old[256..], false);
    assert!(l.residual_rms_db < -10.0, "vs legacy: {l:?}");
}

/// A plug pop mutes the input for 45 ms and fades it back in.
#[test]
fn plug_pop_mutes_and_recovers() {
    let mut x = euterpe_offline::signal::sine(220.0, 0.1, 24_000, 48_000);
    x[4_000] = 0.95;
    let mut p = PreampParams::default();
    // Just the conditioner and the gain.
    p.gate.enabled = false;
    p.compressor.enabled = false;
    p.eq.low.enabled = false;
    p.eq.mid.enabled = false;
    p.eq.high.enabled = false;
    p.eq.air.enabled = false;
    p.cab.enabled = false;
    p.limiter.enabled = false;
    let out = ours::<Preamp>(&p, &x);
    assert!(
        out[3_000..4_000].iter().any(|&s| s.abs() > 0.05),
        "playing before"
    );
    assert!(
        out[4_000..4_000 + 2_160].iter().all(|&s| s == 0.0),
        "muted for 45 ms"
    );
    let peak = |r: &[f32]| r.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    let (fading, back) = (peak(&out[7_000..7_500]), peak(&out[20_000..]));
    assert!(fading < 0.5 * back && back > 0.09, "{fading} {back}");
}
