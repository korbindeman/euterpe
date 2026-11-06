//! DSP building blocks for Euterpe devices.
//!
//! Everything here follows the hot-path rules in `specs/ARCHITECTURE.md`:
//! no allocation, no panics, no dynamic dispatch in `process`.
//!
//! - [`oversample`]: [`Oversampled2`], [`Oversampled4`] and [`Oversampled8`],
//!   which run a processor at 2, 4 or 8 times the device rate through
//!   polyphase half-band filters.

#![no_std]
// An unsuffixed float literal falls back to f64, which is soft-float on the
// Cortex-M7. Code that runs on the device must make every literal's type clear.
#![cfg_attr(not(test), warn(clippy::default_numeric_fallback))]

pub mod oversample;

pub use oversample::{Oversample2x, Oversampled2, Oversampled4, Oversampled8};
