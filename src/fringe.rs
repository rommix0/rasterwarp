//! Edge fringing: the one-sided smear a band-limited video signal, scanned left to right,
//! leaves after every edge. The colorize pass applies it before posterizing, so sharp
//! edges briefly pass through the intermediate levels and show their colors as fringes.

use std::f64::consts::PI;

/// Most pixels the kernel reads: the pixel itself and up to 47 to its left.
pub const TAPS: usize = 48;

/// Below this bandwidth the filter's cutoff would pass a quarter of the pixel rate, where
/// its poles turn negative and the smear would ring even with no ringing set. Smaller
/// bandwidths fade the minimum kernel out toward no smear instead.
const MIN_BANDWIDTH: f64 = 2.0 / PI;

/// Taps from here on fade out, so a long ringing tail doesn't end in a visible step.
const TAPER_FROM: usize = 36;

/// The smear kernel for `bandwidth` (canvas pixels) and `ringing` (0-1).
///
/// `weights[k]` applies to the pixel `k` to the left; they sum to 1, so flat areas are
/// unchanged. Returns the weights and how many of them matter (at least 1).
///
/// The weights are the impulse response of a 2-pole low-pass (biquad) filter with cutoff
/// `1 / (2π · bandwidth)` cycles per pixel and Q `0.5 + 1.5 · ringing`: 0.5 is critically
/// damped (no overshoot), 2.0 rings visibly.
pub fn kernel(bandwidth: f32, ringing: f32) -> ([f32; TAPS], usize) {
    let mut weights = [0.0f32; TAPS];
    weights[0] = 1.0;
    if bandwidth <= 0.0 {
        return (weights, 1);
    }
    let bandwidth = f64::from(bandwidth);
    let response = impulse_response(bandwidth.max(MIN_BANDWIDTH), f64::from(ringing));
    // Fade the minimum kernel toward no smear below the minimum bandwidth.
    let mix = (bandwidth / MIN_BANDWIDTH).min(1.0);
    for (k, w) in weights.iter_mut().enumerate() {
        let identity = if k == 0 { 1.0 } else { 0.0 };
        *w = (identity + (response[k] - identity) * mix) as f32;
    }
    let taps = weights
        .iter()
        .rposition(|w| w.abs() > 1e-5)
        .map_or(1, |last| last + 1);
    (weights, taps)
}

/// The normalized, tapered impulse response of the low-pass filter.
fn impulse_response(bandwidth: f64, ringing: f64) -> [f64; TAPS] {
    // RBJ cookbook low-pass, one sample per pixel.
    let w0 = 1.0 / bandwidth;
    let q = 0.5 + 1.5 * ringing.clamp(0.0, 1.0);
    let alpha = w0.sin() / (2.0 * q);
    let cos = w0.cos();
    let a0 = 1.0 + alpha;
    let (b0, b1, b2) = (
        (1.0 - cos) / 2.0 / a0,
        (1.0 - cos) / a0,
        (1.0 - cos) / 2.0 / a0,
    );
    let (a1, a2) = (-2.0 * cos / a0, (1.0 - alpha) / a0);
    let mut h = [0.0; TAPS];
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    for (n, out) in h.iter_mut().enumerate() {
        let x0 = if n == 0 { 1.0 } else { 0.0 };
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        (x2, x1, y2, y1) = (x1, x0, y1, y0);
        let taper = if n < TAPER_FROM {
            1.0
        } else {
            let t = (n - TAPER_FROM + 1) as f64 / (TAPS - TAPER_FROM + 1) as f64;
            0.5 + 0.5 * (PI * t).cos()
        };
        *out = y0 * taper;
    }
    let sum: f64 = h.iter().sum();
    h.map(|v| v / sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANDWIDTHS: [f32; 7] = [0.1, 0.5, 0.64, 1.0, 2.0, 4.0, 8.0];

    /// The step response: what an edge from 0 to 1 looks like after the smear.
    fn step(weights: &[f32; TAPS]) -> Vec<f32> {
        weights
            .iter()
            .scan(0.0, |sum, w| {
                *sum += w;
                Some(*sum)
            })
            .collect()
    }

    #[test]
    fn weights_sum_to_one() {
        for bandwidth in BANDWIDTHS {
            for ringing in [0.0, 0.2, 0.5, 1.0] {
                let (w, taps) = kernel(bandwidth, ringing);
                let sum: f32 = w[..taps].iter().sum();
                assert!(
                    (sum - 1.0).abs() < 1e-4,
                    "{bandwidth} px, ringing {ringing}: {sum}"
                );
            }
        }
    }

    #[test]
    fn zero_bandwidth_is_no_smear() {
        let (w, taps) = kernel(0.0, 0.7);
        assert_eq!(taps, 1);
        assert_eq!(w[0], 1.0);
        assert!(w[1..].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn no_ringing_never_overshoots() {
        for bandwidth in BANDWIDTHS {
            let (w, _) = kernel(bandwidth, 0.0);
            assert!(w.iter().all(|&v| v >= 0.0), "{bandwidth} px: {w:?}");
            let peak = step(&w).into_iter().fold(0.0, f32::max);
            assert!(peak <= 1.0 + 1e-5, "{bandwidth} px overshoots to {peak}");
        }
    }

    #[test]
    fn ringing_overshoots_after_an_edge() {
        let (w, _) = kernel(2.0, 0.6);
        let peak = step(&w).into_iter().fold(0.0, f32::max);
        assert!(peak > 1.02, "peak {peak}");
    }

    #[test]
    fn wider_bandwidth_smears_further() {
        let reach = |bandwidth| {
            let s = step(&kernel(bandwidth, 0.0).0);
            s.iter().position(|&v| v >= 0.9).unwrap()
        };
        assert!(reach(1.0) < reach(4.0));
        assert!(reach(4.0) < reach(8.0));
    }

    #[test]
    fn weights_change_continuously() {
        let near = |a: (f32, f32), b: (f32, f32)| {
            let (wa, wb) = (kernel(a.0, a.1).0, kernel(b.0, b.1).0);
            wa.iter()
                .zip(&wb)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f32::max)
        };
        for bandwidth in [0.0, 0.3, 0.636, 0.64, 2.0, 7.99] {
            let gap = near((bandwidth, 0.3), (bandwidth + 0.01, 0.3));
            assert!(gap < 0.03, "bandwidth {bandwidth}: {gap}");
        }
        for ringing in [0.0, 0.5, 0.99] {
            let gap = near((2.0, ringing), (2.0, ringing + 0.01));
            assert!(gap < 0.03, "ringing {ringing}: {gap}");
        }
    }
}
