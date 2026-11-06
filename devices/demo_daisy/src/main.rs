//! `demo.drive` on the Daisy Seed: left input through tanh drive to both
//! outputs. The user LED blinks at 1 Hz while every block is within budget.

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use euterpe_core::Platform;
use panic_halt as _;

#[entry]
fn main() -> ! {
    euterpe_daisy::Daisy.run::<euterpe_demo::DriveDevice>()
}
