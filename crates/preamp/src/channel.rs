use dsp::Process;
use dsp::cab_sim::{CabSim, CabSimParams};
use dsp::comp_163::{Comp163, Comp163Params};
use dsp::comp_1176::{Comp1176, Comp1176Params};
use dsp::compressor::{Compressor, CompressorParams};
use dsp::eq::{ThreeBandEq, ThreeBandEqParams};
use dsp::gate::{NoiseGate, NoiseGateParams};
use dsp::limiter::{Limiter, LimiterParams};
use dsp::params::Knob;
use dsp::saturator::{Saturator, SaturatorParams};

use crate::{
    ChannelMode, CompressorType, EQ_BOOST_DB, EQ_HIGH_FREQ_BASS, EQ_HIGH_FREQ_GUITAR,
    EQ_LOW_FREQ_BASS, EQ_LOW_FREQ_GUITAR, EQ_MID_FREQ_BASS, EQ_MID_FREQ_GUITAR, EQ_MID_Q, GAIN_MAX,
    GAIN_MIN, SAMPLE_RATE,
};

pub struct ChannelParams {
    pub gain: Knob,
}

impl ChannelParams {
    pub fn new(sample_rate: u32) -> Self {
        let mut gain = Knob::new("Gain", 1.0, GAIN_MIN, GAIN_MAX);
        gain.init(sample_rate);
        Self { gain }
    }
}

impl Default for ChannelParams {
    fn default() -> Self {
        Self::new(SAMPLE_RATE)
    }
}

pub struct Channel {
    mode: ChannelMode,
    params: ChannelParams,
    gate: NoiseGate,
    // Compressors - only one is active at a time
    compressor_type: CompressorType,
    compressor_standard: Compressor,
    compressor_1176: Comp1176,
    compressor_163: Comp163,
    eq: ThreeBandEq,
    saturator: Saturator,
    cab_sim: CabSim,
    limiter: Limiter,
}

impl Channel {
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        let mut cab_sim = CabSim::new();
        cab_sim.params_mut().enabled.set(true); // Guitar mode default: cab sim on

        let mut saturator = Saturator::with_sample_rate(sample_rate);
        saturator.params_mut().enabled.set(false); // Guitar mode default: saturator off

        Self {
            mode: ChannelMode::Guitar,
            params: ChannelParams::new(sample_rate),
            gate: NoiseGate::new(sample_rate),
            compressor_type: CompressorType::default(),
            compressor_standard: Compressor::new(sample_rate),
            compressor_1176: Comp1176::new(sample_rate),
            compressor_163: Comp163::new(sample_rate),
            eq: ThreeBandEq::new(
                EQ_LOW_FREQ_GUITAR,
                EQ_MID_FREQ_GUITAR,
                EQ_HIGH_FREQ_GUITAR,
                EQ_BOOST_DB,
                EQ_MID_Q,
                sr,
            ),
            saturator,
            cab_sim,
            limiter: Limiter::new(sample_rate),
        }
    }

    // Channel's own params
    pub fn params(&self) -> &ChannelParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut ChannelParams {
        &mut self.params
    }

    // Access to individual processor params
    pub fn gate_params(&self) -> &NoiseGateParams {
        self.gate.params()
    }

    pub fn gate_params_mut(&mut self) -> &mut NoiseGateParams {
        self.gate.params_mut()
    }

    // Compressor type selection
    pub fn compressor_type(&self) -> CompressorType {
        self.compressor_type
    }

    pub fn set_compressor_type(&mut self, comp_type: CompressorType) {
        self.compressor_type = comp_type;
    }

    // Standard compressor params
    pub fn compressor_standard_params(&self) -> &CompressorParams {
        self.compressor_standard.params()
    }

    pub fn compressor_standard_params_mut(&mut self) -> &mut CompressorParams {
        self.compressor_standard.params_mut()
    }

    // 1176 compressor params
    pub fn compressor_1176_params(&self) -> &Comp1176Params {
        self.compressor_1176.params()
    }

    pub fn compressor_1176_params_mut(&mut self) -> &mut Comp1176Params {
        self.compressor_1176.params_mut()
    }

    // 163 compressor params
    pub fn compressor_163_params(&self) -> &Comp163Params {
        self.compressor_163.params()
    }

    pub fn compressor_163_params_mut(&mut self) -> &mut Comp163Params {
        self.compressor_163.params_mut()
    }

    pub fn eq_params(&self) -> &ThreeBandEqParams {
        self.eq.params()
    }

    pub fn eq_params_mut(&mut self) -> &mut ThreeBandEqParams {
        self.eq.params_mut()
    }

    pub fn saturator_params(&self) -> &SaturatorParams {
        self.saturator.params()
    }

    pub fn saturator_params_mut(&mut self) -> &mut SaturatorParams {
        self.saturator.params_mut()
    }

    pub fn cab_sim_params(&self) -> &CabSimParams {
        self.cab_sim.params()
    }

    pub fn cab_sim_params_mut(&mut self) -> &mut CabSimParams {
        self.cab_sim.params_mut()
    }

    pub fn limiter_params(&self) -> &LimiterParams {
        self.limiter.params()
    }

    pub fn limiter_params_mut(&mut self) -> &mut LimiterParams {
        self.limiter.params_mut()
    }

    pub fn set_mode(&mut self, mode: ChannelMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        let sr = SAMPLE_RATE as f32;
        match mode {
            ChannelMode::Guitar => {
                self.eq.reconfigure(
                    EQ_LOW_FREQ_GUITAR,
                    EQ_MID_FREQ_GUITAR,
                    EQ_HIGH_FREQ_GUITAR,
                    sr,
                );
                self.cab_sim_params_mut().enabled.set(true);
                self.saturator_params_mut().enabled.set(false);
            }
            ChannelMode::Bass => {
                self.eq
                    .reconfigure(EQ_LOW_FREQ_BASS, EQ_MID_FREQ_BASS, EQ_HIGH_FREQ_BASS, sr);
                self.cab_sim_params_mut().enabled.set(false);
                self.saturator_params_mut().enabled.set(true);
            }
        }
    }

    pub fn set_ir(&mut self, ir: &[f32]) {
        self.cab_sim.set_ir(ir);
    }

    pub fn mode(&self) -> ChannelMode {
        self.mode
    }
}

impl Process for Channel {
    #[inline]
    fn process(&mut self, sample: f32) -> f32 {
        // Signal flow: Input -> Gate -> Compressor -> EQ -> Saturator -> Cab Sim -> Gain -> Limiter -> Output
        let s = self.gate.process(sample);
        let s = match self.compressor_type {
            CompressorType::Standard => self.compressor_standard.process(s),
            CompressorType::Comp1176 => self.compressor_1176.process(s),
            CompressorType::Comp163 => self.compressor_163.process(s),
        };
        let s = self.eq.process(s);
        let s = self.saturator.process(s);
        let s = self.cab_sim.process(s);
        let s = s * self.params.gain.get_smoothed();
        self.limiter.process(s)
    }
}
