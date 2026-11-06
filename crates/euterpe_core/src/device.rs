//! Devices: complete, named signal graphs that a platform can run.

use crate::processor::Processor;

/// A product definition: a named, complete signal graph.
///
/// The same device runs on every platform (Daisy, desktop, offline). Panel
/// bindings and cycle budgets will be added here as those platforms land.
pub trait Device {
    /// Stable machine name, e.g. `"demo.drive"`. Used by the CLI.
    const NAME: &'static str;
    /// One line for humans and agents.
    const DESCRIPTION: &'static str;

    /// The signal graph.
    type Graph: Processor + Default;
}
