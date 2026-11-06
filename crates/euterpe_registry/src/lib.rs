//! The devices every Euterpe tool knows about: the CLI, the desktop app and
//! the bench firmware all read this one list.
//!
//! To add a device: add its crate as a dependency and list its `Device` type
//! in `devices!` below.

#![no_std]

use euterpe_core::Device;

/// Something to do with one device, generic over its type.
pub trait Visitor {
    type Out;
    fn visit<D: Device>(self) -> Self::Out;
}

/// Something to do with every device in turn.
pub trait Each {
    fn device<D: Device>(&mut self);
}

macro_rules! devices {
    ($($device:ty),* $(,)?) => {
        /// `(name, description)` of every registered device.
        pub const DEVICES: &[(&str, &str)] = &[
            $( (<$device as Device>::NAME, <$device as Device>::DESCRIPTION), )*
        ];

        /// Whether a device called `name` is registered. `const`, so
        /// firmware that picks its device at build time can fail the build
        /// on a typo.
        pub const fn contains(name: &str) -> bool {
            index_of(name).is_some()
        }

        /// The position of the device called `name` in [`DEVICES`].
        pub const fn index_of(name: &str) -> Option<usize> {
            let mut i = 0;
            while i < DEVICES.len() {
                let d = DEVICES[i].0.as_bytes();
                let n = name.as_bytes();
                if d.len() == n.len() {
                    let mut k = 0;
                    while k < d.len() && d[k] == n[k] {
                        k += 1;
                    }
                    if k == d.len() {
                        return Some(i);
                    }
                }
                i += 1;
            }
            None
        }

        /// Run `visitor` with device number `I` (see [`index_of`]). Which
        /// device is settled at compile time, so firmware that picks one
        /// links only that one; [`visit`] compares names at run time and
        /// may link them all.
        pub fn visit_at<const I: usize, V: Visitor>(visitor: V) -> Option<V::Out> {
            $(
                if I == const {
                    match index_of(<$device as Device>::NAME) {
                        Some(k) => k,
                        None => usize::MAX,
                    }
                } {
                    return Some(visitor.visit::<$device>());
                }
            )*
            None
        }

        /// Run `visitor` with the device called `name`.
        pub fn visit<V: Visitor>(name: &str, visitor: V) -> Option<V::Out> {
            $(
                if name == <$device as Device>::NAME {
                    return Some(visitor.visit::<$device>());
                }
            )*
            None
        }

        /// Run `each` for every device, in registration order.
        pub fn each(each: &mut impl Each) {
            $( each.device::<$device>(); )*
        }
    };
}

devices! {
    euterpe_demo::GainDevice,
    euterpe_demo::DriveDevice,
    euterpe_demo::Drive4xDevice,
    euterpe_demo::CabDevice,
    euterpe_demo::RigDevice,
    euterpe_pedals::FuzzDevice,
    euterpe_pedals::TremoloDevice,
    euterpe_pedals::DelayDevice,
    euterpe_pedals::EqDevice,
    euterpe_preamp::PreampDevice,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_matches_names_exactly() {
        assert!(contains("demo.rig"));
        assert!(contains("demo.drive"));
        assert!(!contains("demo.driv"));
        assert!(!contains("demo.drive4"));
        assert!(!contains(""));
        assert_eq!(index_of("demo.gain"), Some(0));
        assert_eq!(index_of(DEVICES[3].0), Some(3));
    }

    /// Every device's graph, built the way the embedded platforms build it,
    /// behaves like its `Default`.
    #[test]
    fn every_graph_init_is_sound() {
        struct Check;
        impl Each for Check {
            fn device<D: Device>(&mut self) {
                let opts = euterpe_offline::check::CheckOptions::default();
                euterpe_offline::check::check_init::<D>(&opts).unwrap();
            }
        }
        each(&mut Check);
    }

    /// Every device's panel binds parameters its schema has, with controls
    /// of the right kind.
    #[test]
    fn every_panel_matches_its_schema() {
        struct Check;
        impl Each for Check {
            fn device<D: Device>(&mut self) {
                use euterpe_core::{ParamSet, Processor};
                let schema = <<D::Graph as Processor>::Params as ParamSet>::SCHEMA;
                if let Err(e) = euterpe_core::panel::check(D::PANEL, schema) {
                    panic!("{}: {e:?}", D::NAME);
                }
            }
        }
        each(&mut Check);
    }

    #[test]
    fn names_are_unique() {
        for (i, (a, _)) in DEVICES.iter().enumerate() {
            assert!(
                DEVICES[i + 1..].iter().all(|(b, _)| a != b),
                "duplicate device {a}"
            );
        }
    }

    #[test]
    fn each_visits_every_device_in_order() {
        struct Names([&'static str; 16], usize);
        impl Each for Names {
            fn device<D: Device>(&mut self) {
                self.0[self.1] = D::NAME;
                self.1 += 1;
            }
        }
        let mut names = Names([""; 16], 0);
        each(&mut names);
        assert_eq!(names.1, DEVICES.len());
        for (i, (name, _)) in DEVICES.iter().enumerate() {
            assert_eq!(names.0[i], *name);
        }
    }
}
