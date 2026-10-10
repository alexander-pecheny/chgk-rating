//! What one tournament says about each player: the gradient and observed
//! Fisher information of its log-likelihood in every `theta_i`.

use super::likelihood::{clamped_exp, phi, step_derivs, u_scale};
use super::{Field, QParams, TeamObs};
use crate::config::{Config, QGradient};
use crate::data::{CORRECT, SKIP};

const HALF: f64 = 0.5;
/// Neighbours whose summed kernel weight is below this say nothing.
const MIN_WEIGHT: f64 = 1e-6;
/// The target score stays this far inside `[0, questions asked]`.
const SCORE_MARGIN: f64 = 0.01;
/// Newton on the shift: steps, tolerance in answers, the derivative floor,
/// and the largest shift in difficulty units.
const SHIFT_ITERS: usize = 8;
const SHIFT_TOL: f64 = 1e-6;
const MIN_SLOPE: f64 = 1e-12;
const MAX_SHIFT: f64 = 4.0;

/// `(player, gradient, information)`, one entry per player, sorted by player.
pub type Grads = Vec<(u32, f64, f64)>;

/// Gradient and observed Fisher information of the tournament log-likelihood in each theta_i,
/// summed per player and sorted by player index.
pub fn player_grads(field: &Field, params: &[QParams], cfg: &Config) -> Grads {
    if cfg.question.gradient == QGradient::Share {
        return share_grads(field, params, cfg);
    }
    let shift = relative_shifts(field, params, cfg);
    let mut out = Grads::new();
    for (t, &dlt) in field.teams.iter().zip(&shift).filter(|(t, _)| !t.z.is_empty()) {
        let acc = team_grads(t, params, dlt, cfg);
        let floor = cfg.question.rel_winner && !cfg.question.rel_place && t.first;
        for (&i, &(g, info)) in t.members.iter().zip(&acc) {
            out.push((i, if floor { g.max(0.0) } else { g }, info));
        }
    }
    merge(out)
}

/// `question.team_effect_sd`: per lineup, `(team, gradient, information)` of
/// the log-likelihood in a strength added to every member, which is the sum
/// of the members' gradients and the whole lineup's curvature.
pub fn team_grads_by_id(field: &Field, params: &[QParams], cfg: &Config) -> Vec<(u32, f64, f64)> {
    if cfg.question.team_effect_sd <= 0.0 {
        return Vec::new();
    }
    let shift = relative_shifts(field, params, cfg);
    let mut out = Vec::new();
    for (t, &dlt) in field.teams.iter().zip(&shift).filter(|(t, _)| !t.z.is_empty()) {
        let (mut g, mut info) = (0.0, 0.0);
        for (q, &y) in t.mask.iter().enumerate().filter(|(_, &y)| y != SKIP) {
            let p = &params[q];
            let (a, m) = (p.a(), p.m());
            let u: f64 = t.z.iter().map(|&zi| clamped_exp(a * (zi - p.b - dlt), cfg)).sum();
            let s = u_scale(u, cfg);
            let (g0, g1, g2) = step_derivs(u * s);
            let (p1, p2) = phi(m * g0, y);
            let d1 = p1 * m * g1;
            let d2 = p2 * (m * g1) * (m * g1) + p1 * m * g2;
            let uz = a * u * s;
            g += d1 * uz;
            info -= d2 * uz * uz + d1 * a * uz;
        }
        out.push((t.team, g, info));
    }
    out
}

/// One lineup's per-member `(gradient, information)` over the questions it
/// was asked, at the difficulties shifted by `dlt`.
fn team_grads(t: &TeamObs, params: &[QParams], dlt: f64, cfg: &Config) -> Vec<(f64, f64)> {
    let mut acc = vec![(0.0, 0.0); t.z.len()];
    let mut lam = Vec::with_capacity(t.z.len());
    for (q, &y) in t.mask.iter().enumerate().filter(|(_, &y)| y != SKIP) {
        let p = &params[q];
        let (a, m) = (p.a(), p.m());
        lam.clear();
        lam.extend(t.z.iter().map(|&zi| clamped_exp(a * (zi - p.b - dlt), cfg)));
        let u: f64 = lam.iter().sum();
        let s = u_scale(u, cfg);
        lam.iter_mut().for_each(|l| *l *= s);
        let (g0, g1, g2) = step_derivs(u * s);
        let (p1, p2) = phi(m * g0, y);
        let d1 = p1 * m * g1;
        let d2 = p2 * (m * g1) * (m * g1) + p1 * m * g2;
        let temp = cfg.question.share_temp;
        if temp > 1.0 {
            // The same total `a u` handed out by the tempered shares.
            let total: f64 = lam.iter().sum();
            let w: f64 = lam.iter().map(|l| l.powf(1.0 / temp)).sum();
            for (k, &l) in lam.iter().enumerate() {
                let uz = a * total * l.powf(1.0 / temp) / w;
                acc[k].0 += d1 * uz;
                acc[k].1 -= d2 * uz * uz + d1 * a * uz;
            }
            continue;
        }
        for (k, &l) in lam.iter().enumerate() {
            let uz = a * l;
            acc[k].0 += d1 * uz;
            acc[k].1 -= d2 * uz * uz + d1 * a * uz;
        }
    }
    acc
}

/// The chance a lineup takes one question, at the difficulty shifted by `d`.
fn p_take(z: &[f64], p: &QParams, d: f64, cfg: &Config) -> f64 {
    let a = p.a();
    let mut u: f64 = z.iter().map(|&zi| clamped_exp(a * (zi - p.b - d), cfg)).sum();
    u *= u_scale(u, cfg);
    (-(-u).exp_m1()).powf(p.m())
}

/// Per team, the difficulty shift of `question.rel_h`: the one number that
/// moves the lineup's expected answers by the mean surplus (actual less
/// expected) of the lineups whose expected score is close to its own, the
/// lineup itself left out. Judged against that, a field whose strong half all
/// fell short of the model by the same four answers costs none of them
/// anything, and only finishing behind one's neighbours does. All zero when
/// the switch is off.
fn relative_shifts(field: &Field, params: &[QParams], cfg: &Config) -> Vec<f64> {
    let n = field.teams.len();
    let h = cfg.question.rel_h;
    if h <= 0.0 {
        return vec![0.0; n];
    }
    let (e, r) = surpluses(field, params, cfg);
    let shift_of = |(i, t): (usize, &TeamObs)| match neighbour_target(i, t, &e, &r, h) {
        Some(target) => shift_to(t, params, target, cfg),
        None => 0.0,
    };
    field.teams.iter().enumerate().map(shift_of).collect()
}

/// Each lineup's expected answers at the fitted difficulties, and its surplus
/// over them; NaN and zero for a result with no lineup.
fn surpluses(field: &Field, params: &[QParams], cfg: &Config) -> (Vec<f64>, Vec<f64>) {
    let n = field.teams.len();
    let (mut e, mut r) = (vec![f64::NAN; n], vec![0.0; n]);
    for (i, t) in field.teams.iter().enumerate().filter(|(_, t)| !t.z.is_empty()) {
        e[i] = t.mask.iter().enumerate().filter(|(_, &y)| y != SKIP).map(|(q, _)| p_take(&t.z, &params[q], 0.0, cfg)).sum();
        r[i] = t.mask.iter().filter(|&&y| y == CORRECT).count() as f64 - e[i];
    }
    (e, r)
}

/// The score lineup `i` would have to make to match its neighbours' mean
/// surplus, under a Gaussian kernel of width `h` answers on expected score.
fn neighbour_target(i: usize, t: &TeamObs, e: &[f64], r: &[f64], h: f64) -> Option<f64> {
    if !e[i].is_finite() {
        return None;
    }
    let (mut num, mut den) = (0.0, 0.0);
    for j in (0..e.len()).filter(|&j| j != i && e[j].is_finite()) {
        let x = (e[j] - e[i]) / h;
        let w = (-HALF * x * x).exp();
        num += w * r[j];
        den += w;
    }
    if den < MIN_WEIGHT {
        return None;
    }
    let asked = t.mask.iter().filter(|&&y| y != SKIP).count() as f64;
    Some((e[i] + num / den).clamp(SCORE_MARGIN, (asked - SCORE_MARGIN).max(SCORE_MARGIN)))
}

/// Newton on the shift: the expectation falls as the difficulty rises,
/// smoothly, so a handful of steps from zero lands within the tolerance.
fn shift_to(t: &TeamObs, params: &[QParams], target: f64, cfg: &Config) -> f64 {
    let mut d = 0.0;
    for _ in 0..SHIFT_ITERS {
        let (f, df) = expected_and_slope(t, params, d, cfg);
        let err = f - target;
        if err.abs() < SHIFT_TOL || df.abs() < MIN_SLOPE {
            break;
        }
        d = (d - err / df).clamp(-MAX_SHIFT, MAX_SHIFT);
    }
    d
}

/// The lineup's expected answers at shift `d`, and its derivative in `d`.
fn expected_and_slope(t: &TeamObs, params: &[QParams], d: f64, cfg: &Config) -> (f64, f64) {
    let (mut f, mut df) = (0.0, 0.0);
    for (q, _) in t.mask.iter().enumerate().filter(|(_, &y)| y != SKIP) {
        let p = &params[q];
        let (a, m) = (p.a(), p.m());
        let u: f64 = t.z.iter().map(|&zi| clamped_exp(a * (zi - p.b - d), cfg)).sum();
        let s = -(-u).exp_m1();
        f += s.powf(m);
        // d/dd of (1 - e^-u)^m with du/dd = -a u.
        df -= m * s.powf(m - 1.0) * (-u).exp() * a * u;
    }
    (f, df)
}

fn merge(mut out: Grads) -> Grads {
    out.sort_unstable_by_key(|e| e.0);
    out.dedup_by(|b, a| {
        let same = a.0 == b.0;
        if same {
            a.1 += b.1;
            a.2 += b.2;
        }
        same
    });
    out
}

/// The spreadsheet gradient: each player of a lineup takes the team's error
/// `y - p` in proportion to their own chance of knowing the question.
fn share_grads(field: &Field, params: &[QParams], cfg: &Config) -> Grads {
    let mut out = Grads::new();
    for t in field.teams.iter().filter(|t| !t.z.is_empty()) {
        let mut acc = vec![(0.0, 0.0); t.z.len()];
        for (q, &y) in t.mask.iter().enumerate().filter(|(_, &y)| y != SKIP) {
            share_question(t, params[q].b, y, &mut acc, cfg);
        }
        out.extend(t.members.iter().zip(&acc).map(|(&i, &(g, info))| (i, g, info)));
    }
    merge(out)
}

fn share_question(t: &TeamObs, b: f64, y: u8, acc: &mut [(f64, f64)], cfg: &Config) {
    let pi: Vec<f64> = t.z.iter().map(|&zi| -(-clamped_exp(zi - b, cfg)).exp_m1()).collect();
    let sum: f64 = pi.iter().sum();
    let miss: f64 = pi.iter().fold(1.0, |m, p| m * (1.0 - p));
    if sum <= 0.0 {
        return;
    }
    let p_team = 1.0 - miss;
    let d = ((y == CORRECT) as u8 as f64) - p_team;
    for (k, &p) in pi.iter().enumerate() {
        let share = p / sum;
        acc[k].0 += d * share;
        acc[k].1 += p_team * miss * share;
    }
}
