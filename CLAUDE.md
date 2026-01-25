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
