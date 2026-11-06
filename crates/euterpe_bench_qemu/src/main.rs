//! Bench firmware for QEMU's `mps2-an500` machine (a Cortex-M7).
//!
//! Don't run this directly: `euterpe bench` builds it, finds the marker
//! functions below in its symbol table and runs it under QEMU with the
//! `tools/qemu/euterpe_insn.c` plugin, which counts instructions between
//! the markers. The firmware prints one `bench` line per scenario over
//! semihosting; the plugin prints one `euterpe-bench` line per scenario, in
//! the same order.
//!
//! Benches every device in `euterpe_registry`.

#![no_std]
#![no_main]

use core::hint::black_box;

use cortex_m_rt::entry;
use cortex_m_semihosting::{debug, hprintln};
use euterpe_bench::{Probe, Sink, bench_device, bench_overhead};
use euterpe_core::Device;

struct BenchAll<'a> {
    probe: &'a mut Markers,
    sink: &'a mut Semihosting,
}

impl euterpe_registry::Each for BenchAll<'_> {
    fn device<D: Device>(&mut self) {
        bench_device::<D>(self.probe, self.sink);
    }
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
    euterpe_registry::each(&mut BenchAll {
        probe: &mut probe,
        sink: &mut sink,
    });
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
