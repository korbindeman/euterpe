#[allow(unused_imports)]
use micromath::F32Ext;

/// Default smoothing time in milliseconds for knob parameters.
const DEFAULT_SMOOTH_TIME_MS: f32 = 10.0;

/// Toggle parameter — binary on/off (e.g., bypass, mode switch)
pub struct Toggle {
    pub name: &'static str,
    value: bool,
    dirty: bool,
}

impl Toggle {
    pub const fn new(name: &'static str, value: bool) -> Self {
        Self {
            name,
            value,
            dirty: true, // Start dirty so initial state is processed
        }
    }

    pub fn get(&self) -> bool {
        self.value
    }

    pub fn set(&mut self, value: bool) {
        if self.value != value {
            self.value = value;
            self.dirty = true;
        }
    }

    /// Set from normalized 0.0..1.0 range (for hardware ADC input).
    /// Values >= 0.5 are true, < 0.5 are false.
    pub fn set_normalized(&mut self, value: f32) {
        self.set(value >= 0.5);
    }

    /// Returns and clears the dirty flag.
    pub fn take_dirty(&mut self) -> bool {
        let was_dirty = self.dirty;
        self.dirty = false;
        was_dirty
    }
}

/// Knob parameter — continuous value over a range (e.g., gain, frequency, Q)
/// Includes built-in smoothing to avoid clicks when values change.
pub struct Knob {
    pub name: &'static str,
    value: f32,
    min: f32,
    max: f32,
    dirty: bool,
    // Smoothing state
    smoothed: f32,
    smooth_coeff: f32,
}

impl Knob {
    /// Create a new knob. Use `init()` to set up smoothing with sample rate.
    pub const fn new(name: &'static str, value: f32, min: f32, max: f32) -> Self {
        Self {
            name,
            value,
            min,
            max,
            dirty: true,
            smoothed: value,
            smooth_coeff: 0.0, // No smoothing until init() is called
        }
    }

    /// Initialize smoothing with the given sample rate.
    /// Call this once after construction.
    pub fn init(&mut self, sample_rate: u32) {
        self.smooth_coeff = Self::calc_coeff(DEFAULT_SMOOTH_TIME_MS, sample_rate);
        self.smoothed = self.value;
    }

    fn calc_coeff(ms: f32, sample_rate: u32) -> f32 {
        if ms <= 0.0 || sample_rate == 0 {
            0.0
        } else {
            (-1.0 / (ms * 0.001 * sample_rate as f32)).exp()
        }
    }

    /// Get the target value (unsmoothed).
    pub fn get(&self) -> f32 {
        self.value
    }

    /// Get the smoothed value and advance by one sample.
    /// Use this in the audio processing loop.
    #[inline]
    pub fn get_smoothed(&mut self) -> f32 {
        self.smoothed = self.smooth_coeff * self.smoothed + (1.0 - self.smooth_coeff) * self.value;
        self.smoothed
    }

    pub fn min(&self) -> f32 {
        self.min
    }

    pub fn max(&self) -> f32 {
        self.max
    }

    pub fn set(&mut self, value: f32) {
        let clamped = if value < self.min {
            self.min
        } else if value > self.max {
            self.max
        } else {
            value
        };
        if self.value != clamped {
            self.value = clamped;
            self.dirty = true;
        }
    }

    /// Set from normalized 0.0..1.0 range (for hardware ADC input).
    /// Maps to min..max range.
    pub fn set_normalized(&mut self, normalized: f32) {
        let clamped = normalized.clamp(0.0, 1.0);
        let value = self.min + clamped * (self.max - self.min);
        self.set(value);
    }

    /// Returns and clears the dirty flag.
    pub fn take_dirty(&mut self) -> bool {
        let was_dirty = self.dirty;
        self.dirty = false;
        was_dirty
    }

    /// Snap smoothed value to target immediately (skip smoothing).
    pub fn snap(&mut self) {
        self.smoothed = self.value;
    }
}

/// A simple one-pole smoother for parameter interpolation.
/// Call `tick()` once per sample to get the smoothed value.
pub struct Smoother {
    target: f32,
    current: f32,
    coeff: f32,
}

impl Smoother {
    /// Create a new smoother.
    /// `smooth_time_ms` is the approximate time to reach 63% of the target.
    pub fn new(initial: f32, smooth_time_ms: f32, sample_rate: u32) -> Self {
        Self {
            target: initial,
            current: initial,
            coeff: Self::calc_coeff(smooth_time_ms, sample_rate),
        }
    }

    fn calc_coeff(ms: f32, sample_rate: u32) -> f32 {
        if ms <= 0.0 {
            0.0 // Instant (no smoothing)
        } else {
            (-1.0 / (ms * 0.001 * sample_rate as f32)).exp()
        }
    }

    /// Set a new target value.
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Get the current smoothed value and advance by one sample.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        self.current = self.coeff * self.current + (1.0 - self.coeff) * self.target;
        self.current
    }

    /// Get the current value without advancing.
    pub fn get(&self) -> f32 {
        self.current
    }

    /// Check if the smoother has reached its target (within epsilon).
    pub fn is_settled(&self) -> bool {
        (self.current - self.target).abs() < 1e-6
    }

    /// Immediately set current value to target (skip smoothing).
    pub fn snap(&mut self) {
        self.current = self.target;
    }

    /// Immediately set both current and target to a value.
    pub fn reset(&mut self, value: f32) {
        self.current = value;
        self.target = value;
    }
}
