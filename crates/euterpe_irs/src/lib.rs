//! Cabinet impulse responses, embedded at build time by `ir_embed`:
//! resampled to 48 kHz, mono, peak-normalized, 2048 taps (43 ms), which is
//! `euterpe_dsp::SplitConvolver<6>`'s capacity. Each is 8 KB of flash, linked
//! only into firmware that uses it.

#![no_std]

/// Fender Deluxe Reverb: the legacy preamp's guitar cabinet.
pub static DELUXE_REVERB: [f32; 2048] = ir_embed::embed_ir!("ir/deluxe_reverb.wav", 2048);

/// Vox AC30.
pub static AC30: [f32; 2048] = ir_embed::embed_ir!("ir/ac30.wav", 2048);

/// Ampeg 1x15 bass cabinet.
pub static AMPEG_115: [f32; 2048] = ir_embed::embed_ir!("ir/ampeg_115.wav", 2048);
