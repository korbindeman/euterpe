//! Measurements with numbers instead of ears: frequency response, harmonic
//! distortion, aliasing and null tests.
//!
//! Every measurement renders a test signal through the processor, skips a
//! settling time (smoothers, filters, DC blockers), then looks at one
//! FFT frame of the steady-state output. Test frequencies are snapped to an
//! *odd* FFT bin (coherent sampling). With an odd bin `b` and a power-of-two
//! frame `N`, harmonic `k·b` and an alias `|k·b − m·N|` can never land on the
//! same bin for any harmonic below `N`, so true harmonics and aliases are
//! told apart exactly. A 4-term Blackman-Harris window (sidelobes at -92 dB)
//! keeps any leftover leakage out of neighbouring components.
//!
//! The analysis runs in f64. All levels are in dB; `-inf` dB is reported
//! where a component is exactly zero.

use std::f64::consts::TAU;

use euterpe_core::Processor;

use crate::render::{RenderOptions, render};
use crate::signal;

/// FFT frame length for every measurement: 2.9 Hz bins at 48 kHz.
pub const FRAME: usize = 16_384;
/// Bins either side of a component's centre that belong to it. The
/// Blackman-Harris main lobe is ±4 bins wide.
const LOBE: usize = 4;
/// Harmonics followed when looking for aliases.
const MAX_HARMONIC: usize = 64;
/// Top of the audible band, for the `audible_*` aliasing figures.
pub const AUDIBLE_HZ: f64 = 20_000.0;

/// Settings shared by every suite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setup {
    pub sample_rate: u32,
    /// Output skipped before the analysed frame, in seconds.
    pub settle_seconds: f64,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            settle_seconds: 0.1,
        }
    }
}

impl Setup {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "settle time is a small, non-negative number of seconds"
    )]
    fn settle_samples(&self) -> usize {
        (self.settle_seconds.max(0.0) * f64::from(self.sample_rate)).round() as usize
    }

    fn bin_hz(&self) -> f64 {
        f64::from(self.sample_rate) / FRAME as f64
    }

    /// The odd bin closest to `freq`, at least 1 and below Nyquist.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "freq is clamped to (0, Nyquist), so the bin fits"
    )]
    fn odd_bin(&self, freq: f64) -> usize {
        let b = (freq / self.bin_hz()).round().max(1.0) as usize;
        let b = if b.is_multiple_of(2) { b + 1 } else { b };
        b.min(FRAME / 2 - 1)
    }

    /// An odd bin near `freq` where no alias of harmonics up to
    /// [`MAX_HARMONIC`] lands within a main lobe of DC, the fundamental or a
    /// true harmonic, so each alias is measured on its own. The odd bin alone
    /// rules out exact coincidence; this also rules out overlapping lobes.
    fn alias_safe_bin(&self, freq: f64) -> usize {
        let base = self.odd_bin(freq);
        (0..200)
            .flat_map(|step| [base + 2 * step, base.saturating_sub(2 * step)])
            .filter(|&b| b % 2 == 1 && b < FRAME / 2)
            .find(|&b| alias_clearance(b) > 2 * LOBE)
            .unwrap_or(base)
    }

    /// Render a sine at bin `bin` through the processor and return the
    /// analysed frame of the output.
    fn render_tone<P: Processor + Default>(
        &self,
        params: &P::Params,
        bin: usize,
        level_db: f64,
    ) -> Vec<f32> {
        let freq = bin as f64 * self.bin_hz();
        let settle = self.settle_samples();
        let input = signal::sine(
            freq,
            db_to_amp_f32(level_db),
            settle + FRAME,
            self.sample_rate,
        );
        let out = render(
            &mut P::default(),
            params,
            &input,
            &RenderOptions::new(self.sample_rate),
        );
        out[settle..].to_vec()
    }
}

// ---------------------------------------------------------------------------
// Spectrum
// ---------------------------------------------------------------------------

/// A windowed, scaled spectrum of one frame. Bin `k` holds the complex
/// amplitude: a full-scale sine centred on bin `k` reads as magnitude 1.
pub struct Spectrum {
    bins: Vec<(f64, f64)>,
}

impl Spectrum {
    /// Spectrum of `frame` (exactly [`FRAME`] samples).
    pub fn of(frame: &[f32]) -> Self {
        assert_eq!(frame.len(), FRAME, "a spectrum frame is FRAME samples");
        let window = blackman_harris(FRAME);
        let gain: f64 = window.iter().sum::<f64>() / 2.0;
        let mut re: Vec<f64> = frame
            .iter()
            .zip(&window)
            .map(|(&x, w)| f64::from(x) * w / gain)
            .collect();
        let mut im = vec![0.0; FRAME];
        fft(&mut re, &mut im);
        Self {
            bins: re.into_iter().zip(im).take(FRAME / 2 + 1).collect(),
        }
    }

    /// Peak complex amplitude of a component centred on `bin`.
    pub fn at(&self, bin: usize) -> (f64, f64) {
        self.bins[bin]
    }

    /// Energy of a component centred on `bin` (sum over its main lobe),
    /// in units where a full-scale sine has energy 1 times the window's
    /// lobe energy. Only ratios of energies are meaningful.
    fn energy(&self, bin: usize) -> f64 {
        let lo = bin.saturating_sub(LOBE);
        let hi = (bin + LOBE).min(self.bins.len() - 1);
        self.bins[lo..=hi].iter().map(|(r, i)| r * r + i * i).sum()
    }

    fn len(&self) -> usize {
        self.bins.len()
    }
}

fn blackman_harris(n: usize) -> Vec<f64> {
    const A: [f64; 4] = [0.358_75, 0.488_29, 0.141_28, 0.011_68];
    (0..n)
        .map(|i| {
            let x = TAU * i as f64 / n as f64;
            A[0] - A[1] * x.cos() + A[2] * (2.0 * x).cos() - A[3] * (3.0 * x).cos()
        })
        .collect()
}

/// In-place iterative radix-2 FFT. `re.len()` must be a power of two.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    assert!(n.is_power_of_two() && im.len() == n);
    // Bit-reversal permutation.
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

fn db(ratio: f64) -> f64 {
    20.0 * ratio.log10()
}

fn power_db(ratio: f64) -> f64 {
    10.0 * ratio.log10()
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a level in dB as an f32 amplitude"
)]
fn db_to_amp_f32(level_db: f64) -> f32 {
    10f64.powf(level_db / 20.0) as f32
}

fn magnitude((re, im): (f64, f64)) -> f64 {
    re.hypot(im)
}

/// The smallest distance, in bins, between an alias of fundamental bin `b`
/// and DC, the fundamental or a true harmonic.
fn alias_clearance(b: usize) -> usize {
    let harmonics: Vec<usize> = (0..=FRAME / 2 / b).map(|j| j * b).collect();
    (2..=MAX_HARMONIC)
        .filter(|&k| k * b > FRAME / 2)
        .map(|k| fold(k, b))
        .flat_map(|a| harmonics.iter().map(move |&h| a.abs_diff(h)))
        .min()
        .unwrap_or(usize::MAX)
}

/// Where harmonic `k` of bin `b` lands after folding around Nyquist.
fn fold(k: usize, b: usize) -> usize {
    let x = (k * b) % FRAME;
    if x > FRAME / 2 { FRAME - x } else { x }
}

// ---------------------------------------------------------------------------
// response
// ---------------------------------------------------------------------------

/// One point of a frequency response.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResponsePoint {
    /// The measured frequency (the requested one snapped to an odd bin).
    pub freq: f64,
    /// Output level relative to input, in dB.
    pub gain_db: f64,
    /// Output phase relative to input, in degrees, wrapped to (-180, 180].
    pub phase_deg: f64,
}

/// Third-octave centre frequencies from 20 Hz to 20 kHz (ISO 266).
pub const THIRD_OCTAVES: [f64; 31] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1_000.0, 1_250.0, 1_600.0, 2_000.0, 2_500.0, 3_150.0, 4_000.0, 5_000.0,
    6_300.0, 8_000.0, 10_000.0, 12_500.0, 16_000.0, 20_000.0,
];

/// Magnitude and phase response with stepped sines at `freqs` (those below
/// Nyquist), at input level `level_db` dBFS.
pub fn response<P: Processor + Default>(
    params: &P::Params,
    setup: &Setup,
    freqs: &[f64],
    level_db: f64,
) -> Vec<ResponsePoint> {
    let nyquist = f64::from(setup.sample_rate) / 2.0;
    let settle = setup.settle_samples();
    freqs
        .iter()
        .filter(|&&f| f > 0.0 && f < nyquist)
        .map(|&f| {
            let bin = setup.odd_bin(f);
            let freq = bin as f64 * setup.bin_hz();
            let input = signal::sine(
                freq,
                db_to_amp_f32(level_db),
                settle + FRAME,
                setup.sample_rate,
            );
            let out = render(
                &mut P::default(),
                params,
                &input,
                &RenderOptions::new(setup.sample_rate),
            );
            let x = Spectrum::of(&input[settle..]).at(bin);
            let y = Spectrum::of(&out[settle..]).at(bin);
            // y / x as complex numbers.
            let denom = x.0 * x.0 + x.1 * x.1;
            let h = (
                (y.0 * x.0 + y.1 * x.1) / denom,
                (y.1 * x.0 - y.0 * x.1) / denom,
            );
            let mut phase = h.1.atan2(h.0).to_degrees();
            if phase <= -180.0 {
                phase += 360.0;
            }
            ResponsePoint {
                freq,
                gain_db: db(magnitude(h)),
                phase_deg: phase,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// thd
// ---------------------------------------------------------------------------

/// Harmonic distortion at one input level.
#[derive(Clone, Debug, PartialEq)]
pub struct ThdPoint {
    pub level_db: f64,
    /// Output level of the fundamental, dBFS.
    pub fundamental_db: f64,
    /// Total harmonic distortion (harmonics 2 to 9 below Nyquist), percent of
    /// the fundamental.
    pub thd_pct: f64,
    /// Harmonics 2 to 9 relative to the fundamental, in dB. `None` above
    /// Nyquist.
    pub harmonics_db: [Option<f64>; 8],
}

/// THD of a sine near `freq` at each input level in `levels_db`.
pub fn thd<P: Processor + Default>(
    params: &P::Params,
    setup: &Setup,
    freq: f64,
    levels_db: &[f64],
) -> Vec<ThdPoint> {
    let bin = setup.odd_bin(freq);
    levels_db
        .iter()
        .map(|&level_db| {
            let s = Spectrum::of(&setup.render_tone::<P>(params, bin, level_db));
            let fundamental = s.energy(bin);
            let mut harmonics_db = [None; 8];
            let mut sum = 0.0;
            for (k, slot) in (2..=9).zip(harmonics_db.iter_mut()) {
                if k * bin < s.len() - LOBE {
                    let e = s.energy(k * bin);
                    sum += e;
                    *slot = Some(power_db(e / fundamental));
                }
            }
            ThdPoint {
                level_db,
                fundamental_db: db(magnitude(s.at(bin))),
                thd_pct: 100.0 * (sum / fundamental).sqrt(),
                harmonics_db,
            }
        })
        .collect()
}

/// Default THD levels: -40 to 0 dBFS in 6 dB steps (and 0).
pub const THD_LEVELS: [f64; 8] = [-42.0, -36.0, -30.0, -24.0, -18.0, -12.0, -6.0, 0.0];

// ---------------------------------------------------------------------------
// aliasing
// ---------------------------------------------------------------------------

/// Aliasing for one test frequency.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AliasPoint {
    /// The measured frequency: the requested one moved to the nearest bin
    /// where every alias can be measured apart from the harmonics.
    pub freq: f64,
    pub fundamental_db: f64,
    /// Energy of the true harmonics below Nyquist, relative to the fundamental.
    pub harmonics_db: f64,
    /// Energy at the frequencies where harmonics above Nyquist fold back,
    /// relative to the fundamental. This is the aliasing.
    pub alias_db: f64,
    /// The loudest single alias, relative to the fundamental.
    pub worst_alias_db: f64,
    pub worst_alias_freq: f64,
    /// [`Self::alias_db`] counting only aliases below [`AUDIBLE_HZ`]. An
    /// oversampler's half-band filters leave aliases between 20 kHz and
    /// Nyquist, from harmonics in their transition band; this leaves them out.
    pub audible_alias_db: f64,
    /// The loudest alias below [`AUDIBLE_HZ`], relative to the fundamental.
    pub worst_audible_alias_db: f64,
    pub worst_audible_alias_freq: f64,
    /// Everything else (noise, and components that are neither), relative to
    /// the fundamental.
    pub residual_db: f64,
}

/// Default aliasing test frequencies.
pub const ALIAS_FREQS: [f64; 6] = [1_000.0, 2_000.0, 4_000.0, 6_000.0, 8_000.0, 10_000.0];

/// Aliasing of sines near each of `freqs` at input level `level_db`.
pub fn aliasing<P: Processor + Default>(
    params: &P::Params,
    setup: &Setup,
    freqs: &[f64],
    level_db: f64,
) -> Vec<AliasPoint> {
    freqs
        .iter()
        .map(|&f| {
            let bin = setup.alias_safe_bin(f);
            let s = Spectrum::of(&setup.render_tone::<P>(params, bin, level_db));
            // Label every bin: 0 = other, 1 = DC or harmonic, 2 = alias.
            let mut label = vec![0u8; s.len()];
            let mark = |centre: usize, value: u8, label: &mut [u8]| {
                let lo = centre.saturating_sub(LOBE);
                let hi = (centre + LOBE).min(label.len() - 1);
                for l in &mut label[lo..=hi] {
                    *l = (*l).max(value);
                }
            };
            for k in 0..=MAX_HARMONIC {
                if k * bin <= FRAME / 2 {
                    mark(k * bin, 1, &mut label);
                }
            }
            let mut aliases = Vec::new();
            for k in 2..=MAX_HARMONIC {
                if k * bin > FRAME / 2 {
                    let at = fold(k, bin);
                    aliases.push(at);
                    mark(at, 2, &mut label);
                }
            }
            let power = |k: usize| {
                let (r, i) = s.at(k);
                r * r + i * i
            };
            let fundamental = s.energy(bin);
            let audible = |k: usize| k as f64 * setup.bin_hz() < AUDIBLE_HZ;
            let (mut harmonic, mut alias, mut audible_alias, mut residual) = (0.0, 0.0, 0.0, 0.0);
            for (k, &l) in label.iter().enumerate() {
                let near_fundamental = k.abs_diff(bin) <= LOBE;
                let near_dc = k <= LOBE;
                match l {
                    _ if near_fundamental || near_dc => {}
                    1 => harmonic += power(k),
                    2 => {
                        alias += power(k);
                        if audible(k) {
                            audible_alias += power(k);
                        }
                    }
                    _ => residual += power(k),
                }
            }
            let loudest = |only_audible: bool| {
                aliases
                    .iter()
                    .filter(|&&a| !only_audible || audible(a))
                    .map(|&a| (a, s.energy(a)))
                    .fold((0, 0.0), |best, x| if x.1 > best.1 { x } else { best })
            };
            let (worst_alias_bin, worst) = loudest(false);
            let (worst_audible_bin, worst_audible) = loudest(true);
            AliasPoint {
                freq: bin as f64 * setup.bin_hz(),
                fundamental_db: db(magnitude(s.at(bin))),
                harmonics_db: power_db(harmonic / fundamental),
                alias_db: power_db(alias / fundamental),
                worst_alias_db: power_db(worst / fundamental),
                worst_alias_freq: worst_alias_bin as f64 * setup.bin_hz(),
                audible_alias_db: power_db(audible_alias / fundamental),
                worst_audible_alias_db: power_db(worst_audible / fundamental),
                worst_audible_alias_freq: worst_audible_bin as f64 * setup.bin_hz(),
                residual_db: power_db(residual / fundamental),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// null
// ---------------------------------------------------------------------------

/// How far a render is from a reference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NullResult {
    /// Gain applied to the render before subtracting, in dB (0 without gain
    /// matching).
    pub gain_db: f64,
    /// RMS of `render − reference`, relative to the reference RMS, in dB.
    pub residual_rms_db: f64,
    /// Peak of `render − reference`, relative to the reference peak, in dB.
    pub residual_peak_db: f64,
    /// Samples compared (the shorter of the two).
    pub samples: usize,
}

/// Compare `rendered` with `reference`. With `gain_match`, `rendered` is
/// first scaled by the least-squares gain that best fits the reference.
pub fn null(rendered: &[f32], reference: &[f32], gain_match: bool) -> NullResult {
    let n = rendered.len().min(reference.len());
    let (y, r) = (&rendered[..n], &reference[..n]);
    let dot = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .map(|(&x, &y)| f64::from(x) * f64::from(y))
            .sum::<f64>()
    };
    let g = if gain_match {
        let yy = dot(y, y);
        if yy > 0.0 { dot(y, r) / yy } else { 1.0 }
    } else {
        1.0
    };
    let mut sq = 0.0;
    let mut peak: f64 = 0.0;
    for (&a, &b) in y.iter().zip(r) {
        let d = g * f64::from(a) - f64::from(b);
        sq += d * d;
        peak = peak.max(d.abs());
    }
    let ref_rms = (dot(r, r) / n.max(1) as f64).sqrt();
    let ref_peak = r.iter().fold(0.0_f64, |m, &x| m.max(f64::from(x).abs()));
    NullResult {
        gain_db: db(g.abs()),
        residual_rms_db: db((sq / n.max(1) as f64).sqrt() / ref_rms),
        residual_peak_db: db(peak / ref_peak),
        samples: n,
    }
}

/// `true` when `x` is within `tol` of `want`, for tests and callers that
/// compare dB values.
pub fn close(x: f64, want: f64, tol: f64) -> bool {
    (x - want).abs() <= tol
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact zero checks")]
mod tests {
    use euterpe_core::{Block, Ctx, ParamKind, params};

    use super::*;

    params! {
        pub struct ShaperParams {
            a: f32 = ("a", "Square term", ParamKind::float(-1.0, 1.0, 0.0)),
            b: f32 = ("b", "Cube term", ParamKind::float(-1.0, 1.0, 0.0)),
            g: f32 = ("g", "Gain", ParamKind::float(0.0, 2.0, 1.0)),
        }
    }

    /// `y = g·x + a·x² + b·x³`: harmonics with closed-form levels.
    #[derive(Default)]
    struct Shaper;
    impl Processor for Shaper {
        type Params = ShaperParams;
        fn prepare(&mut self, _: &Ctx) {}
        fn reset(&mut self) {}
        fn process(&mut self, buf: &mut Block, p: &ShaperParams) {
            for s in buf.iter_mut() {
                let x = *s;
                *s = p.g * x + p.a * x * x + p.b * x * x * x;
            }
        }
    }

    /// One-sample delay: a known phase response.
    #[derive(Default)]
    struct Delay1 {
        z: f32,
    }
    impl Processor for Delay1 {
        type Params = ();
        fn prepare(&mut self, _: &Ctx) {}
        fn reset(&mut self) {
            self.z = 0.0;
        }
        fn process(&mut self, buf: &mut Block, (): &()) {
            for s in buf.iter_mut() {
                (*s, self.z) = (self.z, *s);
            }
        }
    }

    fn shaper(a: f32, b: f32, g: f32) -> ShaperParams {
        ShaperParams { a, b, g }
    }

    #[test]
    fn fft_matches_a_direct_dft() {
        let n = 64;
        let x: Vec<f64> = (0..n).map(|i| ((i * 7 + 3) % 11) as f64 - 5.0).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0; n]);
        fft(&mut re, &mut im);
        for k in 0..n {
            let (mut r, mut i) = (0.0, 0.0);
            for (t, &v) in x.iter().enumerate() {
                let a = -TAU * (k * t) as f64 / n as f64;
                r += v * a.cos();
                i += v * a.sin();
            }
            assert!(
                (re[k] - r).abs() < 1e-9 && (im[k] - i).abs() < 1e-9,
                "bin {k}"
            );
        }
    }

    #[test]
    fn full_scale_sine_reads_as_zero_db() {
        let setup = Setup::default();
        let bin = setup.odd_bin(1_000.0);
        let x = signal::sine(bin as f64 * setup.bin_hz(), 1.0, FRAME, setup.sample_rate);
        assert!(close(db(magnitude(Spectrum::of(&x).at(bin))), 0.0, 1e-4));
    }

    #[test]
    fn gain_is_flat_and_in_phase() {
        let points = response::<Shaper>(
            &shaper(0.0, 0.0, 0.5),
            &Setup::default(),
            &THIRD_OCTAVES,
            -20.0,
        );
        assert_eq!(points.len(), 31);
        for p in &points {
            assert!(close(p.gain_db, db(0.5), 1e-3), "{p:?}");
            assert!(close(p.phase_deg, 0.0, 1e-3), "{p:?}");
        }
    }

    #[test]
    fn one_sample_delay_has_linear_phase() {
        let setup = Setup::default();
        for p in response::<Delay1>(&(), &setup, &[100.0, 1_000.0, 10_000.0], -20.0) {
            let want = -360.0 * p.freq / f64::from(setup.sample_rate);
            assert!(close(p.gain_db, 0.0, 1e-3), "{p:?}");
            assert!(close(p.phase_deg, want, 1e-3), "{p:?} want {want}");
        }
    }

    #[test]
    fn linear_processor_has_no_distortion() {
        let points = thd::<Shaper>(
            &shaper(0.0, 0.0, 1.0),
            &Setup::default(),
            1_000.0,
            &THD_LEVELS,
        );
        for p in &points {
            assert!(p.thd_pct < 1e-4, "{p:?}");
            assert!(close(p.fundamental_db, p.level_db, 1e-3), "{p:?}");
        }
    }

    #[test]
    fn square_term_gives_the_closed_form_second_harmonic() {
        // y = x + a·x², x = A·sin: H2 = a·A²/2, so H2/H1 = a·A/2.
        let (a, level) = (0.5, -6.0);
        let amp = 10f64.powf(level / 20.0);
        let p = &thd::<Shaper>(&shaper(a, 0.0, 1.0), &Setup::default(), 1_000.0, &[level])[0];
        let want = db(f64::from(a) * amp / 2.0);
        assert!(
            close(p.harmonics_db[0].unwrap(), want, 0.01),
            "{p:?} want {want}"
        );
        assert!(
            p.harmonics_db[1].unwrap() < -120.0,
            "no 3rd harmonic: {p:?}"
        );
        assert!(
            close(p.thd_pct, 100.0 * f64::from(a) * amp / 2.0, 0.01),
            "{p:?}"
        );
    }

    #[test]
    fn cube_term_gives_the_closed_form_third_harmonic() {
        // y = x + b·x³: H3 = b·A³/4, H1 = A + 3·b·A³/4.
        let (b, level) = (-0.3, -3.0);
        let amp = 10f64.powf(level / 20.0);
        let b64 = f64::from(b);
        let p = &thd::<Shaper>(&shaper(0.0, b, 1.0), &Setup::default(), 1_000.0, &[level])[0];
        let want = db((b64 * amp.powi(3) / 4.0).abs() / (amp + 0.75 * b64 * amp.powi(3)));
        assert!(
            close(p.harmonics_db[1].unwrap(), want, 0.01),
            "{p:?} want {want}"
        );
        assert!(
            p.harmonics_db[0].unwrap() < -120.0,
            "no 2nd harmonic: {p:?}"
        );
    }

    #[test]
    fn harmonics_above_nyquist_are_none() {
        let p = &thd::<Shaper>(&shaper(0.0, 0.0, 1.0), &Setup::default(), 6_000.0, &[-6.0])[0];
        // 6 kHz: harmonics 2 and 3 (12, 18 kHz) fit below 24 kHz, 4+ don't.
        assert!(
            p.harmonics_db[1].is_some() && p.harmonics_db[2].is_none(),
            "{p:?}"
        );
    }

    #[test]
    fn polynomial_does_not_alias_but_cube_folds_where_predicted() {
        let setup = Setup::default();
        // x³ at 10 kHz: 3rd harmonic at 30 kHz folds to 18 kHz.
        // The folded 3rd harmonic keeps its closed-form level, b·A²/4 / (1 + 3·b·A²/4).
        let p = aliasing::<Shaper>(&shaper(0.0, -0.3, 1.0), &setup, &[10_000.0], -3.0)[0];
        let a2 = 10f64.powf(-3.0 / 10.0);
        let want = db(0.3 * a2 / 4.0 / (1.0 - 0.75 * 0.3 * a2));
        assert!(close(p.alias_db, want, 0.01), "{p:?} want {want}");
        assert!(
            close(p.worst_alias_freq, 48_000.0 - 3.0 * p.freq, setup.bin_hz()),
            "{p:?}"
        );
        // A cubic at 4 kHz stays below Nyquist (12 kHz): nothing to fold.
        let q = aliasing::<Shaper>(&shaper(0.0, -0.3, 1.0), &setup, &[4_000.0], -3.0)[0];
        assert!(q.alias_db < -120.0 && q.harmonics_db > -30.0, "{q:?}");
    }

    #[test]
    fn null_of_identical_signals_is_silent() {
        let x = signal::noise(0.5, 4_096, 7);
        let r = null(&x, &x, false);
        assert_eq!(r.residual_rms_db, f64::NEG_INFINITY);
        assert_eq!(r.gain_db, 0.0);
    }

    #[test]
    fn null_reports_a_gain_error_unless_matched() {
        let x = signal::noise(0.5, 4_096, 7);
        let half: Vec<f32> = x.iter().map(|s| s * 0.5).collect();
        let raw = null(&half, &x, false);
        assert!(close(raw.residual_rms_db, db(0.5), 1e-6), "{raw:?}");
        let matched = null(&half, &x, true);
        assert!(close(matched.gain_db, db(2.0), 1e-6), "{matched:?}");
        assert!(matched.residual_rms_db < -120.0, "{matched:?}");
    }

    #[test]
    fn alias_safe_bins_keep_every_alias_clear_of_harmonics() {
        let setup = Setup::default();
        for f in ALIAS_FREQS {
            let b = setup.alias_safe_bin(f);
            assert!(alias_clearance(b) > 2 * LOBE, "{f} Hz");
            assert!(
                (b as f64 * setup.bin_hz() - f).abs() < 50.0,
                "{f} Hz moved too far"
            );
        }
    }

    #[test]
    fn odd_bins_never_put_an_alias_on_a_harmonic() {
        let setup = Setup::default();
        for f in ALIAS_FREQS {
            let b = setup.odd_bin(f);
            assert_eq!(b % 2, 1);
            for k in 2..=MAX_HARMONIC {
                if k * b > FRAME / 2 {
                    let at = fold(k, b);
                    assert!(
                        !at.is_multiple_of(b) || at == 0,
                        "{f} Hz: harmonic {k} folds onto a harmonic"
                    );
                }
            }
        }
    }
}
