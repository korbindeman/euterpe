//! `#[derive(Params)]` and `#[derive(Chain)]`, used the way a device crate
//! uses them.

#![allow(
    clippy::float_cmp,
    reason = "tests compare exact, bit-identical results on purpose"
)]

use euterpe_core::params::Taper;
use euterpe_core::{
    BLOCK, Block, Chain, Ctx, ParamDesc, ParamError, ParamKind, ParamSet, ParamValue, Params,
    Processor, params,
};

const MODES: &[&str] = &["Clean", "Crunch", "Lead"];

#[derive(Params, Clone, Debug, PartialEq)]
struct Tone {
    #[param(label = "Bass", kind = ParamKind::float(-12.0, 12.0, 0.0).unit("dB"))]
    bass: f32,
    #[param(
        id = "freq",
        label = "Frequency",
        kind = ParamKind::float(20.0, 20_000.0, 1000.0).unit("Hz").taper(Taper::Exponential)
    )]
    freq_hz: f32,
}

#[derive(Params, Clone, Debug, PartialEq)]
struct Amp {
    #[param(id = "drive", label = "Drive", kind = ParamKind::float(0.0, 36.0, 12.0).smooth_ms(20.0))]
    drive_db: f32,
    #[param(label = "Bright", kind = ParamKind::toggle(true))]
    bright: bool,
    #[param(label = "Mode", kind = ParamKind::choice(MODES, 1))]
    mode: u8,
    #[param(nested)]
    tone: Tone,
    #[param(nested = "post")]
    post_tone: Tone,
}

params! {
    struct AmpByMacro {
        drive_db: f32 = ("drive", "Drive", ParamKind::float(0.0, 36.0, 12.0).smooth_ms(20.0)),
        bright: bool = ("bright", "Bright", ParamKind::toggle(true)),
        mode: u8 = ("mode", "Mode", ParamKind::choice(MODES, 1)),
    }
}

fn ids<P: ParamSet>() -> Vec<&'static str> {
    P::SCHEMA.iter().map(|d| d.id).collect()
}

#[test]
fn derive_matches_the_macro_for_leaf_fields() {
    let derived: Vec<ParamDesc> = Amp::SCHEMA[..3].to_vec();
    assert_eq!(derived, AmpByMacro::SCHEMA);
    let a = Amp::default();
    let b = AmpByMacro::default();
    assert_eq!(
        (a.drive_db, a.bright, a.mode),
        (b.drive_db, b.bright, b.mode)
    );
    assert_eq!(Amp::smooth_ms("drive"), 20.0);
}

#[test]
fn nested_sets_are_prefixed() {
    assert_eq!(
        ids::<Amp>(),
        [
            "drive",
            "bright",
            "mode",
            "tone.bass",
            "tone.freq",
            "post.bass",
            "post.freq"
        ]
    );
    let freq = Amp::desc("post.freq").unwrap();
    assert_eq!(freq.label, "Frequency");
    assert_eq!(freq.kind, Tone::desc("freq").unwrap().kind);
}

#[test]
fn get_and_set_reach_nested_fields() {
    let mut p = Amp::default();
    p.set_str("tone.bass", "6").unwrap();
    p.set("post.freq", ParamValue::Float(99_999.0)).unwrap();
    p.set_str("mode", "lead").unwrap();
    assert_eq!(p.tone.bass, 6.0);
    assert_eq!(
        p.post_tone.freq_hz, 20_000.0,
        "clamped by the nested schema"
    );
    assert_eq!(p.get("tone.bass"), Some(ParamValue::Float(6.0)));
    assert_eq!(p.get("post.bass"), Some(ParamValue::Float(0.0)));
    assert_eq!(p.mode, 2);
    assert_eq!(p.set_str("tone.nope", "1"), Err(ParamError::UnknownId));
    assert_eq!(p.get("bass"), None);
    for d in Amp::SCHEMA {
        assert_eq!(
            Amp::default().get(d.id),
            Some(d.kind.default_value()),
            "{}",
            d.id
        );
    }
}

// ---------------------------------------------------------------------------
// Chain
// ---------------------------------------------------------------------------

#[derive(Params, Clone)]
struct ScaleParams {
    #[param(label = "Gain", kind = ParamKind::float(0.0, 4.0, 2.0))]
    gain: f32,
}

/// Multiplies by its gain; records the rate it was prepared at.
#[derive(Default)]
struct Scale {
    rate: f32,
    resets: u32,
}

impl Processor for Scale {
    type Params = ScaleParams;
    fn prepare(&mut self, ctx: &Ctx) {
        self.rate = ctx.sample_rate;
    }
    fn reset(&mut self) {
        self.resets += 1;
    }
    fn process(&mut self, buf: &mut Block, p: &ScaleParams) {
        buf.iter_mut().for_each(|s| *s *= p.gain);
    }
}

/// Adds one; three samples of latency, to check the sum.
#[derive(Default)]
struct Offset;

impl Processor for Offset {
    type Params = ();
    fn prepare(&mut self, _: &Ctx) {}
    fn reset(&mut self) {}
    fn process(&mut self, buf: &mut Block, (): &()) {
        buf.iter_mut().for_each(|s| *s += 1.0);
    }
    fn latency(&self) -> usize {
        3
    }
}

#[derive(Chain, Default)]
struct Rig {
    pre: Scale,
    offset: Offset,
    post: Scale,
}

#[derive(Chain, Default)]
#[chain(params = OuterSettings)]
struct Outer {
    rig: Rig,
    tail: Offset,
}

#[test]
fn chain_runs_stages_in_field_order() {
    let mut rig = Rig::default();
    rig.prepare(&Ctx::new(48_000.0));
    let mut p = RigParams::default();
    p.set_str("pre.gain", "3").unwrap();
    p.set_str("post.gain", "0.5").unwrap();
    let mut buf = [1.0; BLOCK];
    rig.process(&mut buf, &p);
    // (1 * 3 + 1) * 0.5, not 1 * 0.5 * 3 + 1.
    assert!(buf.iter().all(|&s| s == 2.0));
    assert_eq!(rig.latency(), 3);
    assert_eq!((rig.pre.rate, rig.post.rate), (48_000.0, 48_000.0));
    rig.reset();
    assert_eq!((rig.pre.resets, rig.post.resets), (1, 1));
}

#[test]
fn chain_params_nest_by_field_name() {
    assert_eq!(ids::<RigParams>(), ["pre.gain", "post.gain"]);
    assert_eq!(ids::<OuterSettings>(), ["rig.pre.gain", "rig.post.gain"]);
    let mut p = OuterSettings::default();
    p.set_str("rig.post.gain", "4").unwrap();
    assert_eq!(p.rig.post.gain, 4.0);
    assert_eq!(Outer::default().latency(), 6);
    let _: fn(&mut Outer, &mut Block, &OuterSettings) = <Outer as Processor>::process;
}
