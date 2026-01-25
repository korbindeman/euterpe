//! Proc macro for embedding WAV impulse responses at compile time.

use proc_macro::TokenStream;
use std::path::PathBuf;

const SAMPLE_RATE: u32 = 48_000;

/// Embeds a WAV file as a static f32 array.
///
/// Usage:
/// - `embed_ir!("path/to/file.wav")` - uses full resampled length
/// - `embed_ir!("path/to/file.wav", 2048)` - truncates/pads to specified length
///
/// The path is relative to the workspace root.
/// The WAV is converted to mono, resampled to 48kHz, and normalized.
#[proc_macro]
pub fn embed_ir(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();
    let parts: Vec<&str> = input_str.split(',').collect();

    let path_str = parts[0].trim().trim_matches('"');
    let max_length: Option<usize> = parts
        .get(1)
        .map(|s| s.trim().parse().expect("Second argument must be a number"));

    // Resolve path relative to workspace root
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let crate_path = PathBuf::from(&manifest_dir);
    let workspace_root = crate_path.parent().unwrap().parent().unwrap();
    let full_path = workspace_root.join(path_str);

    let samples = load_and_process_ir(&full_path, max_length);

    let samples_str: Vec<String> = samples.iter().map(|s| format!("{}_f32", s)).collect();
    let array_contents = samples_str.join(", ");

    format!("[{}]", array_contents).parse().unwrap()
}

fn load_and_process_ir(path: &PathBuf, max_length: Option<usize>) -> Vec<f32> {
    let reader = hound::WavReader::open(path)
        .unwrap_or_else(|e| panic!("Failed to open WAV file {:?}: {}", path, e));
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let source_rate = spec.sample_rate;
    let bits = spec.bits_per_sample;

    let samples_f32: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let divisor = (1_i64 << (bits - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.unwrap() as f32 / divisor)
                .collect()
        }
        hound::SampleFormat::Float => reader.into_samples::<f32>().map(|s| s.unwrap()).collect(),
    };

    // Convert to mono
    let mono: Vec<f32> = if channels > 1 {
        samples_f32
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        samples_f32
    };

    // Resample if needed
    let resampled = if source_rate != SAMPLE_RATE {
        resample_linear(&mono, source_rate, SAMPLE_RATE)
    } else {
        mono
    };

    // Apply max length if specified
    let mut ir = match max_length {
        Some(len) => {
            let mut padded = vec![0.0_f32; len];
            let copy_len = resampled.len().min(len);
            padded[..copy_len].copy_from_slice(&resampled[..copy_len]);
            padded
        }
        None => resampled,
    };

    // Normalize peak to 1.0
    let peak = ir.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
    if peak > 0.0 {
        for s in &mut ir {
            *s /= peak;
        }
    }

    ir
}

fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = ((input.len() as f64) / ratio).ceil() as usize;
    let mut output = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = (src_pos - idx as f64) as f32;

        let s0 = input.get(idx).copied().unwrap_or(0.0);
        let s1 = input.get(idx + 1).copied().unwrap_or(s0);
        output.push(s0 + (s1 - s0) * frac);
    }

    output
}
