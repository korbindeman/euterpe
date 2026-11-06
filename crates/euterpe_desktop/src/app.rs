//! The window: device picker, audio setup, input source, a control panel
//! generated from the device's schema, meters and load.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use desktop_audio::{AudioEngine, AudioEngineConfig, list_input_devices, list_output_devices};
use eframe::egui;
use euterpe_core::{ParamKind, ParamValue, fastmath};

use crate::host::{Host, Shared, Source};
use crate::params::ParamBank;
use crate::runner::{Build, Built};

/// What the app starts with; the command line fills it in.
pub struct Config {
    pub device: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub sample_rate: u32,
}

/// The device currently loaded, minus the runner (which the audio thread owns).
struct Loaded {
    name: &'static str,
    description: &'static str,
    bank: Arc<ParamBank>,
    budget_cycles_per_block: u32,
}

pub struct App {
    shared: Arc<Shared>,
    engine: Option<AudioEngine>,
    sample_rate: u32,
    status: String,
    device: Option<Loaded>,
    inputs: Vec<String>,
    outputs: Vec<String>,
    input: String,
    output: String,
    file_name: String,
    meters: Meters,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let inputs = list_input_devices();
        let outputs = list_output_devices();
        // Prefer an interface the user named, else a Volt, else the default.
        let pick = |given: Option<String>, list: &[String]| {
            given
                .or_else(|| {
                    list.iter()
                        .find(|d| d.to_lowercase().contains("volt"))
                        .cloned()
                })
                .unwrap_or_default()
        };
        let mut app = Self {
            shared: Arc::new(Shared::default()),
            engine: None,
            sample_rate: config.sample_rate,
            status: String::new(),
            device: None,
            input: pick(config.input, &inputs),
            output: pick(config.output, &outputs),
            inputs,
            outputs,
            file_name: String::new(),
            meters: Meters::default(),
        };
        app.restart_audio();
        app.load_device(&config.device);
        app
    }

    fn restart_audio(&mut self) {
        self.engine = None;
        let name = |s: &String| (!s.is_empty()).then(|| s.clone());
        let config = AudioEngineConfig {
            sample_rate: self.sample_rate,
            input_device: name(&self.input),
            output_device: name(&self.output),
        };
        #[expect(clippy::cast_precision_loss, reason = "sample rates are exact in f32")]
        let host = Host::new(Arc::clone(&self.shared), self.sample_rate as f32);
        match AudioEngine::start(host, config) {
            Ok(engine) => {
                self.status = format!(
                    "{} → {} at {} Hz",
                    engine.input_device_name, engine.output_device_name, engine.sample_rate
                );
                self.input.clone_from(&engine.input_device_name);
                self.output.clone_from(&engine.output_device_name);
                if engine.sample_rate != self.sample_rate {
                    // The interface picked another rate: rebuild the host for it.
                    self.sample_rate = engine.sample_rate;
                    self.engine = None;
                    return self.restart_audio();
                }
                self.engine = Some(engine);
            }
            Err(e) => self.status = format!("no audio: {e}"),
        }
        // A new host has no runner yet: hand it the current device again.
        if let Some(name) = self.device.as_ref().map(|d| d.name) {
            let values: Vec<ParamValue> = self.device.as_ref().map_or_else(Vec::new, |d| {
                (0..d.bank.schema().len()).map(|i| d.bank.get(i)).collect()
            });
            self.load_device(name);
            if let Some(d) = &self.device {
                for (i, v) in values.into_iter().enumerate() {
                    d.bank.set(i, v);
                }
            }
        }
    }

    fn load_device(&mut self, name: &str) {
        #[expect(clippy::cast_precision_loss, reason = "sample rates are exact in f32")]
        let build = Build {
            sample_rate: self.sample_rate as f32,
        };
        let Some(Built {
            name,
            description,
            bank,
            runner,
            budget_cycles_per_block,
            ..
        }) = euterpe_registry::visit(name, build)
        else {
            self.status = format!("unknown device `{name}`");
            return;
        };
        if let Ok(mut pending) = self.shared.pending.lock() {
            *pending = Some(runner);
        }
        self.device = Some(Loaded {
            name,
            description,
            bank,
            budget_cycles_per_block,
        });
    }

    fn load_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .pick_file()
        else {
            return;
        };
        match euterpe_offline::wav::read(&path) {
            Ok(audio) => {
                let Some(first) = audio.channels.first() else {
                    self.status = "that file has no audio".into();
                    return;
                };
                let samples = resample_linear(first, audio.sample_rate, self.sample_rate);
                if let Ok(mut f) = self.shared.pending_file.lock() {
                    *f = Some(Arc::new(samples));
                }
                self.file_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.shared.set_source(Source::File);
            }
            Err(e) => self.status = format!("reading {}: {e}", path.display()),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Device");
            let current = self.device.as_ref().map_or("", |d| d.name);
            let mut chosen = None;
            egui::ComboBox::from_id_salt("device")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (name, description) in euterpe_registry::DEVICES {
                        if ui
                            .selectable_label(*name == current, *name)
                            .on_hover_text(*description)
                            .clicked()
                        {
                            chosen = Some(*name);
                        }
                    }
                });
            if let Some(name) = chosen
                && name != current
            {
                self.load_device(name);
            }
            if let Some(d) = &self.device {
                ui.weak(d.description);
            }
        });
        ui.horizontal(|ui| {
            let mut changed = false;
            changed |= device_combo(ui, "In", &mut self.input, &self.inputs);
            changed |= device_combo(ui, "Out", &mut self.output, &self.outputs);
            if changed || ui.button("Restart audio").clicked() {
                self.restart_audio();
            }
        });
        ui.weak(&self.status);
    }

    fn source_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Input");
        let mut source = self.shared.source();
        ui.horizontal_wrapped(|ui| {
            for s in Source::ALL {
                ui.radio_value(&mut source, s, s.label());
            }
        });
        if source != self.shared.source() {
            self.shared.set_source(source);
        }
        match source {
            Source::Sine => {
                let mut hz = self.shared.sine_hz.get();
                if ui
                    .add(
                        egui::Slider::new(&mut hz, 20.0..=20_000.0)
                            .logarithmic(true)
                            .suffix(" Hz")
                            .text("Frequency"),
                    )
                    .changed()
                {
                    self.shared.sine_hz.set(hz);
                }
            }
            Source::File => {
                ui.horizontal(|ui| {
                    if ui.button("Load WAV…").clicked() {
                        self.load_file();
                    }
                    ui.weak(if self.file_name.is_empty() {
                        "no file"
                    } else {
                        &self.file_name
                    });
                });
            }
            Source::Interface | Source::Noise => {}
        }
        if matches!(source, Source::Sine | Source::Noise) {
            let mut level = self.shared.source_level_db.get();
            if ui
                .add(
                    egui::Slider::new(&mut level, -60.0..=0.0)
                        .suffix(" dBFS")
                        .text("Level"),
                )
                .changed()
            {
                self.shared.source_level_db.set(level);
            }
        }
    }

    fn params_panel(&self, ui: &mut egui::Ui) {
        let Some(device) = &self.device else { return };
        ui.horizontal(|ui| {
            ui.heading("Parameters");
            if ui.small_button("Defaults").clicked() {
                device.bank.reset();
            }
        });
        egui::Grid::new("params")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                for (i, desc) in device.bank.schema().iter().enumerate() {
                    ui.label(desc.label).on_hover_text(desc.id);
                    param_widget(ui, &device.bank, i);
                    ui.end_row();
                }
            });
    }

    fn output_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Output");
        let mut bypass = self.shared.bypass.load(Ordering::Relaxed);
        if ui.checkbox(&mut bypass, "Bypass").changed() {
            self.shared.bypass.store(bypass, Ordering::Relaxed);
        }
        let mut out_db = self.shared.output_db.get();
        if ui
            .add(
                egui::Slider::new(&mut out_db, -60.0..=12.0)
                    .suffix(" dB")
                    .text("Output"),
            )
            .changed()
        {
            self.shared.output_db.set(out_db);
        }
        self.meters.update(&self.shared);
        meter(ui, "In", self.meters.input_db);
        meter(ui, "Out", self.meters.output_db);
        let budget = self
            .device
            .as_ref()
            .map_or(0, |d| d.budget_cycles_per_block);
        ui.label(format!(
            "Host load {:.2}% (peak {:.2}%) · {} blocks · Daisy budget {} cycles/block (see `just bench`)",
            100.0 * self.meters.load,
            100.0 * self.meters.load_peak,
            self.shared.blocks.load(Ordering::Relaxed),
            budget
        ));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Old runners are dropped here, off the audio thread.
        if let Ok(mut retired) = self.shared.retired.try_lock() {
            retired.take();
        }
        egui::TopBottomPanel::top("top").show(ctx, |ui| self.top_bar(ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.source_panel(ui);
                ui.separator();
                self.params_panel(ui);
                ui.separator();
                self.output_panel(ui);
            });
        });
        // Meters move, so keep repainting at a modest rate.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

fn device_combo(ui: &mut egui::Ui, label: &str, selected: &mut String, options: &[String]) -> bool {
    let mut changed = false;
    ui.label(label);
    egui::ComboBox::from_id_salt(label)
        .selected_text(selected.as_str())
        .width(220.0)
        .show_ui(ui, |ui| {
            for o in options {
                if ui.selectable_label(o == selected, o).clicked() && o != selected {
                    selected.clone_from(o);
                    changed = true;
                }
            }
        });
    changed
}

/// The widget for one parameter, from its kind.
fn param_widget(ui: &mut egui::Ui, bank: &ParamBank, index: usize) {
    let desc = &bank.schema()[index];
    match (desc.kind, bank.get(index)) {
        (ParamKind::Float { unit, default, .. }, value @ ParamValue::Float(v)) => {
            // Sliders work in normalized position, so tapers feel right.
            let mut n = desc.kind.to_normalized(value);
            let response = ui.add(
                egui::Slider::new(&mut n, 0.0..=1.0)
                    .show_value(false)
                    .text(format!("{} {unit}", format_value(v))),
            );
            if response.double_clicked() {
                bank.set(index, ParamValue::Float(default));
            } else if response.changed() {
                bank.set(index, desc.kind.from_normalized(n));
            }
        }
        (ParamKind::Toggle { .. }, ParamValue::Bool(mut on)) => {
            if ui.checkbox(&mut on, "").changed() {
                bank.set(index, ParamValue::Bool(on));
            }
        }
        (ParamKind::Choice { options, .. }, ParamValue::Choice(mut i)) => {
            let label = options.get(usize::from(i)).copied().unwrap_or("?");
            egui::ComboBox::from_id_salt(desc.id)
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (j, o) in options.iter().enumerate() {
                        if let Ok(j) = u8::try_from(j)
                            && ui.selectable_value(&mut i, j, *o).changed()
                        {
                            bank.set(index, ParamValue::Choice(i));
                        }
                    }
                });
        }
        _ => {
            ui.label("?");
        }
    }
}

fn format_value(v: f32) -> String {
    if v.abs() >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Peak meters with a 20 dB/s fall, read from the audio thread's peaks.
struct Meters {
    input_db: f32,
    output_db: f32,
    load: f32,
    load_peak: f32,
    last: Instant,
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            input_db: -120.0,
            output_db: -120.0,
            load: 0.0,
            load_peak: 0.0,
            last: Instant::now(),
        }
    }
}

impl Meters {
    fn update(&mut self, shared: &Shared) {
        let dt = self.last.elapsed().as_secs_f32();
        self.last = Instant::now();
        let fall =
            |held: f32, peak: f32| fastmath::gain_to_db(peak).max(held - 20.0 * dt).max(-120.0);
        self.input_db = fall(self.input_db, shared.in_peak.take());
        self.output_db = fall(self.output_db, shared.out_peak.take());
        self.load = shared.load.get();
        self.load_peak = self.load_peak.max(shared.load_peak.take());
    }
}

fn meter(ui: &mut egui::Ui, label: &str, db: f32) {
    ui.horizontal(|ui| {
        ui.label(label);
        let fill = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
        let colour = if db > -0.1 {
            egui::Color32::RED
        } else if db > -6.0 {
            egui::Color32::YELLOW
        } else {
            egui::Color32::GREEN
        };
        ui.add(
            egui::ProgressBar::new(fill)
                .fill(colour)
                .desired_width(240.0)
                .text(if db <= -100.0 {
                    "silent".to_string()
                } else {
                    format!("{db:.1} dBFS")
                }),
        );
    });
}

/// Linear-interpolation resampling, for loaded files at another rate.
pub fn resample_linear(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let step = f64::from(from) / f64::from(to);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "lengths and positions of an audio file"
    )]
    {
        let len = (input.len() as f64 / step).floor() as usize;
        (0..len)
            .map(|i| {
                let pos = i as f64 * step;
                let j = pos as usize;
                let frac = (pos - j as f64) as f32;
                let a = input[j];
                let b = input.get(j + 1).copied().unwrap_or(a);
                a + (b - a) * frac
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_duration_and_interpolates() {
        let x: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let y = resample_linear(&x, 24_000, 48_000);
        assert_eq!(y.len(), 200);
        assert!((y[3] - 1.5).abs() < 1e-6);
        assert_eq!(resample_linear(&x, 48_000, 48_000), x);
    }
}
