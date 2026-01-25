mod state;
mod ui;

use eframe::egui;
use std::sync::Arc;

use desktop_audio::{AudioEngine, AudioEngineConfig};
use state::SharedState;

fn main() -> eframe::Result {
    let state = Arc::new(SharedState::new());

    let processor = state::PreampProcessor::new(Arc::clone(&state));
    let config = AudioEngineConfig {
        sample_rate: preamp::SAMPLE_RATE,
        ..Default::default()
    };

    let (engine, input_name, output_name) = match AudioEngine::start(processor, config) {
        Ok(engine) => {
            let iname = engine.input_device_name.clone();
            let oname = engine.output_device_name.clone();
            (Some(engine), iname, oname)
        }
        Err(e) => {
            eprintln!("Failed to start audio engine: {e}");
            (None, String::new(), String::new())
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 400.0])
            .with_title("Instrument Preamp"),
        ..Default::default()
    };

    eframe::run_native(
        "Instrument Preamp",
        options,
        Box::new(move |cc| {
            Ok(Box::new(ui::PreampApp::new(
                cc,
                Arc::clone(&state),
                engine,
                input_name,
                output_name,
            )))
        }),
    )
}
