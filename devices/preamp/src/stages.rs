//! The channel's stages, each a [`Processor`] with its own params. They
//! follow the legacy `dsp` processors sample for sample; the differences
//! are listed on each.

use euterpe_core::{Block, Ctx, ParamKind, ParamSet, Params, Processor, Smoother, fastmath};
use euterpe_dsp::{
    Ballistics, DcBlocker, DelayLine, Follower, Phasor, SoftKnee, SplitConvolver, Svf, SvfCoeffs,
    dc_blocker_coeff, time_constant,
};

// ---------------------------------------------------------------------------
// Input conditioning
// ---------------------------------------------------------------------------

/// A 20 Hz DC blocker, then a plug-pop suppressor: a jump to near full
/// scale mutes the input for 45 ms and fades it back in over 120 ms, and
/// asks the channel to reset its stages (whatever they were holding is the
/// pop).
#[derive(Default)]
pub struct Conditioner {
    dc: DcBlocker,
    dc_coeff: f32,
    prev: f32,
    gain: f32,
    mute_left: u32,
    mute_samples: u32,
    fade_step: f32,
    reset_requested: bool,
}

impl Conditioner {
    const POP_DELTA: f32 = 0.45;
    const POP_LEVEL: f32 = 0.25;
    const POP_CLIP: f32 = 0.9;
    const MUTE_MS: f32 = 45.0;
    const FADE_IN_MS: f32 = 120.0;

    /// Whether a pop was caught since the last call.
    pub fn take_reset_requested(&mut self) -> bool {
        core::mem::take(&mut self.reset_requested)
    }

    #[inline(always)]
    fn is_pop(&self, x: f32) -> bool {
        let level = x.abs();
        level >= Self::POP_CLIP
            || (level >= Self::POP_LEVEL && (x - self.prev).abs() >= Self::POP_DELTA)
    }
}

impl Processor for Conditioner {
    type Params = ();

    fn prepare(&mut self, ctx: &Ctx) {
        self.dc_coeff = dc_blocker_coeff(20.0 * ctx.dt());
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "45 ms at any audio rate fits easily"
        )]
        let mute = (ctx.ms_to_samples(Self::MUTE_MS) + 0.5) as u32;
        self.mute_samples = mute;
        self.fade_step = 1.0 / ctx.ms_to_samples(Self::FADE_IN_MS).max(1.0);
    }

    fn reset(&mut self) {
        self.dc.reset();
        self.prev = 0.0;
        self.gain = 1.0;
        self.mute_left = 0;
        self.reset_requested = false;
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, (): &()) {
        for s in buf.iter_mut() {
            let x = self.dc.tick(self.dc_coeff, *s);
            if self.is_pop(x) {
                self.mute_left = self.mute_samples;
                self.gain = 0.0;
                self.reset_requested = true;
            }
            self.prev = x;
            *s = if self.mute_left > 0 {
                self.mute_left -= 1;
                0.0
            } else {
                self.gain = (self.gain + self.fade_step).min(1.0);
                x * self.gain
            };
        }
    }
}

// ---------------------------------------------------------------------------
// Gate
// ---------------------------------------------------------------------------

#[derive(Params, Clone, Debug, PartialEq)]
pub struct GateParams {
    #[param(id = "enabled", label = "Enabled", kind = ParamKind::toggle(true))]
    pub enabled: bool,
    #[param(id = "threshold", label = "Threshold", kind = ParamKind::float(-80.0, 0.0, -70.0).unit("dB"))]
    pub threshold_db: f32,
}

/// A noise gate with 3 dB of hysteresis: a peak envelope (1 ms / 50 ms)
/// opens it above the threshold and closes it 3 dB below; the gain moves
/// to 1 in 5 ms and to 0 in 20 ms.
#[derive(Default)]
pub struct Gate {
    env: Follower,
    gain: Follower,
    open: bool,
    env_b: Ballistics,
    gain_b: Ballistics,
}

impl Processor for Gate {
    type Params = GateParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.env_b = Ballistics::new(1.0, 50.0, ctx.sample_rate);
        self.gain_b = Ballistics::new(5.0, 20.0, ctx.sample_rate);
    }

    fn reset(&mut self) {
        self.env = Follower::at(0.0);
        self.gain = Follower::at(1.0);
        self.open = true;
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &GateParams) {
        if !p.enabled {
            return;
        }
        let open_above = fastmath::db_to_gain(p.threshold_db);
        let close_below = fastmath::db_to_gain(p.threshold_db - 3.0);
        for s in buf.iter_mut() {
            let env = self.env.peak(&self.env_b, *s);
            self.open = if self.open {
                env >= close_below
            } else {
                env > open_above
            };
            let target = if self.open { 1.0 } else { 0.0 };
            *s *= self.gain.follow(&self.gain_b, target);
        }
    }
}

// ---------------------------------------------------------------------------
// Compressors
// ---------------------------------------------------------------------------

#[derive(Params, Clone, Debug, PartialEq)]
pub struct StandardParams {
    #[param(id = "enabled", label = "Standard Enabled", kind = ParamKind::toggle(true))]
    pub enabled: bool,
    #[param(id = "threshold", label = "Threshold", kind = ParamKind::float(-60.0, 0.0, -20.0).unit("dB"))]
    pub threshold_db: f32,
    #[param(id = "ratio", label = "Ratio", kind = ParamKind::float(1.0, 20.0, 4.0))]
    pub ratio: f32,
    #[param(id = "attack", label = "Attack", kind = ParamKind::float(0.1, 100.0, 15.0).unit("ms"))]
    pub attack_ms: f32,
    #[param(id = "release", label = "Release", kind = ParamKind::float(10.0, 1_000.0, 100.0).unit("ms"))]
    pub release_ms: f32,
}

/// A feed-forward peak compressor with a 6 dB soft knee.
#[derive(Default)]
pub struct Standard {
    env: Follower,
    rate: f32,
}

impl Processor for Standard {
    type Params = StandardParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.rate = ctx.sample_rate;
    }

    fn reset(&mut self) {
        self.env = Follower::default();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &StandardParams) {
        if !p.enabled {
            return;
        }
        let b = Ballistics::new(p.attack_ms, p.release_ms, self.rate);
        let knee = SoftKnee::new(p.threshold_db, p.ratio, 6.0);
        for s in buf.iter_mut() {
            let env = self.env.peak(&b, *s);
            *s *= fastmath::db_to_gain(knee.gain_db(fastmath::gain_to_db(env)));
        }
    }
}

const FET_RATIOS: &[&str] = &["4:1", "8:1", "12:1", "20:1"];

#[derive(Params, Clone, Debug, PartialEq)]
pub struct FetParams {
    #[param(id = "enabled", label = "1176 Enabled", kind = ParamKind::toggle(true))]
    pub enabled: bool,
    #[param(id = "input", label = "Input", kind = ParamKind::float(-20.0, 40.0, 0.0).unit("dB"))]
    pub input_db: f32,
    #[param(id = "output", label = "Output", kind = ParamKind::float(-20.0, 40.0, 0.0).unit("dB"))]
    pub output_db: f32,
    #[param(id = "ratio", label = "Ratio", kind = ParamKind::choice(FET_RATIOS, 0))]
    pub ratio: u8,
    #[param(id = "attack", label = "Attack", kind = ParamKind::float(0.02, 0.8, 0.4).unit("ms"))]
    pub attack_ms: f32,
    #[param(id = "release", label = "Release", kind = ParamKind::float(50.0, 1_100.0, 300.0).unit("ms"))]
    pub release_ms: f32,
}

/// An 1176-style feedback compressor: the detector reads the compressed
/// output, the threshold is fixed at -20 dB (the input knob drives into
/// it), and attack and release speed up the harder it compresses.
#[derive(Default)]
pub struct Fet {
    env: f32,
    rate: f32,
}

impl Fet {
    const THRESHOLD_DB: f32 = -20.0;
}

impl Processor for Fet {
    type Params = FetParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.rate = ctx.sample_rate;
    }

    fn reset(&mut self) {
        self.env = 0.0;
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &FetParams) {
        if !p.enabled {
            return;
        }
        let input = fastmath::db_to_gain(p.input_db);
        let output = fastmath::db_to_gain(p.output_db);
        let inv_ratio = 1.0 / [4.0, 8.0, 12.0, 20.0][usize::from(p.ratio.min(3))];
        let attack = time_constant(p.attack_ms, self.rate);
        let release = time_constant(p.release_ms, self.rate);
        for s in buf.iter_mut() {
            let env_db = fastmath::gain_to_db(self.env);
            let over = env_db - Self::THRESHOLD_DB;
            let gain_db = if over <= 0.0 {
                0.0
            } else {
                over * inv_ratio - over
            };
            let y = *s * input * fastmath::db_to_gain(gain_db);
            // Program dependence: up to 2x faster attack and 1.4x faster
            // release at 20 dB over.
            let hard = (over * (1.0 / 20.0)).clamp(0.0, 1.0);
            let level = y.abs();
            let c = if level > self.env {
                attack * (1.0 - 0.5 * hard)
            } else {
                release * (1.0 - 0.3 * hard)
            };
            self.env = c * self.env + (1.0 - c) * level;
            *s = y * output;
        }
    }
}

#[derive(Params, Clone, Debug, PartialEq)]
pub struct VcaParams {
    #[param(id = "enabled", label = "163 Enabled", kind = ParamKind::toggle(true))]
    pub enabled: bool,
    #[param(id = "compression", label = "Compression", kind = ParamKind::float(0.0, 1.0, 0.5))]
    pub compression: f32,
    #[param(id = "output", label = "Output", kind = ParamKind::float(-20.0, 20.0, 0.0).unit("dB"))]
    pub output_db: f32,
}

/// A dbx 163-style compressor: one knob sets the threshold (0 to -60 dB)
/// over a 30 dB knee, an RMS detector (15 ms), a 20 ms attack and a
/// constant 120 dB/s release.
///
/// The legacy version blends toward a 0.1 ms attack on transients, but it
/// compares the RMS level with itself, so the blend is always zero and the
/// attack is always 20 ms. This keeps that behavior.
#[derive(Default)]
pub struct Vca {
    ms: f32,
    reduction_db: f32,
    rms_coeff: f32,
    attack: f32,
    release_per_sample: f32,
}

impl Vca {
    const KNEE_DB: f32 = 30.0;

    #[inline(always)]
    fn target_reduction(level_db: f32, threshold_db: f32) -> f32 {
        let half = 0.5 * Self::KNEE_DB;
        let over = level_db - threshold_db;
        if over <= -half {
            0.0
        } else if over < half {
            let n = (over + half) * (1.0 / Self::KNEE_DB);
            n * n * over.max(0.0)
        } else {
            over - half
        }
    }
}

impl Processor for Vca {
    type Params = VcaParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.rms_coeff = time_constant(15.0, ctx.sample_rate);
        self.attack = time_constant(20.0, ctx.sample_rate);
        self.release_per_sample = 120.0 * ctx.dt();
    }

    fn reset(&mut self) {
        self.ms = 0.0;
        self.reduction_db = 0.0;
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &VcaParams) {
        if !p.enabled {
            return;
        }
        let output = fastmath::db_to_gain(p.output_db);
        if p.compression <= 0.0 {
            buf.iter_mut().for_each(|s| *s *= output);
            return;
        }
        let threshold_db = -60.0 * p.compression;
        for s in buf.iter_mut() {
            self.ms = self.rms_coeff * self.ms + (1.0 - self.rms_coeff) * (*s * *s);
            let target = Self::target_reduction(fastmath::power_to_db(self.ms), threshold_db);
            self.reduction_db = if target > self.reduction_db {
                self.attack * self.reduction_db + (1.0 - self.attack) * target
            } else {
                (self.reduction_db - self.release_per_sample).max(target)
            };
            *s *= fastmath::db_to_gain(-self.reduction_db) * output;
        }
    }
}

const COMPRESSOR_TYPES: &[&str] = &["Standard", "1176", "163"];

#[derive(Params, Clone, Debug, PartialEq)]
pub struct CompressorParams {
    #[param(id = "type", label = "Type", kind = ParamKind::choice(COMPRESSOR_TYPES, 2))]
    pub kind: u8,
    #[param(id = "enabled", label = "Enabled", kind = ParamKind::toggle(true))]
    pub enabled: bool,
    #[param(nested)]
    pub standard: StandardParams,
    #[param(nested = "1176")]
    pub fet: FetParams,
    #[param(nested = "163")]
    pub vca: VcaParams,
}

/// The three compressors; `type` picks which one runs. `enabled` and the
/// chosen one's own `enabled` must both be on. (The legacy `compressor.enabled`
/// was an alias for the active one's toggle.)
#[derive(Default)]
pub struct Compressor {
    standard: Standard,
    fet: Fet,
    vca: Vca,
}

impl Processor for Compressor {
    type Params = CompressorParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.standard.prepare(ctx);
        self.fet.prepare(ctx);
        self.vca.prepare(ctx);
    }

    fn reset(&mut self) {
        self.standard.reset();
        self.fet.reset();
        self.vca.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &CompressorParams) {
        if !p.enabled {
            return;
        }
        match p.kind {
            0 => self.standard.process(buf, &p.standard),
            1 => self.fet.process(buf, &p.fet),
            _ => self.vca.process(buf, &p.vca),
        }
    }
}

// ---------------------------------------------------------------------------
// EQ
// ---------------------------------------------------------------------------

#[derive(Params, Clone, Debug, PartialEq)]
pub struct BandParams {
    #[param(id = "enabled", label = "Enabled", kind = ParamKind::toggle(true))]
    pub enabled: bool,
    #[param(id = "gain", label = "Gain", kind = ParamKind::float(-12.0, 12.0, 6.0).unit("dB"))]
    pub gain_db: f32,
}

#[derive(Params, Clone, Debug, PartialEq)]
pub struct AirParams {
    #[param(id = "enabled", label = "Air", kind = ParamKind::toggle(true))]
    pub enabled: bool,
}

#[derive(Params, Clone, Debug, PartialEq)]
pub struct EqParams {
    #[param(nested)]
    pub low: BandParams,
    #[param(nested)]
    pub mid: BandParams,
    #[param(nested)]
    pub high: BandParams,
    #[param(nested)]
    pub air: AirParams,
}

/// Corner frequencies of the EQ bands for each mode: low shelf, mid bell,
/// high shelf, air shelf.
pub const EQ_FREQS: [[f32; 4]; 2] = [
    [200.0, 800.0, 3_000.0, 10_000.0],
    [100.0, 500.0, 2_000.0, 8_000.0],
];

/// Low shelf, mid bell (Q 1), high shelf and a fixed +6 dB air shelf,
/// each switchable. The legacy RBJ biquads, as state-variable filters: the
/// same responses, but a knob turn no longer resets the filter state (a
/// click).
#[derive(Default)]
pub struct Eq {
    bands: [Svf; 4],
    /// Index into [`EQ_FREQS`], set by the channel from its mode.
    pub mode: u8,
    dt: f32,
}

impl Processor for Eq {
    type Params = EqParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.dt = ctx.dt();
    }

    fn reset(&mut self) {
        self.bands.iter_mut().for_each(Svf::reset);
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &EqParams) {
        let [low, mid, high, air] = EQ_FREQS[usize::from(self.mode.min(1))].map(|f| f * self.dt);
        let shelf_q = core::f32::consts::FRAC_1_SQRT_2;
        let pass = SvfCoeffs::PASS;
        let coeffs = [
            if p.low.enabled {
                SvfCoeffs::low_shelf(low, shelf_q, p.low.gain_db)
            } else {
                pass
            },
            if p.mid.enabled {
                SvfCoeffs::bell(mid, 1.0, p.mid.gain_db)
            } else {
                pass
            },
            if p.high.enabled {
                SvfCoeffs::high_shelf(high, shelf_q, p.high.gain_db)
            } else {
                pass
            },
            if p.air.enabled {
                SvfCoeffs::high_shelf(air, shelf_q, 6.0)
            } else {
                pass
            },
        ];
        for s in buf.iter_mut() {
            let mut x = *s;
            for (band, c) in self.bands.iter_mut().zip(&coeffs) {
                x = band.tick(c, x);
            }
            *s = x;
        }
    }
}

// ---------------------------------------------------------------------------
// Saturator
// ---------------------------------------------------------------------------

#[derive(Params, Clone, Debug, PartialEq)]
pub struct SaturatorParams {
    #[param(id = "enabled", label = "Enabled", kind = ParamKind::toggle(false))]
    pub enabled: bool,
    #[param(id = "drive", label = "Drive", kind = ParamKind::float(0.0, 24.0, 12.0).unit("dB").smooth_ms(10.0))]
    pub drive_db: f32,
}

/// `tanh(drive * x) / tanh(1)`.
#[derive(Default)]
pub struct Saturator {
    drive: Smoother,
}

impl Processor for Saturator {
    type Params = SaturatorParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.drive.prepare(ctx, SaturatorParams::smooth_ms("drive"));
    }

    fn reset(&mut self) {
        self.drive.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &SaturatorParams) {
        /// `1 / tanh(1)`.
        const MAKEUP: f32 = 1.313_035_3;
        // Keep the smoother current while off, so turning it on doesn't ramp.
        let mut drive = self.drive.block(fastmath::db_to_gain(p.drive_db));
        if !p.enabled {
            return;
        }
        for s in buf.iter_mut() {
            *s = fastmath::tanh(*s * drive.tick()) * MAKEUP;
        }
    }
}

// ---------------------------------------------------------------------------
// Cab
// ---------------------------------------------------------------------------

#[derive(Params, Clone, Debug, PartialEq)]
pub struct CabParams {
    #[param(id = "enabled", label = "Cab Sim", kind = ParamKind::toggle(true))]
    pub enabled: bool,
}

/// The Deluxe Reverb IR through a zero-latency [`SplitConvolver`]. (The legacy
/// `CabSim` added 256 samples of latency.)
#[derive(Default)]
pub struct Cab {
    conv: SplitConvolver<6>,
    loaded: bool,
}

impl Processor for Cab {
    type Params = CabParams;

    fn prepare(&mut self, _: &Ctx) {
        if !self.loaded {
            self.conv.set_ir(&euterpe_irs::DELUXE_REVERB);
            self.loaded = true;
        }
    }

    fn reset(&mut self) {
        self.conv.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &CabParams) {
        if p.enabled {
            self.conv.process(buf);
        }
    }
}

// ---------------------------------------------------------------------------
// Chorus
// ---------------------------------------------------------------------------

#[derive(Params, Clone, Debug, PartialEq)]
pub struct ChorusParams {
    #[param(id = "enabled", label = "Enabled", kind = ParamKind::toggle(false))]
    pub enabled: bool,
    #[param(id = "rate", label = "Rate", kind = ParamKind::float(0.1, 5.0, 0.5).unit("Hz").smooth_ms(10.0))]
    pub rate_hz: f32,
    #[param(id = "depth", label = "Depth", kind = ParamKind::float(0.0, 1.0, 0.3).smooth_ms(10.0))]
    pub depth: f32,
    #[param(id = "mix", label = "Mix", kind = ParamKind::float(0.0, 1.0, 0.3).smooth_ms(10.0))]
    pub mix: f32,
}

/// One voice, 7 ms ± 3 ms (times depth) of sine-modulated delay, mixed
/// with the dry signal.
#[derive(Default)]
pub struct Chorus {
    line: DelayLine<2048>,
    lfo: Phasor,
    rate: Smoother,
    depth: Smoother,
    mix: Smoother,
    dt: f32,
    sample_rate: f32,
}

impl Processor for Chorus {
    type Params = ChorusParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.rate.prepare(ctx, ChorusParams::smooth_ms("rate"));
        self.depth.prepare(ctx, ChorusParams::smooth_ms("depth"));
        self.mix.prepare(ctx, ChorusParams::smooth_ms("mix"));
        self.dt = ctx.dt();
        self.sample_rate = ctx.sample_rate;
    }

    fn reset(&mut self) {
        self.line.reset();
        self.lfo.reset();
        self.rate.reset();
        self.depth.reset();
        self.mix.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &ChorusParams) {
        let mut rate = self.rate.block(p.rate_hz);
        let mut depth = self.depth.block(p.depth);
        let mut mix = self.mix.block(p.mix);
        if !p.enabled {
            return;
        }
        let center = 0.007 * self.sample_rate;
        let swing = 0.003 * self.sample_rate;
        for s in buf.iter_mut() {
            let lfo = fastmath::sin_turns(self.lfo.tick(rate.tick() * self.dt));
            let x = *s;
            self.line.push(x);
            // `d` back from the next push is `d - 1` back from this one.
            let wet = self
                .line
                .tap_linear(center + lfo * swing * depth.tick() - 1.0);
            let m = mix.tick();
            *s = x * (1.0 - m) + wet * m;
        }
    }
}

// ---------------------------------------------------------------------------
// Limiter
// ---------------------------------------------------------------------------

#[derive(Params, Clone, Debug, PartialEq)]
pub struct LimiterParams {
    #[param(id = "enabled", label = "Enabled", kind = ParamKind::toggle(true))]
    pub enabled: bool,
    #[param(id = "threshold", label = "Threshold", kind = ParamKind::float(-20.0, 0.0, -1.0).unit("dB"))]
    pub threshold_db: f32,
    #[param(id = "release", label = "Release", kind = ParamKind::float(10.0, 500.0, 100.0).unit("ms"))]
    pub release_ms: f32,
}

/// A peak limiter: 0.5 ms attack, gain `threshold / envelope` above the
/// threshold, smoothed over 2 ms. Not brickwall: a fast transient can pass
/// the threshold for a moment.
#[derive(Default)]
pub struct Limiter {
    env: Follower,
    gain: Follower,
    rate: f32,
    gain_b: Ballistics,
}

impl Processor for Limiter {
    type Params = LimiterParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.rate = ctx.sample_rate;
        self.gain_b = Ballistics::symmetric(2.0, ctx.sample_rate);
    }

    fn reset(&mut self) {
        self.env = Follower::at(0.0);
        self.gain = Follower::at(1.0);
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &LimiterParams) {
        if !p.enabled {
            return;
        }
        let b = Ballistics::new(0.5, p.release_ms, self.rate);
        let threshold = fastmath::db_to_gain(p.threshold_db);
        for s in buf.iter_mut() {
            let env = self.env.peak(&b, *s);
            let target = if env > threshold {
                threshold / env
            } else {
                1.0
            };
            *s *= self.gain.follow(&self.gain_b, target);
        }
    }
}
