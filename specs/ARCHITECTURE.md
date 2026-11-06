# Euterpe — Architecture

A Rust toolkit for building audio hardware: effects, amps, instruments. The Daisy Seed is the first target, other boards later. It is built for agentic development: every change can be rendered, measured and budgeted without ears or hardware in the loop.

> Named after Euterpe, the muse of music. Crates use the `euterpe_` prefix; the CLI is `euterpe`.

## Goals

1. **Fast by default.** The idiomatic way to write a processor is also the fast way. Nothing in the hot path is an accident.
2. **Composable at zero cost.** Devices are built from small processors and combinators that monomorphize into one inlined function. No dynamic dispatch, no allocation.
3. **One device definition, every target.** The same device crate runs on the Daisy, as a desktop simulator, and offline (WAV in, WAV out).
4. **Closed-loop for agents.** Render, analyze, benchmark and test from the CLI, with JSON output. Every processor ships with a schema, tests and a cycle budget.

## Non-goals (for now)

- Runtime-reconfigurable graphs (patching at runtime). Graphs are static types.
- Multi-core or threading on the device.
- Plugin formats (VST/CLAP). Possible later as another platform.

---

## Core model (`euterpe_core`, `no_std`)

### Block processing

The unit of work is a fixed-size block, processed in place.

```rust
pub const BLOCK: usize = 32;

pub type Block = [f32; BLOCK];

pub trait Processor {
    type Params: ParamSet;

    /// Called once before processing, and again whenever the context changes.
    fn prepare(&mut self, ctx: &Ctx);

    /// Clear all signal state (delay lines, filter memory). Keeps params and loaded assets.
    fn reset(&mut self);

    /// Process one block in place. Params are already smoothed for this block.
    fn process(&mut self, buf: &mut Block, params: &Self::Params);

    /// Latency in samples at the outer sample rate.
    fn latency(&self) -> usize { 0 }
}
```

Why blocks rather than samples:

- Each stage runs a tight loop with state and coefficients held in registers. In a per-sample chain, every stage's state is reloaded for every sample.
- Block-natured work (convolution, oversampling, lookahead) fits without hacks.
- Fixed-size arrays remove bounds checks and make loop trip counts known to the compiler.

A `BLOCK` of 32 at 48 kHz is 0.67 ms. It is a project-wide constant. Changing it is a recompile, not a runtime option.

### Context

```rust
pub struct Ctx {
    pub sample_rate: f32,   // the rate this processor runs at (already multiplied inside Oversampled)
    pub oversampling: u32,  // factor relative to the device rate
}
```

### Per-sample adapter

Some processors are naturally per-sample (feedback loops, one-pole smoothers). They implement `SampleProcessor` and are wrapped:

```rust
pub trait SampleProcessor {
    type Params: ParamSet;
    fn prepare(&mut self, ctx: &Ctx);
    fn reset(&mut self);
    fn tick(&mut self, x: f32, params: &Self::Params) -> f32;
}

pub struct PerSample<P>(pub P); // impl Processor, loops `tick` over the block
```

This is also the migration path for the existing `dsp` processors.

---

## Parameters

Parameters are data with a static schema, separate from DSP state.

```rust
#[derive(Params)]
pub struct DriveParams {
    #[param(range = 0.0..=24.0, unit = "dB", default = 12.0, smooth_ms = 20.0)]
    pub drive: f32,
    #[param(default = true)]
    pub enabled: bool,
}
```

The derive generates:

- a `const SCHEMA: &[ParamDesc]` (id, label, range, unit, taper, default, smoothing time)
- per-block smoothing (a linear ramp to the target across the block)
- nested ids for composed devices (`amp.drive`, `cab.enabled`)

Rules:

- **Control rate.** Expensive derived values (filter coefficients, table selection) are recomputed at most once per block, in a `ParamSet::update` hook, and only when the target changed. Nothing checks dirty flags per sample.
- **Single source of truth.** The schema drives the desktop UI, hardware bindings, presets, the CLI (`--set amp.drive=6`) and agent introspection (`euterpe schema <device>` dumps JSON).
- **Lock-free handoff.** Targets cross from the control side (UI, ADC loop) to the audio side through atomics or a single-producer queue. They are never shared mutably.

---

## Composition

All combinators are generic structs that implement `Processor`.

| Combinator | Purpose |
| --- | --- |
| `#[derive(Chain)]` on a struct | Runs fields in declaration order. Generates `process`, `reset`, `prepare`, latency sum and the nested param tree. Fields stay directly accessible. |
| `Oversampled<P, const F: usize>` | Polyphase half-band up, `P` at `F`× rate, half-band down. `F` ∈ {2, 4, 8}. Reports its filter latency. |
| `Parallel<A, B, M>` | Splits, processes both, recombines with mixer `M`. Inserts delay compensation from `latency()`. |
| `Bypass<P>` | Click-free crossfade between `P` and dry, latency-compensated. |
| `Switch<(A, B, ...)>` | Selects one of several processors, e.g. amp types. Static dispatch, crossfade on change. |
| `Feedback<P, D>` | Feedback loop with a delay of at least one sample. |
| `Gain`, `Mix`, `Dc` | Small utilities. |

Example device graph:

```rust
#[derive(Chain)]
pub struct Combo {
    pub input: InputStage,
    pub amp: Switch<(Oversampled<Champ, 4>, Oversampled<TweedDeluxe, 4>, Oversampled<Ac30, 4>)>,
    pub cab: Convolver<2048>,
    pub reverb: SpringReverb,
    pub out: OutputStage,
}
```

The existing explicit-field style is kept. `#[derive(Chain)]` just removes the boilerplate.

---

## Performance rules

These apply to everything in the hot path. They are enforced in review and by `euterpe bench`.

1. **No allocation, no panics, no dynamic dispatch** in `process`. Bounds are proven by fixed-size types.
2. **Fast math with known error.** `euterpe_core::fastmath` provides `tanh`, `exp2`, `log2`, `sin` and friends as rational approximations or tables, each with a documented maximum error and a test proving it. Generic `micromath` calls are not used in the hot path.
3. **Compile-time tables.** Nonlinear curves (triode transfer, waveshapers) are generated by `const fn` or `build.rs` and live in flash or DTCM. No startup computation.
4. **Explicit memory placement** on the device:
   - ITCM: the hottest processing code
   - DTCM: processor state, small tables
   - AXI SRAM: convolution buffers, larger tables
   - SDRAM: long delay lines, IR banks, sample memory

   Placement is expressed with wrapper types or link-section attributes in the device crate, not left to chance.
5. **Denormals off.** Flush-to-zero is set at startup on every platform.
6. **Amortized block work.** Anything with a period longer than one block (FFT convolution, analysis) spreads its work across blocks so the worst-case block costs about the same as the average.
7. **Measure before optimizing.** Every device has a cycle budget. Optimization work starts from `euterpe bench` numbers.

---

## Platforms

*Implemented so far as `fn run<D: Device>(self) -> !` only; `Controls` and `Storage` come when a platform needs them.*

```rust
pub trait Platform {
    type Controls: Controls;   // pots, buttons, encoders, LEDs
    type Storage: Storage;     // IR and preset loading
    fn run<D: Device>(self, device: D) -> !;
}
```

| Platform | Audio | Controls | Use |
| --- | --- | --- | --- |
| `euterpe_daisy` | SAI/DMA through the `daisy` crate | ADC, GPIO | The real hardware |
| `euterpe_desktop` | cpal | egui panel generated from the schema | Playing and tuning |
| `euterpe_offline` | WAV in → WAV out, deterministic | Param automation from the CLI or a file | Tests, analysis, agents |

New hardware is a new platform crate. Devices don't change.

### Devices

A device crate is a product definition:

```rust
pub struct ComboDevice;

impl Device for ComboDevice {
    type Graph = Combo;
    const PANEL: &'static [Binding] = &[
        pot("knob1", "amp.gain", Taper::Audio),
        pot("knob2", "amp.tone"),
        button("button1", "amp.type", Action::Cycle),
    ];
    const BUDGET_CYCLES_PER_BLOCK: u32 = 200_000; // ~70% of 480 MHz × 0.67 ms
}
```

---

## Agent tooling (the `euterpe` CLI)

| Command | Does |
| --- | --- |
| `euterpe render <device> --in x.wav --set k=v --out y.wav` | Offline render with param overrides |
| `euterpe analyze <device> <suite>` | Frequency response, THD vs level, aliasing metric (sine sweeps), null test against a reference. JSON output. |
| `euterpe bench [<device>...]` | Instructions per block per device on an emulated Cortex-M7 (QEMU `mps2-an500` with an instruction-counting plugin), estimated cycles against the device's budget, and hot-path checks of the compiled `process` (no panics, soft float, f64 or allocation). Flags regressions against a stored baseline. On the board: DWT cycles per block, to calibrate the estimate. |
| `euterpe schema <device>` | Dump the param tree as JSON |
| `euterpe flash <device>` | Build and flash over DFU or probe-rs |

### Tests every processor gets

A generic test harness runs against any `Processor`:

- no NaN, Inf or denormals for silence, impulses, sweeps and full-scale noise
- bounded output for bounded input
- `reset()` is deterministic: the same input after reset gives bit-identical output
- stable when every param is set to its extremes and swept quickly
- the latency reported matches the measured latency

Each device also has golden-file tests: fixed input, fixed params, a stored output, compared within a tolerance.

### Contract for adding a processor

Documented in `CLAUDE.md`. A processor PR must include:

1. the `Processor` (or `SampleProcessor`) implementation with a `#[derive(Params)]` struct
2. the generic test harness wired in
3. an `analyze` suite if it is nonlinear or filtering
4. a `bench` entry, and the cycle cost stated in the PR

---

## Crate layout

```
crates/
  core/        Processor, Block, Ctx, params, smoothing, fastmath      (no_std)
  macros/      #[derive(Chain)], #[derive(Params)]
  dsp/         filters, dynamics, nonlinear, conv, oversample, modulation, amp/   (no_std)
  platform/
    daisy/
    desktop/
    offline/
  assets/      compile-time embedding of IRs and tables (from ir_embed)
devices/
  preamp/
  combo/
crates/cli/    the `euterpe` binary: render, analyze, bench, schema, flash
specs/
```

---

## Migration from the current repo

1. **Core first.** Create `core` with `Processor`, `Block`, `Ctx`, `PerSample`, and port `params` to the derive-based schema. *Done as `euterpe_core`. Params use the `params!` macro for now; `#[derive(Params)]` and `#[derive(Chain)]` arrive with the `macros` crate.*
2. **Offline platform and `euterpe render` / `analyze`.** Everything after this is verified through them. *`render`, `schema`, `devices` and the generic `check_processor` invariants are done (the CLI is the `euterpe_cli` crate rather than an xtask). `analyze` is done too: `response`, `thd`, `aliasing` and `null` live in `euterpe_offline::analyze` and are exposed as `euterpe analyze <device> <suite>`. Baseline for `Oversampled`: unoversampled `demo.drive` at drive 24 dB and -6 dBFS aliases at -27.7 dB (2 kHz), -13.1 dB (6 kHz) and -8.3 dB (10 kHz), relative to the fundamental.*
   - *Pulled forward: `euterpe bench` (QEMU instruction counts, hot-path checks, baseline) and a minimal `Platform` trait with `euterpe_daisy`, which runs a device on the Seed and measures each block with DWT. The firmware target is built with `target-cpu=cortex-m7`, so f64 is hardware, not soft float. Still to do: calibrating cycles per instruction on the board, and panel controls.*
   - *`euterpe_desktop` exists too: any registered device through cpal, with an egui panel generated from the schema (sliders move in normalized position, so tapers apply), lock-free parameter slots, and devices switched at runtime behind one virtual call per block. Devices are listed once, in `euterpe_registry`. `Device::Graph` is now `Send + 'static` so a host can move it to its audio thread.*
3. **Wrap the existing processors** in `PerSample` so the preamp keeps working. Move the preamp into `devices/preamp`.
4. **Port `CabSim`** to a native block `Convolver`: real FFT, work amortized across blocks, an optional direct-form head for low latency.
5. **`Oversampled` and `fastmath`**, verified with the aliasing metric.
6. **Amp processors:** Champ, then Tweed Deluxe, then AC30, then generalize into the configurable amp engine.
7. **`devices/combo`** on the Daisy, with `bench` numbers against its budget.

## Open questions

- Stereo and multichannel: a `Frames<const C: usize>` block type, or mono processors composed per channel?
- Whether `euterpe_desktop` should hot-reload device crates for fast tuning.
- Whether the probe-rs/defmt path or DFU plus USB serial is the main hardware loop.
