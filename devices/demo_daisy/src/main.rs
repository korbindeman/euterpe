//! Firmware: any registered device on the Daisy Seed.
//!
//! The device is picked at build time, `demo.drive` by default:
//!
//! ```text
//! just flash-device demo.rig      # EUTERPE_DEVICE=demo.rig, then DFU
//! ```
//!
//! An unknown name fails the build. Every registered graph is checked
//! against the Daisy's graph arena at compile time.

#![no_std]
#![no_main]

use core::convert::Infallible;

use cortex_m_rt::entry;
use euterpe_core::{Device, Platform};
use panic_halt as _;

const DEVICE: &str = match option_env!("EUTERPE_DEVICE") {
    Some(name) => name,
    None => "demo.drive",
};

const INDEX: usize = match euterpe_registry::index_of(DEVICE) {
    Some(i) => i,
    None => panic!("EUTERPE_DEVICE is not a registered device (see `just euterpe devices`)"),
};

struct Run;

impl euterpe_registry::Visitor for Run {
    type Out = Infallible;
    fn visit<D: Device>(self) -> Infallible {
        euterpe_daisy::Daisy.run::<D>()
    }
}

#[entry]
fn main() -> ! {
    // Picked at compile time: only this device is linked.
    match euterpe_registry::visit_at::<INDEX, _>(Run) {
        Some(never) => match never {},
        None => unreachable!("checked at compile time"),
    }
}
