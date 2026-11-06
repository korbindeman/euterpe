//! The desktop platform: play any registered device through an audio
//! interface, with a control panel generated from the device's schema.
//!
//! - [`params::ParamBank`]: lock-free parameter values, written by the UI and
//!   read by the audio thread once per block
//! - [`runner`]: a device's graph behind a small object-safe trait, so the
//!   audio thread can switch devices at runtime, plus the adapter from the
//!   interface's buffer sizes to 32-sample blocks
//! - [`host`]: the audio callback: input source, the runner, meters and load
//! - [`app`]: the egui window

pub mod app;
pub mod host;
pub mod params;
pub mod runner;
