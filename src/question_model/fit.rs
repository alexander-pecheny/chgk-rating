//! Each question's difficulty, discrimination and collaboration exponent: the
//! take-rate closed form of DESIGN 3.3, or Newton with step halving on the
//! penalised log-likelihood.

use super::likelihood::{intensity, ll_from_u, team_deriv};
use super::{Field, QParams, TeamObs};
use crate::config::{Config, QFit};
use crate::data::CORRECT;

const HALF: f64 = 0.5;
/// Keeps the log of a lineup's intensity finite.
const MIN_INTENSITY: f64 = 1e-12;
/// The take rate a difficulty is solved from stays inside this range.
const TAKE_RATE: (f64, f64) = (0.02, 0.98);
/// Step halvings tried before a Newton step is abandoned.
const HALVINGS: usize = 8;
/// Damping of the Newton system: where it starts, how far it may grow, and
/// by what factor each time the system is not positive definite.
const DAMPING_START: f64 = 1e-8;
const DAMPING_TRIES: usize = 12;
const DAMPING_GROWTH: f64 = 10.0;
const MIN_PIVOT: f64 = 1e-12;
/// A Newton step longer than this in any coordinate is not trusted.
const MAX_STEP: f64 = 20.0;
/// The parameters `(b, ln a, ln m)`, and the Hessian's diagonal in the upper
/// triangle `bb, bl, bm, ll, lm, mm`.
const DIM_FREE_M: usize = 3;
const DIAG: [usize; 3] = [0, 3, 5];

/// The teams of the selected half that saw question `q`.
fn seen<'a>(field: &'a Field, q: usize, sel: Option<u8>) -> impl Iterator<Item = &'a TeamObs> {
    field.by_q[q].iter().map(|&ti| &field.teams[ti as usize]).filter(move |t| sel.is_none_or(|s| t.half == s))
}

/// The log prior and its gradient and Hessian diagonal in `(b, ln a, ln m)`.
fn prior(p: &QParams, cfg: &Config, free_m: bool) -> (f64, [f64; 3], [f64; 3]) {
    let q = &cfg.question;
    let (vb, va, vm) = (q.s_b * q.s_b, q.s_a * q.s_a, q.s_m * q.s_m);
    let db = p.b - q.b_bar;
    let mut val = -HALF * db * db / vb - HALF * p.la * p.la / va;
    let mut g = [-db / vb, -p.la / va, 0.0];
    let mut h = [-1.0 / vb, -1.0 / va, 0.0];
    if free_m {
        val -= HALF * p.lm * p.lm / vm;
        g[2] = -p.lm / vm;
        h[2] = -1.0 / vm;
    }
    (val, g, h)
}

fn clamp_params(p: &mut QParams, cfg: &Config, free_m: bool) {
    let n = &cfg.numeric;
    p.b = p.b.clamp(n.b_min, n.b_max);
    p.la = p.la.clamp(n.a_min.ln(), n.a_max.ln());
    if free_m {
        p.lm = p.lm.clamp(n.m_min.ln(), n.m_max.ln());
    }
}

fn objective(field: &Field, q: usize, sel: Option<u8>, p: &QParams, cfg: &Config, free_m: bool) -> f64 {
    let (a, m) = (p.a(), p.m());
    let v: f64 = seen(field, q, sel).map(|t| ll_from_u(intensity(&t.z, a, p.b, cfg), m, t.mask[q], cfg)).sum();
    prior(p, cfg, free_m).0 + field.weight * v
}

/// The take rate the lineups that saw this question managed. Under
/// `P = (1 − e^{−u})^m` the step's own rate is `r^{1/m}`, so that is what the
/// difficulty is solved from; at `m = 1` this is the closed form of DESIGN 3.3.
fn init_params(field: &Field, q: usize, sel: Option<u8>, cfg: &Config) -> QParams {
    let (a, la, lm) = (field.a_fix, field.a_fix.ln(), field.m_fix.ln());
    let (mut sum_s, mut n, mut hits) = (0.0, 0.0, 0.0);
    for t in seen(field, q, sel) {
        sum_s += intensity(&t.z, a, 0.0, cfg).max(MIN_INTENSITY).ln();
        n += 1.0;
        hits += (t.mask[q] == CORRECT) as u8 as f64;
    }
    if n == 0.0 {
        return QParams { b: cfg.question.b_bar, la, lm };
    }
    let phat = (hits / n).clamp(TAKE_RATE.0, TAKE_RATE.1).powf(1.0 / field.m_fix);
    let mut p = QParams { b: (sum_s / n - (-(1.0 - phat).ln()).ln()) / a, la, lm };
    clamp_params(&mut p, cfg, false);
    p
}

/// Newton with step halving on the penalised log-likelihood of one question.
pub fn fit_question(field: &Field, q: usize, sel: Option<u8>, cfg: &Config) -> QParams {
    let free_m = cfg.question.s_m > 0.0;
    let dim = if free_m { DIM_FREE_M } else { 2 };
    let mut p = init_params(field, q, sel, cfg);
    if cfg.question.fit == QFit::TakeRate {
        return p;
    }
    let mut f = objective(field, q, sel, &p, cfg, free_m);
    for _ in 0..cfg.numeric.newton_iters {
        let (g, h) = newton_system(field, q, sel, &p, cfg, free_m);
        let Some(step) = solve(&g, &h, dim) else { break };
        let mut moved = 0.0;
        if let Some((cand, fc)) = line_search(field, q, sel, &p, f, &step, cfg, free_m) {
            moved = (cand.b - p.b).abs() + (cand.la - p.la).abs() + (cand.lm - p.lm).abs();
            p = cand;
            f = fc;
        }
        if moved <= cfg.numeric.newton_tol {
            break;
        }
    }
    p
}

/// The penalised gradient and Hessian of one question at `p`.
fn newton_system(field: &Field, q: usize, sel: Option<u8>, p: &QParams, cfg: &Config, free_m: bool) -> ([f64; 3], [f64; 6]) {
    let (_, pg, ph) = prior(p, cfg, free_m);
    let mut g = [0.0; 3];
    let mut h = [0.0; 6];
    for t in seen(field, q, sel) {
        let d = team_deriv(&t.z, t.mask[q], p, cfg);
        g.iter_mut().zip(d.g).for_each(|(a, v)| *a += v);
        h.iter_mut().zip(d.h).for_each(|(a, v)| *a += v);
    }
    for (k, v) in g.iter_mut().enumerate() {
        *v = *v * field.weight + pg[k];
    }
    h.iter_mut().for_each(|v| *v *= field.weight);
    for (k, &i) in DIAG.iter().enumerate() {
        h[i] += ph[k];
    }
    (g, h)
}

/// The first of `step`, `step/2`, `step/4` ... that raises the objective, with
/// its value; `None` when none of them does.
#[allow(clippy::too_many_arguments)]
fn line_search(
    field: &Field,
    q: usize,
    sel: Option<u8>,
    p: &QParams,
    f: f64,
    step: &[f64; 3],
    cfg: &Config,
    free_m: bool,
) -> Option<(QParams, f64)> {
    let mut scale = 1.0;
    for _ in 0..HALVINGS {
        let mut cand = QParams { b: p.b + scale * step[0], la: p.la + scale * step[1], lm: p.lm + scale * step[2] };
        clamp_params(&mut cand, cfg, free_m);
        let fc = objective(field, q, sel, &cand, cfg, free_m);
        if fc > f {
            return Some((cand, fc));
        }
        scale *= HALF;
    }
    None
}

/// Solves `(-H + λI) x = g` for the ascent direction, raising the damping `λ`
/// until the system is positive definite and the step is of a sane length.
fn solve(g: &[f64; 3], h: &[f64; 6], dim: usize) -> Option<[f64; 3]> {
    let mut lambda = DAMPING_START;
    for _ in 0..DAMPING_TRIES {
        let a = [[-h[0] + lambda, -h[1], -h[2]], [-h[1], -h[3] + lambda, -h[4]], [-h[2], -h[4], -h[5] + lambda]];
        if let Some(x) = gauss(a, *g, dim) {
            if x.iter().all(|v| v.is_finite() && v.abs() <= MAX_STEP) {
                return Some(x);
            }
        }
        lambda *= DAMPING_GROWTH;
    }
    None
}

/// Gaussian elimination without pivoting on the leading `dim` rows; `None`
/// at a pivot that is not clearly positive. Coordinates past `dim` are zero.
#[allow(clippy::needless_range_loop)]
fn gauss(mut a: [[f64; 3]; 3], mut x: [f64; 3], dim: usize) -> Option<[f64; 3]> {
    for i in 0..dim {
        let piv = a[i][i];
        if piv <= MIN_PIVOT {
            return None;
        }
        (i..dim).for_each(|j| a[i][j] /= piv);
        x[i] /= piv;
        for r in (i + 1)..dim {
            let f = a[r][i];
            (i..dim).for_each(|j| a[r][j] -= f * a[i][j]);
            x[r] -= f * x[i];
        }
    }
    for i in (0..dim).rev() {
        for j in (i + 1)..dim {
            x[i] -= a[i][j] * x[j];
        }
    }
    x.iter_mut().skip(dim).for_each(|v| *v = 0.0);
    Some(x)
}
