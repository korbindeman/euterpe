# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A dual-channel instrument preamp/DI box with amp simulation. Desktop-first development targeting eventual deployment on Daisy Seed (STM32H750 Cortex-M7).

## Build Commands

```bash
# Desktop app
cargo run -p preamp_desktop

# Daisy Seed firmware (requires ARM toolchain)
cargo build -p preamp_daisy --target thumbv7em-none-eabihf --release

# Check all crates compile
cargo check --workspace

# Run tests
cargo test --workspace
```

## Architecture

### Crate Structure

- **`dsp`** - Core DSP library. `no_std`, zero allocations. All processors implement the `Process` trait for single-sample processing.
- **`preamp`** - Preamp-specific signal chain built on `dsp`. Defines `Channel` struct combining gate, compressor, EQ, saturator, and cab sim. Also `no_std`.
- **`desktop_audio`** - Generic cpal-based audio runtime. Provides `AudioProcessor` trait and `AudioEngine` for device I/O.
- **`preamp_desktop`** - Desktop application using egui. Wires UI controls to the preamp processor.
- **`daisy_runtime`** - Hardware abstraction for Daisy Seed board.
- **`preamp_daisy`** - Firmware entry point for Daisy Seed.

### Signal Flow

```
Input -> NoiseGate -> Compressor -> 3-Band EQ -> Saturator -> CabSim -> Gain -> Output
```

### Key Patterns

**Process trait** (`dsp/src/lib.rs`): Every DSP block implements `fn process(&mut self, sample: f32) -> f32`.

**Parameter system** (`dsp/src/params.rs`): Parameters use `Toggle` (on/off) and `Knob` (continuous) types with dirty flags for expensive recalculations.

**Channel generics**: `Channel<IR_LEN>` uses a const generic for IR buffer size. Desktop uses `Channel<2048>`, Daisy uses `Channel<1>` (cab sim disabled).

**Shared state**: Desktop uses `Arc<SharedState>` with `AtomicBool`/`AtomicU32` for lock-free parameter updates between UI and audio threads.

## Development Notes

- DSP-heavy crates are optimized even in dev builds (see `[profile.dev.package.*]` in root Cargo.toml)
- The `dsp` crate uses `micromath` for `no_std` math functions
- Desktop audio requires `egui` feature on `desktop_audio` crate for UI widgets
