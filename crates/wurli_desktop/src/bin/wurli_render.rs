//! Offline renderer for A/B testing the Wurli processor.
//!
//! ```text
//! cargo run -p wurli_desktop --release --bin wurli_render -- in.wav out.wav [key=value ...]
//! cargo run -p wurli_desktop --release --bin wurli_render -- --demo out_dir
//! ```
//!
//! Keys: purity decay bark attack tone trem_rate trem_depth mix level
//! `--demo` writes a synthetic plucked-guitar phrase (dry.wav) and its
//! Wurli rendering (wurli.wav), so the chain can be heard without a DI file.

use std::path::Path;

use dsp::Process;
use dsp::wurli::Wurli;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--demo") {
        let dir = Path::new(args.get(1).map(String::as_str).unwrap_or("."));
        std::fs::create_dir_all(dir).expect("create output dir");
        let sr = 48_000;
        let dry = demo_guitar(sr);
        write_wav(&dir.join("dry.wav"), &dry, sr);
        let wet = render(&dry, sr, &args[2.min(args.len())..]);
        write_wav(&dir.join("wurli.wav"), &wet, sr);
        report(&dry, &wet);
        println!(
            "wrote {} and {}",
            dir.join("dry.wav").display(),
            dir.join("wurli.wav").display()
        );
        return;
    }

    if args.len() < 2 {
        eprintln!("usage: wurli_render in.wav out.wav [key=value ...] | --demo out_dir");
        std::process::exit(1);
    }
    let (input, sr) = read_wav(Path::new(&args[0]));
    let wet = render(&input, sr, &args[2..]);
    write_wav(Path::new(&args[1]), &wet, sr);
    report(&input, &wet);
}

fn render(input: &[f32], sr: u32, overrides: &[String]) -> Vec<f32> {
    let mut wurli = Wurli::with_sample_rate(sr);
    {
        let p = wurli.params_mut();
        for kv in overrides {
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            let v: f32 = v.parse().expect("numeric value");
            let knob = match k {
                "purity" => &mut p.purity,
                "decay" => &mut p.decay,
                "bark" => &mut p.bark,
                "attack" => &mut p.attack,
                "tone" => &mut p.tone,
                "trem_rate" => &mut p.trem_rate,
                "trem_depth" => &mut p.trem_depth,
                "mix" => &mut p.mix,
                "level" => &mut p.level,
                _ => panic!("unknown param {k}"),
            };
            knob.set(v);
            knob.snap();
        }
    }
    input.iter().map(|&x| wurli.process(x)).collect()
}

fn report(dry: &[f32], wet: &[f32]) {
    let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32).sqrt();
    println!(
        "dry peak {:.3} rms {:.4} | wet peak {:.3} rms {:.4}",
        peak(dry),
        rms(dry),
        peak(wet),
        rms(wet)
    );
}

fn read_wav(path: &Path) -> (Vec<f32>, u32) {
    let mut reader = hound::WavReader::open(path).expect("open wav");
    let spec = reader.spec();
    let ch = spec.channels as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap() as f32 / scale)
                .collect()
        }
    };
    // Guitar is mono: take the first channel.
    let mono = samples.chunks(ch).map(|f| f[0]).collect();
    (mono, spec.sample_rate)
}

fn write_wav(path: &Path, samples: &[f32], sr: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: sr,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for &s in samples {
        w.write_sample((s.clamp(-1.0, 1.0) * 8_388_607.0) as i32)
            .unwrap();
    }
    w.finalize().unwrap();
}

/// Karplus-Strong plucked strings: single notes, then a few chords.
fn demo_guitar(sr: u32) -> Vec<f32> {
    // (start seconds, midi notes, velocity)
    let events: &[(f32, &[u8], f32)] = &[
        (0.0, &[52], 0.5),
        (0.6, &[55], 0.5),
        (1.2, &[59], 0.7),
        (1.8, &[64], 0.9),
        (3.0, &[45, 52, 57, 60, 64], 0.6),     // Am
        (5.0, &[41, 48, 53, 57, 60], 0.6),     // F
        (7.0, &[43, 50, 55, 59, 62], 0.9),     // G, picked hard
        (9.0, &[40, 47, 52, 55, 59, 64], 0.4), // Em, soft
    ];
    let len = (12.0 * sr as f32) as usize;
    let mut out = vec![0.0f32; len];
    let mut seed = 0x1234_5678u32;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    for &(t, notes, vel) in events {
        for (i, &n) in notes.iter().enumerate() {
            let freq = 440.0 * 2.0f32.powf((n as f32 - 69.0) / 12.0);
            let period = (sr as f32 / freq).round() as usize;
            let mut line: Vec<f32> = (0..period).map(|_| rand() * vel).collect();
            // Strum: 12 ms between strings.
            let start = ((t + i as f32 * 0.012) * sr as f32) as usize;
            let mut idx = 0;
            for s in out.iter_mut().skip(start).take(4 * sr as usize) {
                let next = (idx + 1) % period;
                let y = line[idx];
                line[idx] = 0.996 * 0.5 * (line[idx] + line[next]);
                *s += y * 0.25;
                idx = next;
            }
        }
    }
    out
}
