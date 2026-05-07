use audio_app::{
    AppSpec, ParamBinding, ParamError, ParamValue, define_hardware_bindings, define_params,
    define_ui_controls,
};

use crate::{ChannelMode, CompressorType, channel::Channel};

pub const MODE_OPTIONS: &[&str] = &["Guitar", "Bass"];
pub const COMPRESSOR_TYPE_OPTIONS: &[&str] = &["Standard", "1176", "163"];
pub const COMP1176_RATIO_OPTIONS: &[&str] = &["4:1", "8:1", "12:1", "20:1"];

define_params! {
    MODE: choice("mode", "Mode", "Channel", MODE_OPTIONS, 0),
    GAIN: knob("channel.gain", "Gain", "Channel", crate::GAIN_MIN, crate::GAIN_MAX, 1.0),
    GATE_ENABLED: toggle("gate.enabled", "Enabled", "Gate", true),
    GATE_THRESHOLD: knob("gate.threshold", "Threshold", "Gate", -80.0, 0.0, -70.0),
    COMPRESSOR_TYPE: choice(
        "compressor.type",
        "Type",
        "Compressor",
        COMPRESSOR_TYPE_OPTIONS,
        2
    ),
    COMPRESSOR_ENABLED: toggle("compressor.enabled", "Enabled", "Compressor", true),
    STANDARD_ENABLED: toggle(
        "compressor.standard.enabled",
        "Standard Enabled",
        "Compressor",
        true
    ),
    STANDARD_THRESHOLD: knob(
        "compressor.standard.threshold",
        "Threshold",
        "Compressor",
        -60.0,
        0.0,
        -20.0
    ),
    STANDARD_RATIO: knob("compressor.standard.ratio", "Ratio", "Compressor", 1.0, 20.0, 4.0),
    STANDARD_ATTACK: knob(
        "compressor.standard.attack",
        "Attack",
        "Compressor",
        0.1,
        100.0,
        15.0
    ),
    STANDARD_RELEASE: knob(
        "compressor.standard.release",
        "Release",
        "Compressor",
        10.0,
        1000.0,
        100.0
    ),
    COMP1176_ENABLED: toggle("compressor.1176.enabled", "1176 Enabled", "Compressor", true),
    COMP1176_INPUT: knob("compressor.1176.input", "Input", "Compressor", -20.0, 40.0, 0.0),
    COMP1176_OUTPUT: knob("compressor.1176.output", "Output", "Compressor", -20.0, 40.0, 0.0),
    COMP1176_RATIO: choice(
        "compressor.1176.ratio",
        "Ratio",
        "Compressor",
        COMP1176_RATIO_OPTIONS,
        0
    ),
    COMP1176_ATTACK: knob("compressor.1176.attack", "Attack", "Compressor", 0.02, 0.8, 0.4),
    COMP1176_RELEASE: knob(
        "compressor.1176.release",
        "Release",
        "Compressor",
        50.0,
        1100.0,
        300.0
    ),
    COMP163_ENABLED: toggle("compressor.163.enabled", "163 Enabled", "Compressor", true),
    COMP163_COMPRESSION: knob(
        "compressor.163.compression",
        "Compression",
        "Compressor",
        0.0,
        1.0,
        0.5
    ),
    COMP163_OUTPUT: knob("compressor.163.output", "Output", "Compressor", -20.0, 20.0, 0.0),
    EQ_LOW_ENABLED: toggle("eq.low.enabled", "Low", "EQ", true),
    EQ_LOW_GAIN: knob("eq.low.gain", "Low Gain", "EQ", -12.0, 12.0, crate::EQ_BOOST_DB),
    EQ_MID_ENABLED: toggle("eq.mid.enabled", "Mid", "EQ", true),
    EQ_MID_GAIN: knob("eq.mid.gain", "Mid Gain", "EQ", -12.0, 12.0, crate::EQ_BOOST_DB),
    EQ_HIGH_ENABLED: toggle("eq.high.enabled", "High", "EQ", true),
    EQ_HIGH_GAIN: knob("eq.high.gain", "High Gain", "EQ", -12.0, 12.0, crate::EQ_BOOST_DB),
    EQ_AIR_ENABLED: toggle("eq.air.enabled", "Air", "EQ", true),
    SATURATOR_ENABLED: toggle("saturator.enabled", "Enabled", "Saturator", false),
    SATURATOR_DRIVE: knob("saturator.drive", "Drive", "Saturator", 0.0, 24.0, 12.0),
    CAB_ENABLED: toggle("cab.enabled", "Cab Sim", "Cab", true),
    CHORUS_ENABLED: toggle("chorus.enabled", "Enabled", "Chorus", false),
    CHORUS_RATE: knob("chorus.rate", "Rate", "Chorus", 0.1, 5.0, 0.5),
    CHORUS_DEPTH: knob("chorus.depth", "Depth", "Chorus", 0.0, 1.0, 0.3),
    CHORUS_MIX: knob("chorus.mix", "Mix", "Chorus", 0.0, 1.0, 0.3),
    LIMITER_ENABLED: toggle("limiter.enabled", "Enabled", "Limiter", true),
    LIMITER_THRESHOLD: knob("limiter.threshold", "Threshold", "Limiter", -20.0, 0.0, -1.0),
    LIMITER_RELEASE: knob("limiter.release", "Release", "Limiter", 10.0, 500.0, 100.0),
}

define_ui_controls! {
    (param::MODE, "Channel"),
    (param::GAIN, "Channel"),
    (param::GATE_ENABLED, "Gate"),
    (param::GATE_THRESHOLD, "Gate"),
    (param::COMPRESSOR_TYPE, "Compressor"),
    (param::COMPRESSOR_ENABLED, "Compressor"),
    (param::STANDARD_ENABLED, "Compressor"),
    (param::STANDARD_THRESHOLD, "Compressor"),
    (param::STANDARD_RATIO, "Compressor"),
    (param::STANDARD_ATTACK, "Compressor"),
    (param::STANDARD_RELEASE, "Compressor"),
    (param::COMP1176_ENABLED, "Compressor"),
    (param::COMP1176_INPUT, "Compressor"),
    (param::COMP1176_OUTPUT, "Compressor"),
    (param::COMP1176_RATIO, "Compressor"),
    (param::COMP1176_ATTACK, "Compressor"),
    (param::COMP1176_RELEASE, "Compressor"),
    (param::COMP163_ENABLED, "Compressor"),
    (param::COMP163_COMPRESSION, "Compressor"),
    (param::COMP163_OUTPUT, "Compressor"),
    (param::EQ_LOW_ENABLED, "EQ"),
    (param::EQ_LOW_GAIN, "EQ"),
    (param::EQ_MID_ENABLED, "EQ"),
    (param::EQ_MID_GAIN, "EQ"),
    (param::EQ_HIGH_ENABLED, "EQ"),
    (param::EQ_HIGH_GAIN, "EQ"),
    (param::EQ_AIR_ENABLED, "EQ"),
    (param::SATURATOR_ENABLED, "Saturator"),
    (param::SATURATOR_DRIVE, "Saturator"),
    (param::CAB_ENABLED, "Cab"),
    (param::CHORUS_ENABLED, "Chorus"),
    (param::CHORUS_RATE, "Chorus"),
    (param::CHORUS_DEPTH, "Chorus"),
    (param::CHORUS_MIX, "Chorus"),
    (param::LIMITER_ENABLED, "Limiter"),
    (param::LIMITER_THRESHOLD, "Limiter"),
    (param::LIMITER_RELEASE, "Limiter"),
}

pub const UI_SECTIONS: &[&str] = &[
    "Channel",
    "Gate",
    "Compressor",
    "EQ",
    "Saturator",
    "Cab",
    "Chorus",
    "Limiter",
];

define_hardware_bindings! {
    pot("knob1", param::GAIN, normalized, "Main"),
    button("button1", param::MODE, cycle, "Main"),
    button("button2", param::COMPRESSOR_ENABLED, toggle, "Main"),
}

pub const APP: AppSpec = AppSpec::new("preamp.channel")
    .params(PARAMS)
    .hardware(DAISY_BINDINGS)
    .ui(UI_CONTROLS);

pub fn shows_ui_param(channel: &Channel, param_id: &str) -> bool {
    match param_id {
        param::STANDARD_ENABLED
        | param::STANDARD_THRESHOLD
        | param::STANDARD_RATIO
        | param::STANDARD_ATTACK
        | param::STANDARD_RELEASE => channel.compressor_type() == CompressorType::Standard,
        param::COMP1176_ENABLED
        | param::COMP1176_INPUT
        | param::COMP1176_OUTPUT
        | param::COMP1176_RATIO
        | param::COMP1176_ATTACK
        | param::COMP1176_RELEASE => channel.compressor_type() == CompressorType::Comp1176,
        param::COMP163_ENABLED | param::COMP163_COMPRESSION | param::COMP163_OUTPUT => {
            channel.compressor_type() == CompressorType::Comp163
        }
        param::CAB_ENABLED => channel.mode() == ChannelMode::Guitar,
        _ => true,
    }
}

fn get_mode(channel: &Channel) -> ParamValue {
    ParamValue::Choice(match channel.mode() {
        ChannelMode::Guitar => 0,
        ChannelMode::Bass => 1,
    })
}

fn set_mode(channel: &mut Channel, value: ParamValue) -> Result<(), ParamError> {
    match value {
        ParamValue::Choice(0) => {
            channel.set_mode(ChannelMode::Guitar);
            Ok(())
        }
        ParamValue::Choice(_) => {
            channel.set_mode(ChannelMode::Bass);
            Ok(())
        }
        _ => Err(ParamError::TypeMismatch),
    }
}

fn get_compressor_type(channel: &Channel) -> ParamValue {
    ParamValue::Choice(match channel.compressor_type() {
        CompressorType::Standard => 0,
        CompressorType::Comp1176 => 1,
        CompressorType::Comp163 => 2,
    })
}

fn set_compressor_type(channel: &mut Channel, value: ParamValue) -> Result<(), ParamError> {
    match value {
        ParamValue::Choice(0) => {
            channel.set_compressor_type(CompressorType::Standard);
            Ok(())
        }
        ParamValue::Choice(1) => {
            channel.set_compressor_type(CompressorType::Comp1176);
            Ok(())
        }
        ParamValue::Choice(_) => {
            channel.set_compressor_type(CompressorType::Comp163);
            Ok(())
        }
        _ => Err(ParamError::TypeMismatch),
    }
}

fn get_compressor_enabled(channel: &Channel) -> ParamValue {
    ParamValue::Bool(channel.active_compressor_enabled())
}

fn set_compressor_enabled(channel: &mut Channel, value: ParamValue) -> Result<(), ParamError> {
    match value {
        ParamValue::Bool(value) => {
            channel.set_active_compressor_enabled(value);
            Ok(())
        }
        _ => Err(ParamError::TypeMismatch),
    }
}

fn get_comp1176_ratio(channel: &Channel) -> ParamValue {
    ParamValue::Choice(channel.compressor_1176_params().ratio().to_index() as u8)
}

fn set_comp1176_ratio(channel: &mut Channel, value: ParamValue) -> Result<(), ParamError> {
    match value {
        ParamValue::Choice(choice) => {
            channel
                .compressor_1176_params_mut()
                .set_ratio(dsp::comp_1176::Ratio1176::from_index(choice as usize));
            Ok(())
        }
        _ => Err(ParamError::TypeMismatch),
    }
}

pub const SPECIAL_BINDINGS: &[ParamBinding<Channel>] = &[
    ParamBinding {
        id: param::MODE,
        get: get_mode,
        set: set_mode,
    },
    ParamBinding {
        id: param::COMPRESSOR_TYPE,
        get: get_compressor_type,
        set: set_compressor_type,
    },
    ParamBinding {
        id: param::COMPRESSOR_ENABLED,
        get: get_compressor_enabled,
        set: set_compressor_enabled,
    },
    ParamBinding {
        id: param::COMP1176_RATIO,
        get: get_comp1176_ratio,
        set: set_comp1176_ratio,
    },
];
