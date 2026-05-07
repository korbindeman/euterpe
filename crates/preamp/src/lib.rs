#![no_std]

pub mod channel;
pub mod model;

// Re-export dsp for convenience
pub use audio_app;
pub use dsp;

// ---------------------------------------------------------------------------
// Channel mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    Guitar,
    Bass,
}

// ---------------------------------------------------------------------------
// Compressor type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressorType {
    /// Standard soft-knee compressor with full controls
    Standard,
    /// 1176-style FET compressor with feedback topology
    Comp1176,
    /// dbx 163-style compressor with single compression knob
    #[default]
    Comp163,
}

// ---------------------------------------------------------------------------
// Preamp-specific constants
// ---------------------------------------------------------------------------

pub const SAMPLE_RATE: u32 = 48_000;

// EQ — Guitar mode frequencies
pub const EQ_LOW_FREQ_GUITAR: f32 = 200.0;
pub const EQ_MID_FREQ_GUITAR: f32 = 800.0;
pub const EQ_HIGH_FREQ_GUITAR: f32 = 3000.0;
pub const EQ_AIR_FREQ_GUITAR: f32 = 10000.0;

// EQ — Bass mode frequencies
pub const EQ_LOW_FREQ_BASS: f32 = 100.0;
pub const EQ_MID_FREQ_BASS: f32 = 500.0;
pub const EQ_HIGH_FREQ_BASS: f32 = 2000.0;
pub const EQ_AIR_FREQ_BASS: f32 = 8000.0;

// EQ — shared
pub const EQ_BOOST_DB: f32 = 6.0;
pub const EQ_MID_Q: f32 = 1.0;

// Gain range
pub const GAIN_MIN: f32 = 0.0;
pub const GAIN_MAX: f32 = 2.0; // +6 dB
