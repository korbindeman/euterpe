//! `preamp.channel`: the legacy instrument preamp (`crates/preamp`'s
//! `Channel`), ported to Euterpe.
//!
//! ```text
//! input -> conditioner -> gate -> compressor -> EQ -> saturator -> cab
//!       -> chorus -> gain -> limiter -> output
//! ```
//!
//! The parameter ids are the legacy model's (`gate.threshold`,
//! `compressor.standard.ratio`, `eq.low.gain`, ...), and every stage
//! follows its legacy processor sample for sample; the tests null each one
//! against the legacy code. What changed:
//!
//! - It runs in 32-sample blocks, with fastmath in place of micromath.
//! - The cab has no latency (the legacy one had 256 samples).
//! - The EQ is state-variable filters, so a knob turn doesn't click. The
//!   responses are the same.
//! - `mode` sets only the EQ's corner frequencies. The legacy `set_mode`
//!   also switched the cab and the saturator; here those are their own
//!   toggles.
//! - A plug pop resets the stages at the end of the block it lands in,
//!   rather than at that sample. The input is muted from that sample
//!   either way.

#![no_std]
// An unsuffixed float literal falls back to f64, which is soft-float on the
// Cortex-M7. Code that runs on the device must make every literal's type clear.
#![cfg_attr(not(test), warn(clippy::default_numeric_fallback))]

pub mod stages;

use euterpe_core::place::Init;
use euterpe_core::{Block, Ctx, Device, ParamKind, ParamSet, Params, Processor, Smoother};

pub use stages::{
    Cab, CabParams, Chorus, ChorusParams, Compressor, CompressorParams, Conditioner, Eq, EqParams,
    Gate, GateParams, Limiter, LimiterParams, Saturator, SaturatorParams,
};

const MODES: &[&str] = &["Guitar", "Bass"];

#[derive(Params, Clone, Debug, PartialEq)]
pub struct GainParams {
    #[param(id = "gain", label = "Gain", kind = ParamKind::float(0.0, 2.0, 1.0).smooth_ms(10.0))]
    pub gain: f32,
}

#[derive(Params, Clone, Debug, PartialEq)]
pub struct PreampParams {
    #[param(id = "mode", label = "Mode", kind = ParamKind::choice(MODES, 0))]
    pub mode: u8,
    #[param(nested = "channel")]
    pub channel: GainParams,
    #[param(nested)]
    pub gate: GateParams,
    #[param(nested)]
    pub compressor: CompressorParams,
    #[param(nested)]
    pub eq: EqParams,
    #[param(nested)]
    pub saturator: SaturatorParams,
    #[param(nested)]
    pub cab: CabParams,
    #[param(nested)]
    pub chorus: ChorusParams,
    #[param(nested)]
    pub limiter: LimiterParams,
}

/// The whole channel.
#[derive(Default)]
pub struct Preamp {
    conditioner: Conditioner,
    gate: Gate,
    compressor: Compressor,
    eq: Eq,
    saturator: Saturator,
    cab: Cab,
    chorus: Chorus,
    gain: Smoother,
    limiter: Limiter,
}

impl Preamp {
    /// The stages a plug pop clears (the legacy `reset_processing_state`).
    fn reset_after_pop(&mut self) {
        self.gate.reset();
        self.compressor.reset();
        self.cab.reset();
        self.chorus.reset();
        self.limiter.reset();
    }
}

impl Processor for Preamp {
    type Params = PreampParams;

    fn prepare(&mut self, ctx: &Ctx) {
        self.conditioner.prepare(ctx);
        self.gate.prepare(ctx);
        self.compressor.prepare(ctx);
        self.eq.prepare(ctx);
        self.saturator.prepare(ctx);
        self.cab.prepare(ctx);
        self.chorus.prepare(ctx);
        self.gain.prepare(ctx, GainParams::smooth_ms("gain"));
        self.limiter.prepare(ctx);
    }

    fn reset(&mut self) {
        self.conditioner.reset();
        self.gate.reset();
        self.compressor.reset();
        self.eq.reset();
        self.saturator.reset();
        self.cab.reset();
        self.chorus.reset();
        self.gain.reset();
        self.limiter.reset();
    }

    #[inline]
    fn process(&mut self, buf: &mut Block, p: &PreampParams) {
        self.conditioner.process(buf, &());
        self.gate.process(buf, &p.gate);
        self.compressor.process(buf, &p.compressor);
        self.eq.mode = p.mode;
        self.eq.process(buf, &p.eq);
        self.saturator.process(buf, &p.saturator);
        self.cab.process(buf, &p.cab);
        self.chorus.process(buf, &p.chorus);
        let mut gain = self.gain.block(p.channel.gain);
        for s in buf.iter_mut() {
            *s *= gain.tick();
        }
        self.limiter.process(buf, &p.limiter);
        if self.conditioner.take_reset_requested() {
            self.reset_after_pop();
        }
    }
}

/// `preamp.channel`
pub struct PreampDevice;

impl Device for PreampDevice {
    const NAME: &'static str = "preamp.channel";
    const DESCRIPTION: &'static str = "The legacy preamp channel: gate, compressor (standard, 1176, 163), 4-band EQ, saturator, cab, chorus, gain, limiter.";
    type Graph = Preamp;
    // SAFETY: every field of every stage defaults to zero (filter and
    // envelope state, smoothers, coefficients that `prepare` sets, the IR
    // that `prepare` loads, `false` flags); `reset` sets the non-zero
    // starting states. `check_init` tests it.
    const GRAPH_INIT: Init = unsafe { Init::zeroed() };
    const PANEL: &'static [euterpe_core::panel::Binding] = &[
        euterpe_core::panel::pot(0, "channel.gain"),
        euterpe_core::panel::pot(1, "gate.threshold"),
        euterpe_core::panel::pot(2, "compressor.163.compression"),
        euterpe_core::panel::pot(3, "eq.low.gain"),
        euterpe_core::panel::pot(4, "eq.mid.gain"),
        euterpe_core::panel::pot(5, "eq.high.gain"),
        euterpe_core::panel::pot(6, "saturator.drive"),
        euterpe_core::panel::pot(7, "limiter.threshold"),
        euterpe_core::panel::switch(0, "mode"),
        euterpe_core::panel::button(1, "compressor.type"),
        euterpe_core::panel::button(2, "compressor.enabled"),
        euterpe_core::panel::button(3, "saturator.enabled"),
        euterpe_core::panel::button(4, "chorus.enabled"),
        euterpe_core::panel::button(5, "cab.enabled"),
        euterpe_core::panel::switch(6, "eq.air.enabled"),
    ];
}

#[cfg(test)]
mod tests;
