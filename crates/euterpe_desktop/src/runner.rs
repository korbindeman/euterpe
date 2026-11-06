//! A device's graph behind an object-safe trait, and the adapter from the
//! audio interface's buffers to 32-sample blocks.

use std::sync::Arc;

use euterpe_core::{BLOCK, Block, Ctx, Device, ParamDesc, Processor};
use euterpe_registry::Visitor;

use crate::params::ParamBank;

/// A running device. One virtual call per block; the graph inside is fully
/// monomorphized, as on the Daisy.
pub trait Runner: Send {
    fn process(&mut self, block: &mut Block);
    fn latency(&self) -> usize;
}

struct DeviceRunner<P: Processor> {
    graph: P,
    params: P::Params,
    bank: Arc<ParamBank>,
}

impl<P> Runner for DeviceRunner<P>
where
    P: Processor<Params: Send> + Send,
{
    fn process(&mut self, block: &mut Block) {
        self.bank.apply(&mut self.params);
        self.graph.process(block, &self.params);
    }

    fn latency(&self) -> usize {
        self.graph.latency()
    }
}

/// A freshly built device: its runner, its parameter bank, and identity.
pub struct Built {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: &'static [ParamDesc],
    pub budget_cycles_per_block: u32,
    pub bank: Arc<ParamBank>,
    pub runner: Box<dyn Runner>,
}

/// Builds the device it visits, prepared for `sample_rate`.
pub struct Build {
    pub sample_rate: f32,
}

impl Visitor for Build {
    type Out = Built;

    fn visit<D: Device>(self) -> Built {
        let schema = <<D::Graph as Processor>::Params as euterpe_core::ParamSet>::SCHEMA;
        let bank = Arc::new(ParamBank::new(schema));
        let mut graph = D::Graph::default();
        graph.prepare(&Ctx::new(self.sample_rate));
        graph.reset();
        Built {
            name: D::NAME,
            description: D::DESCRIPTION,
            schema,
            budget_cycles_per_block: D::BUDGET_CYCLES_PER_BLOCK,
            bank: Arc::clone(&bank),
            runner: Box::new(DeviceRunner {
                graph,
                params: Default::default(),
                bank,
            }),
        }
    }
}

/// Turns any number of samples into 32-sample blocks, with one block of
/// latency: each input sample comes out [`BLOCK`] samples later.
pub struct BlockAdapter {
    input: Block,
    output: Block,
    pos: usize,
}

impl Default for BlockAdapter {
    fn default() -> Self {
        Self {
            input: [0.0; BLOCK],
            output: [0.0; BLOCK],
            pos: 0,
        }
    }
}

impl BlockAdapter {
    /// Push one input sample, get one output sample. Every [`BLOCK`] samples,
    /// `process` runs on the collected block.
    #[inline]
    pub fn tick(&mut self, x: f32, process: impl FnOnce(&mut Block)) -> f32 {
        let y = self.output[self.pos];
        self.input[self.pos] = x;
        self.pos += 1;
        if self.pos == BLOCK {
            self.pos = 0;
            process(&mut self.input);
            self.output = self.input;
        }
        y
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact sample values through a pure delay")]
mod tests {
    use super::*;

    #[test]
    fn adapter_delays_by_one_block_for_any_buffer_size() {
        let mut a = BlockAdapter::default();
        let input: Vec<f32> = (1..=200).map(|i| i as f32).collect();
        let mut out = Vec::new();
        // Odd chunk sizes, as an interface might deliver.
        for chunk in input.chunks(7) {
            for &x in chunk {
                out.push(a.tick(x, |b| b.iter_mut().for_each(|s| *s *= 2.0)));
            }
        }
        for (i, &y) in out.iter().enumerate() {
            let want = if i < BLOCK {
                0.0
            } else {
                2.0 * input[i - BLOCK]
            };
            assert_eq!(y, want, "sample {i}");
        }
    }

    #[test]
    fn builds_every_registered_device() {
        for (name, _) in euterpe_registry::DEVICES {
            let built = euterpe_registry::visit(
                name,
                Build {
                    sample_rate: 48_000.0,
                },
            )
            .unwrap();
            assert_eq!(built.name, *name);
            assert_eq!(built.bank.schema().len(), built.schema.len());
        }
    }
}
