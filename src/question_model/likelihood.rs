//! The noisy-OR likelihood of DESIGN 3.2: a lineup takes a question with
//! probability `(1 − e^{−u})^m`, `u = Σ exp(a (z_i − b))`, and its derivatives
//! in `b`, `ln a` and `ln m`.

use super::QParams;
use crate::config::Config;
use crate::data::CORRECT;

pub(super) fn clamped_exp(x: f64, cfg: &Config) -> f64 {
    x.clamp(cfg.numeric.exp_lo, cfg.numeric.exp_hi).exp()
}

/// `k` is each member's slope, `question.slope_c`: a strong player's chance
/// falls off more slowly with the difficulty.
pub(super) fn intensity(z: &[f64], k: &[f64], a: f64, b: f64, cfg: &Config) -> f64 {
    let x: Vec<f64> = z.iter().zip(k).map(|(&zi, &ki)| log_intensity(a * ki * (zi - b), cfg)).collect();
    combine(&x, cfg.question.norm_p).0
}

/// A member's log-intensity, clamped as `clamped_exp` clamps its argument.
pub(super) fn log_intensity(x: f64, cfg: &Config) -> f64 {
    x.clamp(cfg.numeric.exp_lo, cfg.numeric.exp_hi)
}

/// `question.norm_p`: a lineup's intensity from its members' log-intensities
/// `x`, the power mean `(Σ e^{p x})^{1/p}`, and each member's weight
/// `e^{p x_i} / Σ e^{p x}` in it. `p` of 1 (or 0) is the plain sum, where the
/// weight is the member's share of the intensity.
pub(super) fn combine(x: &[f64], p: f64) -> (f64, Vec<f64>) {
    let p = if p > 0.0 { p } else { 1.0 };
    let top = x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if !top.is_finite() {
        return (0.0, vec![0.0; x.len()]);
    }
    let e: Vec<f64> = x.iter().map(|&xi| (p * (xi - top)).exp()).collect();
    let sum: f64 = e.iter().sum();
    ((top + sum.ln() / p).exp(), e.iter().map(|v| v / sum).collect())
}

/// The slope a player of strength `z` reads questions at, 1 when `slope_c` is off.
pub(super) fn slope(z: f64, cfg: &Config) -> f64 {
    let q = &cfg.question;
    (1.0 / (1.0 + q.slope_c * (z - q.slope_z0).max(0.0))).max(q.slope_min)
}

/// Saturating the total intensity keeps one observation from costing more than `u_max` nats.
pub(super) fn u_scale(u: f64, cfg: &Config) -> f64 {
    let hi = cfg.numeric.u_max;
    let lo = (-hi).exp();
    if u > hi {
        hi / u
    } else if u < lo {
        lo / u.max(f64::MIN_POSITIVE)
    } else {
        1.0
    }
}

/// log(1 - exp(-u)), accurate at both ends.
pub(super) fn log_p_step(u: f64) -> f64 {
    if u < std::f64::consts::LN_2 {
        (-(-u).exp_m1()).ln()
    } else {
        (-(-u).exp()).ln_1p()
    }
}

/// Log-likelihood of one team-question given the total intensity `u`.
pub(super) fn ll_from_u(u: f64, m: f64, y: u8, cfg: &Config) -> f64 {
    let g0 = log_p_step(u * u_scale(u, cfg));
    if y == CORRECT {
        m * g0
    } else {
        (-(m * g0).exp_m1()).ln()
    }
}

/// First and second derivative of the outcome term in `h = m * log P_step`.
pub(super) fn phi(h: f64, y: u8) -> (f64, f64) {
    if y == CORRECT {
        (1.0, 0.0)
    } else {
        let a = h.exp();
        let om = -h.exp_m1();
        (-a / om, -a / (om * om))
    }
}

/// `log P_step` at `u` and its first two derivatives in `u`.
pub(super) fn step_derivs(u: f64) -> (f64, f64, f64) {
    let t = (-u).exp();
    let om = -(-u).exp_m1();
    (log_p_step(u), t / om, -t / (om * om))
}

/// The gradient in `(b, ln a, ln m)` and the upper triangle of the Hessian,
/// row by row: `bb, bl, bm, ll, lm, mm`.
pub(super) struct Deriv {
    pub g: [f64; 3],
    pub h: [f64; 6],
}

/// The lineup's intensity `u` and its derivatives in `b` and `ln a`:
/// `u, u_b, u_l, u_bb, u_bl, u_ll`, all saturated by the same `u_scale`. With
/// `x_i = a k_i (z_i − b)` and the weights `w` of `combine`, `ln u` moves by
/// the weighted mean of each `∂x_i` and curves by `p` times their weighted
/// variance plus the weighted mean of each `∂²x_i`.
fn u_derivs(z: &[f64], k: &[f64], p: &QParams, cfg: &Config) -> [f64; 6] {
    let a = p.a();
    let pw = if cfg.question.norm_p > 0.0 { cfg.question.norm_p } else { 1.0 };
    let d: Vec<f64> = z.iter().zip(k).map(|(&zi, &ki)| ki * (zi - p.b)).collect();
    let x: Vec<f64> = d.iter().map(|&di| log_intensity(a * di, cfg)).collect();
    let (u0, w) = combine(&x, pw);
    let mean = |f: &dyn Fn(usize) -> f64| w.iter().enumerate().map(|(i, wi)| wi * f(i)).sum::<f64>();
    let (k1, d1) = (mean(&|i| k[i]), mean(&|i| d[i]));
    let (k2, d2, kd) = (mean(&|i| k[i] * k[i]), mean(&|i| d[i] * d[i]), mean(&|i| k[i] * d[i]));
    let (lb, ll) = (-a * k1, a * d1);
    let lbb = pw * a * a * (k2 - k1 * k1);
    let lbl = -a * k1 + pw * a * a * (k1 * d1 - kd);
    let lll = a * d1 + pw * a * a * (d2 - d1 * d1);
    let u = u0 * u_scale(u0, cfg);
    [u, u * lb, u * ll, u * (lbb + lb * lb), u * (lbl + lb * ll), u * (lll + ll * ll)]
}

pub(super) fn team_deriv(z: &[f64], k: &[f64], y: u8, p: &QParams, cfg: &Config) -> Deriv {
    let m = p.m();
    let [u, u_b, u_l, u_bb, u_bl, u_ll] = u_derivs(z, k, p, cfg);
    let (g0, g1, g2) = step_derivs(u);
    let (h_b, h_l, h_m) = (m * g1 * u_b, m * g1 * u_l, m * g0);
    let h_bb = m * (g2 * u_b * u_b + g1 * u_bb);
    let h_bl = m * (g2 * u_b * u_l + g1 * u_bl);
    let h_ll = m * (g2 * u_l * u_l + g1 * u_ll);
    let (p1, p2) = phi(m * g0, y);
    let g = [p1 * h_b, p1 * h_l, p1 * h_m];
    let h_bm = p2 * h_b * h_m + p1 * h_b;
    let h_lm = p2 * h_l * h_m + p1 * h_l;
    let h_mm = p2 * h_m * h_m + p1 * h_m;
    Deriv { g, h: [p2 * h_b * h_b + p1 * h_bb, p2 * h_b * h_l + p1 * h_bl, h_bm, p2 * h_l * h_l + p1 * h_ll, h_lm, h_mm] }
}
