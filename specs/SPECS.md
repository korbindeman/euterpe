# Instrument Preamp - Project Specification

A dual-channel instrument preamp/DI box with amp simulation, built for eventual deployment on a Daisy Seed. Desktop-first development with egui UI.

## Project Structure

Cargo workspace with three crates:

```
instrument-preamp/
  Cargo.toml              # workspace root
  dsp/                    # shared DSP library, no_std compatible
    Cargo.toml
    src/
      lib.rs              # Process trait, constants
      biquad.rs           # Biquad filter implementation
      compressor.rs       # Compressor
      saturator.rs        # Soft saturation (grit)
      cab_sim.rs          # IR convolution
      channel.rs          # Full channel strip combining all processors
  desktop/                # Desktop application
    Cargo.toml
    src/
      main.rs             # Entry point
      audio.rs            # cpal audio I/O
      ui.rs               # egui UI
      ir_loader.rs        # WAV file loading for impulse responses
  daisy/                  # Daisy Seed target (future)
    Cargo.toml
    src/
      main.rs
```

---

## DSP Crate (`dsp/`)

### Requirements

- `no_std` compatible (use `#![no_std]` with `libm` for math functions)
- All processors implement a common `Process` trait
- No allocations — all state in fixed-size structs

### Process Trait

```rust
pub trait Process {
    fn process(&mut self, sample: f32) -> f32;
}
```

### Signal Chain Composition

Processors are composed in structs with named fields. The containing struct calls `process()` on each stage explicitly:

```rust
pub struct Channel {
    gate: NoiseGate,
    compressor: Compressor,
    eq: ThreeBandEq,
    saturator: Saturator,
    cab_sim: CabSim<2048>,
    gain: f32,
}

impl Process for Channel {
    fn process(&mut self, sample: f32) -> f32 {
        let s = self.gate.process(sample);
        let s = self.compressor.process(s);
        let s = self.eq.process(s);
        let s = self.saturator.process(s);
        let s = self.cab_sim.process(s);
        s * self.gain
    }
}
```

This gives direct access to each processor for parameter control.

### Biquad Filter (`biquad.rs`)

Standard biquad implementation with coefficient calculation for:

- Low shelf
- High shelf  
- Peaking (parametric)

```rust
pub struct Biquad {
    // coefficients: b0, b1, b2, a1, a2
    // state: x1, x2, y1, y2
}

impl Biquad {
    pub fn low_shelf(freq: f32, gain_db: f32, sample_rate: f32) -> Self;
    pub fn high_shelf(freq: f32, gain_db: f32, sample_rate: f32) -> Self;
    pub fn peaking(freq: f32, gain_db: f32, q: f32, sample_rate: f32) -> Self;
}
```

### Three-Band EQ

Combine three biquads (low shelf, peaking mid, high shelf) with individual bypass toggles:

```rust
pub struct ThreeBandEq {
    low: Biquad,
    mid: Biquad,
    high: Biquad,
    low_enabled: bool,
    mid_enabled: bool,
    high_enabled: bool,
}
```

**Default values (GUITAR mode):**
| Band | Type | Frequency | Boost | Q |
|------|------|-----------|-------|---|
| L | Low shelf | 200 Hz | +6 dB | - |
| M | Peaking | 800 Hz | +6 dB | 1.0 |
| H | High shelf | 3 kHz | +6 dB | - |

**Default values (BASS/DI mode):**
| Band | Type | Frequency | Boost | Q |
|------|------|-----------|-------|---|
| L | Low shelf | 100 Hz | +6 dB | - |
| M | Peaking | 500 Hz | +6 dB | 1.0 |
| H | High shelf | 2 kHz | +6 dB | - |

### Compressor (`compressor.rs`)

Feed-forward compressor with fixed parameters:

```rust
pub struct Compressor {
    enabled: bool,
    // internal state: envelope follower, gain reduction
}
```

**Fixed parameters:**
- Ratio: 4:1
- Threshold: -20 dB
- Attack: 15 ms (GUITAR) / 20 ms (BASS)
- Release: 100 ms
- Knee: soft

Only `enabled` is controllable at runtime.

### Saturator (`saturator.rs`)

Soft clipping using tanh waveshaper:

```rust
pub struct Saturator {
    enabled: bool,
    drive: f32,      // fixed internal drive amount
    output_comp: f32 // compensation gain
}
```

**Implementation:**
1. Apply drive gain (+12 dB)
2. Soft clip via `tanhf()`
3. Apply output compensation to normalize level

Only `enabled` is controllable at runtime.

### Cabinet Simulator (`cab_sim.rs`)

FIR convolution with loaded impulse response:

```rust
pub struct CabSim<const N: usize> {
    enabled: bool,
    ir: [f32; N],           // impulse response coefficients
    buffer: [f32; N],       // circular buffer for input history
    write_pos: usize,
}
```

- Use direct-form FIR convolution (not FFT)
- Fixed max length, e.g., `N = 2048` samples (~42ms at 48kHz)
- `enabled = false` in BASS/DI mode, `enabled = true` in GUITAR mode

Provide a method to load/replace the IR:

```rust
impl<const N: usize> CabSim<N> {
    pub fn set_ir(&mut self, ir: &[f32]);
}
```

### Channel Strip (`channel.rs`)

Combines all processors for one channel:

```rust
pub enum ChannelMode {
    Guitar,
    Bass, // DI mode
}

pub struct Channel<const IR_LEN: usize> {
    mode: ChannelMode,
    compressor: Compressor,
    eq: ThreeBandEq,
    saturator: Saturator,
    cab_sim: CabSim<IR_LEN>,
    gain: f32, // output gain, 0.0 to ~2.0
}

impl<const IR_LEN: usize> Channel<IR_LEN> {
    pub fn set_mode(&mut self, mode: ChannelMode);
    pub fn set_comp_enabled(&mut self, enabled: bool);
    pub fn set_grit_enabled(&mut self, enabled: bool);
    pub fn set_eq_low(&mut self, enabled: bool);
    pub fn set_eq_mid(&mut self, enabled: bool);
    pub fn set_eq_high(&mut self, enabled: bool);
    pub fn set_gain(&mut self, gain: f32);
    pub fn set_ir(&mut self, ir: &[f32]);
}
```

**Signal flow:**
```
Input → Compressor → EQ (L/M/H) → Saturator → Cab Sim → Gain → Output
```

When mode changes to GUITAR:
- Cab sim enabled
- EQ frequencies set to guitar values
- Compressor attack set to 15ms

When mode changes to BASS:
- Cab sim disabled
- EQ frequencies set to bass values
- Compressor attack set to 20ms

---

## Desktop Crate (`desktop/`)

### Dependencies

```toml
[dependencies]
dsp = { path = "../dsp" }
cpal = "0.15"
eframe = "0.27"
egui = "0.27"
hound = "3.5"           # WAV file reading
rubato = "0.15"         # resampling (if needed)
```

### Audio (`audio.rs`)

- Use cpal to open the default audio input and output
- Process stereo (2 channels)
- Sample rate: 48 kHz preferred, resample if device differs
- Buffer size: as low as stable (64-256 samples)
- Pass samples through two `Channel` instances
- Handle mono/stereo output mode

```rust
pub struct AudioEngine {
    channel1: Channel<2048>,
    channel2: Channel<2048>,
    mono_mode: bool,
}
```

**Output routing:**
- Stereo mode: Ch1 → Left, Ch2 → Right
- Mono mode: (Ch1 + Ch2) / 2 → both Left and Right

### IR Loader (`ir_loader.rs`)

```rust
pub fn load_ir(path: &Path, target_sample_rate: u32, max_length: usize) -> Result<Vec<f32>, Error>;
```

- Read WAV file using `hound`
- Convert to mono if stereo (sum and normalize)
- Resample to target sample rate if needed (use `rubato`)
- Trim or zero-pad to `max_length`
- Normalize peak to 1.0

### UI (`ui.rs`)

egui interface mirroring the hardware layout. Two channel strips side by side, plus global controls.

**Per channel:**
- GAIN: vertical slider (0% to 100%)
- GUITAR/BASS: toggle switch or two radio buttons
- COMP: toggle button (illuminated when on)
- GRIT: toggle button (illuminated when on)
- L: toggle button
- M: toggle button  
- H: toggle button
- IR file: button to open file picker + display loaded filename (only shown in GUITAR mode)

**Global:**
- MONO/STEREO: toggle switch
- Input device selector (dropdown)
- Output device selector (dropdown)
- Sample rate display
- Buffer size display / selector

**Visual style:**
- Dark theme (matching the hardware aesthetic)
- Minimal, clean layout
- Buttons should visually indicate on/off state (lit up when enabled)

### Main (`main.rs`)

- Initialize audio engine
- Initialize egui app
- Wire UI controls to audio engine parameters (use `Arc<Mutex<>>` or message passing)
- Handle IR loading via file dialog

---

## Constants and Defaults

```rust
pub const SAMPLE_RATE: u32 = 48000;
pub const IR_MAX_LENGTH: usize = 2048;

// Compressor
pub const COMP_RATIO: f32 = 4.0;
pub const COMP_THRESHOLD_DB: f32 = -20.0;
pub const COMP_ATTACK_GUITAR_MS: f32 = 15.0;
pub const COMP_ATTACK_BASS_MS: f32 = 20.0;
pub const COMP_RELEASE_MS: f32 = 100.0;

// EQ - Guitar mode
pub const EQ_LOW_FREQ_GUITAR: f32 = 200.0;
pub const EQ_MID_FREQ_GUITAR: f32 = 800.0;
pub const EQ_HIGH_FREQ_GUITAR: f32 = 3000.0;

// EQ - Bass mode  
pub const EQ_LOW_FREQ_BASS: f32 = 100.0;
pub const EQ_MID_FREQ_BASS: f32 = 500.0;
pub const EQ_HIGH_FREQ_BASS: f32 = 2000.0;

// EQ - shared
pub const EQ_BOOST_DB: f32 = 6.0;
pub const EQ_MID_Q: f32 = 1.0;

// Saturator
pub const SATURATOR_DRIVE_DB: f32 = 12.0;

// Gain range
pub const GAIN_MIN: f32 = 0.0;
pub const GAIN_MAX: f32 = 2.0; // +6dB
```

---

## Implementation Order

1. **DSP crate foundation**
   - `Process` trait
   - `Biquad` with shelf and peaking filters
   
2. **DSP processors**
   - `ThreeBandEq`
   - `Compressor`
   - `Saturator`
   - `CabSim`
   - `Channel` (combining all)

3. **Desktop audio**
   - Basic cpal setup with passthrough
   - Wire in single channel processing
   - Add second channel

4. **IR loading**
   - WAV loading with hound
   - Resampling with rubato
   - Integration with CabSim

5. **Desktop UI**
   - Basic egui window with controls
   - Wire controls to audio engine
   - File picker for IR loading
   - Polish and dark theme

6. **Testing and tuning**
   - Test with real guitar/bass
   - Tune default values
   - Adjust EQ frequencies and boost amounts

---

## Notes for Daisy Port (Future)

- The `dsp` crate should compile with `no_std` — verify this works before starting Daisy work
- IR will need to be embedded as `const` array or loaded from SD (if available)
- GPIO reads for toggles, ADC reads for pots
- Use libdaisy-rust or raw STM32H7 PAC for hardware access
- Same `Channel` struct, different I/O wrapper

---

## Hardware Reference (for UI design)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ [Jack] [Gain] GUITAR [COMP] ┌─────────┐  [Jack] [Gain] GUITAR [COMP] ┌─────┐│
│               BASS   [GRIT] │ L  M  H │               BASS   [GRIT] │L M H││
│                             └─────────┘                             └─────┘│
│                                                              STEREO        │
│                                                              MONO          │
└─────────────────────────────────────────────────────────────────────────────┘
                              (back: RCA L/R, power)
```
