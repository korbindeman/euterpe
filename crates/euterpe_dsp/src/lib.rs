//! DSP building blocks for Euterpe devices.
//!
//! Everything here follows the hot-path rules in `specs/ARCHITECTURE.md`:
//! no allocation, no panics, no dynamic dispatch in `process`.
//!
//! - [`oversample`]: [`Oversampled2`], [`Oversampled4`] and [`Oversampled8`],
//!   which run a processor at 2, 4 or 8 times the device rate through
//!   polyphase half-band filters.
//! - [`convolve`]: [`Convolver`], zero-latency partitioned convolution with
//!   an impulse response.
//! - [`split_convolve`]: [`SplitConvolver`], the same for long IRs
//!   (cabinets) at about 70% of the cost: short partitions for the head,
//!   512-point FFTs spread over 8 blocks for the tail.
//! - [`fft`]: the 64- and 512-point real FFTs they run on.
//! - [`filter`]: [`Svf`] (every second-order shape, stable under
//!   modulation), [`OnePole`] and [`DcBlocker`].
//! - [`delay`]: [`DelayLine`], with integer, linear and cubic taps.
//! - [`dynamics`]: [`Follower`] envelopes and gain smoothers, and the
//!   [`SoftKnee`] compressor curve.
//! - [`lfo`]: [`Phasor`] and the LFO [`Shape`]s.

#![no_std]
// An unsuffixed float literal falls back to f64, which is soft-float on the
// Cortex-M7. Code that runs on the device must make every literal's type clear.
#![cfg_attr(not(test), warn(clippy::default_numeric_fallback))]

pub mod convolve;
pub mod delay;
pub mod dynamics;
pub mod fft;
pub mod filter;
pub mod lfo;
pub mod oversample;
pub mod split_convolve;

pub use convolve::Convolver;
pub use delay::DelayLine;
pub use dynamics::{Ballistics, Follower, SoftKnee, time_constant};
pub use filter::{DcBlocker, OnePole, Svf, SvfCoeffs, dc_blocker_coeff, one_pole_coeff};
pub use lfo::{Phasor, Shape};
pub use oversample::{Oversample2x, Oversampled2, Oversampled4, Oversampled8};
pub use split_convolve::SplitConvolver;
