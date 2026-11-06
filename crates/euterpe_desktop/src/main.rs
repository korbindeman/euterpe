//! `euterpe-desktop`: play a Euterpe device through an audio interface.
//!
//! ```text
//! euterpe-desktop [<device>] [--in <name>] [--out <name>] [--rate <hz>]
//! ```
//!
//! Interface names match by substring, case-insensitively. Without them, a
//! connected Volt is used, else the system defaults.

use euterpe_desktop::app::{App, Config};

fn main() -> eframe::Result {
    let mut config = Config {
        device: euterpe_registry::DEVICES
            .first()
            .map_or_else(String::new, |(n, _)| (*n).to_string()),
        input: None,
        output: None,
        sample_rate: 48_000,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--in" => config.input = args.next(),
            "--out" => config.output = args.next(),
            "--rate" => {
                config.sample_rate = args.next().and_then(|r| r.parse().ok()).unwrap_or(48_000);
            }
            "-h" | "--help" => {
                println!("euterpe-desktop [<device>] [--in <name>] [--out <name>] [--rate <hz>]");
                println!("devices:");
                for (name, desc) in euterpe_registry::DEVICES {
                    println!("  {name:<14} {desc}");
                }
                return Ok(());
            }
            device => config.device = device.to_string(),
        }
    }
    eframe::run_native(
        "Euterpe",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([640.0, 560.0])
                .with_title("Euterpe"),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(App::new(cc, config)))),
    )
}
