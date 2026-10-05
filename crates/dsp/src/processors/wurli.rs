//! Guitar-to-Wurlitzer processor (Key9-style, polyphonic, no pitch tracking).
//!
//! Signal flow:
//!
//! ```text
//! in -> HPF -> semitone band-pass bank (E2..D#6)
//!            -> per band: harmonic masking ("purity") + Wurli-style decay
//!            -> sum (+ pick "tine" click on onsets)
//!            -> pickup asymmetry (even harmonics) -> velocity-driven soft clip ("bark")
//!            -> tone LPF + 200A-ish mid bump -> tremolo -> dry/wet -> level
//! ```
//!
//! Because each band is narrow, its output is close to a sine at that band's
//! pitch. Guitar overtones also land in higher bands, so a band is attenuated
//! when the band an octave / twelfth / two octaves below it is louder: that
//! leaves mostly fundamentals, which is the core of the reed sound. The
//! saturator then puts back the even harmonics a Wurli pickup produces.

#[allow(unused_imports)]
use micromath::F32Ext;

use crate::Process;
use crate::biquad::Biquad;
use crate::params::{Knob, Toggle};
use crate::utils::{EnvelopeFollower, Lfo, db_to_linear, tanhf, time_constant};

const DEFAULT_SAMPLE_RATE: u32 = 48_000;

/// Number of semitone bands (4 octaves: E2 .. D#6).
pub const NUM_BANDS: usize = 48;
/// Lowest band center: low E on a guitar.
const LOWEST_HZ: f32 = 82.406_89;
/// Band Q. ~12 gives roughly 1.4 semitones of bandwidth, so bands overlap.
const BAND_Q: f32 = 12.0;
/// Band-bank makeup gain (adjacent bands overlap, so the sum runs hot).
const BANK_GAIN: f32 = 1.6;

/// Harmonic masking: (semitones below, weight) for the 2nd..5th harmonic.
const MASKS: [(usize, f32); 4] = [(12, 1.0), (19, 0.7), (24, 0.5), (28, 0.4)];
/// How much louder than the band(s) below a partial must be to survive,
/// at full purity.
const MASK_STRENGTH: f32 = 6.0;

/// Envelope floor below which a band is treated as silent.
const FLOOR: f32 = 1e-5;
/// Onset if fast envelope exceeds slow envelope by this ratio.
const ONSET_RATIO: f32 = 1.4;

/// Asymmetry of the pickup nonlinearity (adds 2nd harmonic).
const BIAS: f32 = 0.35;

pub struct WurliParams {
    pub enabled: Toggle,
    /// 0..1: how hard upper partials are suppressed in favour of fundamentals.
    pub purity: Knob,
    /// Seconds for the lowest band to decay by 1/e; higher bands decay faster.
    pub decay: Knob,
    /// 0..1: how much the pickup/amp growls on hard picking.
    pub bark: Knob,
    /// 0..1: level of the tine click on note onsets.
    pub attack: Knob,
    /// Tone low-pass cutoff in Hz.
    pub tone: Knob,
    pub trem_rate: Knob,
    pub trem_depth: Knob,
    /// 0 = dry guitar, 1 = full Wurli.
    pub mix: Knob,
    /// Output level in dB.
    pub level: Knob,
}

impl WurliParams {
    pub fn new(sample_rate: u32) -> Self {
        let knob = |name, value, min, max| {
            let mut k = Knob::new(name, value, min, max);
            k.init(sample_rate);
            k
        };
        Self {
            enabled: Toggle::new("Enabled", true),
            purity: knob("Purity", 0.8, 0.0, 1.0),
            decay: knob("Decay (s)", 1.2, 0.2, 4.0),
            bark: knob("Bark", 0.5, 0.0, 1.0),
            attack: knob("Attack click", 0.35, 0.0, 1.0),
            tone: knob("Tone (Hz)", 3500.0, 800.0, 8000.0),
            trem_rate: knob("Tremolo rate", 5.5, 1.0, 10.0),
            trem_depth: knob("Tremolo depth", 0.0, 0.0, 1.0),
            mix: knob("Mix", 1.0, 0.0, 1.0),
            level: knob("Level (dB)", 0.0, -24.0, 12.0),
        }
    }
}

impl Default for WurliParams {
    fn default() -> Self {
        Self::new(DEFAULT_SAMPLE_RATE)
    }
}

struct Band {
    filter: Biquad,
    /// Fast envelope (tracks the note).
    fast: EnvelopeFollower,
    /// Slow envelope (reference for onset detection).
    slow: EnvelopeFollower,
    /// Imposed Wurli envelope: jumps up on onsets, then decays exponentially.
    held: f32,
    /// Per-sample decay coefficient for `held`.
    decay_coeff: f32,
    /// Smoothed output gain, to avoid zipper noise.
    gain: f32,
}

pub struct Wurli {
    params: WurliParams,
    sample_rate: u32,
    input_hpf: Biquad,
    bands: [Band; NUM_BANDS],
    /// Copy of fast envelopes, so masking reads a consistent snapshot.
    env: [f32; NUM_BANDS],
    gain_smooth: f32,
    // Velocity / onset tracking on the full input.
    vel_fast: EnvelopeFollower,
    vel_slow: EnvelopeFollower,
    click_hpf: Biquad,
    dc_block: Biquad,
    click_env: f32,
    click_decay: f32,
    // Post processing.
    tone_lpf: Biquad,
    mid_bump: Biquad,
    lfo: Lfo,
    last_decay: f32,
    last_tone: f32,
    bias_offset: f32,
    bias_norm: f32,
}

impl Wurli {
    pub fn new() -> Self {
        Self::with_sample_rate(DEFAULT_SAMPLE_RATE)
    }

    pub fn with_sample_rate(sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        let params = WurliParams::new(sample_rate);
        let decay = params.decay.get();
        let tone = params.tone.get();

        let bands = core::array::from_fn(|k| {
            let freq = LOWEST_HZ * 2.0f32.powf(k as f32 / 12.0);
            // Release long enough to ride over a few cycles of the band.
            let release_ms = (4000.0 / freq).max(15.0);
            Band {
                filter: Biquad::bandpass(freq, BAND_Q, sr),
                fast: EnvelopeFollower::new(1.0, release_ms, sample_rate),
                slow: EnvelopeFollower::new(60.0, 400.0, sample_rate),
                held: 0.0,
                decay_coeff: band_decay_coeff(k, decay, sample_rate),
                gain: 0.0,
            }
        });

        Self {
            params,
            sample_rate,
            input_hpf: Biquad::highpass(65.0, 0.707, sr),
            bands,
            env: [0.0; NUM_BANDS],
            gain_smooth: 1.0 - time_constant(2.0, sample_rate),
            vel_fast: EnvelopeFollower::new(1.0, 60.0, sample_rate),
            vel_slow: EnvelopeFollower::new(40.0, 300.0, sample_rate),
            click_hpf: Biquad::highpass(2500.0, 0.707, sr),
            dc_block: Biquad::highpass(30.0, 0.707, sr),
            click_env: 0.0,
            click_decay: time_constant(6.0, sample_rate),
            tone_lpf: Biquad::lowpass(tone, 0.707, sr),
            mid_bump: Biquad::peaking(900.0, 3.0, 0.9, sr),
            lfo: Lfo::new(5.5, sample_rate),
            last_decay: decay,
            last_tone: tone,
            bias_offset: tanhf(BIAS),
            bias_norm: 1.0 / (1.0 - tanhf(BIAS) * tanhf(BIAS)),
        }
    }

    pub fn params(&self) -> &WurliParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut WurliParams {
        &mut self.params
    }

    fn update_coefficients(&mut self) {
        let decay = self.params.decay.get();
        if self.params.decay.take_dirty() || decay != self.last_decay {
            self.last_decay = decay;
            for (k, band) in self.bands.iter_mut().enumerate() {
                band.decay_coeff = band_decay_coeff(k, decay, self.sample_rate);
            }
        }
        let tone = self.params.tone.get();
        if self.params.tone.take_dirty() || tone != self.last_tone {
            self.last_tone = tone;
            self.tone_lpf
                .retune(Biquad::lowpass(tone, 0.707, self.sample_rate as f32));
        }
    }
}

/// Flush tiny values to zero: decaying state would otherwise go subnormal,
/// which is very slow on x86 (and pointless everywhere).
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-15 { 0.0 } else { x }
}

/// Higher bands decay faster: time constant halves every two octaves.
fn band_decay_coeff(k: usize, decay_s: f32, sample_rate: u32) -> f32 {
    let tau_ms = decay_s * 1000.0 * 2.0f32.powf(-(k as f32) / 24.0);
    time_constant(tau_ms, sample_rate)
}

impl Default for Wurli {
    fn default() -> Self {
        Self::new()
    }
}

impl Process for Wurli {
    fn process(&mut self, sample: f32) -> f32 {
        if !self.params.enabled.get() {
            return sample;
        }
        self.update_coefficients();

        let purity = self.params.purity.get_smoothed();
        let bark = self.params.bark.get_smoothed();
        let attack = self.params.attack.get_smoothed();
        let trem_depth = self.params.trem_depth.get_smoothed();
        let mix = self.params.mix.get_smoothed();
        let level = db_to_linear(self.params.level.get_smoothed());
        self.lfo.set_rate(self.params.trem_rate.get_smoothed());

        let x = self.input_hpf.process(sample);

        // Global velocity + onset (drives click and bark).
        let vel = self.vel_fast.process(x);
        let vel_ref = self.vel_slow.process(x);
        if vel > ONSET_RATIO * vel_ref + 1e-3 {
            self.click_env = self.click_env.max((vel / (vel_ref + 1e-3)).min(4.0) * 0.25);
        }
        self.click_env = flush(self.click_env * self.click_decay);

        // Pass 1: filter every band and update its envelopes.
        let mut band_out = [0.0f32; NUM_BANDS];
        for (k, band) in self.bands.iter_mut().enumerate() {
            let y = band.filter.process(x);
            let fast = band.fast.process(y);
            let slow = band.slow.process(y);
            if fast > ONSET_RATIO * slow + FLOOR {
                band.held = band.held.max(fast);
            } else {
                band.held = flush(band.held * band.decay_coeff);
            }
            band_out[k] = y;
            self.env[k] = fast;
        }

        // Pass 2: harmonic masking + imposed decay, then sum.
        let mut sum = 0.0;
        for k in 0..NUM_BANDS {
            let band = &mut self.bands[k];
            let e = self.env[k];

            let mut mask = 0.0;
            for &(below, weight) in MASKS.iter() {
                if k >= below {
                    mask += weight * self.env[k - below];
                }
            }
            // Ratio of "this is probably an overtone" evidence to own level.
            let r = purity * MASK_STRENGTH * mask / (e + FLOOR);
            let purity_gain = 1.0 / (1.0 + r * r * r);
            let decay_gain = if e > FLOOR {
                (band.held / e).min(1.0)
            } else {
                0.0
            };

            let target = purity_gain * decay_gain;
            band.gain = flush(band.gain + self.gain_smooth * (target - band.gain));
            sum += band_out[k] * band.gain;
        }
        sum *= BANK_GAIN;

        // Tine click on onsets.
        sum += self.click_hpf.process(x) * self.click_env * attack * 2.0;

        // Pickup: a Wurli's electrostatic pickup is strongly asymmetric, so
        // add a rectified copy (2nd harmonic, level independent), more of it
        // the harder you pick, then block the DC it creates.
        let pick = (vel * 20.0).min(1.0);
        let even = bark * (0.25 + 0.5 * pick);
        let pickup = self.dc_block.process(sum + even * sum.abs());

        // Amp bark: biased soft clip. Drive rises with picking strength;
        // dividing by sqrt(drive) keeps hard notes a little louder while the
        // clipper flattens their peaks.
        let drive = 1.0 + bark * (2.0 + 40.0 * vel);
        let shaped =
            (tanhf(drive * pickup + BIAS) - self.bias_offset) * self.bias_norm / drive.sqrt();

        // Speaker-ish voicing.
        let voiced = self.tone_lpf.process(self.mid_bump.process(shaped));

        // Tremolo.
        let trem = 1.0 - trem_depth * (0.5 + 0.5 * self.lfo.tick());
        let wet = voiced * trem;

        (sample * (1.0 - mix) + wet * mix) * level
    }
}
