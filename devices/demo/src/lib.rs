//! Demo devices. They are deliberately small: each shows the full pattern of
//! a processor (params, smoothing, prepare/reset/process) and a device.
//!
//! - `demo.gain`: a smoothed gain stage
//! - `demo.drive`: tanh saturation with drive and output level. Not
//!   oversampled, so it aliases at high drive.
//! - `demo.drive4x`: the same [`Drive`], run at 4x through
//!   [`euterpe_dsp::Oversampled4`]. Wrapping is the whole change.

#![no_std]
// An unsuffixed float literal falls back to f64, which is soft-float on the
// Cortex-M7. Code that runs on the device must make every literal's type clear.
#![cfg_attr(not(test), warn(clippy::default_numeric_fallback))]

use euterpe_core::{
    Block, Ctx, Device, ParamKind, ParamSet, Processor, Smoother, fastmath, params,
};

// ---------------------------------------------------------------------------
// Gain
// ---------------------------------------------------------------------------

params! {
    pub struct GainParams {
        gain_db: f32 = ("gain", "Gain", ParamKind::float(-60.0, 12.0, 0.0).unit("dB").smooth_ms(20.0)),
    }
}

/// A smoothed gain stage.
#[derive(Default)]
pub struct Gain {
    gain: Smoother,
}

impl Processor for Gain {
    type Params = GainParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.gain.prepare(ctx, GainParams::smooth_ms("gain"));
    }

    fn reset(&mut self) {
        self.gain.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, params: &GainParams) {
        let mut g = self.gain.block(fastmath::db_to_gain(params.gain_db));
        if g.is_constant() {
            let g = g.value;
            buf.iter_mut().for_each(|s| *s *= g);
        } else {
            buf.iter_mut().for_each(|s| *s *= g.tick());
        }
    }
}

/// `demo.gain`
pub struct GainDevice;

impl Device for GainDevice {
    const NAME: &'static str = "demo.gain";
    const DESCRIPTION: &'static str = "Smoothed gain stage, -60 to +12 dB.";
    type Graph = Gain;
}

// ---------------------------------------------------------------------------
// Drive
// ---------------------------------------------------------------------------

params! {
    pub struct DriveParams {
        drive_db: f32 = ("drive", "Drive", ParamKind::float(0.0, 36.0, 12.0).unit("dB").smooth_ms(20.0)),
        level_db: f32 = ("level", "Level", ParamKind::float(-24.0, 6.0, -6.0).unit("dB").smooth_ms(20.0)),
    }
}

/// Symmetric tanh saturation: `level * tanh(drive * x)`.
#[derive(Default)]
pub struct Drive {
    drive: Smoother,
    level: Smoother,
}

impl Processor for Drive {
    type Params = DriveParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.drive.prepare(ctx, DriveParams::smooth_ms("drive"));
        self.level.prepare(ctx, DriveParams::smooth_ms("level"));
    }

    fn reset(&mut self) {
        self.drive.reset();
        self.level.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, params: &DriveParams) {
        let mut drive = self.drive.block(fastmath::db_to_gain(params.drive_db));
        let mut level = self.level.block(fastmath::db_to_gain(params.level_db));
        for s in buf.iter_mut() {
            *s = fastmath::tanh(*s * drive.tick()) * level.tick();
        }
    }
}

/// `demo.drive`
pub struct DriveDevice;

impl Device for DriveDevice {
    const NAME: &'static str = "demo.drive";
    const DESCRIPTION: &'static str =
        "Tanh saturation with drive and output level (not oversampled).";
    type Graph = Drive;
}

/// `demo.drive4x`
pub struct Drive4xDevice;

impl Device for Drive4xDevice {
    const NAME: &'static str = "demo.drive4x";
    const DESCRIPTION: &'static str =
        "demo.drive at 4x oversampling: the same tanh, without the aliasing.";
    type Graph = euterpe_dsp::Oversampled4<Drive>;
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec;

    use super::*;
    use euterpe_core::ParamSet;
    use euterpe_offline::check::{CheckOptions, check_processor};
    use euterpe_offline::{RenderOptions, render, signal};

    #[test]
    fn gain_passes_invariants() {
        check_processor::<Gain>(&CheckOptions::default()).unwrap();
    }

    #[test]
    fn drive_passes_invariants() {
        check_processor::<Drive>(&CheckOptions::default()).unwrap();
        check_processor::<<Drive4xDevice as Device>::Graph>(&CheckOptions::default()).unwrap();
    }

    #[test]
    fn gain_starts_at_target_then_ramps_on_change() {
        let input = vec![1.0; 4_800];
        let mut params = GainParams::default();
        params.set_str("gain", "-6").unwrap();
        let half = fastmath::db_to_gain(-6.0);
        let out = render::render_with(
            &mut Gain::default(),
            &mut params,
            &input,
            &RenderOptions::new(48_000),
            |pos, p| {
                if pos == 2_400 {
                    p.set_str("gain", "0").unwrap();
                }
            },
        );
        // No ramp from the initial state: the first sample is already at -6 dB.
        assert!((out[0] - half).abs() < 1e-6);
        // The change at 2400 ramps over 20 ms (960 samples), not instantly.
        assert!((out[2_400] - half).abs() < 1e-6);
        assert!(out[2_400 + 480] > half && out[2_400 + 480] < 1.0);
        assert!(out[2_400 + 1_000..].iter().all(|&s| (s - 1.0).abs() < 1e-6));
    }

    #[test]
    fn drive_is_near_linear_for_small_signals() {
        let mut params = DriveParams::default();
        params.set_str("drive", "0").unwrap();
        params.set_str("level", "0").unwrap();
        let input = signal::sine(1_000.0, 0.001, 4_800, 48_000);
        let out = render(
            &mut Drive::default(),
            &params,
            &input,
            &RenderOptions::new(48_000),
        );
        for (x, y) in input.iter().zip(&out) {
            assert!((x - y).abs() < 1e-6);
        }
    }

    #[test]
    #[allow(
        clippy::large_stack_arrays,
        reason = "test-only input, 100 ms at 48 kHz"
    )]
    fn drive_saturates_to_level() {
        let mut params = DriveParams::default();
        params.set_str("drive", "36").unwrap();
        params.set_str("level", "-6").unwrap();
        let out = render(
            &mut Drive::default(),
            &params,
            &[1.0; 4_800],
            &RenderOptions::new(48_000),
        );
        let level = fastmath::db_to_gain(-6.0);
        assert!(out.iter().all(|&s| s <= level + 1e-6));
        assert!((out[4_799] - level).abs() < 1e-4);
    }

    #[test]
    fn gain_measures_flat_with_no_distortion() {
        use euterpe_offline::analyze::{self, Setup};
        let mut p = GainParams::default();
        p.set_str("gain", "-6").unwrap();
        let setup = Setup::default();
        for r in analyze::response::<Gain>(&p, &setup, &analyze::THIRD_OCTAVES, -20.0) {
            assert!((r.gain_db + 6.0).abs() < 1e-3, "{r:?}");
        }
        for t in analyze::thd::<Gain>(&p, &setup, 1_000.0, &analyze::THD_LEVELS) {
            assert!(t.thd_pct < 1e-4, "{t:?}");
        }
    }

    /// The baseline the `Oversampled` wrapper has to beat: tanh at high
    /// drive with no oversampling aliases audibly at 6-10 kHz.
    #[test]
    fn drive_aliases_at_high_frequencies_without_oversampling() {
        use euterpe_offline::analyze::{self, Setup};
        let mut p = DriveParams::default();
        p.set_str("drive", "24").unwrap();
        let points =
            analyze::aliasing::<Drive>(&p, &Setup::default(), &[1_000.0, 6_000.0, 10_000.0], -6.0);
        assert!(points[0].alias_db < points[2].alias_db, "{points:?}");
        for a in &points[1..] {
            assert!(a.alias_db > -60.0, "aliasing should be measurable: {a:?}");
        }
    }

    /// The same drive at 4x: audible aliasing drops by 40 dB or more where
    /// the plain one is worst (-13 dB at 6 kHz, -9 dB at 10 kHz).
    #[test]
    fn drive4x_cuts_audible_aliasing() {
        use euterpe_offline::analyze::{self, Setup};
        let mut p = DriveParams::default();
        p.set_str("drive", "24").unwrap();
        let freqs = [2_000.0, 6_000.0, 10_000.0];
        let setup = Setup::default();
        let plain = analyze::aliasing::<Drive>(&p, &setup, &freqs, -6.0);
        let os = analyze::aliasing::<<Drive4xDevice as Device>::Graph>(&p, &setup, &freqs, -6.0);
        for (a, b) in plain.iter().zip(&os) {
            assert!(b.audible_alias_db < -38.0, "{b:?}");
            assert!(
                b.audible_alias_db < a.audible_alias_db - 25.0,
                "{a:?} {b:?}"
            );
            // Same fundamental: the wrapper is transparent in the band.
            assert!((a.fundamental_db - b.fundamental_db).abs() < 0.01);
        }
    }
}
