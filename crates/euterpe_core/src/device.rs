//! Devices: complete, named signal graphs that a platform can run.

use crate::processor::Processor;

/// Bytes the embedded platforms reserve for a device's graph: an
/// [`Arena`](crate::place::Arena) in the Daisy's DTCM, next to the stack.
/// A graph larger than this fails to build for the Daisy, and `euterpe
/// bench` reports it. 80 KB leaves about 48 KB of DTCM for the stack and
/// the other statics.
pub const GRAPH_ARENA_BYTES: usize = 80 * 1024;

/// A product definition: a named, complete signal graph.
///
/// The same device runs on every platform (Daisy, desktop, offline). Panel
/// bindings will be added here as those platforms land.
pub trait Device {
    /// Stable machine name, e.g. `"demo.drive"`. Used by the CLI.
    const NAME: &'static str;
    /// One line for humans and agents.
    const DESCRIPTION: &'static str;

    /// Cycle budget per [`BLOCK`](crate::BLOCK) on the Daisy Seed.
    ///
    /// A block at 48 kHz is 0.67 ms, which is 320k cycles at 480 MHz. The
    /// default leaves about 30% for audio I/O, controls and interrupts.
    /// `euterpe bench` reports every device against its budget.
    const BUDGET_CYCLES_PER_BLOCK: u32 = 200_000;

    /// The signal graph. Its `Default` should be cheap (zeroed buffers), so
    /// it can be built in place; load assets in `prepare`. See
    /// [`place`](crate::place).
    ///
    /// `Send + 'static` so a host can move it to its audio thread;
    /// graphs are plain data, so this holds unless a processor keeps a raw
    /// pointer.
    type Graph: Processor<Params: Send + 'static> + Default + Send + 'static;
}
