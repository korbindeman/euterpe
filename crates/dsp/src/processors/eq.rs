use crate::Process;
use crate::biquad::Biquad;
use crate::params::{Knob, Toggle};

/// Default EQ mid Q factor.
pub const DEFAULT_MID_Q: f32 = 1.0;

/// Default air band boost in dB when enabled.
pub const AIR_BAND_BOOST_DB: f32 = 6.0;

pub struct ThreeBandEqParams {
    pub low_enabled: Toggle,
    pub mid_enabled: Toggle,
    pub high_enabled: Toggle,
    pub air_enabled: Toggle,
    pub low_gain: Knob,
    pub mid_gain: Knob,
    pub high_gain: Knob,
}

impl ThreeBandEqParams {
    pub fn new(default_gain_db: f32) -> Self {
        Self {
            low_enabled: Toggle::new("Low", true),
            mid_enabled: Toggle::new("Mid", true),
            high_enabled: Toggle::new("High", true),
            air_enabled: Toggle::new("Air", true),
            low_gain: Knob::new("Low Gain", default_gain_db, -12.0, 12.0),
            mid_gain: Knob::new("Mid Gain", default_gain_db, -12.0, 12.0),
            high_gain: Knob::new("High Gain", default_gain_db, -12.0, 12.0),
        }
    }
}

/// Three-band equalizer (low shelf, peaking mid, high shelf) with optional air band.
pub struct ThreeBandEq {
    params: ThreeBandEqParams,
    low: Biquad,
    mid: Biquad,
    high: Biquad,
    air: Biquad,
    // Configuration
    low_freq: f32,
    mid_freq: f32,
    high_freq: f32,
    air_freq: f32,
    mid_q: f32,
    sample_rate: f32,
}

impl ThreeBandEq {
    /// Create a new three-band EQ with optional air band.
    ///
    /// * `low_freq` — low shelf frequency in Hz
    /// * `mid_freq` — mid peaking frequency in Hz
    /// * `high_freq` — high shelf frequency in Hz
    /// * `air_freq` — air band high shelf frequency in Hz
    /// * `default_gain_db` — initial gain for all bands
    /// * `mid_q` — Q factor for mid band
    /// * `sample_rate` — sample rate in Hz
    pub fn new(
        low_freq: f32,
        mid_freq: f32,
        high_freq: f32,
        air_freq: f32,
        default_gain_db: f32,
        mid_q: f32,
        sample_rate: f32,
    ) -> Self {
        Self {
            params: ThreeBandEqParams::new(default_gain_db),
            low: Biquad::low_shelf(low_freq, default_gain_db, sample_rate),
            mid: Biquad::peaking(mid_freq, default_gain_db, mid_q, sample_rate),
            high: Biquad::high_shelf(high_freq, default_gain_db, sample_rate),
            air: Biquad::high_shelf(air_freq, AIR_BAND_BOOST_DB, sample_rate),
            low_freq,
            mid_freq,
            high_freq,
            air_freq,
            mid_q,
            sample_rate,
        }
    }

    pub fn params(&self) -> &ThreeBandEqParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut ThreeBandEqParams {
        &mut self.params
    }

    fn update_coefficients(&mut self) {
        if self.params.low_gain.take_dirty() {
            self.low.reconfigure(Biquad::low_shelf(
                self.low_freq,
                self.params.low_gain.get(),
                self.sample_rate,
            ));
        }
        if self.params.mid_gain.take_dirty() {
            self.mid.reconfigure(Biquad::peaking(
                self.mid_freq,
                self.params.mid_gain.get(),
                self.mid_q,
                self.sample_rate,
            ));
        }
        if self.params.high_gain.take_dirty() {
            self.high.reconfigure(Biquad::high_shelf(
                self.high_freq,
                self.params.high_gain.get(),
                self.sample_rate,
            ));
        }
        // Clear toggle dirty flags (they don't trigger recalculation)
        let _ = self.params.low_enabled.take_dirty();
        let _ = self.params.mid_enabled.take_dirty();
        let _ = self.params.high_enabled.take_dirty();
        let _ = self.params.air_enabled.take_dirty();
    }

    /// Reconfigure all bands with new frequencies (resets filter state).
    pub fn reconfigure(
        &mut self,
        low_freq: f32,
        mid_freq: f32,
        high_freq: f32,
        air_freq: f32,
        sample_rate: f32,
    ) {
        self.low_freq = low_freq;
        self.mid_freq = mid_freq;
        self.high_freq = high_freq;
        self.air_freq = air_freq;
        self.sample_rate = sample_rate;
        self.low.reconfigure(Biquad::low_shelf(
            low_freq,
            self.params.low_gain.get(),
            sample_rate,
        ));
        self.mid.reconfigure(Biquad::peaking(
            mid_freq,
            self.params.mid_gain.get(),
            self.mid_q,
            sample_rate,
        ));
        self.high.reconfigure(Biquad::high_shelf(
            high_freq,
            self.params.high_gain.get(),
            sample_rate,
        ));
        self.air
            .reconfigure(Biquad::high_shelf(air_freq, AIR_BAND_BOOST_DB, sample_rate));
    }
}

impl Process for ThreeBandEq {
    #[inline]
    fn process(&mut self, sample: f32) -> f32 {
        self.update_coefficients();

        let mut s = sample;
        if self.params.low_enabled.get() {
            s = self.low.process(s);
        }
        if self.params.mid_enabled.get() {
            s = self.mid.process(s);
        }
        if self.params.high_enabled.get() {
            s = self.high.process(s);
        }
        if self.params.air_enabled.get() {
            s = self.air.process(s);
        }
        s
    }
}
