//! Live Wurli proof of concept: guitar in (input 1) -> Wurli -> both outputs.
//!
//! `cargo run -p wurli_desktop --release`

use std::sync::{Arc, Mutex};

use desktop_audio::{
    AudioEngine, AudioEngineConfig, AudioProcessor, KnobUi, ToggleUi, list_input_devices,
    list_output_devices,
};
use dsp::Process;
use dsp::wurli::Wurli;
use eframe::egui;

const SAMPLE_RATE: u32 = 48_000;

struct WurliProcessor {
    wurli: Arc<Mutex<Wurli>>,
}

impl AudioProcessor for WurliProcessor {
    fn process_buffer(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]]) {
        let mut wurli = self.wurli.lock().unwrap();
        for (frame_in, frame_out) in input.iter().zip(output.iter_mut()) {
            let y = wurli.process(frame_in[0]).clamp(-1.0, 1.0);
            *frame_out = [y, y];
        }
    }
}

struct App {
    wurli: Arc<Mutex<Wurli>>,
    engine: Option<AudioEngine>,
    input_devices: Vec<String>,
    output_devices: Vec<String>,
    selected_input: String,
    selected_output: String,
    error: Option<String>,
}

impl App {
    fn start(&mut self) {
        self.engine = None;
        let config = AudioEngineConfig {
            sample_rate: SAMPLE_RATE,
            input_device: non_empty(&self.selected_input),
            output_device: non_empty(&self.selected_output),
        };
        let processor = WurliProcessor {
            wurli: Arc::clone(&self.wurli),
        };
        match AudioEngine::start(processor, config) {
            Ok(engine) => {
                self.selected_input = engine.input_device_name.clone();
                self.selected_output = engine.output_device_name.clone();
                self.engine = Some(engine);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Audio failed to start: {e}")),
        }
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// Prefer the Volt 1 (as the preamp app does), else system default.
fn pick(devices: &[String]) -> String {
    devices
        .iter()
        .find(|d| d.to_lowercase().contains("volt 1"))
        .cloned()
        .unwrap_or_default()
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut restart = false;
        egui::TopBottomPanel::top("devices").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (label, list, selected) in [
                    ("Input:", &self.input_devices, &mut self.selected_input),
                    ("Output:", &self.output_devices, &mut self.selected_output),
                ] {
                    ui.label(label);
                    let prev = selected.clone();
                    egui::ComboBox::from_id_salt(label)
                        .selected_text(prev.as_str())
                        .show_ui(ui, |ui| {
                            for name in list {
                                ui.selectable_value(selected, name.clone(), name);
                            }
                        });
                    restart |= *selected != prev;
                    ui.add_space(12.0);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Wurli");
            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, err);
            }
            let mut wurli = self.wurli.lock().unwrap();
            let p = wurli.params_mut();
            p.enabled.ui(ui);
            ui.separator();
            ui.label("Reed");
            p.purity.ui(ui);
            p.decay.ui(ui);
            ui.label("Pickup / amp");
            p.bark.ui(ui);
            p.attack.ui(ui);
            p.tone.ui(ui);
            ui.label("Tremolo");
            p.trem_rate.ui(ui);
            p.trem_depth.ui(ui);
            ui.separator();
            p.mix.ui(ui);
            p.level.ui(ui);
        });

        if restart {
            self.start();
        }
    }
}

fn main() -> eframe::Result {
    let input_devices = list_input_devices();
    let output_devices = list_output_devices();
    let mut app = App {
        wurli: Arc::new(Mutex::new(Wurli::with_sample_rate(SAMPLE_RATE))),
        engine: None,
        selected_input: pick(&input_devices),
        selected_output: pick(&output_devices),
        input_devices,
        output_devices,
        error: None,
    };
    app.start();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 520.0])
            .with_title("Wurli"),
        ..Default::default()
    };
    eframe::run_native(
        "Wurli",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(app))
        }),
    )
}
