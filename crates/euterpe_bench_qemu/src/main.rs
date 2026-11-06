//! Bench firmware for QEMU's `mps2-an500` machine (a Cortex-M7).
//!
//! Don't run this directly: `euterpe bench` builds it, finds the marker
//! functions below in its symbol table and runs it under QEMU with the
//! `tools/qemu/euterpe_insn.c` plugin, which counts instructions between
//! the markers. The firmware prints one `bench` line per scenario over
//! semihosting; the plugin prints one `euterpe-bench` line per scenario, in
//! the same order.
//!
//! To bench a new device, add it to `devices!` below (and to the CLI's
//! registry).

#![no_std]
#![no_main]

use core::hint::black_box;

use cortex_m_rt::entry;
use cortex_m_semihosting::{debug, hprintln};
use euterpe_bench::{Probe, Sink, bench_device, bench_overhead};

macro_rules! devices {
    ($($device:ty),* $(,)?) => {
        fn bench_all(probe: &mut Markers, sink: &mut Semihosting) {
            $( bench_device::<$device>(probe, sink); )*
        }
    };
}

devices! {
    euterpe_demo::GainDevice,
    euterpe_demo::DriveDevice,
}

// The plugin hooks the first instruction of each marker. They must stay out
// of line and distinct, so each does something different with its argument.

#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn euterpe_bench_start(x: u32) {
    black_box(x);
}

#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn euterpe_bench_stop(x: u32) {
    black_box(x.wrapping_add(1));
}

#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn euterpe_bench_end(x: u32) {
    black_box(x.wrapping_add(2));
}

struct Markers;

impl Probe for Markers {
    fn start(&mut self) {
        euterpe_bench_start(0);
    }
    fn stop(&mut self) {
        euterpe_bench_stop(0);
    }
    fn end(&mut self) -> Option<u64> {
        euterpe_bench_end(0);
        None
    }
}

struct Semihosting;

impl Sink for Semihosting {
    fn scenario(&mut self, device: &str, scenario: &str, blocks: u32, _: Option<u64>) {
        hprintln!("bench {} {} {}", device, scenario, blocks);
    }
}

#[entry]
fn main() -> ! {
    let (mut probe, mut sink) = (Markers, Semihosting);
    bench_overhead(&mut probe, &mut sink);
    bench_all(&mut probe, &mut sink);
    debug::exit(debug::EXIT_SUCCESS);
    // Unreachable: QEMU exits above.
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    hprintln!("panic: {}", info);
    debug::exit(debug::EXIT_FAILURE);
    // Unreachable: QEMU exits above.
    loop {
        core::hint::spin_loop();
    }
}
