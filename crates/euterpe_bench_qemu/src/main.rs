//! Bench firmware for QEMU's `mps2-an500` machine (a Cortex-M7).
//!
//! Don't run this directly: `euterpe bench` builds it, finds the marker
//! functions below in its symbol table and runs it under QEMU with the
//! `tools/qemu/euterpe_insn.c` plugin, which counts instructions between
//! the markers. The firmware prints one `bench` line per scenario over
//! semihosting; the plugin prints one `euterpe-bench` line per scenario, in
//! the same order.
//!
//! Benches every device in `euterpe_registry`, and measures each one's peak
//! stack use (building the graph included) by painting the free stack
//! before it runs and finding the deepest overwritten word after. That line
//! is `stack <device> <bytes>`.

#![no_std]
#![no_main]

use core::hint::black_box;

use cortex_m_rt::entry;
use cortex_m_semihosting::{debug, hprintln};
use euterpe_bench::{Probe, Sink, bench_device, bench_overhead};
use euterpe_core::Device;
use euterpe_core::device::GRAPH_ARENA_BYTES;
use euterpe_core::place::{Arena, ArenaRef};

struct BenchAll<'a> {
    arena: &'a mut ArenaRef<GRAPH_ARENA_BYTES>,
    probe: &'a mut Markers,
    sink: &'a mut Semihosting,
}

/// The graph's home, as on the Daisy.
static GRAPH: Arena<GRAPH_ARENA_BYTES> = Arena::new();

impl euterpe_registry::Each for BenchAll<'_> {
    fn device<D: Device>(&mut self) {
        let painted = Stack::paint(cortex_m::register::msp::read() as usize);
        bench_out_of_line::<D>(self.arena, self.probe, self.sink);
        hprintln!("stack {} {}", D::NAME, painted.used());
        hprintln!("graph {} {}", D::NAME, size_of::<D::Graph>());
    }
}

/// Keeps everything the bench puts on the stack (the graph included) in
/// frames below the measured stack pointer.
#[inline(never)]
fn bench_out_of_line<D: Device>(
    arena: &mut ArenaRef<GRAPH_ARENA_BYTES>,
    probe: &mut Markers,
    sink: &mut Semihosting,
) {
    bench_device::<D, GRAPH_ARENA_BYTES>(arena, probe, sink);
}

/// Stack high-water measurement by painting.
struct Stack {
    /// Lowest painted address.
    low: usize,
    /// Top of the painted range.
    painted_top: usize,
    /// The caller's stack pointer: usage is measured from here.
    sp: usize,
}

impl Stack {
    const PATTERN: u32 = 0x5AA5_C33C;
    /// How far below the stack pointer to paint. The Daisy's whole DTCM is
    /// 128 KB, so anything deeper is already a failure.
    const DEPTH: usize = 512 * 1024;
    /// Left unpainted below `sp`, for `paint`'s own frame. Usage is
    /// reported as at least this much.
    const MARGIN: usize = 256;

    #[inline(never)]
    fn paint(sp: usize) -> Self {
        unsafe extern "C" {
            static _stack_end: u32;
        }
        let floor = (&raw const _stack_end) as usize;
        let painted_top = (sp - Self::MARGIN) & !3;
        let low = painted_top.saturating_sub(Self::DEPTH).max(floor);
        let mut a = low;
        while a < painted_top {
            // SAFETY: [low, painted_top) is free stack below this frame,
            // inside the stack region the linker reserved.
            unsafe { (a as *mut u32).write_volatile(Self::PATTERN) };
            a += 4;
        }
        Self {
            low,
            painted_top,
            sp,
        }
    }

    /// Peak bytes used below `sp` since `paint` (at least `MARGIN`).
    fn used(&self) -> usize {
        let mut a = self.low;
        // SAFETY: the same painted range, read back.
        while a < self.painted_top && unsafe { (a as *const u32).read_volatile() } == Self::PATTERN
        {
            a += 4;
        }
        self.sp - a
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
    let mut arena = GRAPH.take().expect("main runs once");
    euterpe_registry::each(&mut BenchAll {
        arena: &mut arena,
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
