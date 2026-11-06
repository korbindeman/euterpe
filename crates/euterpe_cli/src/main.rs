//! `euterpe`: list devices, dump parameter schemas, render audio offline.
//!
//! All commands print JSON to stdout, so agents and scripts can parse them.
//! Errors go to stderr with a non-zero exit code.

mod bench;
mod json;
mod registry;

use std::process::ExitCode;

use euterpe_core::params::Taper;
use euterpe_core::{Device, ParamKind, ParamSet, ParamValue, Processor};
use euterpe_offline::wav::{self, Audio, Encoding};
use euterpe_offline::{RenderOptions, render, signal};

use json::Json;
use registry::Visitor;

const HELP: &str = "\
euterpe — offline tools for Euterpe devices. Output is JSON on stdout.

USAGE:
    euterpe devices
        List every device.

    euterpe schema <device>
        Print the device's parameters: ids, ranges, units, defaults.

    euterpe render <device> (--in <file.wav> | --gen <signal>) --out <file.wav> [options]
        Render audio through the device and print a level summary.
        Each input channel runs through its own instance of the device.

{BENCH}
RENDER OPTIONS:
    --in <file.wav>       Input file (any PCM or float WAV).
    --gen <signal>        Generate input instead: impulse | silence | noise
                          | sine:<hz> | sweep:<from_hz>:<to_hz>
    --level <dB>          Level of the generated signal in dBFS [default: -6]
    --seconds <s>         Length of the generated signal [default: 1]
    --rate <hz>           Sample rate of the generated signal [default: 48000]
    --set <id>=<value>    Set a parameter (repeatable). Floats are clamped to
                          their range; toggles take on/off; choices take a
                          name or an index.
    --tail <s>            Extra output after the input ends [default: 0]
    --encoding <enc>      f32 | pcm16 | pcm24 [default: f32]

EXAMPLES:
    euterpe schema demo.drive
    euterpe render demo.drive --gen sine:220 --set drive=24 --out drive.wav
    euterpe render demo.gain --in guitar.wav --set gain=-12 --out quiet.wav
    euterpe bench demo.drive --baseline benches/qemu-baseline.txt
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.first().map(String::as_str) == Some("bench") {
        bench::Options::parse(&args[1..]).and_then(|opts| bench::run(&opts))
    } else {
        run(&args).map(|json| (json, true))
    };
    match result {
        Ok((json, pass)) => {
            println!("{}", json.render());
            // A bench that ran but found regressions or hot-path problems.
            if pass {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<Json, String> {
    match args.first().map(String::as_str) {
        None | Some("help" | "-h" | "--help") => {
            print!("{}", HELP.replace("{BENCH}", bench::HELP));
            std::process::exit(0);
        }
        Some("devices") => Ok(Json::Arr(
            registry::DEVICES
                .iter()
                .map(|(name, desc)| {
                    Json::Obj(vec![
                        ("name", Json::str(*name)),
                        ("description", Json::str(*desc)),
                    ])
                })
                .collect(),
        )),
        Some("schema") => {
            let name = args.get(1).ok_or("usage: euterpe schema <device>")?;
            registry::visit(name, SchemaJob).ok_or_else(|| unknown_device(name))
        }
        Some("render") => {
            let name = args
                .get(1)
                .ok_or("usage: euterpe render <device> ... (see --help)")?;
            let job = RenderJob::parse(&args[2..])?;
            registry::visit(name, job).ok_or_else(|| unknown_device(name))?
        }
        Some(other) => Err(format!("unknown command `{other}`; run `euterpe --help`")),
    }
}

pub(crate) fn unknown_device(name: &str) -> String {
    let names: Vec<&str> = registry::DEVICES.iter().map(|(n, _)| *n).collect();
    format!("unknown device `{name}`; available: {}", names.join(", "))
}

// ---------------------------------------------------------------------------
// schema
// ---------------------------------------------------------------------------

struct SchemaJob;

impl Visitor for SchemaJob {
    type Out = Json;

    fn visit<D: Device>(self) -> Json {
        Json::Obj(vec![
            ("device", Json::str(D::NAME)),
            ("description", Json::str(D::DESCRIPTION)),
            ("params", schema_json::<<D::Graph as Processor>::Params>()),
        ])
    }
}

fn schema_json<P: ParamSet>() -> Json {
    Json::Arr(
        P::SCHEMA
            .iter()
            .map(|d| {
                let mut fields = vec![("id", Json::str(d.id)), ("label", Json::str(d.label))];
                match d.kind {
                    ParamKind::Float {
                        min,
                        max,
                        default,
                        unit,
                        taper,
                        smooth_ms,
                    } => fields.extend([
                        ("type", Json::str("float")),
                        ("min", Json::num(min)),
                        ("max", Json::num(max)),
                        ("default", Json::num(default)),
                        ("unit", Json::str(unit)),
                        (
                            "taper",
                            Json::str(match taper {
                                Taper::Linear => "linear",
                                Taper::Exponential => "exponential",
                            }),
                        ),
                        ("smooth_ms", Json::num(smooth_ms)),
                    ]),
                    ParamKind::Toggle { default } => fields.extend([
                        ("type", Json::str("toggle")),
                        ("default", Json::Bool(default)),
                    ]),
                    ParamKind::Choice { options, default } => fields.extend([
                        ("type", Json::str("choice")),
                        (
                            "options",
                            Json::Arr(options.iter().map(|o| Json::str(*o)).collect()),
                        ),
                        ("default", Json::str(options[default as usize])),
                    ]),
                }
                Json::Obj(fields)
            })
            .collect(),
    )
}

fn value_json(kind: &ParamKind, value: ParamValue) -> Json {
    match (kind, value) {
        (_, ParamValue::Float(v)) => Json::num(v),
        (_, ParamValue::Bool(b)) => Json::Bool(b),
        (ParamKind::Choice { options, .. }, ParamValue::Choice(i)) => {
            Json::str(options[i as usize])
        }
        (_, ParamValue::Choice(i)) => Json::num(i),
    }
}

// ---------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------

struct RenderJob {
    input: Audio,
    input_desc: String,
    out: String,
    sets: Vec<(String, String)>,
    tail_seconds: f64,
    encoding: Encoding,
}

impl RenderJob {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut input_path = None;
        let mut gen_spec = None;
        let mut out = None;
        let mut sets = Vec::new();
        let mut level_db: f64 = -6.0;
        let mut seconds: f64 = 1.0;
        let mut rate = 48_000u32;
        let mut tail_seconds: f64 = 0.0;
        let mut encoding = Encoding::Float32;

        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
            match flag.as_str() {
                "--in" => input_path = Some(value()?.clone()),
                "--gen" => gen_spec = Some(value()?.clone()),
                "--out" => out = Some(value()?.clone()),
                "--set" => {
                    let kv = value()?;
                    let (k, v) = kv
                        .split_once('=')
                        .ok_or_else(|| format!("--set expects id=value, got `{kv}`"))?;
                    sets.push((k.to_string(), v.to_string()));
                }
                "--level" => level_db = parse_num(flag, value()?)?,
                "--seconds" => seconds = parse_num(flag, value()?)?,
                "--rate" => rate = parse_num(flag, value()?)?,
                "--tail" => tail_seconds = parse_num(flag, value()?)?,
                "--encoding" => {
                    encoding = match value()?.as_str() {
                        "f32" => Encoding::Float32,
                        "pcm16" => Encoding::Pcm16,
                        "pcm24" => Encoding::Pcm24,
                        other => {
                            return Err(format!(
                                "unknown encoding `{other}`; use f32, pcm16 or pcm24"
                            ));
                        }
                    }
                }
                other => return Err(format!("unknown option `{other}`; run `euterpe --help`")),
            }
        }

        let out = out.ok_or("--out <file.wav> is required")?;
        // Written to also reject NaN.
        let seconds_ok = seconds.is_finite() && seconds > 0.0;
        let tail_ok = tail_seconds.is_finite() && tail_seconds >= 0.0;
        if !seconds_ok || !tail_ok || rate == 0 {
            return Err("--seconds and --rate must be positive, --tail non-negative".into());
        }
        let (input, input_desc) = match (input_path, gen_spec) {
            (Some(path), None) => {
                let audio = wav::read(&path).map_err(|e| format!("reading {path}: {e}"))?;
                (audio, path)
            }
            (None, Some(spec)) => {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "seconds is a validated, non-negative CLI value; level is a dB value well within f32"
                )]
                let (len, amp) = (
                    (seconds * f64::from(rate)).round() as usize,
                    10f32.powf(level_db as f32 / 20.0),
                );
                (
                    Audio::mono(rate, generate(&spec, amp, len, rate)?),
                    format!("gen:{spec}"),
                )
            }
            _ => return Err("give exactly one of --in <file.wav> or --gen <signal>".into()),
        };

        Ok(Self {
            input,
            input_desc,
            out,
            sets,
            tail_seconds,
            encoding,
        })
    }
}

fn parse_num<T: std::str::FromStr>(flag: &str, s: &str) -> Result<T, String> {
    s.parse()
        .map_err(|_| format!("{flag}: `{s}` is not a number"))
}

fn generate(spec: &str, amp: f32, len: usize, rate: u32) -> Result<Vec<f32>, String> {
    let parts: Vec<&str> = spec.split(':').collect();
    let hz = |s: &str| -> Result<f64, String> {
        s.parse::<f64>()
            .ok()
            .filter(|f| *f > 0.0 && *f < f64::from(rate) / 2.0)
            .ok_or_else(|| format!("`{s}` is not a frequency between 0 and {} Hz", rate / 2))
    };
    Ok(match parts.as_slice() {
        ["impulse"] => signal::impulse(len, amp),
        ["silence"] => vec![0.0; len],
        ["noise"] => signal::noise(amp, len, 1),
        ["sine", f] => signal::sine(hz(f)?, amp, len, rate),
        ["sweep", f0, f1] => signal::log_sweep(hz(f0)?, hz(f1)?, amp, len, rate),
        _ => {
            return Err(format!(
                "unknown signal `{spec}`; use impulse, silence, noise, sine:<hz> or sweep:<hz>:<hz>"
            ));
        }
    })
}

impl Visitor for RenderJob {
    type Out = Result<Json, String>;

    fn visit<D: Device>(self) -> Result<Json, String> {
        type Params<D> = <<D as Device>::Graph as Processor>::Params;

        let mut params = Params::<D>::default();
        for (id, text) in &self.sets {
            params.set_str(id, text).map_err(|e| {
                let ids: Vec<&str> = Params::<D>::SCHEMA.iter().map(|d| d.id).collect();
                format!("--set {id}={text}: {e} (parameters: {})", ids.join(", "))
            })?;
        }

        let rate = self.input.sample_rate;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "tail_seconds is a validated, non-negative CLI value"
        )]
        let opts = RenderOptions {
            tail: (self.tail_seconds * f64::from(rate)).round() as usize,
            ..RenderOptions::new(rate)
        };
        let mut latency = 0;
        let channels = self
            .input
            .channels
            .iter()
            .map(|ch| {
                let mut graph = D::Graph::default();
                let out = render(&mut graph, &params, ch, &opts);
                latency = graph.latency();
                out
            })
            .collect();
        let output = Audio {
            sample_rate: rate,
            channels,
        };
        wav::write(&self.out, &output, self.encoding)
            .map_err(|e| format!("writing {}: {e}", self.out))?;

        let param_values = Params::<D>::SCHEMA
            .iter()
            .map(|d| {
                (
                    d.id,
                    params
                        .get(d.id)
                        .map_or(Json::Null, |v| value_json(&d.kind, v)),
                )
            })
            .collect();

        Ok(Json::Obj(vec![
            ("device", Json::str(D::NAME)),
            ("input", Json::str(self.input_desc)),
            ("output", Json::str(self.out)),
            ("sample_rate", Json::num(rate)),
            ("channels", Json::num(output.channels.len() as f64)),
            ("samples", Json::num(output.len() as f64)),
            ("latency_samples", Json::num(latency as f64)),
            ("params", Json::Obj(param_values)),
            ("input_levels", levels(&self.input)),
            ("output_levels", levels(&output)),
        ]))
    }
}

/// Peak and RMS level in dBFS across all channels.
fn levels(audio: &Audio) -> Json {
    let samples = audio.channels.iter().flatten();
    let (mut peak, mut sum_sq, mut n) = (0f64, 0f64, 0usize);
    for &s in samples {
        peak = peak.max(f64::from(s).abs());
        sum_sq += f64::from(s) * f64::from(s);
        n += 1;
    }
    let db = |x: f64| {
        if x > 0.0 {
            20.0 * x.log10()
        } else {
            f64::NEG_INFINITY
        }
    };
    let rms = if n > 0 {
        (sum_sq / n as f64).sqrt()
    } else {
        0.0
    };
    // -inf (silence) serializes as null.
    Json::Obj(vec![
        ("peak_dbfs", Json::num(round2(db(peak)))),
        ("rms_dbfs", Json::num(round2(db(rms)))),
    ])
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn schema_lists_params() {
        let out = run(&args("schema demo.drive")).unwrap().render();
        assert!(out.contains("\"id\": \"drive\""));
        assert!(out.contains("\"unit\": \"dB\""));
    }

    #[test]
    fn unknown_device_lists_alternatives() {
        let err = run(&args("schema nope")).unwrap_err();
        assert!(err.contains("demo.gain") && err.contains("demo.drive"));
    }

    #[test]
    fn render_round_trip() {
        let dir = std::env::temp_dir().join(format!("euterpe-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out.wav");
        let cmd = format!(
            "render demo.gain --gen sine:1000 --level -6 --seconds 0.5 --set gain=-6 --out {}",
            out.display()
        );
        let report = run(&args(&cmd)).unwrap().render();
        assert!(report.contains("\"gain\": -6"), "{report}");
        let audio = wav::read(&out).unwrap();
        assert_eq!(audio.len(), 24_000);
        let peak = audio.channels[0].iter().fold(0f32, |m, s| m.max(s.abs()));
        // -6 dBFS sine through -6 dB of gain peaks at -12 dBFS.
        assert!((20.0 * peak.log10() + 12.0).abs() < 0.01, "peak {peak}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_set_names_valid_params() {
        let err = run(&args(
            "render demo.drive --gen noise --set volume=3 --out /dev/null",
        ))
        .unwrap_err();
        assert!(err.contains("drive, level"), "{err}");
    }
}
