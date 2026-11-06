//! Invariant checks every processor should pass.
//!
//! Call [`check_processor`] from a processor's tests:
//!
//! ```ignore
//! #[test]
//! fn invariants() {
//!     euterpe_offline::check::check_processor::<MyProcessor>(&Default::default()).unwrap();
//! }
//! ```

use euterpe_core::{Device, ParamDesc, ParamKind, ParamSet, ParamValue, Processor};

use crate::render::{RenderOptions, render, render_with};
use crate::signal;

#[derive(Clone, Copy, Debug)]
pub struct CheckOptions {
    pub sample_rate: u32,
    /// Largest output magnitude allowed. The default, 16.0, is +24 dBFS.
    pub max_output: f32,
    /// Length of each test signal in samples.
    pub len: usize,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            max_output: 16.0,
            len: 4_800,
        }
    }
}

/// Run all checks. Returns every failure found, not just the first.
///
/// - output is finite and bounded for silence, impulse, DC, Nyquist,
///   full-scale noise and a log sweep
/// - the same holds at every parameter's extremes (each float's min and
///   max, both toggle states, every choice)
/// - the same holds while all parameters jump to random values every block
/// - rendering twice gives bit-identical output (`reset` clears all state)
pub fn check_processor<P: Processor + Default>(opts: &CheckOptions) -> Result<(), Vec<String>> {
    let mut failures = Vec::new();
    let signals = test_signals(opts);
    let render_opts = RenderOptions::new(opts.sample_rate);
    let mut p = P::default();

    let defaults = P::Params::default();
    for (name, input) in &signals {
        let out = render(&mut p, &defaults, input, &render_opts);
        check_output(
            &mut failures,
            &format!("default params, {name}"),
            &out,
            opts,
        );
    }

    let noise = &signals.iter().find(|(n, _)| *n == "noise").unwrap().1;
    for desc in P::Params::SCHEMA {
        for value in extremes(desc) {
            let mut params = P::Params::default();
            if let Err(e) = params.set(desc.id, value) {
                failures.push(format!("setting {}={value:?} failed: {e}", desc.id));
                continue;
            }
            let out = render(&mut p, &params, noise, &render_opts);
            check_output(
                &mut failures,
                &format!("{}={value:?}, noise", desc.id),
                &out,
                opts,
            );
        }
    }

    if !P::Params::SCHEMA.is_empty() {
        let mut params = P::Params::default();
        let mut rng = 0x9e37_79b9_7f4a_7c15u64;
        let out = render_with(&mut p, &mut params, noise, &render_opts, |_, params| {
            for desc in P::Params::SCHEMA {
                rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                let n = (rng >> 40) as f32 / (1u64 << 24) as f32;
                let _ = params.set(desc.id, desc.kind.from_normalized(n));
            }
        });
        check_output(&mut failures, "random automation every block", &out, opts);
    }

    let a = render(&mut p, &defaults, noise, &render_opts);
    let b = render(&mut p, &defaults, noise, &render_opts);
    if a.iter().zip(&b).any(|(x, y)| x.to_bits() != y.to_bits()) {
        failures
            .push("two renders of the same input differ: reset() does not clear all state".into());
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// Check that `D`'s graph built the way the embedded platforms build it
/// ([`Device::GRAPH_INIT`]) behaves exactly like `Default`: the same output,
/// bit for bit, for every test signal with every parameter automated.
/// Trivial for [`Init::DEFAULT`](euterpe_core::place::Init::DEFAULT); for a
/// zeroed init it tests the safety claim of
/// [`Init::zeroed`](euterpe_core::place::Init::zeroed).
///
/// # Errors
///
/// One message per signal whose outputs differ.
pub fn check_init<D: Device>(opts: &CheckOptions) -> Result<(), Vec<String>> {
    let mut failures = Vec::new();
    for (name, input) in test_signals(opts) {
        let mut placed = Box::<D::Graph>::new_uninit();
        // SAFETY: a fresh, aligned allocation; `GRAPH_INIT`'s contract is
        // what's being tested, and a valid `Init::DEFAULT` always holds.
        let mut placed = unsafe {
            D::GRAPH_INIT.build(placed.as_mut_ptr());
            placed.assume_init()
        };
        let mut built = Box::<D::Graph>::default();
        let automate = |i: usize, p: &mut <D::Graph as Processor>::Params| {
            let n = if (i / 32).is_multiple_of(2) { 0.2 } else { 0.9 };
            for d in <D::Graph as Processor>::Params::SCHEMA {
                let _ = p.set_normalized(d.id, n);
            }
        };
        let ro = RenderOptions::new(opts.sample_rate);
        let a = render_with(&mut *placed, &mut Default::default(), &input, &ro, automate);
        let b = render_with(&mut *built, &mut Default::default(), &input, &ro, automate);
        if a.iter()
            .map(|s| s.to_bits())
            .ne(b.iter().map(|s| s.to_bits()))
        {
            failures.push(format!(
                "{}: built with {:?}, `{name}` renders differently than with Default",
                D::NAME,
                D::GRAPH_INIT
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

fn test_signals(opts: &CheckOptions) -> Vec<(&'static str, Vec<f32>)> {
    let n = opts.len;
    vec![
        ("silence", vec![0.0; n]),
        ("impulse", signal::impulse(n, 1.0)),
        ("dc", vec![1.0; n]),
        (
            "nyquist",
            (0..n)
                .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
                .collect(),
        ),
        ("noise", signal::noise(1.0, n, 1)),
        (
            "sweep",
            signal::log_sweep(20.0, 20_000.0, 1.0, n, opts.sample_rate),
        ),
    ]
}

fn extremes(desc: &ParamDesc) -> Vec<ParamValue> {
    match desc.kind {
        ParamKind::Float { min, max, .. } => vec![ParamValue::Float(min), ParamValue::Float(max)],
        ParamKind::Toggle { .. } => vec![ParamValue::Bool(false), ParamValue::Bool(true)],
        ParamKind::Choice { options, .. } => {
            let count = u8::try_from(options.len()).unwrap_or(u8::MAX);
            (0..count).map(ParamValue::Choice).collect()
        }
    }
}

fn check_output(failures: &mut Vec<String>, case: &str, out: &[f32], opts: &CheckOptions) {
    if let Some(i) = out.iter().position(|s| !s.is_finite()) {
        failures.push(format!(
            "{case}: non-finite output {} at sample {i}",
            out[i]
        ));
        return;
    }
    if let Some(i) = out.iter().position(|s| s.abs() > opts.max_output) {
        failures.push(format!(
            "{case}: output {} at sample {i} exceeds the bound {}",
            out[i], opts.max_output
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use euterpe_core::place::Init;
    use euterpe_core::{Block, Ctx, params};

    /// Adds an offset whose default is not zero, and wrongly claims a
    /// zeroed init.
    struct Offset(f32);
    impl Default for Offset {
        fn default() -> Self {
            Self(0.5)
        }
    }
    impl Processor for Offset {
        type Params = ();
        fn prepare(&mut self, _: &Ctx) {}
        fn reset(&mut self) {}
        fn process(&mut self, buf: &mut Block, (): &()) {
            buf.iter_mut().for_each(|s| *s += self.0);
        }
    }
    struct Lying;
    impl Device for Lying {
        const NAME: &'static str = "test.lying";
        const DESCRIPTION: &'static str = "";
        type Graph = Offset;
        // SAFETY: zeroes are a valid `Offset` (an f32); the claim that they
        // behave like the default is the lie under test.
        const GRAPH_INIT: Init = unsafe { Init::zeroed() };
    }

    #[test]
    fn check_init_catches_a_wrong_zeroed_claim() {
        let err = check_init::<Lying>(&CheckOptions::default()).unwrap_err();
        assert!(err[0].contains("renders differently"), "{err:?}");
    }

    params! {
        struct GainParams {
            gain: f32 = ("gain", "Gain", ParamKind::float(0.0, 2.0, 1.0)),
        }
    }

    #[derive(Default)]
    struct Gain;
    impl Processor for Gain {
        type Params = GainParams;
        fn prepare(&mut self, _: &Ctx) {}
        fn reset(&mut self) {}
        fn process(&mut self, buf: &mut Block, p: &GainParams) {
            buf.iter_mut().for_each(|s| *s *= p.gain);
        }
    }

    /// Keeps state across renders because `reset()` does nothing.
    #[derive(Default)]
    struct Leaky {
        acc: f32,
    }
    impl Processor for Leaky {
        type Params = ();
        fn prepare(&mut self, _: &Ctx) {}
        fn reset(&mut self) {}
        fn process(&mut self, buf: &mut Block, (): &()) {
            for s in buf.iter_mut() {
                self.acc = self.acc * 0.5 + *s;
                *s = self.acc * 0.1;
            }
        }
    }

    /// Divides by its input: blows up on silence.
    #[derive(Default)]
    struct Explodes;
    impl Processor for Explodes {
        type Params = ();
        fn prepare(&mut self, _: &Ctx) {}
        fn reset(&mut self) {}
        fn process(&mut self, buf: &mut Block, (): &()) {
            buf.iter_mut().for_each(|s| *s = 1.0 / *s);
        }
    }

    #[test]
    fn good_processor_passes() {
        check_processor::<Gain>(&CheckOptions::default()).unwrap();
    }

    #[test]
    fn catches_state_surviving_reset() {
        let errs = check_processor::<Leaky>(&CheckOptions::default()).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("reset()")), "{errs:?}");
    }

    #[test]
    fn catches_non_finite_output() {
        let errs = check_processor::<Explodes>(&CheckOptions::default()).unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("silence") && e.contains("non-finite")),
            "{errs:?}"
        );
    }
}
