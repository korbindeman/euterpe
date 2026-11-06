//! `euterpe analyze`: measurements in numbers, as JSON.

use euterpe_core::{Device, ParamSet, Processor};
use euterpe_offline::analyze::{self, Setup};
use euterpe_offline::wav;
use euterpe_offline::{RenderOptions, render};

use crate::json::Json;
use crate::registry::Visitor;

pub const HELP: &str = "\
    euterpe analyze <device> <suite> [options]
        Measure the device and print numbers. Suites:
          response  gain and phase vs frequency, third octaves 20 Hz-20 kHz
          thd       harmonic distortion of a sine vs input level
          aliasing  energy where harmonics above Nyquist fold back, per
                    test frequency, relative to the fundamental
          null      render --in, compare with --reference

ANALYZE OPTIONS:
    --set <id>=<value>    Set a parameter (repeatable).
    --rate <hz>           Sample rate [default: 48000]
    --level <dB>          Input level in dBFS [default: -20 for response,
                          -6 for aliasing; thd sweeps -42..0]
    --freq <hz>           Test frequency (repeatable). thd: one [1000];
                          aliasing: several [1000 2000 4000 6000 8000 10000];
                          response: overrides the third octaves.
    --settle <s>          Output skipped before measuring [default: 0.1]
    --in <file.wav>       null: input to render (first channel).
    --reference <file.wav>
                          null: expected output (first channel).
    --gain-match          null: scale the render to best fit the reference
                          first, so only shape differences remain.
";

const SUITES: &str = "response, thd, aliasing, null";

pub struct AnalyzeJob {
    suite: String,
    sets: Vec<(String, String)>,
    setup: Setup,
    level_db: Option<f64>,
    freqs: Vec<f64>,
    input: Option<String>,
    reference: Option<String>,
    gain_match: bool,
}

impl AnalyzeJob {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let suite = args
            .first()
            .ok_or_else(|| format!("usage: euterpe analyze <device> <suite>; suites: {SUITES}"))?
            .clone();
        if !SUITES.split(", ").any(|s| s == suite) {
            return Err(format!("unknown suite `{suite}`; available: {SUITES}"));
        }
        let mut job = AnalyzeJob {
            suite,
            sets: Vec::new(),
            setup: Setup::default(),
            level_db: None,
            freqs: Vec::new(),
            input: None,
            reference: None,
            gain_match: false,
        };
        let mut it = args[1..].iter();
        while let Some(flag) = it.next() {
            let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
            match flag.as_str() {
                "--set" => {
                    let kv = value()?;
                    let (k, v) = kv
                        .split_once('=')
                        .ok_or_else(|| format!("--set expects id=value, got `{kv}`"))?;
                    job.sets.push((k.into(), v.into()));
                }
                "--rate" => job.setup.sample_rate = crate::parse_num(flag, value()?)?,
                "--level" => job.level_db = Some(crate::parse_num(flag, value()?)?),
                "--freq" => job.freqs.push(crate::parse_num(flag, value()?)?),
                "--settle" => job.setup.settle_seconds = crate::parse_num(flag, value()?)?,
                "--in" => job.input = Some(value()?.clone()),
                "--reference" => job.reference = Some(value()?.clone()),
                "--gain-match" => job.gain_match = true,
                other => {
                    return Err(format!(
                        "unknown option `{other}`; valid: --set, --rate, --level, --freq, --settle, \
                         --in, --reference, --gain-match"
                    ));
                }
            }
        }
        let rate_ok = job.setup.sample_rate >= 8_000;
        let settle_ok = job.setup.settle_seconds.is_finite() && job.setup.settle_seconds >= 0.0;
        let level_ok = job.level_db.is_none_or(|l| l.is_finite() && l <= 0.0);
        let nyquist = f64::from(job.setup.sample_rate) / 2.0;
        let freqs_ok = job.freqs.iter().all(|&f| f > 0.0 && f < nyquist);
        if !(rate_ok && settle_ok && level_ok && freqs_ok) {
            return Err(
                "--rate must be at least 8000, --settle non-negative, --level at most 0 dBFS, \
                 and every --freq between 0 and Nyquist"
                    .into(),
            );
        }
        if job.suite == "null" && (job.input.is_none() || job.reference.is_none()) {
            return Err("null needs --in <file.wav> and --reference <file.wav>".into());
        }
        Ok(job)
    }
}

impl Visitor for AnalyzeJob {
    type Out = Result<Json, String>;

    #[expect(
        clippy::too_many_lines,
        reason = "one flat JSON layout per suite reads best side by side"
    )]
    fn visit<D: Device>(self) -> Result<Json, String> {
        type Params<D> = <<D as Device>::Graph as Processor>::Params;
        let mut params = Params::<D>::default();
        for (id, text) in &self.sets {
            params.set_str(id, text).map_err(|e| {
                let ids: Vec<&str> = Params::<D>::SCHEMA.iter().map(|d| d.id).collect();
                format!("--set {id}={text}: {e} (parameters: {})", ids.join(", "))
            })?;
        }
        let setup = &self.setup;
        let num = Json::num;
        let opt = |x: Option<f64>| x.map_or(Json::Null, Json::num);

        let results = match self.suite.as_str() {
            "response" => {
                let freqs = if self.freqs.is_empty() {
                    analyze::THIRD_OCTAVES.to_vec()
                } else {
                    self.freqs.clone()
                };
                let level = self.level_db.unwrap_or(-20.0);
                let points = analyze::response::<D::Graph>(&params, setup, &freqs, level);
                Json::Obj(vec![
                    ("level_db", num(level)),
                    (
                        "points",
                        Json::Arr(
                            points
                                .iter()
                                .map(|p| {
                                    Json::Obj(vec![
                                        ("freq", num(round(p.freq, 2))),
                                        ("gain_db", num(round(p.gain_db, 3))),
                                        ("phase_deg", num(round(p.phase_deg, 2))),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            }
            "thd" => {
                let freq = self.freqs.first().copied().unwrap_or(1_000.0);
                let levels = self
                    .level_db
                    .map_or_else(|| analyze::THD_LEVELS.to_vec(), |l| vec![l]);
                let points = analyze::thd::<D::Graph>(&params, setup, freq, &levels);
                Json::Obj(vec![
                    ("freq", num(freq)),
                    (
                        "points",
                        Json::Arr(
                            points
                                .iter()
                                .map(|p| {
                                    Json::Obj(vec![
                                        ("level_db", num(p.level_db)),
                                        ("fundamental_db", num(round(p.fundamental_db, 3))),
                                        ("thd_pct", num(round_sig(p.thd_pct))),
                                        (
                                            "harmonics_db",
                                            Json::Obj(
                                                ["h2", "h3", "h4", "h5", "h6", "h7", "h8", "h9"]
                                                    .into_iter()
                                                    .zip(p.harmonics_db)
                                                    .map(|(k, v)| (k, opt(v.map(|d| round(d, 2)))))
                                                    .collect(),
                                            ),
                                        ),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            }
            "aliasing" => {
                let freqs = if self.freqs.is_empty() {
                    analyze::ALIAS_FREQS.to_vec()
                } else {
                    self.freqs.clone()
                };
                let level = self.level_db.unwrap_or(-6.0);
                let points = analyze::aliasing::<D::Graph>(&params, setup, &freqs, level);
                Json::Obj(vec![
                    ("level_db", num(level)),
                    (
                        "points",
                        Json::Arr(
                            points
                                .iter()
                                .map(|p| {
                                    Json::Obj(vec![
                                        ("freq", num(round(p.freq, 2))),
                                        ("fundamental_db", num(round(p.fundamental_db, 3))),
                                        ("harmonics_db", num(round(p.harmonics_db, 2))),
                                        ("alias_db", num(round(p.alias_db, 2))),
                                        ("worst_alias_db", num(round(p.worst_alias_db, 2))),
                                        ("worst_alias_freq", num(round(p.worst_alias_freq, 1))),
                                        ("residual_db", num(round(p.residual_db, 2))),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            }
            "null" => {
                let read = |path: &str| -> Result<wav::Audio, String> {
                    wav::read(path).map_err(|e| format!("reading {path}: {e}"))
                };
                let (input_path, ref_path) = (
                    self.input.as_deref().unwrap_or(""),
                    self.reference.as_deref().unwrap_or(""),
                );
                let input = read(input_path)?;
                let reference = read(ref_path)?;
                if input.sample_rate != reference.sample_rate {
                    return Err(format!(
                        "--in is {} Hz but --reference is {} Hz",
                        input.sample_rate, reference.sample_rate
                    ));
                }
                let first = |a: &wav::Audio, p: &str| {
                    a.channels
                        .first()
                        .cloned()
                        .ok_or_else(|| format!("{p} has no channels"))
                };
                let rendered = render(
                    &mut D::Graph::default(),
                    &params,
                    &first(&input, input_path)?,
                    &RenderOptions::new(input.sample_rate),
                );
                let r = analyze::null(&rendered, &first(&reference, ref_path)?, self.gain_match);
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "sample counts are far below 2^52"
                )]
                let samples = r.samples as f64;
                Json::Obj(vec![
                    ("in", Json::str(input_path)),
                    ("reference", Json::str(ref_path)),
                    ("gain_match", Json::Bool(self.gain_match)),
                    ("gain_db", num(round(r.gain_db, 4))),
                    ("residual_rms_db", num(round(r.residual_rms_db, 2))),
                    ("residual_peak_db", num(round(r.residual_peak_db, 2))),
                    ("samples", num(samples)),
                ])
            }
            other => return Err(format!("unknown suite `{other}`; available: {SUITES}")),
        };

        Ok(Json::Obj(vec![
            ("device", Json::str(D::NAME)),
            ("suite", Json::str(&self.suite)),
            ("sample_rate", Json::num(setup.sample_rate)),
            ("settle_seconds", Json::num(setup.settle_seconds)),
            (
                "params",
                Json::Obj(
                    Params::<D>::SCHEMA
                        .iter()
                        .filter_map(|d| Some((d.id, crate::value_json(&d.kind, params.get(d.id)?))))
                        .collect(),
                ),
            ),
            ("results", results),
        ]))
    }
}

/// Round to `places` decimals. Infinities pass through (and print as null).
fn round(x: f64, places: i32) -> f64 {
    let s = 10f64.powi(places);
    (x * s).round() / s + 0.0
}

/// Four significant digits, for values that span many decades (THD %).
fn round_sig(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a decimal exponent of a finite f64"
    )]
    let places = 3 - x.abs().log10().floor() as i32;
    round(x, places)
}
