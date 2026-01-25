//! egui UI extensions for DSP parameters.
//!
//! This module provides extension traits that add `.ui()` methods to
//! DSP parameter types when the `egui` feature is enabled.

use dsp::params::{Knob, Toggle};

/// Extension trait for rendering a Toggle as an egui checkbox.
pub trait ToggleUi {
    fn ui(&mut self, ui: &mut egui::Ui);
}

impl ToggleUi for Toggle {
    fn ui(&mut self, ui: &mut egui::Ui) {
        let mut value = self.get();
        if ui.checkbox(&mut value, self.name).changed() {
            self.set(value);
        }
    }
}

/// Extension trait for rendering a Knob as an egui slider.
pub trait KnobUi {
    fn ui(&mut self, ui: &mut egui::Ui);
}

impl KnobUi for Knob {
    fn ui(&mut self, ui: &mut egui::Ui) {
        let mut value = self.get();
        if ui
            .add(egui::Slider::new(&mut value, self.min()..=self.max()).text(self.name))
            .changed()
        {
            self.set(value);
        }
    }
}
