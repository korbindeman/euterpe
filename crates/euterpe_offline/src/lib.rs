//! The offline Euterpe platform.
//!
//! Runs any [`Processor`](euterpe_core::Processor) over audio files, with no
//! audio device and fully deterministic output. This is the platform tests,
//! analysis and agents use.
//!
//! - [`wav`]: dependency-free WAV reading and writing
//! - [`render()`]: block rendering with latency compensation
//! - [`signal`]: test signals (impulse, sine, log sweep, noise)
//! - [`check`]: invariant checks every processor should pass

pub mod check;
pub mod fpu;
pub mod render;
pub mod signal;
pub mod wav;

pub use render::{RenderOptions, render};
