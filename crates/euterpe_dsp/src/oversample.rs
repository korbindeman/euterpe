//! Oversampling: run a processor at 2, 4 or 8 times the device rate.
//!
//! Nonlinear processors create harmonics above Nyquist, which fold back into
//! the audio band as inharmonic aliases. Running them at a higher rate moves
//! those harmonics up, where a lowpass removes them before decimation.
//!
//! Each 2x stage is a pair of polyphase half-band FIR filters. A half-band
//! filter has every odd tap zero except the centre (0.5), so the upsampler
//! computes one FIR of `N` taps per input sample (the other phase is a plain
//! delayed copy) and the decimator one per output sample. Higher factors nest
//! stages, steepest at the device rate, where the transition band is the
//! narrowest:
//!
//! | type | stages | latency at 48 kHz |
//! | --- | --- | --- |
//! | [`Oversampled2`] | [`STEEP`] | 39 samples (0.81 ms) |
//! | [`Oversampled4`] | [`STEEP`], [`MEDIUM`] | 46 samples (0.96 ms) |
//! | [`Oversampled8`] | [`STEEP`], [`MEDIUM`], [`GENTLE`] | 48 samples (1.0 ms) |
//!
//! Every stage passes 0-20 kHz within 0.0002 dB and rejects images and
//! aliases of that band by at least 100 dB (see `tools/design/halfband.py`).
//! The latency is exact and integer: when a stage's inner latency is odd at
//! its rate, the stage pads it by one inner sample.

use euterpe_core::{BLOCK, Block, Ctx, Processor};

mod taps;

pub use taps::{GENTLE, MEDIUM, STEEP};

/// `P` at twice the device rate, through the steepest half-band stage.
pub type Oversampled2<P> = Oversample2x<P, 40>;
/// `P` at four times the device rate.
pub type Oversampled4<P> = Oversample2x<Oversample2x<P, 14>, 40>;
/// `P` at eight times the device rate.
pub type Oversampled8<P> = Oversample2x<Oversample2x<Oversample2x<P, 10>, 14>, 40>;

/// History capacity of one filter phase. Must be a power of two, at least
/// the longest filter's even-tap count.
const CAP: usize = 64;

/// The last [`CAP`] samples of one phase, stored twice (`buf[i]` and
/// `buf[i + CAP]`) so that any window of up to `CAP` recent samples is one
/// contiguous slice.
#[derive(Clone)]
struct History {
    buf: [f32; 2 * CAP],
    pos: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            buf: [0.0; 2 * CAP],
            pos: 0,
        }
    }
}

impl History {
    #[inline(always)]
    fn push(&mut self, x: f32) {
        let p = self.pos & (CAP - 1);
        if let Some(a) = self.buf.get_mut(p) {
            *a = x;
        }
        if let Some(b) = self.buf.get_mut(p + CAP) {
            *b = x;
        }
        self.pos = (p + 1) & (CAP - 1);
    }

    /// The newest `N` samples, oldest first.
    #[inline(always)]
    fn window<const N: usize>(&self) -> Option<&[f32; N]> {
        const { assert!(N <= CAP) };
        let start = (self.pos & (CAP - 1)) + CAP - N;
        self.buf.get(start..)?.first_chunk::<N>()
    }

    /// The sample `lag` pushes ago (0 = the newest).
    #[inline(always)]
    fn lag(&self, lag: usize) -> f32 {
        let i = ((self.pos & (CAP - 1)) + CAP - 1).wrapping_sub(lag);
        self.buf.get(i).copied().unwrap_or(0.0)
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Proofs of the history indexing (`cargo kani -p euterpe_dsp`).
#[cfg(kani)]
mod proofs {
    use super::*;

    fn history_at_any_position() -> History {
        let mut h = History::default();
        h.pos = kani::any();
        h
    }

    /// From any position (even a corrupted one), every window the filters
    /// read exists: `process` never takes its fallback.
    #[kani::proof]
    fn windows_always_exist() {
        let h = history_at_any_position();
        assert!(h.window::<40>().is_some());
        assert!(h.window::<14>().is_some());
        assert!(h.window::<10>().is_some());
        assert!(h.window::<{ CAP }>().is_some());
    }

    /// The window is the newest samples in order, and `lag` reads back
    /// exactly what was pushed, from any starting position.
    #[kani::proof]
    fn pushes_read_back_in_order() {
        let mut h = history_at_any_position();
        let (a, b, c): (f32, f32, f32) = (kani::any(), kani::any(), kani::any());
        h.push(a);
        h.push(b);
        h.push(c);
        assert!(h.pos < CAP);
        assert!(h.lag(0).to_bits() == c.to_bits());
        assert!(h.lag(1).to_bits() == b.to_bits());
        assert!(h.lag(2).to_bits() == a.to_bits());
        let w = h.window::<3>().unwrap();
        assert!(w.map(f32::to_bits) == [a, b, c].map(f32::to_bits));
    }

    /// Every lag the decimators use (`m <= 19`) indexes inside the buffer.
    #[kani::proof]
    fn lags_stay_in_bounds() {
        let h = history_at_any_position();
        let lag: usize = kani::any();
        kani::assume(lag < CAP);
        let i = ((h.pos & (CAP - 1)) + CAP - 1).wrapping_sub(lag);
        assert!(i < 2 * CAP);
    }
}

/// Dot product of a symmetric tap set with a window, folding the symmetry so
/// each tap is multiplied once.
#[inline(always)]
fn symmetric_dot<const N: usize>(taps: &[f32; N], w: &[f32; N]) -> f32 {
    let mut acc = 0.0;
    for k in 0..N / 2 {
        acc += taps[k] * (w[k] + w[N - 1 - k]);
    }
    acc
}

/// Polyphase half-band interpolator: one input sample in, two out.
///
/// `taps` are the `N` nonzero even taps of a `2N - 1` tap half-band filter.
/// The delay is `N - 1` samples at the output rate.
#[derive(Clone)]
pub struct Upsampler<const N: usize> {
    taps: &'static [f32; N],
    hist: History,
}

impl<const N: usize> Upsampler<N> {
    #[must_use]
    pub fn new(taps: &'static [f32; N]) -> Self {
        const { assert!(N.is_multiple_of(2) && N >= 2 && N <= CAP) };
        Self {
            taps,
            hist: History::default(),
        }
    }

    pub fn reset(&mut self) {
        self.hist.clear();
    }

    /// Interpolate `input` into `output`, two samples per input sample.
    /// Stops at the shorter of the two.
    #[inline]
    pub fn process(&mut self, input: &[f32], output: &mut [[f32; 2]]) {
        for (&x, out) in input.iter().zip(output) {
            self.hist.push(x);
            if let Some(w) = self.hist.window::<N>() {
                // Zero-stuffing halves the energy; the 2 restores unity gain.
                out[0] = 2.0 * symmetric_dot(self.taps, w);
                // The other phase only meets the centre tap: 2 * 0.5 * x[n - m].
                out[1] = w[N / 2];
            }
        }
    }
}

/// Polyphase half-band decimator: two input samples in, one out.
///
/// The delay is `N - 1` samples at the input rate.
#[derive(Clone)]
pub struct Downsampler<const N: usize> {
    taps: &'static [f32; N],
    even: History,
    odd: History,
}

impl<const N: usize> Downsampler<N> {
    #[must_use]
    pub fn new(taps: &'static [f32; N]) -> Self {
        const { assert!(N.is_multiple_of(2) && N >= 2 && N <= CAP) };
        Self {
            taps,
            even: History::default(),
            odd: History::default(),
        }
    }

    pub fn reset(&mut self) {
        self.even.clear();
        self.odd.clear();
    }

    /// Decimate `input` into `output`, one sample per input pair.
    #[inline]
    pub fn process(&mut self, input: &[[f32; 2]], output: &mut [f32]) {
        let m = (N - 2) / 2;
        for (&[e, o], y) in input.iter().zip(output) {
            self.even.push(e);
            if let Some(w) = self.even.window::<N>() {
                // The odd phase meets only the centre tap, `m` pairs back.
                *y = symmetric_dot(self.taps, w) + 0.5 * self.odd.lag(m);
            }
            self.odd.push(o);
        }
    }
}

/// `P` at twice the rate of the context it is prepared with.
///
/// Usually named through [`Oversampled2`], [`Oversampled4`] or
/// [`Oversampled8`]. The inner processor sees two blocks per outer block and
/// a [`Ctx`] with the doubled rate. Its parameters are the inner ones.
pub struct Oversample2x<P, const N: usize> {
    /// The oversampled processor.
    pub inner: P,
    up: Upsampler<N>,
    down: Downsampler<N>,
    hi: [Block; 2],
    /// One inner sample of delay when the inner latency is odd.
    pad: Option<f32>,
}

impl<P, const N: usize> Oversample2x<P, N> {
    /// Wrap `inner`, filtering with the `N` even taps of a half-band filter.
    pub fn new(inner: P, taps: &'static [f32; N]) -> Self {
        Self {
            inner,
            up: Upsampler::new(taps),
            down: Downsampler::new(taps),
            hi: [[0.0; BLOCK]; 2],
            pad: None,
        }
    }
}

macro_rules! default_stage {
    ($($n:literal => $taps:ident),* $(,)?) => {$(
        impl<P: Default> Default for Oversample2x<P, $n> {
            fn default() -> Self {
                Self::new(P::default(), &$taps)
            }
        }
    )*};
}

default_stage!(40 => STEEP, 14 => MEDIUM, 10 => GENTLE);

impl<P: Processor, const N: usize> Oversample2x<P, N> {
    fn sync_pad(&mut self) {
        self.pad = (!self.inner.latency().is_multiple_of(2)).then_some(0.0);
    }
}

impl<P: Processor, const N: usize> Processor for Oversample2x<P, N> {
    type Params = P::Params;

    fn prepare(&mut self, ctx: &Ctx) {
        self.inner.prepare(&ctx.oversampled(2));
        self.up.reset();
        self.down.reset();
        self.sync_pad();
    }

    fn reset(&mut self) {
        self.inner.reset();
        self.up.reset();
        self.down.reset();
        self.sync_pad();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, params: &Self::Params) {
        const HALF: usize = BLOCK / 2;
        let (lo_a, lo_b) = buf.split_at(HALF);
        let [hi_a, hi_b] = &mut self.hi;
        self.up.process(lo_a, hi_a.as_chunks_mut::<2>().0);
        self.up.process(lo_b, hi_b.as_chunks_mut::<2>().0);

        self.inner.process(hi_a, params);
        self.inner.process(hi_b, params);

        if let Some(z) = &mut self.pad {
            for s in self.hi.as_flattened_mut() {
                core::mem::swap(s, z);
            }
        }

        let [hi_a, hi_b] = &self.hi;
        let (lo_a, lo_b) = buf.split_at_mut(HALF);
        self.down.process(hi_a.as_chunks::<2>().0, lo_a);
        self.down.process(hi_b.as_chunks::<2>().0, lo_b);
    }

    /// `N - 1` samples for this stage's filters (they add up to `2N - 2` at
    /// the inner rate), plus the inner latency, padded to even and halved.
    fn latency(&self) -> usize {
        let inner = self.inner.latency();
        N - 1 + inner.div_ceil(2)
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact, bit-identical results on purpose"
)]
mod tests {
    extern crate std;
    use std::vec;
    use std::vec::Vec;

    use super::*;
    use euterpe_offline::analyze::{self, Setup};
    use euterpe_offline::check::{CheckOptions, check_processor};
    use euterpe_offline::{RenderOptions, render, signal};

    /// The full `2N - 1` tap filter the even taps describe.
    fn full<const N: usize>(even: &[f32; N]) -> Vec<f64> {
        let mut h = vec![0.0; 2 * N - 1];
        for (i, &t) in even.iter().enumerate() {
            h[2 * i] = f64::from(t);
        }
        h[N - 1] = 0.5;
        h
    }

    fn mag_db(h: &[f64], f: f64) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (n, &c) in h.iter().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "small tap index")]
            let w = core::f64::consts::TAU * f * n as f64;
            re += c * w.cos();
            im -= c * w.sin();
        }
        20.0 * re.hypot(im).log10()
    }

    /// Passband and stopband of each tap set, as stored in f32. `f` is in
    /// cycles per sample at the filter's (high) rate; `band` is the audio
    /// band edge and `stop` the first frequency that folds into it.
    #[test]
    fn stages_meet_their_spec_in_f32() {
        let stages: [(&[f32], f64, f64); 3] = [
            (&STEEP, 20.0 / 96.0, 28.0 / 96.0),
            (&MEDIUM, 20.0 / 192.0, 76.0 / 192.0),
            (&GENTLE, 20.0 / 384.0, 172.0 / 384.0),
        ];
        for (even, band, stop) in stages {
            let mut h = vec![0.0; 2 * even.len() - 1];
            for (i, &t) in even.iter().enumerate() {
                h[2 * i] = f64::from(t);
            }
            h[even.len() - 1] = 0.5;
            let pass = (0..=200).map(|i| mag_db(&h, band * f64::from(i) / 200.0));
            let (lo, hi) = pass.fold((f64::MAX, f64::MIN), |(a, b), g| (a.min(g), b.max(g)));
            assert!(lo > -0.0002 && hi < 0.0002, "ripple {lo}..{hi}");
            let worst = (0..=400)
                .map(|i| mag_db(&h, stop + (0.5 - stop) * f64::from(i) / 400.0))
                .fold(f64::MIN, f64::max);
            assert!(worst < -99.5, "stopband {worst}");
        }
    }

    /// The polyphase interpolator equals zero-stuffing followed by the full
    /// filter with gain 2.
    #[test]
    fn upsampler_matches_direct_convolution() {
        let h = full(&STEEP);
        let x: Vec<f32> = (0..200)
            .map(|i| ((i * 37 % 101) as f32 / 50.0) - 1.0)
            .collect();
        let mut out = vec![[0.0; 2]; x.len()];
        let mut up = Upsampler::new(&STEEP);
        up.process(&x[..77], &mut out[..77]);
        up.process(&x[77..], &mut out[77..]);
        for k in 0..2 * x.len() {
            let mut want = 0.0;
            for (i, &c) in h.iter().enumerate() {
                if i <= k && (k - i) % 2 == 0 {
                    want += 2.0 * c * f64::from(x[(k - i) / 2]);
                }
            }
            let got = f64::from(out[k / 2][k % 2]);
            assert!((got - want).abs() < 1e-5, "k={k}: {got} vs {want}");
        }
    }

    /// The polyphase decimator equals the full filter, keeping even outputs.
    #[test]
    fn downsampler_matches_direct_convolution() {
        let h = full(&MEDIUM);
        let v: Vec<f32> = (0..300)
            .map(|i| ((i * 53 % 97) as f32 / 48.0) - 1.0)
            .collect();
        let pairs: Vec<[f32; 2]> = v.as_chunks::<2>().0.to_vec();
        let mut out = vec![0.0; pairs.len()];
        let mut down = Downsampler::new(&MEDIUM);
        down.process(&pairs[..31], &mut out[..31]);
        down.process(&pairs[31..], &mut out[31..]);
        for (n, &got) in out.iter().enumerate() {
            let k = 2 * n;
            let want: f64 = h
                .iter()
                .enumerate()
                .filter(|&(i, _)| i <= k)
                .map(|(i, &c)| c * f64::from(v[k - i]))
                .sum();
            assert!((f64::from(got) - want).abs() < 1e-5, "n={n}");
        }
    }

    /// A processor with a fixed delay, to exercise odd inner latencies.
    #[derive(Default)]
    struct Delay<const D: usize> {
        line: [f32; 8],
        pos: usize,
    }

    impl<const D: usize> Processor for Delay<D> {
        type Params = ();
        fn prepare(&mut self, _: &Ctx) {
            self.reset();
        }
        fn reset(&mut self) {
            *self = Self::default();
        }
        fn process(&mut self, buf: &mut Block, (): &()) {
            for s in buf.iter_mut() {
                self.line[self.pos % 8] = *s;
                *s = self.line[(self.pos + 8 - D) % 8];
                self.pos += 1;
            }
        }
        fn latency(&self) -> usize {
            D
        }
    }

    type Id = Delay<0>;

    #[test]
    fn wrappers_pass_invariants() {
        let opts = CheckOptions::default();
        check_processor::<Oversampled2<Id>>(&opts).unwrap();
        check_processor::<Oversampled4<Id>>(&opts).unwrap();
        check_processor::<Oversampled8<Delay<3>>>(&opts).unwrap();
    }

    fn reproduces<P: Processor<Params = ()> + Default>() {
        let x = signal::sine(997.0, 0.5, 9_600, 48_000);
        let y = render(&mut P::default(), &(), &x, &RenderOptions::new(48_000));
        let err = x[200..9_000]
            .iter()
            .zip(&y[200..9_000])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(err < 2e-5, "{}: {err}", core::any::type_name::<P>());
    }

    #[test]
    fn latency_is_documented_and_exact() {
        assert_eq!(Oversampled2::<Id>::default().latency(), 39);
        assert_eq!(Oversampled4::<Id>::default().latency(), 46);
        assert_eq!(Oversampled8::<Id>::default().latency(), 48);
        // An odd inner latency is padded, then halved.
        assert_eq!(Oversampled2::<Delay<3>>::default().latency(), 39 + 2);

        // Exact means a latency-compensated render reproduces the input.
        reproduces::<Oversampled2<Id>>();
        reproduces::<Oversampled4<Id>>();
        reproduces::<Oversampled8<Id>>();
        reproduces::<Oversampled2<Delay<3>>>();
        reproduces::<Oversampled4<Delay<1>>>();
    }

    #[test]
    fn audio_band_is_flat_with_zero_phase() {
        let setup = Setup::default();
        for p in analyze::response::<Oversampled4<Id>>(&(), &setup, &analyze::THIRD_OCTAVES, -20.0)
        {
            if p.freq <= 20_000.0 {
                assert!(p.gain_db.abs() < 0.001, "{p:?}");
                assert!(p.phase_deg.abs() < 0.01, "{p:?}");
            }
        }
    }

    #[test]
    fn inner_runs_at_the_multiplied_rate() {
        #[derive(Default)]
        struct Rate(f32, u32);
        impl Processor for Rate {
            type Params = ();
            fn prepare(&mut self, ctx: &Ctx) {
                self.0 = ctx.sample_rate;
                self.1 = ctx.oversampling;
            }
            fn reset(&mut self) {}
            fn process(&mut self, _: &mut Block, (): &()) {}
        }
        let mut os = Oversampled8::<Rate>::default();
        os.prepare(&Ctx::new(48_000.0));
        let r = &os.inner.inner.inner;
        assert_eq!((r.0, r.1), (384_000.0, 8));
    }

    /// A clipped cubic: a strong, broadband nonlinearity that aliases.
    #[derive(Default)]
    struct Clip;
    impl Processor for Clip {
        type Params = ();
        fn prepare(&mut self, _: &Ctx) {}
        fn reset(&mut self) {}
        fn process(&mut self, buf: &mut Block, (): &()) {
            for s in buf {
                let x = (*s * 4.0).clamp(-1.0, 1.0);
                *s = x - x * x * x / 3.0;
            }
        }
    }

    #[test]
    fn oversampling_cuts_aliasing() {
        let setup = Setup::default();
        let freqs = [3_000.0, 7_000.0, 11_000.0];
        let alias = |pts: Vec<analyze::AliasPoint>| -> Vec<f64> {
            pts.iter().map(|p| p.alias_db).collect()
        };
        let x1 = alias(analyze::aliasing::<Clip>(&(), &setup, &freqs, -6.0));
        let x2 = alias(analyze::aliasing::<Oversampled2<Clip>>(
            &(),
            &setup,
            &freqs,
            -6.0,
        ));
        let x4 = alias(analyze::aliasing::<Oversampled4<Clip>>(
            &(),
            &setup,
            &freqs,
            -6.0,
        ));
        let x8 = alias(analyze::aliasing::<Oversampled8<Clip>>(
            &(),
            &setup,
            &freqs,
            -6.0,
        ));
        for i in 0..freqs.len() {
            assert!(x2[i] < x1[i] - 6.0, "{x1:?} {x2:?}");
            assert!(x4[i] < x2[i] - 6.0, "{x2:?} {x4:?}");
            assert!(x8[i] <= x4[i], "{x4:?} {x8:?}");
        }
    }
}
