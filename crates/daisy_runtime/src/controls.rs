//! Hardware control abstractions for buttons and potentiometers.

/// A debounced button that detects press edges.
pub struct Button {
    state: bool,
    debounce_count: u8,
    debounce_threshold: u8,
}

impl Button {
    /// Create a new button with the given debounce threshold.
    /// The threshold is the number of consecutive reads required to register a state change.
    pub fn new(debounce_threshold: u8) -> Self {
        Self {
            state: false,
            debounce_count: 0,
            debounce_threshold,
        }
    }

    /// Poll the button with a raw reading and return true if a press edge was detected.
    /// Pass `true` if the button is currently pressed, `false` otherwise.
    /// Call this at a regular interval (e.g., 100Hz).
    pub fn poll(&mut self, pressed: bool) -> bool {
        if pressed != self.state {
            self.debounce_count += 1;
            if self.debounce_count >= self.debounce_threshold {
                self.debounce_count = 0;
                self.state = pressed;
                // Return true only on press (not release).
                return self.state;
            }
        } else {
            self.debounce_count = 0;
        }
        false
    }

    /// Get the current debounced state (true = pressed).
    pub fn is_pressed(&self) -> bool {
        self.state
    }
}

/// A potentiometer that scales raw ADC values to a range.
pub struct Pot {
    min: f32,
    max: f32,
}

impl Pot {
    /// Create a new potentiometer scaling helper with the given output range.
    pub fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    /// Convert a raw ADC value (0-65535 for 16-bit) to the scaled output range.
    pub fn scale(&self, raw: u32) -> f32 {
        let normalized = raw as f32 / 65535.0;
        self.min + normalized * (self.max - self.min)
    }
}
