//! The Daisy Seed platform.
//!
//! [`Daisy::run`] brings the board up and runs a [`Device`]'s graph on the
//! codec, one 32-sample block per audio interrupt:
//!
//! - instruction and data caches on
//! - denormals flushed to zero, in thread mode and (through FPDSCR) in every
//!   interrupt handler, which is where audio runs
//! - the DWT cycle counter on, so every block's cost is measured
//! - mono: the left input runs through the graph and goes to both outputs
//!
//! Every block's cycle count goes into [`stats`]. With no debugger attached,
//! the user LED shows the result: a slow blink (1 Hz) while the worst block
//! is within the device's budget, a fast blink (5 Hz) once any block went
//! over it.
//!
//! With the `rtt` feature, the same numbers stream over RTT about once a
//! second (`probe-rs run` prints them): `just probe-run <device>`.
//!
//! Controls: the device's [`Device::PANEL`] is read about once a
//! millisecond from the Seed's pins (see [`controls`] for the wiring),
//! smoothed and debounced by [`euterpe_core::panel::Panel`], and the
//! parameters go to the audio interrupt in a short critical section (a copy
//! of the parameter struct) only when one changed. A panel that names a
//! missing parameter is a compile error.
//!
//! Memory: the graph is built in place, never on the stack, so the stack
//! (the rest of DTCM) only holds call frames. It goes in an 80 KB arena in
//! DTCM ([`GRAPH_ARENA_BYTES`]) if it fits, and otherwise in a 480 KB arena
//! in AXI SRAM ([`LARGE_GRAPH_ARENA_BYTES`]), behind the data cache: that is
//! where delay lines put a graph. A graph too big for both is a compile
//! error. Sample memory beyond that will go to SDRAM.

#![no_std]
#![cfg_attr(not(test), warn(clippy::default_numeric_fallback))]

pub mod controls;

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m::interrupt::Mutex;
use cortex_m::peripheral::DWT;
use daisy::audio;
use euterpe_core::device::{GRAPH_ARENA_BYTES, LARGE_GRAPH_ARENA_BYTES};
use euterpe_core::panel::{self, Panel, Readings};
use euterpe_core::place::{Arena, ArenaRef, Init, LinkedArena, Region};
use euterpe_core::{BLOCK, Block, Ctx, Device, ParamSet, Platform, Processor};
use stm32h7xx_hal::pac::{self, interrupt};

const _: () = assert!(
    audio::BLOCK_LENGTH == BLOCK,
    "the codec block must match euterpe_core::BLOCK"
);

/// Cycle accounting for the audio interrupt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Blocks processed so far.
    pub blocks: u32,
    /// Cycles of the most recent block.
    pub last: u32,
    /// Cycles of the slowest block so far.
    pub max: u32,
    /// The device's budget per block.
    pub budget: u32,
}

static BLOCKS: AtomicU32 = AtomicU32::new(0);
static LAST: AtomicU32 = AtomicU32::new(0);
static MAX: AtomicU32 = AtomicU32::new(0);
static BUDGET: AtomicU32 = AtomicU32::new(u32::MAX);

/// The audio interrupt's cycle accounting so far. Also readable from a
/// debugger: the statics are `euterpe_daisy::{BLOCKS, LAST, MAX, BUDGET}`.
pub fn stats() -> Stats {
    Stats {
        blocks: BLOCKS.load(Ordering::Relaxed),
        last: LAST.load(Ordering::Relaxed),
        max: MAX.load(Ordering::Relaxed),
        budget: BUDGET.load(Ordering::Relaxed),
    }
}

/// The running graph, type-erased so the interrupt handler (which can't be
/// generic) can call it: one indirect call per block, none per sample.
#[derive(Clone, Copy)]
struct Runner {
    state: *mut (),
    process: fn(*mut (), &mut audio::Block),
}

// SAFETY: single core. The pointer is only dereferenced by the audio
// interrupt, after `run` has set it up, and the pointee lives in the static
// arena for the rest of the program.
unsafe impl Send for Runner {}

static RUNNER: Mutex<RefCell<Option<Runner>>> = Mutex::new(RefCell::new(None));
static AUDIO: Mutex<RefCell<Option<audio::Interface>>> = Mutex::new(RefCell::new(None));

/// The graph and its parameters, as one value the runner points at.
struct Running<P: Processor> {
    graph: P,
    params: P::Params,
    buf: Block,
}

/// The graph's home if it fits: DTCM (`.bss` is DTCM on the Daisy),
/// zero-wait-state and outside the caches.
static GRAPH: Arena<GRAPH_ARENA_BYTES> = Arena::new();

/// The home of a graph too big for DTCM: AXI SRAM, behind the data cache.
/// `.sram` is `NOLOAD`, so the arena's flag lives in `.bss`.
#[unsafe(link_section = ".sram")]
static LARGE_MEM: Region<LARGE_GRAPH_ARENA_BYTES> = Region::new();
static LARGE_GRAPH: LinkedArena<LARGE_GRAPH_ARENA_BYTES> = LinkedArena::new(&LARGE_MEM);

/// Build the graph in DTCM if it fits, else in AXI SRAM, field by field so
/// that no part of it passes through the stack.
fn place_graph<P: Processor + Default>(init: Init) -> &'static mut Running<P> {
    const {
        assert!(
            ArenaRef::<LARGE_GRAPH_ARENA_BYTES>::fits::<Running<P>>(),
            "the graph is bigger than the Daisy's AXI SRAM arena"
        );
    };
    let build = |r: *mut Running<P>| {
        // SAFETY: `r` is aligned and valid for writes (the arena), and
        // every field is written.
        unsafe {
            init.build(&raw mut (*r).graph);
            (&raw mut (*r).params).write(P::Params::default());
            (&raw mut (*r).buf).write([0.0; BLOCK]);
        }
    };
    let (fast, large) = arenas();
    // SAFETY: `build` initializes the whole `Running`.
    unsafe { fast.try_place_forever_by(build) }.unwrap_or_else(|_| {
        // SAFETY: as above.
        unsafe { large.try_place_forever_by(build) }
            .ok()
            .expect("checked at compile time")
    })
}

fn process_block<P: Processor>(state: *mut (), frames: &mut audio::Block) {
    // SAFETY: `state` is the `Running<P>` that `Daisy::run::<D>` created for
    // this same `P`, and only this interrupt touches it.
    let running = unsafe { &mut *state.cast::<Running<P>>() };
    for (s, frame) in running.buf.iter_mut().zip(frames.iter()) {
        *s = frame.0;
    }
    running.graph.process(&mut running.buf, &running.params);
    for (frame, &s) in frames.iter_mut().zip(running.buf.iter()) {
        *frame = (s, s);
    }
}

/// The Daisy Seed (rev 1.2 codec).
pub struct Daisy;

impl Platform for Daisy {
    fn run<D: Device>(self) -> ! {
        const {
            assert!(
                panel::check(
                    D::PANEL,
                    <<D::Graph as Processor>::Params as ParamSet>::SCHEMA
                )
                .is_ok(),
                "the device's PANEL binds a parameter its schema doesn't have, or the wrong kind"
            );
        };
        let mut cp = cortex_m::Peripherals::take().expect("run is called once");
        let dp = pac::Peripherals::take().expect("run is called once");

        init_core(&mut cp);
        #[cfg(feature = "rtt")]
        rtt_target::rtt_init_print!();

        let board = daisy::Board::take().expect("run is called once");
        let ccdr = daisy::board_freeze_clocks!(board, dp);
        let pins = daisy::board_split_gpios!(board, ccdr, dp);
        let mut led = daisy::board_split_leds!(pins).USER;
        let audio_interface = daisy::board_split_audio!(ccdr, pins);

        let mut delay = stm32h7xx_hal::delay::Delay::new(cp.SYST, ccdr.clocks);
        let adc = stm32h7xx_hal::adc::Adc::adc1(
            dp.ADC1,
            stm32h7xx_hal::time::Hertz::MHz(4),
            &mut delay,
            ccdr.peripheral.ADC12,
            &ccdr.clocks,
        )
        .enable();
        let mut controls = controls::Controls::new(adc, pins.GPIO, controls::bound_pots(D::PANEL));

        #[expect(clippy::cast_precision_loss, reason = "48 kHz is exact in f32")]
        let ctx = Ctx::new(audio::FS.to_Hz() as f32);
        let running = place_graph::<D::Graph>(D::GRAPH_INIT);

        // The panel as it is set at power-up, before the first block.
        let mut panel = Panel::default();
        let mut readings = Readings::default();
        let mut params = <D::Graph as Processor>::Params::default();
        controls.read(&mut readings);
        panel.update(D::PANEL, &readings, &mut params);
        running.params = params.clone();

        running.graph.prepare(&ctx);
        running.graph.reset();
        BUDGET.store(D::BUDGET_CYCLES_PER_BLOCK, Ordering::Relaxed);

        // From here the interrupt owns `running`; this loop only writes
        // the parameters, through this pointer, with interrupts off.
        let shared_params: *mut <D::Graph as Processor>::Params = &raw mut running.params;
        cortex_m::interrupt::free(|cs| {
            RUNNER.borrow(cs).replace(Some(Runner {
                state: (&raw mut *running).cast(),
                process: process_block::<D::Graph>,
            }));
            let ai = audio_interface.spawn().expect("audio interface starts");
            AUDIO.borrow(cs).replace(Some(ai));
        });

        let sys_hz = ccdr.clocks.sys_ck().to_Hz();
        #[cfg(feature = "rtt")]
        rtt_target::rprintln!(
            "euterpe {} sys_hz={} budget={}",
            D::NAME,
            sys_hz,
            D::BUDGET_CYCLES_PER_BLOCK
        );

        // The control loop: one scan per millisecond, timed by DWT.
        let tick = sys_hz / 1000;
        let mut next = DWT::cycle_count();
        let mut ticks = 0_u32;
        loop {
            next = next.wrapping_add(tick);
            while next.wrapping_sub(DWT::cycle_count()) <= tick {}
            ticks = ticks.wrapping_add(1);

            controls.read(&mut readings);
            if panel.update(D::PANEL, &readings, &mut params) {
                cortex_m::interrupt::free(|_| {
                    // SAFETY: `shared_params` points into the graph's arena,
                    // which lives forever; the only other user is the audio
                    // interrupt, which can't run inside this critical section.
                    unsafe { *shared_params = params.clone() };
                });
            }

            // Blink: 1 Hz while within budget, 5 Hz once over it.
            let s = stats();
            let half_period_ms = if s.max > s.budget { 100 } else { 500 };
            if ticks.is_multiple_of(half_period_ms) {
                led.toggle();
            }
            // About once a second: the cycle accounting, over RTT.
            #[cfg(feature = "rtt")]
            if ticks.is_multiple_of(1000) {
                report(&s);
            }
        }
    }
}

/// One line of cycle accounting over RTT: `stats blocks=.. last=.. max=..
/// budget=.. max_pct=..`.
#[cfg(feature = "rtt")]
fn report(s: &Stats) {
    let pct = u64::from(s.max) * 100 / u64::from(s.budget.max(1));
    rtt_target::rprintln!(
        "stats blocks={} last={} max={} budget={} max_pct={}",
        s.blocks,
        s.last,
        s.max,
        s.budget,
        pct
    );
}

/// Core setup shared by every Daisy firmware: instruction and data caches
/// on, denormals flushed to zero (thread mode and every handler), and the
/// DWT cycle counter running.
pub fn init_core(cp: &mut cortex_m::Peripherals) {
    cp.SCB.enable_icache();
    cp.SCB.enable_dcache(&mut cp.CPUID);
    flush_denormals(&mut cp.FPU);
    cp.DCB.enable_trace();
    DWT::unlock();
    cp.DWT.enable_cycle_counter();
}

/// Both graph arenas: DTCM, then AXI SRAM. The first call only, so a
/// firmware that benches devices can use the same memory `run` would.
///
/// # Panics
///
/// If called twice, or after [`Daisy::run`].
#[must_use]
pub fn arenas() -> (
    ArenaRef<GRAPH_ARENA_BYTES>,
    ArenaRef<LARGE_GRAPH_ARENA_BYTES>,
) {
    (
        GRAPH.take().expect("the arenas are taken once"),
        LARGE_GRAPH.take().expect("the arenas are taken once"),
    )
}

/// Set FZ (flush-to-zero) in FPSCR for thread mode, and in FPDSCR, the value
/// FPSCR takes on entry to every exception handler.
fn flush_denormals(fpu: &mut cortex_m::peripheral::FPU) {
    const FZ: u32 = 1 << 24;
    // SAFETY: FZ only changes how denormal results are rounded.
    unsafe {
        fpu.fpdscr.modify(|v| v | FZ);
        let fpscr = cortex_m::register::fpscr::read();
        cortex_m::register::fpscr::write(cortex_m::register::fpscr::Fpscr::from_bits(
            fpscr.bits() | FZ,
        ));
    }
}

#[interrupt]
fn DMA1_STR1() {
    cortex_m::interrupt::free(|cs| {
        let runner = *RUNNER.borrow(cs).borrow();
        let mut audio = AUDIO.borrow(cs).borrow_mut();
        let (Some(runner), Some(audio)) = (runner, audio.as_mut()) else {
            return;
        };
        // An error here means the DMA state was unexpected; drop the block
        // rather than stop the audio.
        let _ = audio.handle_interrupt_dma1_str1(|frames| {
            let start = DWT::cycle_count();
            (runner.process)(runner.state, frames);
            let cycles = DWT::cycle_count().wrapping_sub(start);
            LAST.store(cycles, Ordering::Relaxed);
            MAX.fetch_max(cycles, Ordering::Relaxed);
            BLOCKS.fetch_add(1, Ordering::Relaxed);
        });
    });
}
