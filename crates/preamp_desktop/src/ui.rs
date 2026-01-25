use eframe::egui;
use std::path::Path;
use std::sync::Arc;

use desktop_audio::{
    AudioEngine, AudioEngineConfig, KnobUi, ToggleUi, list_input_devices, list_output_devices,
    load_ir,
};
use preamp::ChannelMode;

use crate::state::{PreampProcessor, SharedState};

/// Default IR file for guitar mode.
const DEFAULT_GUITAR_IR: &str = "ir/deluxe_reverb.wav";

pub struct PreampApp {
    state: Arc<SharedState>,
    engine: Option<AudioEngine>,
    // Device selection
    input_devices: Vec<String>,
    output_devices: Vec<String>,
    selected_input: String,
    selected_output: String,
    // IR filenames (for display only)
    ch1_ir_filename: String,
    ch2_ir_filename: String,
}

impl PreampApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        state: Arc<SharedState>,
        engine: Option<AudioEngine>,
        input_name: String,
        output_name: String,
    ) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let input_devices = list_input_devices();
        let output_devices = list_output_devices();

        // Apply default guitar preset: disable saturation and load default IR
        let mut ch1_ir_filename = String::new();
        let mut ch2_ir_filename = String::new();

        if let Ok(mut ch1) = state.ch1.lock() {
            ch1.saturator_params_mut().enabled.set(false);
        }
        if let Ok(mut ch2) = state.ch2.lock() {
            ch2.saturator_params_mut().enabled.set(false);
        }

        // Load default IR for both channels
        if let Ok(ir) = load_ir(
            Path::new(DEFAULT_GUITAR_IR),
            preamp::SAMPLE_RATE,
            preamp::IR_MAX_LENGTH,
        ) {
            ch1_ir_filename = "deluxe_reverb.wav".to_string();
            ch2_ir_filename = "deluxe_reverb.wav".to_string();

            if let Ok(mut pending) = state.ch1_pending_ir.lock() {
                pending.ir = Some(ir.clone());
            }
            if let Ok(mut pending) = state.ch2_pending_ir.lock() {
                pending.ir = Some(ir);
            }
        }

        Self {
            state,
            engine,
            input_devices,
            output_devices,
            selected_input: input_name,
            selected_output: output_name,
            ch1_ir_filename,
            ch2_ir_filename,
        }
    }

    fn channel_strip(&mut self, ui: &mut egui::Ui, channel_index: usize) {
        let label = if channel_index == 0 {
            "Channel 1"
        } else {
            "Channel 2"
        };

        let ir_filename = if channel_index == 0 {
            &mut self.ch1_ir_filename
        } else {
            &mut self.ch2_ir_filename
        };

        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.heading(label);
                ui.add_space(4.0);

                // Lock the channel to access and modify params
                let channel_mutex = if channel_index == 0 {
                    &self.state.ch1
                } else {
                    &self.state.ch2
                };

                if let Ok(mut channel) = channel_mutex.lock() {
                    // Mode selector
                    ui.label("Mode");
                    let mut mode = channel.mode();
                    ui.horizontal(|ui| {
                        if ui
                            .radio_value(&mut mode, ChannelMode::Guitar, "Guitar")
                            .changed()
                        {
                            channel.set_mode(mode);
                        }
                        if ui
                            .radio_value(&mut mode, ChannelMode::Bass, "Bass")
                            .changed()
                        {
                            channel.set_mode(mode);
                        }
                    });
                    ui.add_space(4.0);

                    // Channel gain
                    ui.heading("Channel");
                    channel.params_mut().gain.ui(ui);

                    // Gate
                    ui.separator();
                    ui.heading("Gate");
                    channel.gate_params_mut().enabled.ui(ui);
                    channel.gate_params_mut().threshold.ui(ui);

                    // Compressor
                    ui.separator();
                    ui.heading("Compressor");
                    channel.compressor_params_mut().enabled.ui(ui);
                    channel.compressor_params_mut().threshold.ui(ui);
                    channel.compressor_params_mut().ratio.ui(ui);
                    channel.compressor_params_mut().attack.ui(ui);
                    channel.compressor_params_mut().release.ui(ui);

                    // EQ
                    ui.separator();
                    ui.heading("EQ");
                    channel.eq_params_mut().low_enabled.ui(ui);
                    channel.eq_params_mut().low_gain.ui(ui);
                    channel.eq_params_mut().mid_enabled.ui(ui);
                    channel.eq_params_mut().mid_gain.ui(ui);
                    channel.eq_params_mut().high_enabled.ui(ui);
                    channel.eq_params_mut().high_gain.ui(ui);

                    // Saturator
                    ui.separator();
                    ui.heading("Saturator");
                    channel.saturator_params_mut().enabled.ui(ui);
                    channel.saturator_params_mut().drive.ui(ui);

                    // Cab Sim
                    ui.separator();
                    ui.heading("Cab Sim");
                    channel.cab_sim_params_mut().enabled.ui(ui);

                    ui.add_space(4.0);

                    // IR loader (guitar mode only, cab sim is auto-enabled)
                    let is_guitar = channel.mode() == ChannelMode::Guitar;
                    if is_guitar {
                        ui.separator();
                        ui.label("Load Cabinet IR");
                        if ui.button("Browse...").clicked() {
                            // Release lock before opening file dialog
                            drop(channel);
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("WAV", &["wav"])
                                .pick_file()
                            {
                                match desktop_audio::load_ir(
                                    &path,
                                    preamp::SAMPLE_RATE,
                                    preamp::IR_MAX_LENGTH,
                                ) {
                                    Ok(ir) => {
                                        let filename = path
                                            .file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_default();
                                        *ir_filename = filename;

                                        // Store IR in pending slot for audio thread to pick up
                                        let pending = if channel_index == 0 {
                                            &self.state.ch1_pending_ir
                                        } else {
                                            &self.state.ch2_pending_ir
                                        };
                                        if let Ok(mut p) = pending.lock() {
                                            p.ir = Some(ir);
                                        }
                                    }
                                    Err(e) => {
                                        *ir_filename = format!("Error: {e}");
                                    }
                                }
                            }
                        } else if !ir_filename.is_empty() {
                            ui.label(ir_filename.as_str());
                        }
                    }
                } else {
                    ui.label("Channel locked...");
                }
            });
        });
    }

    fn restart_audio(&mut self) {
        // Drop old engine first
        self.engine = None;

        let processor = PreampProcessor::new(Arc::clone(&self.state));
        let config = AudioEngineConfig {
            sample_rate: preamp::SAMPLE_RATE,
            input_device: Some(self.selected_input.clone()),
            output_device: Some(self.selected_output.clone()),
        };

        match AudioEngine::start(processor, config) {
            Ok(engine) => self.engine = Some(engine),
            Err(e) => eprintln!("Failed to restart audio engine: {e}"),
        }
    }
}

impl eframe::App for PreampApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut restart = false;

        // Device selection panel at the top
        egui::TopBottomPanel::top("devices").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Input:");
                let prev_input = self.selected_input.clone();
                egui::ComboBox::from_id_salt("input_device")
                    .selected_text(&prev_input)
                    .show_ui(ui, |ui| {
                        for name in self.input_devices.iter() {
                            ui.selectable_value(&mut self.selected_input, name.clone(), name);
                        }
                    });
                if self.selected_input != prev_input {
                    restart = true;
                }

                ui.add_space(16.0);

                ui.label("Output:");
                let prev_output = self.selected_output.clone();
                egui::ComboBox::from_id_salt("output_device")
                    .selected_text(&prev_output)
                    .show_ui(ui, |ui| {
                        for name in self.output_devices.iter() {
                            ui.selectable_value(&mut self.selected_output, name.clone(), name);
                        }
                    });
                if self.selected_output != prev_output {
                    restart = true;
                }
            });
        });

        // Channel strips and global controls
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                self.channel_strip(ui, 0);
                ui.add_space(8.0);
                self.channel_strip(ui, 1);
                ui.add_space(16.0);

                // Global controls
                ui.vertical(|ui| {
                    ui.heading("Global");
                    ui.add_space(4.0);

                    let mut mono = self
                        .state
                        .mono_mode
                        .load(std::sync::atomic::Ordering::Relaxed);
                    let label = if mono { "Mono" } else { "Stereo" };
                    if toggle_button(ui, label, &mut mono) {
                        self.state
                            .mono_mode
                            .store(mono, std::sync::atomic::Ordering::Relaxed);
                    }
                });
            });
        });

        // Restart audio engine if device selection changed
        if restart {
            self.restart_audio();
        }
    }
}

fn toggle_button(ui: &mut egui::Ui, label: &str, active: &mut bool) -> bool {
    let color = if *active {
        egui::Color32::from_rgb(80, 160, 80)
    } else {
        egui::Color32::from_rgb(60, 60, 60)
    };
    let button = egui::Button::new(label).fill(color);
    let response = ui.add(button);
    if response.clicked() {
        *active = !*active;
    }
    response.clicked()
}
