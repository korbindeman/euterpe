//! Building blocks for dynamics: gates, compressors, limiters.
//!
//! - [`time_constant`]: the coefficient of a one-pole that covers `1 - 1/e`
//!   of a step in a given time.
//! - [`Follower`]: a one-pole that moves toward its target with one
//!   coefficient going up (attack) and another going down (release). On
//!   `|x|` it is a peak envelope; on a gain it is a gain smoother.
//! - [`SoftKnee`]: the static curve of a compressor, in dB.
//!
//! Coefficients are computed per block (they need `exp`); the per-sample
//! work is a compare and a multiply-add.

use euterpe_core::fastmath;

/// `e^(-1 / (ms * sample_rate / 1000))`: the pole of a one-pole that covers
/// `1 - 1/e` (63%) of a step in `ms`. 0 for `ms <= 0` (no smoothing).
#[must_use]
#[inline]
pub fn time_constant(ms: f32, sample_rate: f32) -> f32 {
    let samples = ms * sample_rate / 1000.0;
    if samples > 0.0 {
        fastmath::exp(-1.0 / samples)
    } else {
        0.0
    }
}

/// Attack and release poles for a [`Follower`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ballistics {
    pub attack: f32,
    pub release: f32,
}

impl Ballistics {
    /// From times in milliseconds, at `sample_rate`.
    #[must_use]
    pub fn new(attack_ms: f32, release_ms: f32, sample_rate: f32) -> Self {
        Self {
            attack: time_constant(attack_ms, sample_rate),
            release: time_constant(release_ms, sample_rate),
        }
    }

    /// The same time both ways.
    #[must_use]
    pub fn symmetric(ms: f32, sample_rate: f32) -> Self {
        let c = time_constant(ms, sample_rate);
        Self {
            attack: c,
            release: c,
        }
    }
}

/// A one-pole that rises with the attack pole and falls with the release
/// pole.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Follower {
    pub value: f32,
}

impl Follower {
    /// A follower starting at `value`.
    #[must_use]
    pub const fn at(value: f32) -> Self {
        Self { value }
    }

    /// Move toward `target` and return the new value.
    #[inline(always)]
    pub fn follow(&mut self, b: &Ballistics, target: f32) -> f32 {
        let c = if target > self.value {
            b.attack
        } else {
            b.release
        };
        self.value = c * self.value + (1.0 - c) * target;
        self.value
    }

    /// A peak envelope: follow `|x|`.
    #[inline(always)]
    pub fn peak(&mut self, b: &Ballistics, x: f32) -> f32 {
        self.follow(b, x.abs())
    }
}

/// A compressor's static curve: no change below the knee, `ratio:1` above
/// it, and a quadratic blend across a knee `knee_db` wide centred on the
/// threshold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoftKnee {
    threshold_db: f32,
    half_knee_db: f32,
    /// `1 - 1/ratio`: dB of reduction per dB over the threshold.
    slope: f32,
    /// `slope / (2 * knee)`, for the knee's quadratic.
    knee_curve: f32,
}

impl SoftKnee {
    #[must_use]
    pub fn new(threshold_db: f32, ratio: f32, knee_db: f32) -> Self {
        let slope = 1.0 - 1.0 / ratio.max(1.0);
        let knee = knee_db.max(0.0);
        Self {
            threshold_db,
            half_knee_db: 0.5 * knee,
            slope,
            knee_curve: if knee > 0.0 {
                slope / (2.0 * knee)
            } else {
                0.0
            },
        }
    }

    /// Gain change in dB (zero or negative) for a level in dB.
    #[inline(always)]
    #[must_use]
    pub fn gain_db(&self, level_db: f32) -> f32 {
        let over = level_db - self.threshold_db;
        if over <= -self.half_knee_db {
            0.0
        } else if over >= self.half_knee_db {
            -over * self.slope
        } else {
            let x = over + self.half_knee_db;
            -x * x * self.knee_curve
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact at the curve's breakpoints")]
mod tests {
    use super::*;

    #[test]
    fn time_constant_covers_63_percent_in_its_time() {
        let c = time_constant(10.0, 48_000.0);
        let mut f = Follower::default();
        let b = Ballistics {
            attack: c,
            release: c,
        };
        for _ in 0..480 {
            f.follow(&b, 1.0);
        }
        assert!(
            (f.value - (1.0 - (-1.0_f32).exp())).abs() < 1e-3,
            "{}",
            f.value
        );
        assert_eq!(time_constant(0.0, 48_000.0), 0.0);
    }

    #[test]
    fn follower_attacks_fast_and_releases_slow() {
        let b = Ballistics::new(1.0, 100.0, 48_000.0);
        let mut f = Follower::default();
        for _ in 0..480 {
            f.peak(&b, -1.0);
        }
        assert!(f.value > 0.999, "attack: {}", f.value);
        for _ in 0..480 {
            f.peak(&b, 0.0);
        }
        assert!(f.value > 0.9, "release: {}", f.value);
    }

    #[test]
    fn soft_knee_is_continuous_and_hits_the_ratio() {
        let k = SoftKnee::new(-20.0, 4.0, 6.0);
        assert_eq!(k.gain_db(-30.0), 0.0);
        assert_eq!(k.gain_db(-23.0), 0.0);
        // Above the knee: 3/4 dB of reduction per dB over.
        assert!((k.gain_db(0.0) + 15.0).abs() < 1e-5);
        // Continuous at both ends of the knee, and halfway at threshold.
        assert!((k.gain_db(-17.0) + 3.0 * 0.75).abs() < 1e-5);
        assert!((k.gain_db(-20.0) + 0.75 * 9.0 / 12.0).abs() < 1e-5);
        let hard = SoftKnee::new(-20.0, 2.0, 0.0);
        assert_eq!(hard.gain_db(-20.0), 0.0);
        assert_eq!(hard.gain_db(-10.0), -5.0);
    }
}
