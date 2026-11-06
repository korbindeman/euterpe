//! Bench firmware for the Daisy Seed.
//!
//! Runs the same scenarios as `euterpe bench` on QEMU, on the real board
//! (480 MHz, caches on, denormals flushed), measured with the DWT cycle
//! counter. Each device's graph is placed where `euterpe_daisy` would put
//! it: DTCM if it fits, else AXI SRAM. Results go over RTT, one line per
//! scenario:
//!
//! ```text
//! board <device> <scenario> <blocks> <cycles>
//! ```
//!
//! The first line, `board overhead empty`, is the cost of the probe itself.
//! When every device is done it prints `board done` and exits through
//! semihosting, so `probe-rs run` returns. `just board-bench` runs it and
//! hands the log to `euterpe bench --board-log`, which puts cycles next to
//! QEMU's instruction counts.
//!
//! Needs a debug probe: semihosting without one halts the core.

#![no_std]
#![no_main]

use cortex_m::peripheral::DWT;
use cortex_m_rt::entry;
use euterpe_bench::{Probe, Sink, bench_device, bench_overhead};
use euterpe_core::Device;
use euterpe_core::device::{GRAPH_ARENA_BYTES, LARGE_GRAPH_ARENA_BYTES};
use euterpe_core::place::ArenaRef;
use rtt_target::{rprintln, rtt_init_print};
use stm32h7xx_hal::pac;

/// Cycles between `start` and `stop`, summed over a scenario.
#[derive(Default)]
struct Cycles {
    started: u32,
    total: u64,
}

impl Probe for Cycles {
    #[inline(always)]
    fn start(&mut self) {
        self.started = DWT::cycle_count();
    }
    #[inline(always)]
    fn stop(&mut self) {
        let now = DWT::cycle_count();
        self.total += u64::from(now.wrapping_sub(self.started));
    }
    fn end(&mut self) -> Option<u64> {
        Some(core::mem::take(&mut self.total))
    }
}

struct Rtt;

impl Sink for Rtt {
    fn scenario(&mut self, device: &str, scenario: &str, blocks: u32, total: Option<u64>) {
        rprintln!(
            "board {} {} {} {}",
            device,
            scenario,
            blocks,
            total.unwrap_or(0)
        );
    }
}

struct BenchAll {
    fast: ArenaRef<GRAPH_ARENA_BYTES>,
    large: ArenaRef<LARGE_GRAPH_ARENA_BYTES>,
    probe: Cycles,
    sink: Rtt,
}

impl euterpe_registry::Each for BenchAll {
    fn device<D: Device>(&mut self) {
        let (fast, large) = (&mut self.fast, &mut self.large);
        let ran = bench_device::<D, GRAPH_ARENA_BYTES>(fast, &mut self.probe, &mut self.sink)
            || bench_device::<D, LARGE_GRAPH_ARENA_BYTES>(large, &mut self.probe, &mut self.sink);
        if !ran {
            rprintln!("board-skip {} too big for both arenas", D::NAME);
        }
    }
}

#[entry]
fn main() -> ! {
    rtt_init_print!();
    let mut cp = cortex_m::Peripherals::take().expect("main runs once");
    let dp = pac::Peripherals::take().expect("main runs once");
    euterpe_daisy::init_core(&mut cp);
    let board = daisy::Board::take().expect("main runs once");
    let ccdr = daisy::board_freeze_clocks!(board, dp);
    rprintln!("board start sys_hz={}", ccdr.clocks.sys_ck().to_Hz());

    let (fast, large) = euterpe_daisy::arenas();
    let mut all = BenchAll {
        fast,
        large,
        probe: Cycles::default(),
        sink: Rtt,
    };
    bench_overhead(&mut all.probe, &mut all.sink);
    euterpe_registry::each(&mut all);
    rprintln!("board done");
    cortex_m_semihosting::debug::exit(cortex_m_semihosting::debug::EXIT_SUCCESS);
    loop {
        cortex_m::asm::wfi();
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    rprintln!("board panic: {}", info);
    cortex_m_semihosting::debug::exit(cortex_m_semihosting::debug::EXIT_FAILURE);
    loop {
        cortex_m::asm::wfi();
    }
}
