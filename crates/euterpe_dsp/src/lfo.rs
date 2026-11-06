//! Low-frequency oscillators: a phase accumulator and the shapes read from
//! its phase.

use euterpe_core::fastmath;

/// A phase in `[0, 1)` turns that advances by a given increment each tick.
///
/// The phase is a `u32` fraction of a turn, so it wraps exactly and never
/// drifts: an f32 accumulator would round every step and run up to 0.1% off
/// pitch at LFO rates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Phasor {
    phase: u32,
}

/// `2^32`: one turn of a [`Phasor`].
const TURN: f32 = 4_294_967_296.0;

impl Phasor {
    /// The current phase, then advance by `inc` turns (`hz / sample_rate`,
    /// clamped to `[0, 0.5]`).
    #[inline(always)]
    pub fn tick(&mut self, inc: f32) -> f32 {
        let p = self.phase();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to [0, 2^31]"
        )]
        let step = (inc.clamp(0.0, 0.5) * TURN) as u32;
        self.phase = self.phase.wrapping_add(step);
        p
    }

    /// The phase in turns, in `[0, 1)`.
    #[must_use]
    #[inline(always)]
    pub fn phase(&self) -> f32 {
        // The top 24 bits convert exactly, so the result is never rounded up
        // to 1.
        #[expect(clippy::cast_precision_loss, reason = "24 bits are exact")]
        let p = (self.phase >> 8_u32) as f32;
        p * (1.0 / 16_777_216.0)
    }

    /// Jump to `phase` turns, wrapped into `[0, 1)`.
    pub fn set_phase(&mut self, phase: f32) {
        let p = phase - fastmath::floor_i32(phase) as f32;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "p is in [0, 1]; 1 saturates to u32::MAX, a hair under a turn"
        )]
        let phase = (p * TURN) as u32;
        self.phase = phase;
    }

    pub fn reset(&mut self) {
        self.phase = 0;
    }
}

/// An LFO waveform, bipolar in `[-1, 1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Sine,
    Triangle,
    /// A square with softened edges (about 4% of the period), so that a
    /// tremolo doesn't click.
    Square,
    /// Rising saw.
    Saw,
}

impl Shape {
    /// All shapes, in choice-param order.
    pub const ALL: [Self; 4] = [Self::Sine, Self::Triangle, Self::Square, Self::Saw];
    /// Labels for a [`ParamKind::choice`](euterpe_core::ParamKind::choice).
    pub const LABELS: &[&str] = &["Sine", "Triangle", "Square", "Saw"];

    /// The shape for a choice index; out of range is [`Shape::Sine`].
    #[must_use]
    pub fn from_index(i: u8) -> Self {
        Self::ALL.get(usize::from(i)).copied().unwrap_or_default()
    }

    /// The value at `phase` turns, `phase` in `[0, 1)`. All shapes start at
    /// 0 rising, except the saw, which starts at -1.
    #[inline(always)]
    #[must_use]
    pub fn at(self, phase: f32) -> f32 {
        match self {
            Self::Sine => fastmath::sin_turns(phase),
            // 0 -> 1 at 1/4, -> -1 at 3/4, -> 0.
            Self::Triangle => {
                let p = phase + 0.25;
                let p = if p >= 1.0 { p - 1.0 } else { p };
                1.0 - 4.0 * (p - 0.5).abs()
            }
            Self::Square => (8.0 * fastmath::sin_turns(phase)).clamp(-1.0, 1.0),
            Self::Saw => 2.0 * phase - 1.0,
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact values at exact phases")]
mod tests {
    use super::*;

    #[test]
    fn phasor_wraps_and_counts_periods() {
        let mut p = Phasor::default();
        let mut wraps = 0;
        let mut last = 0.0;
        for _ in 0..48_100 {
            let ph = p.tick(5.0 / 48_000.0);
            assert!((0.0..1.0).contains(&ph));
            if ph < last {
                wraps += 1;
            }
            last = ph;
        }
        assert_eq!(wraps, 5, "5 Hz for just over a second");
        p.set_phase(-0.25);
        assert_eq!(p.phase(), 0.75);
    }

    #[test]
    fn shapes_hit_their_landmarks() {
        for s in Shape::ALL {
            for i in 0..1000_u16 {
                let v = s.at(f32::from(i) / 1000.0);
                assert!((-1.0..=1.0).contains(&v), "{s:?}");
            }
        }
        let t = Shape::Triangle;
        assert_eq!(
            [t.at(0.0), t.at(0.25), t.at(0.5), t.at(0.75)],
            [0.0, 1.0, 0.0, -1.0]
        );
        assert_eq!(Shape::Saw.at(0.0), -1.0);
        assert_eq!(Shape::Square.at(0.25), 1.0);
        assert_eq!(Shape::Square.at(0.75), -1.0);
        assert!(Shape::Sine.at(0.25) > 0.999_999);
        assert_eq!(Shape::from_index(2), Shape::Square);
        assert_eq!(Shape::from_index(9), Shape::Sine);
    }
}
