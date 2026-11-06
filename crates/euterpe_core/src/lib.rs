//! Core of the Euterpe audio toolkit.
//!
//! Everything here is `no_std`, allocation-free and safe to call from an
//! audio interrupt. See `specs/ARCHITECTURE.md` for the design.
//!
//! The pieces:
//! - [`Processor`]: the block-based unit of DSP work, and [`SampleProcessor`] +
//!   [`PerSample`] for processors that are naturally per-sample.
//! - [`ParamSet`]: a static, introspectable parameter schema, declared with
//!   `#[derive(Params)]` (or the older [`params!`]).
//! - `#[derive(Chain)]`: a struct of processors as one [`Processor`], run in
//!   field order, with each stage's parameters nested under its field name.
//! - [`Smoother`]: control-rate smoothing that hands out per-block [`Ramp`]s.
//! - [`fastmath`]: approximations with documented, tested error bounds.
//! - [`Device`]: a named, complete signal graph that platforms can run, with
//!   its [`panel`] of pots, switches and buttons.
//! - [`Platform`]: something that runs a device (the Daisy, offline, ...).
//! - [`place`]: building a graph in static memory instead of on the stack.

#![no_std]
// An unsuffixed float literal falls back to f64, which is soft-float on the
// Cortex-M7. Code that runs on the device must make every literal's type clear.
#![cfg_attr(not(test), warn(clippy::default_numeric_fallback))]

// Lets the derive macros' `::euterpe_core` paths resolve inside this crate.
extern crate self as euterpe_core;

pub mod device;
pub mod fastmath;
pub mod panel;
pub mod params;
pub mod place;
pub mod platform;
pub mod processor;
pub mod smooth;

pub use device::Device;
pub use euterpe_macros::{Chain, Params};
pub use params::{ParamDesc, ParamError, ParamKind, ParamSet, ParamValue};
pub use platform::Platform;
pub use processor::{BLOCK, Block, Ctx, PerSample, Processor, SampleProcessor};
pub use smooth::{Ramp, Smoother};
