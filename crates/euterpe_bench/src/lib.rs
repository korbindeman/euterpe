//! Benchmark harness for Euterpe devices.
//!
//! The harness is the same on every target. A [`Probe`] says how to measure
//! (instruction counts under QEMU, DWT cycles on the Daisy) and a [`Sink`]
//! says where results go. Each scenario runs [`WARMUP_BLOCKS`] unmeasured
//! blocks, then [`BLOCKS`] measured ones. Only the `process` call is inside
//! the measured region: input generation and parameter changes are outside.
//!
//! Scenarios:
//! - `static`: default parameters, so smoothers are settled.
//! - `automated`: every parameter jumps between its extremes every block, so
//!   smoothers are always ramping. This is the worst case for most processors.

#![no_std]
#![cfg_attr(not(test), warn(clippy::default_numeric_fallback))]

use core::hint::black_box;

use euterpe_core::place::{ArenaRef, Init};
use euterpe_core::{BLOCK, Block, Ctx, Device, ParamSet, Processor};

/// Measured blocks per scenario.
pub const BLOCKS: u32 = 64;
/// Unmeasured blocks before measuring, so caches and branch state settle.
pub const WARMUP_BLOCKS: u32 = 8;
/// Sample rate the benches run at.
pub const SAMPLE_RATE: f32 = 48_000.0;

/// Brackets the measured regions of one scenario.
pub trait Probe {
    /// Start a measured region.
    fn start(&mut self);
    /// End a measured region.
    fn stop(&mut self);
    /// End the scenario. Returns the total over all regions, if this probe
    /// can read it on the target (DWT can; under QEMU the plugin reports it).
    fn end(&mut self) -> Option<u64>;
}

/// Receives results.
pub trait Sink {
    fn scenario(&mut self, device: &str, scenario: &str, blocks: u32, total: Option<u64>);
}

/// A deterministic noise source at about -6 dBFS (xorshift32).
struct Noise(u32);

impl Noise {
    fn fill(&mut self, buf: &mut Block) {
        for s in buf.iter_mut() {
            let mut x = self.0;
            x ^= x << 13_u32;
            x ^= x >> 17_u32;
            x ^= x << 5_u32;
            self.0 = x;
            // Top 24 bits to [-0.5, 0.5).
            *s = (x >> 8_u32) as f32 / 16_777_216.0 - 0.5;
        }
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    Static,
    Automated,
}

impl Scenario {
    const ALL: [Scenario; 2] = [Scenario::Static, Scenario::Automated];

    fn name(self) -> &'static str {
        match self {
            Scenario::Static => "static",
            Scenario::Automated => "automated",
        }
    }
}

/// Measure the probe's own overhead: an empty region, [`BLOCKS`] times.
/// Reported as device `"overhead"`, scenario `"empty"`, so results can be
/// corrected for it.
pub fn bench_overhead(probe: &mut impl Probe, sink: &mut impl Sink) {
    for _ in 0..BLOCKS {
        probe.start();
        probe.stop();
    }
    let total = probe.end();
    sink.scenario("overhead", "empty", BLOCKS, total);
}

/// Run every scenario for device `D`, building its graph in `arena` the
/// way the embedded platforms do.
pub fn bench_device<D: Device, const N: usize>(
    arena: &mut ArenaRef<N>,
    probe: &mut impl Probe,
    sink: &mut impl Sink,
) {
    for scenario in Scenario::ALL {
        let total = run::<D::Graph, N>(arena, D::GRAPH_INIT, scenario, probe);
        sink.scenario(D::NAME, scenario.name(), BLOCKS, total);
    }
}

/// The measured call, kept out of line.
///
/// `euterpe bench` walks the call graph from this symbol
/// (`<euterpe_bench::Hot<Graph>>::process`). Without it the optimizer may
/// inline the graph's `process` into the bench loop, and there would be no
/// function left to check.
///
/// `repr(transparent)`, so a `Hot<P>` built by zeroing is a zeroed `P`.
#[derive(Default)]
#[repr(transparent)]
pub struct Hot<P>(pub P);

impl<P: Processor> Hot<P> {
    #[inline(never)]
    pub fn process(&mut self, buf: &mut Block, params: &P::Params) {
        self.0.process(buf, params);
    }
}

fn run<P: Processor + Default, const N: usize>(
    arena: &mut ArenaRef<N>,
    init: Init,
    scenario: Scenario,
    probe: &mut impl Probe,
) -> Option<u64> {
    let mut processor = arena.place_with::<Hot<P>>(init);
    processor.0.prepare(&Ctx::new(SAMPLE_RATE));
    processor.0.reset();
    let mut params = P::Params::default();
    let mut noise = Noise(0x1234_5678);
    let mut buf: Block = [0.0; BLOCK];

    for i in 0..WARMUP_BLOCKS + BLOCKS {
        if let Scenario::Automated = scenario {
            let n = if i % 2 == 0 { 0.0 } else { 1.0 };
            for desc in P::Params::SCHEMA {
                // Every id comes from the schema, so this cannot fail.
                let _ = params.set_normalized(desc.id, n);
            }
        }
        noise.fill(&mut buf);
        // Keep the work inside the region: the buffer escapes on both sides.
        black_box(&mut buf);
        let measured = i >= WARMUP_BLOCKS;
        if measured {
            probe.start();
        }
        processor.process(&mut buf, black_box(&params));
        if measured {
            probe.stop();
        }
        black_box(&mut buf);
    }
    probe.end()
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "checks the noise is not one repeated value"
)]
mod tests {
    extern crate std;
    use std::{string::String, vec::Vec};

    use super::*;

    #[derive(Default)]
    struct Counting {
        starts: u32,
        stops: u32,
    }
    impl Probe for Counting {
        fn start(&mut self) {
            self.starts += 1;
        }
        fn stop(&mut self) {
            self.stops += 1;
        }
        fn end(&mut self) -> Option<u64> {
            Some(u64::from(core::mem::take(&mut self.stops)))
        }
    }

    #[derive(Default)]
    struct Collect(Vec<(String, String, u32, Option<u64>)>);
    impl Sink for Collect {
        fn scenario(&mut self, device: &str, scenario: &str, blocks: u32, total: Option<u64>) {
            self.0.push((device.into(), scenario.into(), blocks, total));
        }
    }

    struct Dev;
    impl Device for Dev {
        const NAME: &'static str = "test.dev";
        const DESCRIPTION: &'static str = "";
        type Graph = Nop;
    }
    #[derive(Default)]
    struct Nop;
    impl Processor for Nop {
        type Params = ();
        fn prepare(&mut self, _: &Ctx) {}
        fn reset(&mut self) {}
        fn process(&mut self, _: &mut Block, (): &()) {}
    }

    #[test]
    fn every_scenario_measures_exactly_blocks_regions() {
        static ARENA: euterpe_core::place::Arena<64> = euterpe_core::place::Arena::new();
        let mut probe = Counting::default();
        let mut sink = Collect::default();
        bench_device::<Dev, 64>(&mut ARENA.take().unwrap(), &mut probe, &mut sink);
        let got: Vec<_> = sink.0.iter().map(|r| (r.1.as_str(), r.3)).collect();
        let blocks = Some(u64::from(BLOCKS));
        assert_eq!(got, [("static", blocks), ("automated", blocks)]);
        assert_eq!(probe.starts, 2 * BLOCKS);
    }

    #[test]
    fn noise_is_bounded_and_not_constant() {
        let mut n = Noise(0x1234_5678);
        let mut buf = [0.0; BLOCK];
        n.fill(&mut buf);
        assert!(buf.iter().all(|s| (-0.5..0.5).contains(s)));
        assert!(buf.iter().any(|&s| s != buf[0]));
    }
}
