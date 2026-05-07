use core::f32::consts::PI;

use audio_app::{AppSpec, ParamError, ParamModel, ParamValue};
#[allow(unused_imports)]
use dsp::F32Ext;
use dsp::Process;
use dsp::cab_sim::{CabSim, CabSimParams};
use dsp::chorus::{Chorus, ChorusParams};
use dsp::comp_163::{Comp163, Comp163Params};
use dsp::comp_1176::{Comp1176, Comp1176Params};
use dsp::compressor::{Compressor, CompressorParams};
use dsp::eq::{ThreeBandEq, ThreeBandEqParams};
use dsp::gate::{NoiseGate, NoiseGateParams};
use dsp::limiter::{Limiter, LimiterParams};
use dsp::params::{Knob, Toggle};
use dsp::saturator::{Saturator, SaturatorParams};

use crate::{
    ChannelMode, CompressorType, EQ_AIR_FREQ_BASS, EQ_AIR_FREQ_GUITAR, EQ_BOOST_DB,
    EQ_HIGH_FREQ_BASS, EQ_HIGH_FREQ_GUITAR, EQ_LOW_FREQ_BASS, EQ_LOW_FREQ_GUITAR, EQ_MID_FREQ_BASS,
    EQ_MID_FREQ_GUITAR, EQ_MID_Q, GAIN_MAX, GAIN_MIN, SAMPLE_RATE, model,
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
    input_conditioner: InputConditioner,
    gate: NoiseGate,
    // Compressors - only one is active at a time
    compressor_type: CompressorType,
    compressor_standard: Compressor,
    compressor_1176: Comp1176,
    compressor_163: Comp163,
    eq: ThreeBandEq,
    saturator: Saturator,
    cab_sim: CabSim,
    chorus: Chorus,
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
            input_conditioner: InputConditioner::new(sample_rate),
            gate: NoiseGate::new(sample_rate),
            compressor_type: CompressorType::default(),
            compressor_standard: Compressor::new(sample_rate),
            compressor_1176: Comp1176::new(sample_rate),
            compressor_163: Comp163::new(sample_rate),
            eq: ThreeBandEq::new(
                EQ_LOW_FREQ_GUITAR,
                EQ_MID_FREQ_GUITAR,
                EQ_HIGH_FREQ_GUITAR,
                EQ_AIR_FREQ_GUITAR,
                EQ_BOOST_DB,
                EQ_MID_Q,
                sr,
            ),
            saturator,
            cab_sim,
            chorus: Chorus::with_sample_rate(sample_rate),
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

    pub fn chorus_params(&self) -> &ChorusParams {
        self.chorus.params()
    }

    pub fn chorus_params_mut(&mut self) -> &mut ChorusParams {
        self.chorus.params_mut()
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
                    EQ_AIR_FREQ_GUITAR,
                    sr,
                );
                self.cab_sim_params_mut().enabled.set(true);
                self.saturator_params_mut().enabled.set(false);
            }
            ChannelMode::Bass => {
                self.eq.reconfigure(
                    EQ_LOW_FREQ_BASS,
                    EQ_MID_FREQ_BASS,
                    EQ_HIGH_FREQ_BASS,
                    EQ_AIR_FREQ_BASS,
                    sr,
                );
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

    fn reset_processing_state(&mut self) {
        self.gate.reset();
        self.compressor_standard.reset();
        self.compressor_1176.reset();
        self.compressor_163.reset();
        self.cab_sim.reset();
        self.chorus.reset();
        self.limiter.reset();
    }

    pub(crate) fn active_compressor_enabled(&self) -> bool {
        match self.compressor_type {
            CompressorType::Standard => self.compressor_standard.params().enabled.get(),
            CompressorType::Comp1176 => self.compressor_1176.params().enabled.get(),
            CompressorType::Comp163 => self.compressor_163.params().enabled.get(),
        }
    }

    pub(crate) fn set_active_compressor_enabled(&mut self, value: bool) {
        match self.compressor_type {
            CompressorType::Standard => self.compressor_standard.params_mut().enabled.set(value),
            CompressorType::Comp1176 => self.compressor_1176.params_mut().enabled.set(value),
            CompressorType::Comp163 => self.compressor_163.params_mut().enabled.set(value),
        }
    }

    fn toggle_param(&self, id: &str) -> Option<&Toggle> {
        match id {
            model::param::GATE_ENABLED => Some(&self.gate.params().enabled),
            model::param::STANDARD_ENABLED => Some(&self.compressor_standard.params().enabled),
            model::param::COMP1176_ENABLED => Some(&self.compressor_1176.params().enabled),
            model::param::COMP163_ENABLED => Some(&self.compressor_163.params().enabled),
            model::param::EQ_LOW_ENABLED => Some(&self.eq.params().low_enabled),
            model::param::EQ_MID_ENABLED => Some(&self.eq.params().mid_enabled),
            model::param::EQ_HIGH_ENABLED => Some(&self.eq.params().high_enabled),
            model::param::EQ_AIR_ENABLED => Some(&self.eq.params().air_enabled),
            model::param::SATURATOR_ENABLED => Some(&self.saturator.params().enabled),
            model::param::CAB_ENABLED => Some(&self.cab_sim.params().enabled),
            model::param::CHORUS_ENABLED => Some(&self.chorus.params().enabled),
            model::param::LIMITER_ENABLED => Some(&self.limiter.params().enabled),
            _ => None,
        }
    }

    fn toggle_param_mut(&mut self, id: &str) -> Option<&mut Toggle> {
        match id {
            model::param::GATE_ENABLED => Some(&mut self.gate.params_mut().enabled),
            model::param::STANDARD_ENABLED => {
                Some(&mut self.compressor_standard.params_mut().enabled)
            }
            model::param::COMP1176_ENABLED => Some(&mut self.compressor_1176.params_mut().enabled),
            model::param::COMP163_ENABLED => Some(&mut self.compressor_163.params_mut().enabled),
            model::param::EQ_LOW_ENABLED => Some(&mut self.eq.params_mut().low_enabled),
            model::param::EQ_MID_ENABLED => Some(&mut self.eq.params_mut().mid_enabled),
            model::param::EQ_HIGH_ENABLED => Some(&mut self.eq.params_mut().high_enabled),
            model::param::EQ_AIR_ENABLED => Some(&mut self.eq.params_mut().air_enabled),
            model::param::SATURATOR_ENABLED => Some(&mut self.saturator.params_mut().enabled),
            model::param::CAB_ENABLED => Some(&mut self.cab_sim.params_mut().enabled),
            model::param::CHORUS_ENABLED => Some(&mut self.chorus.params_mut().enabled),
            model::param::LIMITER_ENABLED => Some(&mut self.limiter.params_mut().enabled),
            _ => None,
        }
    }

    fn knob_param(&self, id: &str) -> Option<&Knob> {
        match id {
            model::param::GAIN => Some(&self.params.gain),
            model::param::GATE_THRESHOLD => Some(&self.gate.params().threshold),
            model::param::STANDARD_THRESHOLD => Some(&self.compressor_standard.params().threshold),
            model::param::STANDARD_RATIO => Some(&self.compressor_standard.params().ratio),
            model::param::STANDARD_ATTACK => Some(&self.compressor_standard.params().attack),
            model::param::STANDARD_RELEASE => Some(&self.compressor_standard.params().release),
            model::param::COMP1176_INPUT => Some(&self.compressor_1176.params().input),
            model::param::COMP1176_OUTPUT => Some(&self.compressor_1176.params().output),
            model::param::COMP1176_ATTACK => Some(&self.compressor_1176.params().attack),
            model::param::COMP1176_RELEASE => Some(&self.compressor_1176.params().release),
            model::param::COMP163_COMPRESSION => Some(&self.compressor_163.params().compression),
            model::param::COMP163_OUTPUT => Some(&self.compressor_163.params().output),
            model::param::EQ_LOW_GAIN => Some(&self.eq.params().low_gain),
            model::param::EQ_MID_GAIN => Some(&self.eq.params().mid_gain),
            model::param::EQ_HIGH_GAIN => Some(&self.eq.params().high_gain),
            model::param::SATURATOR_DRIVE => Some(&self.saturator.params().drive),
            model::param::CHORUS_RATE => Some(&self.chorus.params().rate),
            model::param::CHORUS_DEPTH => Some(&self.chorus.params().depth),
            model::param::CHORUS_MIX => Some(&self.chorus.params().mix),
            model::param::LIMITER_THRESHOLD => Some(&self.limiter.params().threshold),
            model::param::LIMITER_RELEASE => Some(&self.limiter.params().release),
            _ => None,
        }
    }

    fn knob_param_mut(&mut self, id: &str) -> Option<&mut Knob> {
        match id {
            model::param::GAIN => Some(&mut self.params.gain),
            model::param::GATE_THRESHOLD => Some(&mut self.gate.params_mut().threshold),
            model::param::STANDARD_THRESHOLD => {
                Some(&mut self.compressor_standard.params_mut().threshold)
            }
            model::param::STANDARD_RATIO => Some(&mut self.compressor_standard.params_mut().ratio),
            model::param::STANDARD_ATTACK => {
                Some(&mut self.compressor_standard.params_mut().attack)
            }
            model::param::STANDARD_RELEASE => {
                Some(&mut self.compressor_standard.params_mut().release)
            }
            model::param::COMP1176_INPUT => Some(&mut self.compressor_1176.params_mut().input),
            model::param::COMP1176_OUTPUT => Some(&mut self.compressor_1176.params_mut().output),
            model::param::COMP1176_ATTACK => Some(&mut self.compressor_1176.params_mut().attack),
            model::param::COMP1176_RELEASE => Some(&mut self.compressor_1176.params_mut().release),
            model::param::COMP163_COMPRESSION => {
                Some(&mut self.compressor_163.params_mut().compression)
            }
            model::param::COMP163_OUTPUT => Some(&mut self.compressor_163.params_mut().output),
            model::param::EQ_LOW_GAIN => Some(&mut self.eq.params_mut().low_gain),
            model::param::EQ_MID_GAIN => Some(&mut self.eq.params_mut().mid_gain),
            model::param::EQ_HIGH_GAIN => Some(&mut self.eq.params_mut().high_gain),
            model::param::SATURATOR_DRIVE => Some(&mut self.saturator.params_mut().drive),
            model::param::CHORUS_RATE => Some(&mut self.chorus.params_mut().rate),
            model::param::CHORUS_DEPTH => Some(&mut self.chorus.params_mut().depth),
            model::param::CHORUS_MIX => Some(&mut self.chorus.params_mut().mix),
            model::param::LIMITER_THRESHOLD => Some(&mut self.limiter.params_mut().threshold),
            model::param::LIMITER_RELEASE => Some(&mut self.limiter.params_mut().release),
            _ => None,
        }
    }
}

impl Process for Channel {
    #[inline]
    fn process(&mut self, sample: f32) -> f32 {
        let conditioned = self.input_conditioner.process(sample);
        if self.input_conditioner.take_reset_requested() {
            self.reset_processing_state();
        }

        // Signal flow: Input -> Gate -> Compressor -> EQ -> Saturator -> Cab Sim -> Chorus -> Gain -> Limiter -> Output
        let s = self.gate.process(conditioned);
        let s = match self.compressor_type {
            CompressorType::Standard => self.compressor_standard.process(s),
            CompressorType::Comp1176 => self.compressor_1176.process(s),
            CompressorType::Comp163 => self.compressor_163.process(s),
        };
        let s = self.eq.process(s);
        let s = self.saturator.process(s);
        let s = self.cab_sim.process(s);
        let s = self.chorus.process(s);
        let s = s * self.params.gain.get_smoothed();
        self.limiter.process(s)
    }
}

impl ParamModel for Channel {
    fn app_spec(&self) -> &'static AppSpec {
        &model::APP
    }

    fn get_param(&self, id: &str) -> Option<ParamValue> {
        if let Some(toggle) = self.toggle_param(id) {
            return Some(ParamValue::Bool(toggle.get()));
        }

        if let Some(knob) = self.knob_param(id) {
            return Some(ParamValue::Float(knob.get()));
        }

        audio_app::get_bound_param(model::SPECIAL_BINDINGS, self, id)
    }

    fn set_param(&mut self, id: &str, value: ParamValue) -> Result<(), ParamError> {
        if let (Some(toggle), ParamValue::Bool(value)) = (self.toggle_param_mut(id), value) {
            toggle.set(value);
            return Ok(());
        }

        if let (Some(knob), ParamValue::Float(value)) = (self.knob_param_mut(id), value) {
            knob.set(value);
            return Ok(());
        }

        audio_app::set_bound_param(model::SPECIAL_BINDINGS, self, id, value)
    }
}

struct InputConditioner {
    dc_blocker: DcBlocker,
    transient_suppressor: InputTransientSuppressor,
}

impl InputConditioner {
    fn new(sample_rate: u32) -> Self {
        Self {
            dc_blocker: DcBlocker::new(sample_rate),
            transient_suppressor: InputTransientSuppressor::new(sample_rate),
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let dc_blocked = self.dc_blocker.process(input);
        self.transient_suppressor.process(dc_blocked)
    }

    fn take_reset_requested(&mut self) -> bool {
        self.transient_suppressor.take_reset_requested()
    }
}

struct DcBlocker {
    prev_input: f32,
    prev_output: f32,
    pole: f32,
}

impl DcBlocker {
    const CUTOFF_HZ: f32 = 20.0;

    fn new(sample_rate: u32) -> Self {
        let sample_rate = sample_rate as f32;
        let pole = (-2.0 * PI * Self::CUTOFF_HZ / sample_rate).exp();
        Self {
            prev_input: 0.0,
            prev_output: 0.0,
            pole,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = input - self.prev_input + self.pole * self.prev_output;
        self.prev_input = input;
        self.prev_output = output;
        output
    }
}

struct InputTransientSuppressor {
    prev_input: f32,
    gain: f32,
    mute_samples_remaining: usize,
    mute_samples: usize,
    fade_in_step: f32,
    reset_requested: bool,
}

impl InputTransientSuppressor {
    const POP_DELTA_THRESHOLD: f32 = 0.45;
    const POP_LEVEL_THRESHOLD: f32 = 0.25;
    const POP_CLIP_THRESHOLD: f32 = 0.9;
    const MUTE_MS: f32 = 45.0;
    const FADE_IN_MS: f32 = 120.0;

    fn new(sample_rate: u32) -> Self {
        let fade_in_samples = ((sample_rate as f32 * Self::FADE_IN_MS) / 1000.0).max(1.0);
        Self {
            prev_input: 0.0,
            gain: 1.0,
            mute_samples_remaining: 0,
            mute_samples: ((sample_rate as f32 * Self::MUTE_MS) / 1000.0).round() as usize,
            fade_in_step: 1.0 / fade_in_samples,
            reset_requested: false,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        if self.is_plug_transient(input) {
            self.mute_samples_remaining = self.mute_samples;
            self.gain = 0.0;
            self.reset_requested = true;
        }

        self.prev_input = input;

        if self.mute_samples_remaining > 0 {
            self.mute_samples_remaining -= 1;
            return 0.0;
        }

        if self.gain < 1.0 {
            self.gain = (self.gain + self.fade_in_step).min(1.0);
        }

        input * self.gain
    }

    fn take_reset_requested(&mut self) -> bool {
        let reset_requested = self.reset_requested;
        self.reset_requested = false;
        reset_requested
    }

    fn is_plug_transient(&self, input: f32) -> bool {
        let level = input.abs();
        let delta = (input - self.prev_input).abs();
        level >= Self::POP_CLIP_THRESHOLD
            || (level >= Self::POP_LEVEL_THRESHOLD && delta >= Self::POP_DELTA_THRESHOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::{DcBlocker, InputTransientSuppressor};

    #[test]
    fn dc_blocker_removes_constant_offset() {
        let mut blocker = DcBlocker::new(48_000);
        let mut last = 0.0;

        for _ in 0..4_096 {
            last = blocker.process(0.25);
        }

        assert!(last.abs() < 0.001);
    }

    #[test]
    fn suppresses_large_input_step_and_requests_reset() {
        let mut suppressor = InputTransientSuppressor::new(48_000);

        for _ in 0..16 {
            let frame = suppressor.process(0.01);
            assert!(frame > 0.0);
        }

        let muted = suppressor.process(0.95);
        assert_eq!(muted, 0.0);
        assert!(suppressor.take_reset_requested());
        assert!(!suppressor.take_reset_requested());
    }
}
