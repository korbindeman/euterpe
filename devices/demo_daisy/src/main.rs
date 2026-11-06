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

const _: () = assert!(
    euterpe_registry::contains(DEVICE),
    "EUTERPE_DEVICE is not a registered device (see `just euterpe devices`)"
);

struct Run;

impl euterpe_registry::Visitor for Run {
    type Out = Infallible;
    fn visit<D: Device>(self) -> Infallible {
        euterpe_daisy::Daisy.run::<D>()
    }
}

#[entry]
fn main() -> ! {
    // `contains` was checked at compile time, so the lookup finds it.
    match euterpe_registry::visit(DEVICE, Run) {
        Some(never) => match never {},
        None => unreachable!("checked at compile time"),
    }
}
