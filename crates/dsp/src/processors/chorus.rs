use crate::Process;
use crate::params::{Knob, Toggle};
use crate::utils::{DelayLine, Lfo};

/// Default sample rate.
const DEFAULT_SAMPLE_RATE: u32 = 48000;

/// Default chorus rate in Hz.
const DEFAULT_RATE_HZ: f32 = 0.5;

/// Default chorus depth (0.0 - 1.0).
const DEFAULT_DEPTH: f32 = 0.3;

/// Default wet/dry mix (0.0 - 1.0).
const DEFAULT_MIX: f32 = 0.3;

/// Center delay time in seconds.
const CENTER_DELAY_SEC: f32 = 0.007; // 7ms center delay

/// Modulation depth in seconds.
const MOD_DEPTH_SEC: f32 = 0.003; // +/- 3ms modulation

pub struct ChorusParams {
    pub enabled: Toggle,
    pub rate: Knob,
    pub depth: Knob,
    pub mix: Knob,
}

impl ChorusParams {
    pub fn new(sample_rate: u32) -> Self {
        let mut rate = Knob::new("Rate", DEFAULT_RATE_HZ, 0.1, 5.0);
        rate.init(sample_rate);

        let mut depth = Knob::new("Depth", DEFAULT_DEPTH, 0.0, 1.0);
        depth.init(sample_rate);

        let mut mix = Knob::new("Mix", DEFAULT_MIX, 0.0, 1.0);
        mix.init(sample_rate);

        Self {
            enabled: Toggle::new("Enabled", false),
            rate,
            depth,
            mix,
        }
    }
}

impl Default for ChorusParams {
    fn default() -> Self {
        Self::new(DEFAULT_SAMPLE_RATE)
    }
}

pub struct Chorus {
    params: ChorusParams,
    sample_rate: f32,
    delay: DelayLine<2048>, // ~42ms at 48kHz
    lfo: Lfo,
}

impl Chorus {
    pub fn new() -> Self {
        Self::with_sample_rate(DEFAULT_SAMPLE_RATE)
    }

    pub fn with_sample_rate(sample_rate: u32) -> Self {
        Self {
            params: ChorusParams::new(sample_rate),
            sample_rate: sample_rate as f32,
            delay: DelayLine::new(),
            lfo: Lfo::new(DEFAULT_RATE_HZ, sample_rate),
        }
    }

    pub fn params(&self) -> &ChorusParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut ChorusParams {
        &mut self.params
    }

    pub fn reset(&mut self) {
        self.delay.clear();
        self.lfo.reset();
    }
}

impl Default for Chorus {
    fn default() -> Self {
        Self::new()
    }
}

impl Process for Chorus {
    #[inline]
    fn process(&mut self, sample: f32) -> f32 {
        if !self.params.enabled.get() {
            return sample;
        }

        let rate = self.params.rate.get_smoothed();
        let depth = self.params.depth.get_smoothed();
        let mix = self.params.mix.get_smoothed();

        // Update LFO rate
        self.lfo.set_rate(rate);

        // Get LFO value (sine wave, range -1 to 1)
        let lfo = self.lfo.tick();

        // Write input to delay buffer
        self.delay.write(sample);

        // Calculate modulated delay time
        let center_samples = CENTER_DELAY_SEC * self.sample_rate;
        let mod_samples = MOD_DEPTH_SEC * self.sample_rate * depth;
        let delay_samples = center_samples + lfo * mod_samples;

        // Read from delay line
        let delayed = self.delay.read(delay_samples);

        // Mix dry and wet signals
        sample * (1.0 - mix) + delayed * mix
    }
}
