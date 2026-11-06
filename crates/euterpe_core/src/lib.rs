//! Core of the Euterpe audio toolkit.
//!
//! Everything here is `no_std`, allocation-free and safe to call from an
//! audio interrupt. See `specs/ARCHITECTURE.md` for the design.
//!
//! The pieces:
//! - [`Processor`]: the block-based unit of DSP work, and [`SampleProcessor`] +
//!   [`PerSample`] for processors that are naturally per-sample.
//! - [`ParamSet`]: a static, introspectable parameter schema, usually declared
//!   with [`params!`].
//! - [`Smoother`]: control-rate smoothing that hands out per-block [`Ramp`]s.
//! - [`fastmath`]: approximations with documented, tested error bounds.
//! - [`Device`]: a named, complete signal graph that platforms can run.

#![no_std]

pub mod device;
pub mod fastmath;
pub mod params;
pub mod processor;
pub mod smooth;

pub use device::Device;
pub use params::{ParamDesc, ParamError, ParamKind, ParamSet, ParamValue};
pub use processor::{BLOCK, Block, Ctx, PerSample, Processor, SampleProcessor};
pub use smooth::{Ramp, Smoother};
