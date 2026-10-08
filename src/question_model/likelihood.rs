//! The noisy-OR likelihood of DESIGN 3.2: a lineup takes a question with
//! probability `(1 − e^{−u})^m`, `u = Σ exp(a (z_i − b))`, and its derivatives
//! in `b`, `ln a` and `ln m`.

use super::QParams;
use crate::config::Config;
use crate::data::CORRECT;

pub(super) fn clamped_exp(x: f64, cfg: &Config) -> f64 {
    x.clamp(cfg.numeric.exp_lo, cfg.numeric.exp_hi).exp()
}

pub(super) fn intensity(z: &[f64], a: f64, b: f64, cfg: &Config) -> f64 {
    z.iter().map(|&zi| clamped_exp(a * (zi - b), cfg)).sum()
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

/// `u`, `Σ d_i λ_i` and `Σ d_i² λ_i` over the lineup, `d_i = z_i − b`, all
/// saturated by the same `u_scale`.
fn moments(z: &[f64], p: &QParams, cfg: &Config) -> (f64, f64, f64) {
    let a = p.a();
    let (mut u, mut u1, mut u2) = (0.0, 0.0, 0.0);
    for &zi in z {
        let d = zi - p.b;
        let l = clamped_exp(a * d, cfg);
        u += l;
        u1 += d * l;
        u2 += d * d * l;
    }
    let s = u_scale(u, cfg);
    (u * s, u1 * s, u2 * s)
}

pub(super) fn team_deriv(z: &[f64], y: u8, p: &QParams, cfg: &Config) -> Deriv {
    let (a, m) = (p.a(), p.m());
    let (u, u1, u2) = moments(z, p, cfg);
    let (g0, g1, g2) = step_derivs(u);
    let (u_b, u_l) = (-a * u, a * u1);
    let (u_bb, u_bl, u_ll) = (a * a * u, -a * u - a * a * u1, a * (u1 + a * u2));
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
