//! Platforms: where a [`Device`] runs.

use crate::device::Device;

/// Something that can run a device: the Daisy, the desktop, the offline
/// renderer. A device crate doesn't change between platforms.
///
/// This is the minimal form. Panel controls and asset storage will be added
/// as associated types (see the architecture spec) when the first platform
/// needs them.
pub trait Platform {
    /// Bring the platform up and run `D` forever.
    fn run<D: Device>(self) -> !;
}
