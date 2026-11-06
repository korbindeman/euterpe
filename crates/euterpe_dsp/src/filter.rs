//! Filters: a state-variable filter for everything second-order, and a
//! one-pole for tone controls and DC blocking.
//!
//! Both are topology-preserving transforms (Zavalishin; the SVF as in
//! Andrew Simper's "Linear Trapezoidal Integrated SVF"): the bilinear
//! transform of the analog filter, prewarped so the cutoff lands exactly,
//! and stable when the coefficients change every block. Compute coefficients
//! once per block, from smoothed parameters, and run the filter per sample.
//!
//! Frequencies are normalized: `f = hz / sample_rate`, in `(0, 0.49]`
//! (`hz * ctx.dt()`).

use euterpe_core::fastmath;

/// Coefficients of an [`Svf`]: the shared core `a1..a3` and the output mix
/// `m0 * input + m1 * band + m2 * low`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvfCoeffs {
    a1: f32,
    a2: f32,
    a3: f32,
    m0: f32,
    m1: f32,
    m2: f32,
}

impl Default for SvfCoeffs {
    fn default() -> Self {
        Self::PASS
    }
}

impl SvfCoeffs {
    /// Passes the input through unchanged.
    pub const PASS: Self = Self {
        a1: 1.0,
        a2: 0.0,
        a3: 0.0,
        m0: 1.0,
        m1: 0.0,
        m2: 0.0,
    };

    /// The core for prewarped `g = tan(pi f)` and damping `k = 1 / Q`.
    #[inline]
    fn core(g: f32, k: f32, m0: f32, m1: f32, m2: f32) -> Self {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self {
            a1,
            a2,
            a3: g * a2,
            m0,
            m1,
            m2,
        }
    }

    /// `k = 1 / Q`, with `Q` kept away from 0.
    #[inline]
    fn damping(q: f32) -> f32 {
        1.0 / q.max(0.05)
    }

    /// 12 dB/octave lowpass with resonance `q` (0.707 is Butterworth).
    #[must_use]
    pub fn lowpass(f: f32, q: f32) -> Self {
        Self::core(fastmath::tan_pi(f), Self::damping(q), 0.0, 0.0, 1.0)
    }

    /// 12 dB/octave highpass.
    #[must_use]
    pub fn highpass(f: f32, q: f32) -> Self {
        let k = Self::damping(q);
        Self::core(fastmath::tan_pi(f), k, 1.0, -k, -1.0)
    }

    /// Bandpass with unity gain at the center.
    #[must_use]
    pub fn bandpass(f: f32, q: f32) -> Self {
        let k = Self::damping(q);
        Self::core(fastmath::tan_pi(f), k, 0.0, k, 0.0)
    }

    /// Notch: zero gain at the center.
    #[must_use]
    pub fn notch(f: f32, q: f32) -> Self {
        let k = Self::damping(q);
        Self::core(fastmath::tan_pi(f), k, 1.0, -k, 0.0)
    }

    /// Allpass: unity gain, phase turns through 180 degrees at the center.
    #[must_use]
    pub fn allpass(f: f32, q: f32) -> Self {
        let k = Self::damping(q);
        Self::core(fastmath::tan_pi(f), k, 1.0, -2.0 * k, 0.0)
    }

    /// Peaking EQ: `gain_db` at the center, unity far from it.
    #[must_use]
    pub fn bell(f: f32, q: f32, gain_db: f32) -> Self {
        let a = fastmath::db_to_gain(0.5 * gain_db);
        let k = Self::damping(q) / a;
        Self::core(fastmath::tan_pi(f), k, 1.0, k * (a * a - 1.0), 0.0)
    }

    /// Low shelf: `gain_db` below `f`, unity above, half the gain (in dB)
    /// at `f`.
    #[must_use]
    pub fn low_shelf(f: f32, q: f32, gain_db: f32) -> Self {
        let a = fastmath::db_to_gain(0.5 * gain_db);
        let k = Self::damping(q);
        let g = fastmath::tan_pi(f) / fastmath::db_to_gain(0.25 * gain_db);
        Self::core(g, k, 1.0, k * (a - 1.0), a * a - 1.0)
    }

    /// High shelf: `gain_db` above `f`, unity below.
    #[must_use]
    pub fn high_shelf(f: f32, q: f32, gain_db: f32) -> Self {
        let a = fastmath::db_to_gain(0.5 * gain_db);
        let k = Self::damping(q);
        let g = fastmath::tan_pi(f) * fastmath::db_to_gain(0.25 * gain_db);
        Self::core(g, k, a * a, k * (1.0 - a) * a, 1.0 - a * a)
    }
}

/// A second-order state-variable filter. Its response is set by the
/// [`SvfCoeffs`] passed to each call, so one set of coefficients can drive
/// several channels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// Filter one sample.
    #[inline(always)]
    pub fn tick(&mut self, c: &SvfCoeffs, x: f32) -> f32 {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        c.m0 * x + c.m1 * v1 + c.m2 * v2
    }

    /// Filter a buffer in place.
    #[inline]
    pub fn run(&mut self, c: &SvfCoeffs, buf: &mut [f32]) {
        for s in buf {
            *s = self.tick(c, *s);
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Coefficient of a [`OnePole`] with cutoff `f`.
#[must_use]
#[inline]
pub fn one_pole_coeff(f: f32) -> f32 {
    let g = fastmath::tan_pi(f);
    g / (1.0 + g)
}

/// A one-pole (6 dB/octave) filter giving lowpass and highpass outputs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OnePole {
    s: f32,
}

impl OnePole {
    /// The lowpass output, for the coefficient from [`one_pole_coeff`].
    #[inline(always)]
    pub fn lowpass(&mut self, c: f32, x: f32) -> f32 {
        let v = (x - self.s) * c;
        let y = v + self.s;
        self.s = y + v;
        y
    }

    /// The highpass output: the input minus the lowpass.
    #[inline(always)]
    pub fn highpass(&mut self, c: f32, x: f32) -> f32 {
        x - self.lowpass(c, x)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A DC blocker: `y[n] = x[n] - x[n-1] + r y[n-1]`, a zero exactly at DC
/// and a pole just inside it. Unlike a [`OnePole`] highpass at a few Hz,
/// whose f32 state stalls about -87 dB short of removing DC, this removes it
/// completely.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DcBlocker {
    x1: f32,
    y1: f32,
}

/// The pole of a [`DcBlocker`] with its -3 dB point near `f`.
#[must_use]
#[inline]
pub fn dc_blocker_coeff(f: f32) -> f32 {
    1.0 - core::f32::consts::TAU * f.clamp(0.0, 0.01)
}

impl DcBlocker {
    #[inline(always)]
    pub fn tick(&mut self, r: f32, x: f32) -> f32 {
        let y = x - self.x1 + r * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;

    use super::*;

    type C64 = (f64, f64);

    fn mul(a: C64, b: C64) -> C64 {
        (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
    }

    fn div(a: C64, b: C64) -> C64 {
        let d = b.0 * b.0 + b.1 * b.1;
        ((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
    }

    /// `sum c_i s^i` for coefficients from the constant term up.
    fn poly(c: &[f64], s: C64) -> C64 {
        c.iter().rev().fold((0.0, 0.0), |acc, &ci| {
            let m = mul(acc, s);
            (m.0 + ci, m.1)
        })
    }

    /// The magnitude in dB of `filter`'s response at `freq` (normalized),
    /// from the DFT of its impulse response.
    fn measured_db(mut filter: impl FnMut(f32) -> f32, freq: f64) -> f64 {
        let ir: Vec<f64> = (0..1 << 15)
            .map(|n| f64::from(filter(if n == 0 { 1.0 } else { 0.0 })))
            .collect();
        let (mut re, mut im) = (0.0, 0.0);
        for (n, &h) in ir.iter().enumerate() {
            let a = -core::f64::consts::TAU * freq * n as f64;
            re += h * a.cos();
            im += h * a.sin();
        }
        10.0 * (re * re + im * im).log10()
    }

    /// The analog prototype `num(s) / den(s)` (cutoff 1 rad/s), through the
    /// prewarped bilinear transform to cutoff `fc`, at `freq`, in dB.
    fn prototype_db(num: &[f64], den: &[f64], fc: f64, freq: f64) -> f64 {
        let w = (core::f64::consts::PI * freq).tan() / (core::f64::consts::PI * fc).tan();
        let h = div(poly(num, (0.0, w)), poly(den, (0.0, w)));
        10.0 * (h.0 * h.0 + h.1 * h.1).log10()
    }

    const FREQS: [f64; 9] = [
        20.0 / 48e3,
        100.0 / 48e3,
        400.0 / 48e3,
        900.0 / 48e3,
        1e3 / 48e3,
        1.2e3 / 48e3,
        4e3 / 48e3,
        10e3 / 48e3,
        20e3 / 48e3,
    ];

    /// Every mode against its textbook analog prototype (the RBJ cookbook's
    /// for the EQ shapes), across the band.
    #[test]
    fn svf_matches_the_analog_prototypes() {
        let fc = 1e3 / 48e3;
        let q = 2.0_f64;
        let (gain_db, a) = (9.0_f32, 10.0_f64.powf(9.0 / 40.0));
        let sa = a.sqrt();
        let iq = 1.0 / q;
        let den = [1.0, iq, 1.0];
        #[expect(clippy::cast_possible_truncation, reason = "test constants")]
        let (f, qf) = (fc as f32, q as f32);
        let cases: [(&str, SvfCoeffs, Vec<f64>, Vec<f64>); 8] = [
            (
                "lowpass",
                SvfCoeffs::lowpass(f, qf),
                [1.0].into(),
                den.into(),
            ),
            (
                "highpass",
                SvfCoeffs::highpass(f, qf),
                [0.0, 0.0, 1.0].into(),
                den.into(),
            ),
            (
                "bandpass",
                SvfCoeffs::bandpass(f, qf),
                [0.0, iq].into(),
                den.into(),
            ),
            (
                "notch",
                SvfCoeffs::notch(f, qf),
                [1.0, 0.0, 1.0].into(),
                den.into(),
            ),
            (
                "allpass",
                SvfCoeffs::allpass(f, qf),
                [1.0, -iq, 1.0].into(),
                den.into(),
            ),
            (
                "bell",
                SvfCoeffs::bell(f, qf, gain_db),
                [1.0, a / q, 1.0].into(),
                [1.0, 1.0 / (a * q), 1.0].into(),
            ),
            (
                "low_shelf",
                SvfCoeffs::low_shelf(f, qf, gain_db),
                [a * a, a * sa / q, a].into(),
                [1.0, sa / q, a].into(),
            ),
            (
                "high_shelf",
                SvfCoeffs::high_shelf(f, qf, gain_db),
                [a, a * sa / q, a * a].into(),
                [a, sa / q, 1.0].into(),
            ),
        ];
        for (name, c, num, den) in cases {
            for freq in FREQS {
                let mut svf = Svf::default();
                let got = measured_db(|x| svf.tick(&c, x), freq);
                let want = prototype_db(&num, &den, fc, freq);
                // The notch's zero has no finite dB; it's far below either way.
                if want < -60.0 {
                    assert!(got < -50.0, "{name} at {freq}: {got} dB");
                    continue;
                }
                assert!(
                    (got - want).abs() < 0.01,
                    "{name} at {}: {got} vs {want} dB",
                    freq * 48e3
                );
            }
        }
    }

    #[test]
    fn eq_shapes_hit_their_gains() {
        let f = 1e3 / 48e3;
        let mut svf = Svf::default();
        let c = SvfCoeffs::bell(f, 0.7, -6.0);
        assert!((measured_db(|x| svf.tick(&c, x), f64::from(f)) + 6.0).abs() < 1e-3);
        let mut svf = Svf::default();
        let c = SvfCoeffs::low_shelf(f, 0.707, 12.0);
        assert!((measured_db(|x| svf.tick(&c, x), 1.0 / 48e3) - 12.0).abs() < 0.01);
        assert!((measured_db(|x| svf.tick(&c, x), f64::from(f)) - 6.0).abs() < 0.01);
        let mut svf = Svf::default();
        let c = SvfCoeffs::high_shelf(f, 0.707, -12.0);
        assert!((measured_db(|x| svf.tick(&c, x), 0.49) + 12.0).abs() < 0.05);
        let mut svf = Svf::default();
        let c = SvfCoeffs::PASS;
        assert_eq!(svf.tick(&c, 0.25), 0.25);
    }

    #[test]
    fn one_pole_is_minus_3_db_at_cutoff_and_splits_the_signal() {
        let f = 500.0 / 48e3;
        let c = one_pole_coeff(f);
        let mut lp = OnePole::default();
        let db = measured_db(|x| lp.lowpass(c, x), f64::from(f));
        assert!((db + 3.0103).abs() < 1e-3, "{db}");
        let mut hp = OnePole::default();
        let db = measured_db(|x| hp.highpass(c, x), f64::from(f));
        assert!((db + 3.0103).abs() < 1e-3, "{db}");
        // At a low cutoff the f32 state stalls short of removing DC.
        let mut p = OnePole::default();
        let last = (0..48_000)
            .map(|_| p.highpass(one_pole_coeff(10.0 / 48e3), 1.0))
            .last();
        assert!(last.unwrap().abs() < 1e-4);
    }

    #[test]
    fn dc_blocker_removes_dc_and_passes_audio() {
        let r = dc_blocker_coeff(10.0 / 48e3);
        let mut p = DcBlocker::default();
        let last = (0..48_000).map(|_| p.tick(r, 1.0)).last();
        assert!(last.unwrap().abs() < 1e-7);
        let mut p = DcBlocker::default();
        let db = measured_db(|x| p.tick(r, x), 10.0 / 48e3);
        assert!((db + 3.0).abs() < 0.1, "{db}");
        let mut p = DcBlocker::default();
        assert!(measured_db(|x| p.tick(r, x), 100.0 / 48e3).abs() < 0.05);
    }

    /// Coefficients jumping every block (a swept cutoff) stay stable and
    /// don't blow up.
    #[test]
    fn svf_is_stable_under_modulation() {
        let mut svf = Svf::default();
        let mut peak = 0.0_f32;
        for n in 0..48_000_u32 {
            #[expect(clippy::cast_precision_loss, reason = "small counter")]
            let t = (n / 32) as f32;
            let f = 0.002 + 0.45 * (0.5 + 0.5 * fastmath::sin_turns(t * 0.013));
            let c = SvfCoeffs::lowpass(f, 8.0);
            let x = if n % 64 < 32 { 0.5 } else { -0.5 };
            peak = peak.max(svf.tick(&c, x).abs());
        }
        assert!(peak.is_finite() && peak < 20.0, "{peak}");
    }
}
