use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub enum IrError {
    Io(std::io::Error),
    Wav(String),
    Resample(String),
}

impl fmt::Display for IrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IrError::Io(e) => write!(f, "IO error: {e}"),
            IrError::Wav(msg) => write!(f, "WAV error: {msg}"),
            IrError::Resample(msg) => write!(f, "resample error: {msg}"),
        }
    }
}

impl std::error::Error for IrError {}

impl From<std::io::Error> for IrError {
    fn from(e: std::io::Error) -> Self {
        IrError::Io(e)
    }
}

impl From<hound::Error> for IrError {
    fn from(e: hound::Error) -> Self {
        match e {
            hound::Error::IoError(io) => IrError::Io(io),
            other => IrError::Wav(other.to_string()),
        }
    }
}

impl From<rubato::ResampleError> for IrError {
    fn from(e: rubato::ResampleError) -> Self {
        IrError::Resample(e.to_string())
    }
}

impl From<rubato::ResamplerConstructionError> for IrError {
    fn from(e: rubato::ResamplerConstructionError) -> Self {
        IrError::Resample(e.to_string())
    }
}

/// Load an impulse response WAV file, returning samples resampled to
/// `target_sample_rate` and trimmed/padded to exactly `max_length` samples,
/// with peak normalized to 1.0.
pub fn load_ir(
    path: &Path,
    target_sample_rate: u32,
    max_length: usize,
) -> Result<Vec<f32>, IrError> {
    // 1. Read WAV file
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let source_sample_rate = spec.sample_rate;
    let bits_per_sample = spec.bits_per_sample;

    let samples_f32: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let divisor = (1_i64 << (bits_per_sample - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 / divisor))
                .collect::<Result<Vec<f32>, _>>()?
        }
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<f32>, _>>()?,
    };

    if samples_f32.is_empty() {
        // Empty file: return zero-padded buffer
        return Ok(vec![0.0; max_length]);
    }

    // 2. Convert to mono by averaging channels
    let mono: Vec<f32> = if channels > 1 {
        samples_f32
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        samples_f32
    };

    // 3. Resample if needed
    let resampled: Vec<f32> = if source_sample_rate != target_sample_rate {
        resample(&mono, source_sample_rate, target_sample_rate)?
    } else {
        mono
    };

    // 4. Trim or zero-pad to max_length
    let mut ir = vec![0.0_f32; max_length];
    let copy_len = resampled.len().min(max_length);
    ir[..copy_len].copy_from_slice(&resampled[..copy_len]);

    // 5. Normalize peak to 1.0
    let peak = ir.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
    if peak > 0.0 {
        for s in &mut ir {
            *s /= peak;
        }
    }

    Ok(ir)
}

/// Resample a mono f32 signal from `from_rate` to `to_rate` using rubato.
fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>, IrError> {
    use rubato::{FftFixedIn, Resampler};

    let chunk_size = 1024;
    let sub_chunks = 2;
    let num_channels = 1;

    let mut resampler = FftFixedIn::<f64>::new(
        from_rate as usize,
        to_rate as usize,
        chunk_size,
        sub_chunks,
        num_channels,
    )?;

    // rubato works with Vec<Vec<f64>> (one inner vec per channel)
    let input_f64: Vec<f64> = input.iter().map(|&s| s as f64).collect();

    let mut output_all: Vec<f64> = Vec::new();
    let mut pos = 0;

    // Process full chunks
    while pos + chunk_size <= input_f64.len() {
        let chunk = vec![input_f64[pos..pos + chunk_size].to_vec()];
        let result = resampler.process(&chunk, None)?;
        output_all.extend_from_slice(&result[0]);
        pos += chunk_size;
    }

    // Process remaining samples: pad with zeros to fill a final chunk
    if pos < input_f64.len() {
        let mut last_chunk = vec![0.0_f64; chunk_size];
        let remaining = input_f64.len() - pos;
        last_chunk[..remaining].copy_from_slice(&input_f64[pos..]);
        let chunk = vec![last_chunk];
        let result = resampler.process(&chunk, None)?;
        // Only keep the output corresponding to the remaining input
        let expected_out = (remaining as f64 * to_rate as f64 / from_rate as f64).ceil() as usize;
        let take = expected_out.min(result[0].len());
        output_all.extend_from_slice(&result[0][..take]);
    }

    Ok(output_all.iter().map(|&s| s as f32).collect())
}
