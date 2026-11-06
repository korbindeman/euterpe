use eframe::egui;
use std::path::Path;
use std::sync::Arc;

use desktop_audio::{
    AudioEngine, AudioEngineConfig, list_input_devices, list_output_devices, load_ir,
};
use preamp::audio_app::{ParamKind, ParamModel, ParamValue};
use preamp::dsp::cab_sim::IR_MAX_LENGTH;
use preamp::model;

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
            let _ = ch1.set_param(model::param::SATURATOR_ENABLED, ParamValue::Bool(false));
        }
        if let Ok(mut ch2) = state.ch2.lock() {
            let _ = ch2.set_param(model::param::SATURATOR_ENABLED, ParamValue::Bool(false));
        }

        // Load default IR for both channels
        if let Ok(ir) = load_ir(
            Path::new(DEFAULT_GUITAR_IR),
            preamp::SAMPLE_RATE,
            IR_MAX_LENGTH,
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
                    for section in model::UI_SECTIONS {
                        let has_visible_controls = model::UI_CONTROLS.iter().any(|control| {
                            control.section == *section
                                && model::shows_ui_param(&channel, control.param_id)
                        });
                        if !has_visible_controls {
                            continue;
                        }

                        ui.separator();
                        ui.heading(*section);
                        for control in model::UI_CONTROLS.iter() {
                            if control.section != *section {
                                continue;
                            }
                            if !model::shows_ui_param(&channel, control.param_id) {
                                continue;
                            }
                            render_param(ui, &mut *channel, control.param_id);
                        }
                    }

                    // Cab Sim asset handling remains frontend-specific.
                    let show_cab_ir = model::shows_ui_param(&channel, model::param::CAB_ENABLED);
                    let mut browse_clicked = false;
                    if show_cab_ir {
                        ui.label("Load Cabinet IR");
                        browse_clicked = ui.button("Browse...").clicked();
                        if !browse_clicked && !ir_filename.is_empty() {
                            ui.label(ir_filename.as_str());
                        }
                    }

                    // Handle IR file dialog after releasing the channel lock
                    drop(channel);
                    if browse_clicked
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("WAV", &["wav"])
                            .pick_file()
                    {
                        match desktop_audio::load_ir(&path, preamp::SAMPLE_RATE, IR_MAX_LENGTH) {
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
            egui::ScrollArea::vertical().show(ui, |ui| {
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

fn render_param<M: ParamModel>(ui: &mut egui::Ui, model: &mut M, param_id: &str) {
    let Some(spec) = model.app_spec().param(param_id).copied() else {
        return;
    };

    match spec.kind {
        ParamKind::Toggle { .. } => {
            let mut value = matches!(model.get_param(param_id), Some(ParamValue::Bool(true)));
            if ui.checkbox(&mut value, spec.label).changed() {
                let _ = model.set_param(param_id, ParamValue::Bool(value));
            }
        }
        ParamKind::Knob { min, max, .. } => {
            let mut value = match model.get_param(param_id) {
                Some(ParamValue::Float(value)) => value,
                _ => return,
            };
            if ui
                .add(egui::Slider::new(&mut value, min..=max).text(spec.label))
                .changed()
            {
                let _ = model.set_param(param_id, ParamValue::Float(value));
            }
        }
        ParamKind::Choice { options, .. } => {
            let current = match model.get_param(param_id) {
                Some(ParamValue::Choice(index)) => index,
                _ => 0,
            };
            ui.horizontal(|ui| {
                ui.label(spec.label);
                for (index, option) in options.iter().enumerate() {
                    if ui
                        .selectable_label(current == index as u8, *option)
                        .clicked()
                    {
                        let _ = model.set_param(param_id, ParamValue::Choice(index as u8));
                    }
                }
            });
        }
    }
}
