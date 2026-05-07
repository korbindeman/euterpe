#[allow(unused_imports)]
use micromath::F32Ext;

use crate::Process;

/// Standard biquad filter (Direct Form I).
///
/// Coefficients follow the Audio EQ Cookbook by Robert Bristow-Johnson.
pub struct Biquad {
    // Normalized coefficients (a0 is folded in during construction).
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    // Delay line state (Direct Form I).
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Process for Biquad {
    #[inline]
    fn process(&mut self, sample: f32) -> f32 {
        let out = self.b0 * sample + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;

        self.x2 = self.x1;
        self.x1 = sample;
        self.y2 = self.y1;
        self.y1 = out;

        out
    }
}

impl Biquad {
    /// Create a low-shelf biquad filter.
    ///
    /// * `freq` — shelf transition frequency in Hz
    /// * `gain_db` — boost (positive) or cut (negative) in dB
    /// * `sample_rate` — sample rate in Hz
    pub fn low_shelf(freq: f32, gain_db: f32, sample_rate: f32) -> Self {
        let a = (10.0f32.powf(gain_db / 20.0)).sqrt();
        let w0 = 2.0 * core::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        // Q = 1/sqrt(2) for a standard shelf slope.
        let alpha = sin_w0 / (2.0 * core::f32::consts::FRAC_1_SQRT_2);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);

        Self::from_raw(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }

    /// Create a high-shelf biquad filter.
    ///
    /// * `freq` — shelf transition frequency in Hz
    /// * `gain_db` — boost (positive) or cut (negative) in dB
    /// * `sample_rate` — sample rate in Hz
    pub fn high_shelf(freq: f32, gain_db: f32, sample_rate: f32) -> Self {
        let a = (10.0f32.powf(gain_db / 20.0)).sqrt();
        let w0 = 2.0 * core::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * core::f32::consts::FRAC_1_SQRT_2);
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;
        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);

        Self::from_raw(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }

    /// Create a peaking EQ biquad filter.
    ///
    /// * `freq` — center frequency in Hz
    /// * `gain_db` — boost (positive) or cut (negative) in dB
    /// * `q` — quality factor
    /// * `sample_rate` — sample rate in Hz
    pub fn peaking(freq: f32, gain_db: f32, q: f32, sample_rate: f32) -> Self {
        let a = (10.0f32.powf(gain_db / 20.0)).sqrt();
        let w0 = 2.0 * core::f32::consts::PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;

        Self::from_raw(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
    }

    /// Replace all coefficients (and reset state) from another `Biquad`.
    pub fn reconfigure(&mut self, other: Biquad) {
        *self = other;
    }

    /// Build from pre-normalized coefficients with zeroed state.
    fn from_raw(b0: f32, b1: f32, b2: f32, a1: f32, a2: f32) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }
}
