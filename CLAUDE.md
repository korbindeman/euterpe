# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A dual-channel instrument preamp/DI box with amp simulation. Desktop-first development targeting eventual deployment on Daisy Seed (STM32H750 Cortex-M7).

## Build Commands

```bash
# Desktop app: any Euterpe device through your interface
just play demo.drive

# Legacy preamp desktop app
cargo run -p preamp_desktop

# Daisy Seed firmware (requires ARM toolchain)
cargo build -p preamp_daisy --target thumbv7em-none-eabihf --release

# Check all crates compile (host crates on the host, firmware/no_std crates for the Daisy)
just check

# Lint, same split (all warnings are errors)
just clippy

# Run tests for every host crate
just test
```

`cargo check --workspace` on the host fails by design: the firmware crates (`preamp_daisy`, `daisy_runtime`, `euterpe_daisy`, `demo_daisy`, `euterpe_bench_qemu`) only build for `thumbv7em-none-eabihf`. The justfile keeps the lists. The Daisy target builds with `target-cpu=cortex-m7` (`.cargo/config.toml`), which gives hardware f64 and the M7 instruction schedule.

## Euterpe (toolkit in progress)

This repo is becoming **Euterpe**, a toolkit for building audio hardware. The design is in `specs/ARCHITECTURE.md`; read it before adding processors or devices. New work goes into the `euterpe_*` crates. The older crates below are migrated step by step.

- **`crates/euterpe_core`** (`no_std`): `Processor` (block-based, `BLOCK = 32`), `SampleProcessor` + `PerSample`, `Ctx`, `params!` / `ParamSet` schemas, `Smoother` / `Ramp`, `fastmath` (tested error bounds), `Device`.
- **`crates/euterpe_offline`**: WAV I/O, `render` / `render_with` (latency-compensated, deterministic), test signals, `check::check_processor`, and `analyze` (response, THD, aliasing, null).
- **`crates/euterpe_dsp`** (`no_std`): DSP blocks. So far `Oversampled2/4/8<P>`: polyphase half-band stages (taps from `tools/design/halfband.py`, ≥100 dB, flat to 20 kHz), exact integer latency.
- **`crates/euterpe_registry`** (`no_std`): the one list of devices, read by every tool.
- **`crates/euterpe_cli`**: the `euterpe` binary.
- **`devices/demo`**: `demo.gain`, `demo.drive` and `demo.drive4x` (the drive wrapped in `Oversampled4`), the reference examples for writing a processor and a device.
- **`crates/euterpe_bench`** (`no_std`): the bench harness (`static` and `automated` scenarios), shared by every bench target.
- **`crates/euterpe_bench_qemu`**: bench firmware for QEMU's `mps2-an500` (a Cortex-M7). Run through `euterpe bench`, never directly.
- **`crates/euterpe_daisy`**: the Daisy Seed `Platform`. Runs a device on the codec, measures every block with the DWT cycle counter, blinks the LED fast if a block goes over budget.
- **`devices/demo_daisy`**: firmware running `demo.drive` on the Seed (`just flash demo_daisy`).
- **`crates/euterpe_desktop`**: the desktop platform, `euterpe-desktop` (`just play demo.drive`). Plays any registered device through an audio interface (a Volt if connected), with a panel generated from the schema, test-signal and WAV-loop sources, bypass, meters and host load.

```bash
just euterpe devices                      # list devices (JSON)
just euterpe schema demo.drive            # parameter schema (JSON)
just euterpe render demo.drive --gen sine:220 --set drive=24 --out out.wav
just euterpe analyze demo.drive aliasing --set drive=24   # suites: response, thd, aliasing, null
just test-euterpe                         # tests for the Euterpe crates only
just bench                                # instruction counts + hot-path checks vs benches/qemu-baseline.txt
just bench-baseline                       # accept the current counts after an intended change
```

### `euterpe analyze`

`euterpe_offline::analyze` holds the measurements, so device tests call them directly; the CLI prints them as JSON. Each one renders a sine, skips a settling time, and takes one 16384-point Blackman-Harris FFT frame. Test frequencies are moved to an odd FFT bin (for `aliasing`, the nearest one where no alias lobe overlaps a harmonic), so harmonics and aliases are measured apart exactly. `aliasing` reports `alias_db`: energy at the frequencies where harmonics above Nyquist fold back, relative to the fundamental. `audible_alias_db` counts only aliases below 20 kHz: an oversampler leaves aliases between 20 kHz and Nyquist from harmonics in its half-band transition band, and those are inaudible. Everything else is `residual_db`, which sits at the f32 noise floor (about -116 dB) when the accounting is complete.

### `euterpe bench`

Builds `euterpe_bench_qemu`, runs it on QEMU with the `tools/qemu/euterpe_insn.c` plugin, and prints JSON per device and scenario: instructions per block, an estimated cycle count against `Device::BUDGET_CYCLES_PER_BLOCK`, and a hot-path check. The check walks the call graph from the graph's `Processor::process` in the firmware's disassembly and fails on panics, soft-float calls, f64 instructions or allocation. The walk starts at `euterpe_bench::Hot<Graph>::process`, an `#[inline(never)]` wrapper around the measured call, so there is always a symbol to check. It exits 1 on a failed check, a device over budget, or a regression against `--baseline`.

- Instruction counts are deterministic, so the baseline has zero tolerance by default. They are not cycles: until DWT calibration from a board exists, cycles are estimated as one per instruction.
- Needs `qemu-system-arm` (with plugin support), a C compiler with glib headers, and `llvm-objdump` (`rustup component add llvm-tools`).
- When a change moves the numbers on purpose, run `just bench-baseline` and commit the new baseline with the change.

### Rules for new processors

1. Implement `Processor` (or `SampleProcessor` wrapped in `PerSample`) with params declared via `params!`. No allocation, panics or dynamic dispatch in `process`.
2. Smooth audible params with `Smoother`: `prepare` it with `Params::smooth_ms(id)`, call `reset()` from the processor's `reset`, and call `block(target)` once per block.
3. Use `euterpe_core::fastmath` in the hot path, not `micromath` or `std` math.
4. Add a test that calls `euterpe_offline::check::check_processor::<YourProcessor>`, plus tests for the processor's actual behavior. Filters and nonlinear processors also get `analyze` tests (`response`, `thd`, `aliasing`) with numbers from a closed form or an independent reference.
5. Register new devices in `devices!` in `crates/euterpe_registry/src/lib.rs`. The CLI, the desktop app and the bench firmware all read that list.
6. Run `just bench`. The hot path must pass, and the PR states the instructions per block.

### CI and the toolchain

`.github/workflows/ci.yml` runs `just check-format`, `just clippy`, `just check`, `just test` and `just bench` (against the baseline) on every PR. The toolchain is pinned in `rust-toolchain.toml` because a new rustc changes instruction counts. To bump it, change the version, run `just bench-baseline`, and commit both together.

### Lints

Euterpe crates opt in to `[workspace.lints]` (root `Cargo.toml`) with `[lints] workspace = true`: clippy pedantic, minus the lints that are noise in DSP code. Legacy crates keep clippy's defaults until they are migrated.

- `allow`/`expect` must carry a `reason = "..."`. Prefer `#[expect]` on the smallest item, so it fails once the lint no longer fires.
- Casts that can truncate or lose sign (`f32 as i32`, `usize as u32`) are warnings. Use `From`/`try_from` when the value can be out of range; `#[expect]` with a reason when it is range-checked or saturation is intended.
- `euterpe_core` and device crates warn on `default_numeric_fallback` outside tests: an unsuffixed float literal is f64, which is soft-float on the Daisy.
- Test modules may `allow(clippy::float_cmp)`: exact comparisons there are deliberate (bit-identical renders, exact-at-integers math).

## Architecture (legacy crates)

### Crate Structure

- **`dsp`** - Core DSP library. `no_std`, zero allocations. All processors implement the `Process` trait for single-sample processing.
- **`preamp`** - Preamp-specific signal chain built on `dsp`. Defines `Channel` struct combining gate, compressor, EQ, saturator, and cab sim. Also `no_std`.
- **`desktop_audio`** - Generic cpal-based audio runtime. Provides `AudioProcessor` trait and `AudioEngine` for device I/O.
- **`preamp_desktop`** - Desktop application using egui. Wires UI controls to the preamp processor.
- **`daisy_runtime`** - Hardware abstraction for Daisy Seed board.
- **`preamp_daisy`** - Firmware entry point for Daisy Seed.
- **`ir_embed`** - Proc macro for embedding WAV impulse responses at compile time.

### Signal Flow

```
Input -> NoiseGate -> Compressor -> 3-Band EQ -> Saturator -> CabSim -> Gain -> Output
```

### Key Patterns

**Process trait** (`dsp/src/lib.rs`): Every DSP block implements `fn process(&mut self, sample: f32) -> f32`.

**Parameter system** (`dsp/src/params.rs`): Parameters use `Toggle` (on/off) and `Knob` (continuous) types with dirty flags for expensive recalculations.

**Shared state**: Desktop uses `Arc<SharedState>` with `AtomicBool`/`AtomicU32` for lock-free parameter updates between UI and audio threads.

### IR (Impulse Response) Loading

**Desktop**: Uses `desktop_audio::load_ir()` to load WAV files at runtime. Supports any sample rate (resampled to 48kHz) and stereo (converted to mono).

**Daisy Seed**: Uses `ir_embed::embed_ir!()` macro to embed WAV files at compile time:
```rust
pub static DELUXE_REVERB: &[f32] = &embed_ir!("ir/deluxe_reverb.wav", 2048);
```
The second argument limits length (required due to 128KB flash limit).

**Cab sim limits**: `CabSim` uses partitioned FFT convolution with 8 partitions × 256 samples = 2048 samples max (~43ms at 48kHz). See `dsp::cab_sim::IR_MAX_LENGTH`.

## Development Notes

- DSP-heavy crates are optimized even in dev builds (see `[profile.dev.package.*]` in root Cargo.toml)
- The `dsp` crate uses `micromath` for `no_std` math functions
- Desktop audio requires `egui` feature on `desktop_audio` crate for UI widgets

## Future: SD Card IR Loading on Daisy

Currently IRs are embedded at compile time (limited to 2048 samples by flash size). For longer/more IRs, load from SD card into SDRAM at runtime.

### Hardware Available
- **SDRAM**: 64MB at 0xC0000000 - use `board_split_sdram!` macro
- **SD Card**: SDMMC1 on pins D2-D7 - use `stm32h7xx_hal::sdmmc`
- **QSPI Flash**: 8MB at 0x90000000 - alternative for read-only data

### Implementation Steps

1. **Initialize SDRAM** (before loading IRs):
   ```rust
   let sdram = daisy::board_split_sdram!(ccdr, dp, cp, pins);
   ```

2. **Initialize SD card**:
   ```rust
   // Configure SDMMC pins (D2-D7 on Daisy)
   let mut sdmmc: Sdmmc<_, SdCard> = dp.SDMMC1.sdmmc(pins, 2.MHz());
   sdmmc.init(clock);
   ```

3. **Mount FAT filesystem** (use `embedded-sdmmc` crate):
   ```rust
   let mut volume_mgr = VolumeManager::new(sdmmc, FakeTimesource);
   let volume = volume_mgr.open_volume(VolumeIdx(0))?;
   let root_dir = volume.open_root_dir()?;
   ```

4. **Read WAV file**:
   ```rust
   let file = root_dir.open_file_in_dir("ir/deluxe.wav", Mode::ReadOnly)?;
   let mut wav_data = [0u8; MAX_WAV_SIZE];
   let bytes_read = file.read(&mut wav_data)?;
   ```

5. **Parse WAV header** (simple `no_std` parser needed):
   - Read RIFF header, find "data" chunk
   - Extract sample rate, bit depth, channels
   - Get raw sample data offset and length

6. **Process samples** (reuse logic from `ir_embed`):
   - Convert to f32 (handle 16/24/32-bit int or float)
   - Convert stereo to mono
   - Resample to 48kHz if needed (linear interpolation)
   - Normalize peak to 1.0
   - Store in SDRAM buffer

7. **Pass to channel**:
   ```rust
   channel.set_ir(ir_samples_in_sdram);
   ```

### Dependencies Needed
```toml
[dependencies]
embedded-sdmmc = "0.7"  # FAT filesystem
```

### Considerations
- WAV parsing is simple (~50 lines) - no crate needed
- Resampling code already exists in `ir_embed/src/lib.rs`
- SDRAM can hold many IRs (64MB = ~700 IRs at 2048 samples each)
- Could scan directory for all .wav files and build IR list
- IR switching at runtime just calls `channel.set_ir()` with different slice
