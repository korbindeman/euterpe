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
//! Parameters stay at their defaults for now; panel bindings come with the
//! `Controls` part of the platform.
//!
//! Memory: the graph is built in place in an 80 KB arena in DTCM
//! ([`GRAPH_ARENA_BYTES`]), never on the stack, so the stack (the rest of
//! DTCM) only holds call frames. A graph that doesn't fit is a compile error.
//! Delay lines and sample memory beyond that will go to SDRAM, per the
//! spec's performance rules.

#![no_std]
#![cfg_attr(not(test), warn(clippy::default_numeric_fallback))]

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m::interrupt::Mutex;
use cortex_m::peripheral::DWT;
use daisy::audio;
use euterpe_core::device::GRAPH_ARENA_BYTES;
use euterpe_core::place::Arena;
use euterpe_core::{BLOCK, Block, Ctx, Device, Platform, Processor};
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

impl<P: Processor + Default> Default for Running<P> {
    fn default() -> Self {
        Self {
            graph: P::default(),
            params: P::Params::default(),
            buf: [0.0; BLOCK],
        }
    }
}

/// The graph's home: DTCM (`.bss` is DTCM on the Daisy), zero-wait-state
/// and outside the caches.
static GRAPH: Arena<GRAPH_ARENA_BYTES> = Arena::new();

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
        let mut cp = cortex_m::Peripherals::take().expect("run is called once");
        let dp = pac::Peripherals::take().expect("run is called once");

        cp.SCB.enable_icache();
        cp.SCB.enable_dcache(&mut cp.CPUID);
        flush_denormals(&mut cp.FPU);
        cp.DCB.enable_trace();
        DWT::unlock();
        cp.DWT.enable_cycle_counter();

        let board = daisy::Board::take().expect("run is called once");
        let ccdr = daisy::board_freeze_clocks!(board, dp);
        let pins = daisy::board_split_gpios!(board, ccdr, dp);
        let mut led = daisy::board_split_leds!(pins).USER;
        let audio_interface = daisy::board_split_audio!(ccdr, pins);

        #[expect(clippy::cast_precision_loss, reason = "48 kHz is exact in f32")]
        let ctx = Ctx::new(audio::FS.to_Hz() as f32);
        let running: &'static mut Running<D::Graph> =
            GRAPH.take().expect("run is called once").place_forever();
        running.graph.prepare(&ctx);
        running.graph.reset();
        BUDGET.store(D::BUDGET_CYCLES_PER_BLOCK, Ordering::Relaxed);

        cortex_m::interrupt::free(|cs| {
            RUNNER.borrow(cs).replace(Some(Runner {
                state: (&raw mut *running).cast(),
                process: process_block::<D::Graph>,
            }));
            let ai = audio_interface.spawn().expect("audio interface starts");
            AUDIO.borrow(cs).replace(Some(ai));
        });

        // Blink: 1 Hz while within budget, 5 Hz once over it.
        let sys_hz = ccdr.clocks.sys_ck().to_Hz();
        loop {
            let s = stats();
            let half_period = if s.max > s.budget {
                sys_hz / 10
            } else {
                sys_hz / 2
            };
            led.toggle();
            // `running` is in the arena for good, and only the interrupt
            // touches it from here on.
            cortex_m::asm::delay(half_period);
        }
    }
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
