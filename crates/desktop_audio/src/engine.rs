use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::AudioProcessor;

static DROP_COUNT: AtomicU64 = AtomicU64::new(0);
static UNDERRUN_COUNT: AtomicU64 = AtomicU64::new(0);

/// Configuration for the audio engine.
pub struct AudioEngineConfig {
    /// Preferred sample rate (will fall back if not available).
    pub sample_rate: u32,
    /// Optional input device name (uses default if None).
    pub input_device: Option<String>,
    /// Optional output device name (uses default if None).
    pub output_device: Option<String>,
}

impl Default for AudioEngineConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            input_device: None,
            output_device: None,
        }
    }
}

/// The audio engine manages input/output streams and runs the audio processor.
pub struct AudioEngine {
    _input_stream: cpal::Stream,
    _output_stream: cpal::Stream,
    _stop: Arc<AtomicBool>,
    /// Name of the input device being used.
    pub input_device_name: String,
    /// Name of the output device being used.
    pub output_device_name: String,
    /// Actual sample rate being used.
    pub sample_rate: u32,
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self._stop.store(true, Ordering::Relaxed);
    }
}

fn find_device(host: &cpal::Host, name: &str, input: bool) -> Option<cpal::Device> {
    let name_lower = name.to_lowercase();
    if input {
        host.input_devices().ok()?.find(|d| {
            d.name()
                .map(|n| n.to_lowercase().contains(&name_lower))
                .unwrap_or(false)
        })
    } else {
        host.output_devices().ok()?.find(|d| {
            d.name()
                .map(|n| n.to_lowercase().contains(&name_lower))
                .unwrap_or(false)
        })
    }
}

/// Select a supported stream config, preferring the given sample rate and stereo f32.
fn select_config(
    device: &cpal::Device,
    input: bool,
    target_rate: u32,
) -> Result<cpal::SupportedStreamConfig, Box<dyn std::error::Error>> {
    let supported: Vec<cpal::SupportedStreamConfigRange> = if input {
        device.supported_input_configs()?.collect()
    } else {
        device.supported_output_configs()?.collect()
    };

    let target_rate = cpal::SampleRate(target_rate);

    // Try to find a stereo f32 config at our preferred sample rate.
    for cfg in &supported {
        if cfg.channels() >= 2
            && cfg.sample_format() == cpal::SampleFormat::F32
            && cfg.min_sample_rate() <= target_rate
            && cfg.max_sample_rate() >= target_rate
        {
            return Ok(cfg.clone().with_sample_rate(target_rate));
        }
    }

    // Fall back to any config that supports our preferred rate.
    for cfg in &supported {
        if cfg.min_sample_rate() <= target_rate && cfg.max_sample_rate() >= target_rate {
            return Ok(cfg.clone().with_sample_rate(target_rate));
        }
    }

    // Last resort: use the device default.
    let default = if input {
        device.default_input_config()?
    } else {
        device.default_output_config()?
    };
    Ok(default)
}

impl AudioEngine {
    /// Start the audio engine with the given processor and configuration.
    pub fn start<P: AudioProcessor>(
        mut processor: P,
        config: AudioEngineConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let host = cpal::default_host();

        let input_device = match &config.input_device {
            Some(name) => find_device(&host, name, true)
                .ok_or_else(|| format!("input device '{name}' not found"))?,
            None => host
                .default_input_device()
                .ok_or("no default input device found")?,
        };
        let output_device = match &config.output_device {
            Some(name) => find_device(&host, name, false)
                .ok_or_else(|| format!("output device '{name}' not found"))?,
            None => host
                .default_output_device()
                .ok_or("no default output device found")?,
        };

        let in_name = input_device.name().unwrap_or_else(|_| "unknown".into());
        let out_name = output_device.name().unwrap_or_else(|_| "unknown".into());
        eprintln!("Input device:  {in_name}");
        eprintln!("Output device: {out_name}");

        let input_config = select_config(&input_device, true, config.sample_rate)?;
        let output_config = select_config(&output_device, false, config.sample_rate)?;

        let actual_sample_rate = output_config.sample_rate().0;

        eprintln!(
            "Input config:  {} ch, {} Hz, {:?}",
            input_config.channels(),
            input_config.sample_rate().0,
            input_config.sample_format()
        );
        eprintln!(
            "Output config: {} ch, {} Hz, {:?}",
            output_config.channels(),
            output_config.sample_rate().0,
            output_config.sample_format()
        );

        let input_channels = input_config.channels() as usize;
        let output_channels = output_config.channels() as usize;

        // Lock-free ring buffer between input and output callbacks.
        let (mut producer, mut consumer) = rtrb::RingBuffer::<[f32; 2]>::new(2048);

        // Pre-fill with silence so the output has a cushion against clock drift.
        for _ in 0..512 {
            let _ = producer.push([0.0; 2]);
        }

        // --- Input stream ---
        let input_stream = input_device.build_input_stream(
            &input_config.into(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let frame_count = data.len() / input_channels.max(1);
                for i in 0..frame_count {
                    let left = data[i * input_channels];
                    // Only use input channel 1 for ch1; ch2 is silent.
                    if producer.push([left, 0.0]).is_err() {
                        DROP_COUNT.fetch_add(1, Ordering::Relaxed);
                    }
                }
            },
            |err| eprintln!("input stream error: {err}"),
            None,
        )?;

        // Scratch buffers for batch processing
        let max_buffer_size = 4096;
        let mut input_buffer: Vec<[f32; 2]> = vec![[0.0; 2]; max_buffer_size];
        let mut output_buffer: Vec<[f32; 2]> = vec![[0.0; 2]; max_buffer_size];

        // --- Output stream ---
        let output_stream = output_device.build_output_stream(
            &output_config.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let frame_count = data.len() / output_channels.max(1);
                let mut last_frame = [0.0f32; 2];

                // Collect input frames into buffer
                for i in 0..frame_count {
                    let input_frame = match consumer.pop() {
                        Ok(f) => f,
                        Err(_) => {
                            UNDERRUN_COUNT.fetch_add(1, Ordering::Relaxed);
                            last_frame // hold previous frame instead of zero
                        }
                    };
                    last_frame = input_frame;
                    input_buffer[i] = input_frame;
                }

                // Process entire buffer at once
                processor.process_buffer(
                    &input_buffer[..frame_count],
                    &mut output_buffer[..frame_count],
                );

                // Write output
                for i in 0..frame_count {
                    let [left, right] = output_buffer[i];
                    let base = i * output_channels;
                    if output_channels >= 2 {
                        data[base] = left;
                        data[base + 1] = right;
                        for ch in 2..output_channels {
                            data[base + ch] = 0.0;
                        }
                    } else {
                        data[base] = (left + right) / 2.0;
                    }
                }
            },
            |err| eprintln!("output stream error: {err}"),
            None,
        )?;

        // Monitor thread — prints only when drops or underruns occur.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_monitor = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop_monitor.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let drops = DROP_COUNT.swap(0, Ordering::Relaxed);
                let underruns = UNDERRUN_COUNT.swap(0, Ordering::Relaxed);
                if drops > 0 || underruns > 0 {
                    eprintln!("[audio] drops={drops} underruns={underruns}");
                }
            }
        });

        input_stream.play()?;
        output_stream.play()?;

        Ok(AudioEngine {
            _input_stream: input_stream,
            _output_stream: output_stream,
            _stop: stop,
            input_device_name: in_name,
            output_device_name: out_name,
            sample_rate: actual_sample_rate,
        })
    }
}

/// List available input device names.
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// List available output device names.
pub fn list_output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devices| devices.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}
