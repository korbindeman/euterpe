//! Embedded impulse responses for cab simulation.

use ir_embed::embed_ir;

pub static DELUXE_REVERB: &[f32] = &embed_ir!("ir/deluxe_reverb.wav", 2048);

pub static ALL_IRS: [&[f32]; 1] = [DELUXE_REVERB];

/// Returns the IR at the given index, or None if out of bounds.
pub fn get_ir(index: usize) -> Option<&'static [f32]> {
    ALL_IRS.get(index).copied()
}
