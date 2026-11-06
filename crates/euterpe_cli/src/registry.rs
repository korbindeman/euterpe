//! The devices the CLI knows about.
//!
//! To add a device: add its crate as a dependency and list its `Device` type
//! in `devices!` below.

use euterpe_core::Device;

/// Something to do with a device, generic over its type.
pub trait Visitor {
    type Out;
    fn visit<D: Device>(self) -> Self::Out;
}

macro_rules! devices {
    ($($device:ty),* $(,)?) => {
        /// `(name, description)` of every registered device.
        pub const DEVICES: &[(&str, &str)] = &[
            $( (<$device as Device>::NAME, <$device as Device>::DESCRIPTION), )*
        ];

        /// Run `visitor` with the device called `name`.
        pub fn visit<V: Visitor>(name: &str, visitor: V) -> Option<V::Out> {
            $(
                if name == <$device as Device>::NAME {
                    return Some(visitor.visit::<$device>());
                }
            )*
            None
        }
    };
}

devices! {
    euterpe_demo::GainDevice,
    euterpe_demo::DriveDevice,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        for (i, (a, _)) in DEVICES.iter().enumerate() {
            assert!(
                DEVICES[i + 1..].iter().all(|(b, _)| a != b),
                "duplicate device {a}"
            );
        }
    }
}
