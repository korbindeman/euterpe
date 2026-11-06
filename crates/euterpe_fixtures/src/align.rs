//! Time alignment of a hardware output against its DI.
//!
//! The upstream pairs are not sample-aligned (Guitar-TECHS warns of up to
//! 100 ms; EGFxSet files differ in length). We find the lag that maximizes
//! the absolute normalized cross-correlation, coarse on a decimated copy and
//! then exact at full rate. The absolute value catches polarity flips from
//! a mic or an inverting stage.

/// Largest lag searched, in seconds.
pub const MAX_LAG_S: f32 = 0.15;

const DECIMATE: usize = 4;
const REFINE: isize = 2 * DECIMATE as isize;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Alignment {
    /// `out[start + lag + i]` lines up with `di_window[i]`.
    pub lag: isize,
    pub inverted: bool,
    /// Normalized correlation at `lag`, 0..1.
    pub correlation: f32,
}

fn at(x: &[f32], i: isize) -> f32 {
    if i >= 0 && (i as usize) < x.len() {
        x[i as usize]
    } else {
        0.0
    }
}

/// Normalized correlation of `a` with `b[offset..]`.
fn corr(a: &[f32], b: &[f32], offset: isize, a_energy: f64) -> f64 {
    let mut num = 0.0f64;
    let mut b_energy = 0.0f64;
    for (i, &av) in a.iter().enumerate() {
        let bv = at(b, offset + i as isize) as f64;
        num += av as f64 * bv;
        b_energy += bv * bv;
    }
    let den = (a_energy * b_energy).sqrt();
    if den > 0.0 { num / den } else { 0.0 }
}

fn decimate(x: &[f32]) -> Vec<f32> {
    x.chunks(DECIMATE)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect()
}

/// Align `out` to `di_window`, which was taken from the DI at `start`.
pub fn align(di_window: &[f32], out: &[f32], start: usize, max_lag: usize) -> Alignment {
    // Coarse pass on decimated signals.
    let a = decimate(di_window);
    let b = decimate(out);
    let a_energy: f64 = a.iter().map(|&v| (v as f64).powi(2)).sum();
    let s = (start / DECIMATE) as isize;
    let m = (max_lag / DECIMATE) as isize;
    let mut best = (0isize, 0.0f64);
    for lag in -m..=m {
        let c = corr(&a, &b, s + lag, a_energy);
        if c.abs() > best.1.abs() {
            best = (lag, c);
        }
    }

    // Exact pass around the coarse peak.
    let coarse = best.0 * DECIMATE as isize;
    let a_energy: f64 = di_window.iter().map(|&v| (v as f64).powi(2)).sum();
    let mut fine = (coarse, 0.0f64);
    for lag in coarse - REFINE..=coarse + REFINE {
        if lag.unsigned_abs() > max_lag {
            continue;
        }
        let c = corr(di_window, out, start as isize + lag, a_energy);
        if c.abs() > fine.1.abs() {
            fine = (lag, c);
        }
    }
    Alignment {
        lag: fine.0,
        inverted: fine.1 < 0.0,
        correlation: fine.1.abs() as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect()
    }

    #[test]
    fn finds_delay_and_inversion_through_a_nonlinearity() {
        let di = noise(48_000, 7);
        let delay = 1_234isize;
        // A delayed, inverted, saturated and lowpassed copy, like a mic'd amp.
        let mut out = vec![0.0f32; di.len() + 4_000];
        let mut lp = 0.0f32;
        for (i, &x) in di.iter().enumerate() {
            lp += 0.5 * (-(3.0 * x).tanh() - lp);
            out[i + delay as usize] = lp;
        }
        let start = 10_000;
        let w = &di[start..start + 24_000];
        let a = align(w, &out, start, 7_200);
        assert!((a.lag - delay).abs() <= 1, "{a:?}");
        assert!(a.inverted);
        assert!(a.correlation > 0.8, "{a:?}");
    }

    #[test]
    fn finds_negative_lag() {
        let di = noise(48_000, 3);
        let out: Vec<f32> = di[500..].to_vec(); // output early by 500
        let a = align(&di[20_000..30_000], &out, 20_000, 7_200);
        assert_eq!(a.lag, -500);
        assert!(!a.inverted);
        assert!(a.correlation > 0.999);
    }
}
