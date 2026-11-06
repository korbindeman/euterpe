//! Basic pedals, built from the `euterpe_dsp` blocks. Each is a complete
//! device: params, smoothing, tests against closed forms, and bench numbers.
//!
//! - `pedal.fuzz`: asymmetric tanh clipping at 4x oversampling, a DC
//!   blocker and a lowpass tone control
//! - `pedal.tremolo`: amplitude modulation by an LFO (sine, triangle,
//!   square, saw)
//! - `pedal.delay`: a feedback delay up to 1 s, with a lowpass in the
//!   feedback path. Its 256 KB line puts it in the Daisy's AXI SRAM.
//! - `pedal.eq`: bass and treble shelves and a sweepable mid bell

#![no_std]
// An unsuffixed float literal falls back to f64, which is soft-float on the
// Cortex-M7. Code that runs on the device must make every literal's type clear.
#![cfg_attr(not(test), warn(clippy::default_numeric_fallback))]

pub mod delay;
pub mod eq;
pub mod fuzz;
pub mod tremolo;

pub use delay::{Delay, DelayDevice};
pub use eq::{Eq, EqDevice};
pub use fuzz::{Fuzz, FuzzDevice};
pub use tremolo::{Tremolo, TremoloDevice};
