//! Minimal WAV reading and writing, with no dependencies.
//!
//! Reads PCM 8/16/24/32-bit and IEEE float 32/64-bit, including
//! `WAVE_FORMAT_EXTENSIBLE`. Writes 32-bit float (default) or 16/24-bit PCM.

use std::fmt;
use std::fs;
use std::path::Path;

/// Decoded audio: one `Vec<f32>` per channel, samples in -1..1.
#[derive(Clone, Debug, PartialEq)]
pub struct Audio {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl Audio {
    pub fn mono(sample_rate: u32, samples: Vec<f32>) -> Self {
        Self {
            sample_rate,
            channels: vec![samples],
        }
    }

    /// Frames per channel.
    pub fn len(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Average all channels into one.
    pub fn to_mono(&self) -> Vec<f32> {
        match self.channels.len() {
            0 => Vec::new(),
            1 => self.channels[0].clone(),
            n => (0..self.len())
                .map(|i| self.channels.iter().map(|c| c[i]).sum::<f32>() / n as f32)
                .collect(),
        }
    }
}

/// Sample encoding for writing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Encoding {
    #[default]
    Float32,
    Pcm16,
    Pcm24,
}

#[derive(Debug)]
pub enum WavError {
    Io(std::io::Error),
    Format(&'static str),
}

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WavError::Io(e) => write!(f, "io error: {e}"),
            WavError::Format(m) => write!(f, "invalid wav: {m}"),
        }
    }
}

impl std::error::Error for WavError {}

impl From<std::io::Error> for WavError {
    fn from(e: std::io::Error) -> Self {
        WavError::Io(e)
    }
}

const FORMAT_PCM: u16 = 1;
const FORMAT_FLOAT: u16 = 3;
const FORMAT_EXTENSIBLE: u16 = 0xfffe;

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn read(path: impl AsRef<Path>) -> Result<Audio, WavError> {
    decode(&fs::read(path)?)
}

pub fn decode(bytes: &[u8]) -> Result<Audio, WavError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::Format("missing RIFF/WAVE header"));
    }

    let mut fmt: Option<(u16, u16, u32, u16)> = None; // format, channels, rate, bits
    let mut data: Option<&[u8]> = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32_at(bytes, pos + 4) as usize;
        let body_start = pos + 8;
        // Tolerate a truncated final chunk (common with streamed writers).
        let body_end = (body_start + size).min(bytes.len());
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err(WavError::Format("fmt chunk too short"));
                }
                let mut format = u16_at(body, 0);
                if format == FORMAT_EXTENSIBLE {
                    if body.len() < 26 {
                        return Err(WavError::Format("extensible fmt chunk too short"));
                    }
                    // The sub-format GUID starts with the real format code.
                    format = u16_at(body, 24);
                }
                fmt = Some((format, u16_at(body, 2), u32_at(body, 4), u16_at(body, 14)));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        // Chunks are padded to even sizes.
        pos = body_start + size + (size & 1);
    }

    let (format, channels, sample_rate, bits) = fmt.ok_or(WavError::Format("no fmt chunk"))?;
    let data = data.ok_or(WavError::Format("no data chunk"))?;
    if channels == 0 {
        return Err(WavError::Format("zero channels"));
    }

    let bytes_per_sample = (bits as usize).div_ceil(8);
    let decode_sample: fn(&[u8]) -> f32 = match (format, bits) {
        (FORMAT_PCM, 8) => |b| (b[0] as f32 - 128.0) / 128.0,
        (FORMAT_PCM, 16) => |b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32_768.0,
        (FORMAT_PCM, 24) => {
            |b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0
        }
        (FORMAT_PCM, 32) => {
            |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0
        }
        (FORMAT_FLOAT, 32) => |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        (FORMAT_FLOAT, 64) => {
            |b| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as f32
        }
        _ => return Err(WavError::Format("unsupported sample format")),
    };

    let frame_bytes = bytes_per_sample * channels as usize;
    let frames = data.len() / frame_bytes;
    let mut out = vec![Vec::with_capacity(frames); channels as usize];
    for frame in data.chunks_exact(frame_bytes) {
        for (ch, sample) in frame.chunks_exact(bytes_per_sample).enumerate() {
            out[ch].push(decode_sample(sample));
        }
    }

    Ok(Audio {
        sample_rate,
        channels: out,
    })
}

pub fn write(path: impl AsRef<Path>, audio: &Audio, encoding: Encoding) -> Result<(), WavError> {
    fs::write(path, encode(audio, encoding)?)?;
    Ok(())
}

pub fn encode(audio: &Audio, encoding: Encoding) -> Result<Vec<u8>, WavError> {
    let channels = audio.channels.len();
    if channels == 0 || channels > u16::MAX as usize {
        return Err(WavError::Format("channel count out of range"));
    }
    let frames = audio.len();
    if audio.channels.iter().any(|c| c.len() != frames) {
        return Err(WavError::Format("channels have different lengths"));
    }

    let (format, bits): (u16, u16) = match encoding {
        Encoding::Float32 => (FORMAT_FLOAT, 32),
        Encoding::Pcm16 => (FORMAT_PCM, 16),
        Encoding::Pcm24 => (FORMAT_PCM, 24),
    };
    let block_align = channels * bits as usize / 8;
    let data_len = frames * block_align;
    if data_len > u32::MAX as usize - 36 {
        return Err(WavError::Format("audio too long for a wav file"));
    }

    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&format.to_le_bytes());
    b.extend_from_slice(&(channels as u16).to_le_bytes());
    b.extend_from_slice(&audio.sample_rate.to_le_bytes());
    b.extend_from_slice(&(audio.sample_rate * block_align as u32).to_le_bytes());
    b.extend_from_slice(&(block_align as u16).to_le_bytes());
    b.extend_from_slice(&bits.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());

    for i in 0..frames {
        for ch in &audio.channels {
            let s = ch[i];
            match encoding {
                Encoding::Float32 => b.extend_from_slice(&s.to_le_bytes()),
                Encoding::Pcm16 => {
                    // Same scale as decoding, so a round trip is exact to half an LSB.
                    let v = (s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16;
                    b.extend_from_slice(&v.to_le_bytes());
                }
                Encoding::Pcm24 => {
                    let v = (s * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32;
                    b.extend_from_slice(&v.to_le_bytes()[..3]);
                }
            }
        }
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_audio() -> Audio {
        let n = 1000;
        Audio {
            sample_rate: 44_100,
            channels: vec![
                (0..n).map(|i| (i as f32 * 0.05).sin() * 0.9).collect(),
                (0..n).map(|i| (i as f32 * 0.013).cos() * -0.5).collect(),
            ],
        }
    }

    #[test]
    fn float_round_trip_is_exact() {
        let a = test_audio();
        let back = decode(&encode(&a, Encoding::Float32).unwrap()).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn pcm_round_trips_within_quantization() {
        let a = test_audio();
        for (enc, tol) in [
            (Encoding::Pcm16, 0.5 / 32_768.0),
            (Encoding::Pcm24, 0.5 / 8_388_608.0),
        ] {
            let back = decode(&encode(&a, enc).unwrap()).unwrap();
            assert_eq!(back.sample_rate, 44_100);
            for (x, y) in a
                .channels
                .iter()
                .flatten()
                .zip(back.channels.iter().flatten())
            {
                assert!((x - y).abs() <= tol, "{enc:?}: {x} vs {y}");
            }
        }
    }

    #[test]
    fn pcm24_negative_full_scale() {
        let a = Audio::mono(48_000, vec![-1.0, 1.0, 0.0]);
        let back = decode(&encode(&a, Encoding::Pcm24).unwrap()).unwrap();
        assert!((back.channels[0][0] + 1.0).abs() < 1e-6);
        assert!((back.channels[0][1] - 1.0).abs() < 1e-6);
        assert_eq!(back.channels[0][2], 0.0);
    }

    #[test]
    fn skips_unknown_chunks() {
        let a = Audio::mono(48_000, vec![0.25, -0.25]);
        let mut b = encode(&a, Encoding::Float32).unwrap();
        // Insert an odd-sized LIST chunk (with pad byte) before "data".
        let list = [b"LIST".as_slice(), &3u32.to_le_bytes(), b"abc", &[0]].concat();
        b.splice(36..36, list);
        assert_eq!(decode(&b).unwrap(), a);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"not a wav file at all").is_err());
    }
}
