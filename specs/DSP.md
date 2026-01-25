# DSP

## Overview

The DSP crate is a general-purpose audio DSP toolkit written in Rust. It is designed to run on Daisy Seed hardware (Cortex-M7) and be usable on desktop for development and debugging.

## Design Principles

- **Daisy hardware is the target.** All performance work is aimed at the Cortex-M7. Desktop only needs to work correctly, not be fast.
- **`no_std`, zero allocations.** The library must run on bare metal with fixed-size state.
- **Explicit signal chains.** Processors are composed in structs with named fields. The containing struct calls `process()` on each stage explicitly. This gives direct access to each processor for parameter control. (We intentionally avoid a `>>` chaining operator — while ergonomic for defining flow, it moves processors into a nested struct and prevents direct access to individual nodes for parameter mutation.)
- **Single-sample processing.** All processors operate one sample at a time for real-time use.
- **Self-describing parameters.** Every processor declares its parameters with typed definitions (toggle, knob, etc.). UI and hardware controls are derived from these definitions.
- **Dirty-flagged parameters.** Parameters that trigger expensive recalculations (e.g., biquad coefficients) use dirty flags. The flag is checked per-sample (cheap), and recalculation only happens when something actually changed.
- **Incremental performance optimization.** Profile on real hardware and find gains over time. Don't optimize speculatively.

## Parameter System

Each DSP processor defines its controllable parameters using typed descriptors. This is a core abstraction alongside the `Process` trait.

### Parameter Types

- **Toggle** — binary on/off (e.g., bypass, mode switch)
- **Knob** — continuous value over a range (e.g., gain, frequency, Q)

Additional types can be added as needed.

### Hardware Interface Modeling

Each project (preamp, reverb pedal, etc.) has a physical hardware interface — knobs, switches, LEDs. The parameter system models this interface exactly. Both the hardware and the desktop app use the same parameter definitions to control the DSP.

### Desktop (egui)

On desktop, parameter definitions automatically generate egui widgets — toggles become checkboxes, knobs become sliders, etc. Beyond the hardware-equivalent controls, the desktop view provides additional debugging affordances: exact numeric readouts, preset save/load, and other tweaking views that don't exist on the hardware.

egui widget generation is behind a feature flag in the DSP crate so it compiles out for Daisy builds.

## Scope

The DSP crate is not limited to the preamp/channel-strip. It is a growing library of audio processors that can be composed into different signal chains. Current and planned processors include:

### Implemented

- Biquad filter (low shelf, high shelf, peaking)
- Three-band EQ
- Compressor
- Noise gate
- Saturator (tanh soft clip)
- Cabinet simulator (FIR convolution)
- Channel strip (preamp signal chain)

### Planned

- Reverb
- Delay
- Additional compressor designs

## Target Hardware

- **Daisy Seed** — STM32H750 (Cortex-M7), 480 MHz, 64 MB SDRAM, 8 MB flash
- Sample rate: 48 kHz
- Block size determined by Daisy firmware
