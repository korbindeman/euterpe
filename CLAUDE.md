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
- **`crates/euterpe_offline`**: WAV I/O, `render` / `render_with` (latency-compensated, deterministic), test signals, and `check::check_processor`.
- **`crates/euterpe_cli`**: the `euterpe` binary. Devices are registered in `src/registry.rs`.
- **`devices/demo`**: `demo.gain` and `demo.drive`, the reference examples for writing a processor and a device.
- **`crates/euterpe_bench`** (`no_std`): the bench harness (`static` and `automated` scenarios), shared by every bench target.
- **`crates/euterpe_bench_qemu`**: bench firmware for QEMU's `mps2-an500` (a Cortex-M7). Run through `euterpe bench`, never directly.
- **`crates/euterpe_daisy`**: the Daisy Seed `Platform`. Runs a device on the codec, measures every block with the DWT cycle counter, blinks the LED fast if a block goes over budget.
- **`devices/demo_daisy`**: firmware running `demo.drive` on the Seed (`just flash demo_daisy`).

```bash
just euterpe devices                      # list devices (JSON)
just euterpe schema demo.drive            # parameter schema (JSON)
just euterpe render demo.drive --gen sine:220 --set drive=24 --out out.wav
just test-euterpe                         # tests for the Euterpe crates only
just bench                                # instruction counts + hot-path checks vs benches/qemu-baseline.txt
just bench-baseline                       # accept the current counts after an intended change
```

### `euterpe bench`

Builds `euterpe_bench_qemu`, runs it on QEMU with the `tools/qemu/euterpe_insn.c` plugin, and prints JSON per device and scenario: instructions per block, an estimated cycle count against `Device::BUDGET_CYCLES_PER_BLOCK`, and a hot-path check. The check walks the call graph from the graph's `Processor::process` in the firmware's disassembly and fails on panics, soft-float calls, f64 instructions or allocation. It exits 1 on a failed check, a device over budget, or a regression against `--baseline`.

- Instruction counts are deterministic, so the baseline has zero tolerance by default. They are not cycles: until DWT calibration from a board exists, cycles are estimated as one per instruction.
- Needs `qemu-system-arm` (with plugin support), a C compiler with glib headers, and `llvm-objdump` (`rustup component add llvm-tools`).
- When a change moves the numbers on purpose, run `just bench-baseline` and commit the new baseline with the change.

### Rules for new processors

1. Implement `Processor` (or `SampleProcessor` wrapped in `PerSample`) with params declared via `params!`. No allocation, panics or dynamic dispatch in `process`.
2. Smooth audible params with `Smoother`: `prepare` it with `Params::smooth_ms(id)`, call `reset()` from the processor's `reset`, and call `block(target)` once per block.
3. Use `euterpe_core::fastmath` in the hot path, not `micromath` or `std` math.
4. Add a test that calls `euterpe_offline::check::check_processor::<YourProcessor>`, plus tests for the processor's actual behavior.
5. Register new devices in `crates/euterpe_cli/src/registry.rs` and in `devices!` in `crates/euterpe_bench_qemu/src/main.rs`.
6. Run `just bench`. The hot path must pass, and the PR states the instructions per block.

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
