//! Real guitar test fixtures.
//!
//! Two tiers:
//!
//! - **Committed** (`fixtures/committed/`, a few MB): short CC BY clips cut
//!   from the datasets below. Always present, so any test can use them.
//!   Load them with [`load`] or iterate [`manifest::EXCERPTS`].
//! - **Cache** (`fixtures/cache/`, gitignored, ~18 GB unpacked): the full
//!   upstream datasets, pinned by SHA-256 in [`manifest::ARCHIVES`]. Fill it
//!   with `just fixtures fetch`. Tests that need it should skip when
//!   [`cached`] returns `None`, so CI never needs the network.
//!
//! ```ignore
//! let fx = euterpe_fixtures::load("gtechs_palm_mute").unwrap();
//! let out = render(&mut amp, &params, &fx.di, &RenderOptions::new(fx.sample_rate));
//! // fx.out is the same performance through a real amp, time-aligned.
//! ```

pub mod align;
pub mod manifest;
pub mod sha256;

use std::path::{Path, PathBuf};

use euterpe_offline::wav::{self, Audio, Encoding};

pub use manifest::{
    ARCHIVES, Align, Archive, DATASETS, Dataset, EXCERPTS, Excerpt, Kind, Source, Start,
};

/// Every committed fixture is at this rate.
pub const SAMPLE_RATE: u32 = 48_000;

/// `fixtures/` at the workspace root, or `$EUTERPE_FIXTURES_DIR`.
pub fn root() -> PathBuf {
    std::env::var_os("EUTERPE_FIXTURES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures"))
}

pub fn committed_dir() -> PathBuf {
    root().join("committed")
}

/// The download cache, or `$EUTERPE_FIXTURE_CACHE` (useful to share one
/// cache between checkouts).
pub fn cache_dir() -> PathBuf {
    std::env::var_os("EUTERPE_FIXTURE_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("cache"))
}

/// Path of a source file inside the unpacked cache, whether or not it exists.
pub fn source_path(dataset: &Dataset, source: &Source) -> PathBuf {
    let dir = source
        .archive
        .strip_suffix(".zip")
        .unwrap_or(source.archive);
    cache_dir().join(dataset.id).join(dir).join(source.member)
}

/// A cached source file, if the dataset has been fetched.
pub fn cached(dataset: &Dataset, source: &Source) -> Option<PathBuf> {
    let p = source_path(dataset, source);
    p.is_file().then_some(p)
}

/// A loaded committed fixture.
#[derive(Clone, Debug)]
pub struct Fixture {
    pub excerpt: &'static Excerpt,
    pub sample_rate: u32,
    /// The clean DI.
    pub di: Vec<f32>,
    /// For paired fixtures, the hardware output, aligned to `di`.
    pub out: Option<Vec<f32>>,
}

#[derive(Debug)]
pub enum Error {
    UnknownFixture(String),
    Wav(PathBuf, wav::WavError),
    Format(PathBuf, String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::UnknownFixture(id) => {
                let ids: Vec<_> = EXCERPTS.iter().map(|e| e.id).collect();
                write!(f, "unknown fixture `{id}`; valid: {}", ids.join(", "))
            }
            Error::Wav(p, e) => write!(f, "{}: {e}", p.display()),
            Error::Format(p, m) => write!(f, "{}: {m}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

fn read_mono(path: &Path) -> Result<Vec<f32>, Error> {
    let audio = wav::read(path).map_err(|e| Error::Wav(path.into(), e))?;
    if audio.sample_rate != SAMPLE_RATE {
        return Err(Error::Format(
            path.into(),
            format!(
                "sample rate {} (fixtures must be {SAMPLE_RATE})",
                audio.sample_rate
            ),
        ));
    }
    // Stereo sources in these datasets are dual mono, so take channel 0
    // rather than averaging (which would be identical, or smear real stereo).
    Ok(audio.channels.into_iter().next().unwrap_or_default())
}

/// Load a committed fixture by id.
pub fn load(id: &str) -> Result<Fixture, Error> {
    let excerpt = manifest::excerpt(id).ok_or_else(|| Error::UnknownFixture(id.into()))?;
    let files = excerpt.files();
    let dir = committed_dir();
    let di = read_mono(&dir.join(&files[0]))?;
    let out = match files.get(1) {
        Some(f) => Some(read_mono(&dir.join(f))?),
        None => None,
    };
    Ok(Fixture {
        excerpt,
        sample_rate: SAMPLE_RATE,
        di,
        out,
    })
}

/// Load every committed fixture.
pub fn load_all() -> Result<Vec<Fixture>, Error> {
    EXCERPTS.iter().map(|e| load(e.id)).collect()
}

/// What [`cut`] did, for reporting and pinning.
#[derive(Clone, Debug, PartialEq)]
pub struct CutReport {
    pub start_s: f32,
    /// Output delay relative to the DI, in samples (positive: output late).
    pub lag: Option<isize>,
    /// Output polarity was flipped to match the DI.
    pub inverted: bool,
    /// Normalized correlation at the chosen lag, 0..1.
    pub correlation: Option<f32>,
}

const FADE_S: f32 = 0.005;

/// Onset threshold, as a fraction of the clip's peak.
pub const ATTACK_ONSET: f32 = 0.1;

/// Raised-cosine fades so a clip cut mid-note doesn't start or end with a
/// step (which a drive stage would turn into a broadband click).
fn fade(x: &mut [f32], fade_in: bool) {
    let n = ((FADE_S * SAMPLE_RATE as f32) as usize).min(x.len() / 2);
    for i in 0..n {
        let g = 0.5 - 0.5 * (std::f32::consts::PI * i as f32 / n as f32).cos();
        if fade_in {
            x[i] *= g;
        }
        let j = x.len() - 1 - i;
        x[j] *= g;
    }
}

fn window(x: &[f32], start: isize, len: usize) -> Vec<f32> {
    (0..len as isize)
        .map(|i| {
            let j = start + i;
            if j >= 0 && (j as usize) < x.len() {
                x[j as usize]
            } else {
                0.0
            }
        })
        .collect()
}

/// Start of the loudest `len`-sample window, on a 0.5 s hop.
pub fn loudest_start(x: &[f32], len: usize) -> usize {
    let hop = SAMPLE_RATE as usize / 2;
    if x.len() <= len {
        return 0;
    }
    let mut best = (0, -1.0f64);
    let mut s = 0;
    while s + len <= x.len() {
        let e: f64 = x[s..s + len].iter().map(|&v| (v as f64) * (v as f64)).sum();
        if e > best.1 {
            best = (s, e);
        }
        s += hop;
    }
    best.0
}

/// Cut one excerpt from the cache into `committed_dir()`.
pub fn cut(excerpt: &Excerpt) -> Result<CutReport, Error> {
    let (di_src, out_src) = match excerpt.kind {
        Kind::Di { di } => (di, None),
        Kind::Paired { di, out, align, .. } => (di, Some((out, align))),
    };
    let di = read_mono(&source_path(&excerpt.dataset, &di_src))?;
    let len = (excerpt.len_s * SAMPLE_RATE as f32) as usize;
    let start = match excerpt.start {
        Start::At(s) => (s * SAMPLE_RATE as f32) as usize,
        Start::Loudest => loudest_start(&di, len),
    };
    let mut report = CutReport {
        start_s: start as f32 / SAMPLE_RATE as f32,
        lag: None,
        inverted: false,
        correlation: None,
    };

    let mut di_cut = window(&di, start as isize, len);
    let mut clips = vec![];
    let out_cut = match out_src {
        Some((src, how)) => {
            let out = read_mono(&source_path(&excerpt.dataset, &src))?;
            let ms = |v: f32| (v * SAMPLE_RATE as f32 / 1000.0) as usize;
            let a = match how {
                Align::Window { max_lag_ms } => align::align(&di_cut, &out, start, ms(max_lag_ms)),
                Align::Attack { max_lag_ms } => {
                    // 20 ms before the DI's attack to 100 ms after it.
                    let at = align::onset(&di_cut, ATTACK_ONSET).saturating_sub(ms(20.0));
                    let end = (at + ms(120.0)).min(di_cut.len());
                    align::align(&di_cut[at..end], &out, start + at, ms(max_lag_ms))
                }
            };
            report.lag = Some(a.lag);
            report.inverted = a.inverted;
            report.correlation = Some(a.correlation);
            let mut o = window(&out, start as isize + a.lag, len);
            if a.inverted {
                o.iter_mut().for_each(|v| *v = -*v);
            }
            Some(o)
        }
        None => None,
    };

    let fade_in = start > 0;
    fade(&mut di_cut, fade_in);
    clips.push(di_cut);
    if let Some(mut o) = out_cut {
        fade(&mut o, fade_in);
        clips.push(o);
    }

    let dir = committed_dir();
    std::fs::create_dir_all(&dir).map_err(|e| Error::Wav(dir.clone(), e.into()))?;
    for (file, samples) in excerpt.files().iter().zip(clips) {
        let path = dir.join(file);
        wav::write(&path, &Audio::mono(SAMPLE_RATE, samples), Encoding::Pcm24)
            .map_err(|e| Error::Wav(path.clone(), e))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_ids_and_files_are_unique() {
        let mut files: Vec<String> = EXCERPTS.iter().flat_map(|e| e.files()).collect();
        let n = files.len();
        files.sort();
        files.dedup();
        assert_eq!(files.len(), n);
    }

    #[test]
    fn every_excerpt_source_is_in_a_pinned_archive() {
        for e in EXCERPTS {
            let sources = match e.kind {
                Kind::Di { di } => vec![di],
                Kind::Paired { di, out, .. } => vec![di, out],
            };
            for s in sources {
                assert!(
                    manifest::archive(e.dataset.id, s.archive).is_some(),
                    "{}: {} is not in ARCHIVES",
                    e.id,
                    s.archive
                );
            }
        }
    }

    #[test]
    fn archive_hashes_look_like_sha256() {
        for a in ARCHIVES {
            assert_eq!(a.sha256.len(), 64, "{}", a.file);
            assert!(
                a.sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
        }
    }

    #[test]
    fn committed_budget_is_small() {
        // 24-bit mono at 48 kHz is 144 kB/s. Keep the committed tier under 10 MB.
        let seconds: f32 = EXCERPTS
            .iter()
            .map(|e| e.len_s * e.files().len() as f32)
            .sum();
        assert!(
            seconds * 144_000.0 < 10.0e6,
            "{seconds} s of committed audio"
        );
    }

    #[test]
    fn loudest_start_finds_the_burst() {
        let mut x = vec![0.01f32; SAMPLE_RATE as usize * 10];
        for v in &mut x[SAMPLE_RATE as usize * 6..SAMPLE_RATE as usize * 8] {
            *v = 0.5;
        }
        assert_eq!(
            loudest_start(&x, SAMPLE_RATE as usize * 2),
            SAMPLE_RATE as usize * 6
        );
    }

    #[test]
    fn fades_reach_zero_at_the_edges() {
        let mut x = vec![1.0f32; 1000];
        fade(&mut x, true);
        assert_eq!(x[0], 0.0);
        assert!(x[999] < 1e-3);
        assert_eq!(x[500], 1.0);
    }

    /// The committed clips decode, are 48 kHz mono, have the declared length,
    /// and are finite and unclipped. Skipped until `just fixtures excerpt`
    /// has been run at least once.
    #[test]
    fn committed_fixtures_are_sane() {
        if !committed_dir()
            .join(EXCERPTS[0].files()[0].as_str())
            .is_file()
        {
            eprintln!("no committed fixtures yet; skipping");
            return;
        }
        for fx in load_all().unwrap() {
            let len = (fx.excerpt.len_s * SAMPLE_RATE as f32) as usize;
            let mut clips = vec![&fx.di];
            clips.extend(fx.out.as_ref());
            for c in clips {
                assert_eq!(c.len(), len, "{}", fx.excerpt.id);
                assert!(
                    c.iter().all(|v| v.is_finite() && v.abs() < 1.0),
                    "{}",
                    fx.excerpt.id
                );
                let rms = (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt();
                assert!(rms > 1e-4, "{} is near silent (rms {rms})", fx.excerpt.id);
            }
            assert_eq!(
                fx.out.is_some(),
                matches!(fx.excerpt.kind, Kind::Paired { .. })
            );
        }
    }
}
