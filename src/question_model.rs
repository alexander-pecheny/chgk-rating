use crate::config::{Config, PUpdate, QFit, QGradient};
use crate::data::{Tournament, CORRECT, SKIP};
use rayon::prelude::*;

#[derive(Debug, Clone, Copy)]
pub struct QParams {
    pub b: f64,
    pub la: f64,
    pub lm: f64,
}

impl QParams {
    pub fn a(&self) -> f64 {
        self.la.exp()
    }
    pub fn m(&self) -> f64 {
        self.lm.exp()
    }
}

/// One team of one tournament, reduced to what the question model needs.
pub struct TeamObs {
    pub z: Vec<f64>,
    pub members: Vec<u32>,
    pub mask: Vec<u8>,
    pub half: u8,
}

pub struct Field {
    pub teams: Vec<TeamObs>,
    pub by_q: Vec<Vec<u32>>,
    pub n_q: usize,
    pub weight: f64,
}

pub struct PlayerState {
    pub theta_raw: f64,
    pub var: f64,
    /// Per-player volatility: how fast strength may drift, learned from surprise.
    pub vol: f64,
    pub aff: Vec<f64>,
    pub aff_var: Vec<f64>,
    pub games: u32,
    pub last_day: i32,
    pub seen: bool,
}

pub struct Players {
    pub state: Vec<PlayerState>,
    pub shift_cum: f64,
    pub editors: Vec<f64>,
    pub rank: usize,
    /// Count, sum and sum of squares of the standardised surprise.
    pub zstat: (f64, f64, f64),
}

impl Players {
    pub fn new(n_players: usize, n_editors: usize, cfg: &Config) -> Players {
        let rank = cfg.question.aff_rank;
        let mut rng = crate::util::Rng::new(cfg.numeric.crossfit_seed ^ 0x5eed);
        let editors = (0..n_editors * rank)
            .map(|_| rng.normal() * cfg.question.s_aff)
            .collect();
        let state = (0..n_players)
            .map(|_| PlayerState {
                theta_raw: cfg.player.mu_new,
                var: cfg.player.sigma_new * cfg.player.sigma_new,
                vol: cfg.player.tau_p,
                aff: vec![0.0; rank],
                aff_var: vec![cfg.question.s_aff * cfg.question.s_aff; rank],
                games: 0,
                last_day: 0,
                seen: false,
            })
            .collect();
        Players {
            state,
            shift_cum: 0.0,
            editors,
            rank,
            zstat: (0.0, 0.0, 0.0),
        }
    }

    pub fn theta(&self, i: usize) -> f64 {
        self.state[i].theta_raw - self.shift_cum
    }

    /// Uncertainty as of `day`, including the diffusion since the last game.
    pub fn sigma_at(&self, i: usize, day: i32, cfg: &Config) -> f64 {
        let s = &self.state[i];
        if cfg.player.update == PUpdate::Schedule {
            return if s.games >= 5 { 0.1 } else { 0.5 };
        }
        let dt = ((day - s.last_day).max(0) as f64) / 365.25;
        (s.var + s.vol * s.vol * dt).sqrt()
    }

    pub fn editor_mean(&self, t: &Tournament) -> Vec<f64> {
        let mut v = vec![0.0; self.rank];
        if self.rank == 0 || t.editors.is_empty() {
            return v;
        }
        for &e in &t.editors {
            let s = e as usize * self.rank;
            for (k, x) in v.iter_mut().enumerate() {
                *x += self.editors[s + k];
            }
        }
        let n = t.editors.len() as f64;
        for x in v.iter_mut() {
            *x /= n;
        }
        v
    }

    pub fn affinity(&self, i: usize, vbar: &[f64]) -> f64 {
        self.state[i].aff.iter().zip(vbar).map(|(a, v)| a * v).sum()
    }

    /// Restarts the chronology with the players of `snap` already at strength,
    /// which is what removes the cold start of the first seasons. Players absent
    /// from `snap` enter later at the newcomer prior, so nothing after the
    /// snapshot date has seen the future.
    pub fn warm_restart(&mut self, snap: &[Option<f64>], day: i32, cfg: &Config) {
        let var = cfg.player.warm_sigma * cfg.player.warm_sigma;
        for (s, warm) in self.state.iter_mut().zip(snap) {
            s.games = 0;
            s.last_day = day;
            match warm {
                Some(theta) => {
                    s.theta_raw = *theta;
                    s.var = var;
                    s.seen = true;
                }
                None => {
                    s.theta_raw = cfg.player.mu_new;
                    s.var = cfg.player.sigma_new * cfg.player.sigma_new;
                    s.seen = false;
                }
            }
            s.vol = cfg.player.tau_p;
        }
        self.shift_cum = 0.0;
        self.zstat = (0.0, 0.0, 0.0);
    }

    /// Published strength of every player who has already appeared.
    pub fn snapshot(&self) -> Vec<Option<f64>> {
        (0..self.state.len())
            .map(|i| self.state[i].seen.then(|| self.theta(i)))
            .collect()
    }

    /// Pulls each lineup member towards their lineup's mean, a blunt answer to the
    /// noisy-OR habit of crediting one answer to whoever is already strongest.
    pub fn shrink_to_lineups(&mut self, t: &Tournament, cfg: &Config) {
        let f = cfg.player.teammate_shrink;
        if f <= 0.0 {
            return;
        }
        for team in &t.teams {
            if team.members.len() < 2 {
                continue;
            }
            let mean = team
                .members
                .iter()
                .map(|&i| self.state[i as usize].theta_raw)
                .sum::<f64>()
                / team.members.len() as f64;
            for &i in &team.members {
                let s = &mut self.state[i as usize];
                s.theta_raw += f * (mean - s.theta_raw);
            }
        }
    }

    /// Gives players their first appearance a prior on the current gauge.
    pub fn seed(&mut self, t: &Tournament, cfg: &Config) {
        for team in &t.teams {
            for &i in &team.members {
                let s = &mut self.state[i as usize];
                if !s.seen {
                    s.seen = true;
                    s.theta_raw = cfg.player.mu_new + self.shift_cum;
                    s.last_day = t.day;
                }
            }
        }
    }

    /// One Kalman step per player from this tournament's gradients, then the
    /// editor SGD. `ed_seen` counts each editor's tournaments so far.
    pub fn absorb(
        &mut self,
        t: &Tournament,
        grads: &[(u32, f64, f64)],
        ed_seen: &mut [f64],
        collect: bool,
        cfg: &Config,
    ) {
        let free_aff = self.rank > 0;
        let clamp = 5.0 * cfg.question.s_aff;
        let tau_aff2 = cfg.player.tau_aff * cfg.player.tau_aff;
        let (lo, hi, eta) = (
            cfg.player.v_min.ln(),
            cfg.player.v_max.ln(),
            cfg.player.v_eta,
        );
        let vbar = self.editor_mean(t);
        let mut ed_grad = vec![0.0; self.rank];
        for &(i, g, info) in grads {
            let s = &mut self.state[i as usize];
            let dt = ((t.day - s.last_day).max(0) as f64) / 365.25;
            if cfg.player.update == PUpdate::Schedule {
                let step = cfg.player.eta0 / (cfg.player.c0 + s.games as f64).sqrt();
                s.theta_raw += step * t.weight * g;
            } else {
                s.var += s.vol * s.vol * dt;
                let prior = s.var;
                let ieff = t.weight * info * cfg.player.info_scale;
                let prec = 1.0 / prior + ieff.max(cfg.numeric.info_floor);
                s.theta_raw += (t.weight * cfg.player.info_scale * g / prec)
                    .clamp(-cfg.numeric.theta_max_step, cfg.numeric.theta_max_step);
                s.var = 1.0 / prec;
                if ieff > cfg.numeric.info_floor {
                    // Standardised surprise of this tournament against the prior.
                    let z = (g / info) / (prior + 1.0 / ieff).sqrt();
                    let lv = s.vol.ln();
                    s.vol = (lv + eta * (if z.abs() > 1.0 { hi } else { lo } - lv)).exp();
                    if collect && s.games > 6 {
                        self.zstat.0 += 1.0;
                        self.zstat.1 += z;
                        self.zstat.2 += z * z;
                    }
                }
            }
            s.games += 1;
            s.last_day = t.day;
            if free_aff {
                #[allow(clippy::needless_range_loop)]
                for k in 0..self.rank {
                    ed_grad[k] += g * s.aff[k];
                    let av = &mut s.aff_var[k];
                    *av += tau_aff2 * dt;
                    let p = 1.0 / *av + t.weight * info * vbar[k] * vbar[k];
                    s.aff[k] = (s.aff[k] + t.weight * g * vbar[k] / p).clamp(-clamp, clamp);
                    *av = 1.0 / p;
                }
            }
        }
        if !free_aff || t.editors.is_empty() {
            return;
        }
        let n_ed = t.editors.len() as f64;
        let lr = cfg.question.eta_v * t.weight;
        let shrink = 1.0 / (cfg.question.s_aff * cfg.question.s_aff);
        for &e in &t.editors {
            ed_seen[e as usize] += 1.0;
            // The prior is spread over the editor's own tournaments, not reapplied at each.
            let pull = shrink / ed_seen[e as usize];
            let base = e as usize * self.rank;
            let slot = &mut self.editors[base..base + self.rank];
            for (v, g) in slot.iter_mut().zip(&ed_grad) {
                *v = (*v + lr * (g / n_ed - *v * pull)).clamp(-clamp, clamp);
            }
        }
    }
}

/// DESIGN-adjacent: ten times the share of the package a reference lineup of six
/// players of strength `theta_ref` would miss. None when nothing was answered.
pub fn truedl(field: &Field, params: &[QParams], cfg: &Config) -> Option<f64> {
    let th = cfg.truedl.theta_ref;
    let mut sum = 0.0;
    let mut n = 0u32;
    for (q, p) in params.iter().enumerate() {
        if field.by_q.get(q).is_none_or(|v| v.is_empty()) {
            continue;
        }
        let u = 6.0 * clamped_exp(p.a() * (th - p.b), cfg);
        sum += (p.m() * log_p_step(u)).exp();
        n += 1;
    }
    (n > 0).then(|| 10.0 * (1.0 - sum / n as f64))
}

/// Strength at which a lone player matches `target` correct answers on these
/// questions; the realised team strength used to calibrate `sigma_model`.
pub fn realised_strength(
    params: &[QParams],
    mask: &[u8],
    target: f64,
    cfg: &Config,
) -> Option<f64> {
    let expected = |s: f64| -> f64 {
        mask.iter()
            .zip(params)
            .filter(|(&y, _)| y != SKIP)
            .map(|(_, p)| {
                let u = clamped_exp(p.a() * (s - p.b), cfg);
                (p.m() * log_p_step(u * u_scale(u, cfg))).exp()
            })
            .sum()
    };
    let (mut lo, mut hi) = (-15.0f64, 15.0f64);
    if expected(lo) >= target || expected(hi) <= target {
        return None;
    }
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if expected(mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

fn clamped_exp(x: f64, cfg: &Config) -> f64 {
    x.clamp(cfg.numeric.exp_lo, cfg.numeric.exp_hi).exp()
}

fn intensity(z: &[f64], a: f64, b: f64, cfg: &Config) -> f64 {
    z.iter().map(|&zi| clamped_exp(a * (zi - b), cfg)).sum()
}

/// Saturating the total intensity keeps one observation from costing more than `u_max` nats.
fn u_scale(u: f64, cfg: &Config) -> f64 {
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
fn log_p_step(u: f64) -> f64 {
    if u < std::f64::consts::LN_2 {
        (-(-u).exp_m1()).ln()
    } else {
        (-(-u).exp()).ln_1p()
    }
}

/// Log-likelihood of one team-question given the total intensity `u`.
fn ll_from_u(u: f64, m: f64, y: u8, cfg: &Config) -> f64 {
    let g0 = log_p_step(u * u_scale(u, cfg));
    if y == CORRECT {
        m * g0
    } else {
        (-(m * g0).exp_m1()).ln()
    }
}

struct Deriv {
    g: [f64; 3],
    h: [f64; 6],
}

fn team_deriv(z: &[f64], y: u8, p: &QParams, cfg: &Config) -> Deriv {
    let a = p.a();
    let m = p.m();
    let (mut u, mut u1, mut u2) = (0.0, 0.0, 0.0);
    for &zi in z {
        let d = zi - p.b;
        let l = clamped_exp(a * d, cfg);
        u += l;
        u1 += d * l;
        u2 += d * d * l;
    }
    let s = u_scale(u, cfg);
    u *= s;
    u1 *= s;
    u2 *= s;
    let t = (-u).exp();
    let om = -(-u).exp_m1();
    let g0 = log_p_step(u);
    let g1 = t / om;
    let g2 = -t / (om * om);

    let u_b = -a * u;
    let u_l = a * u1;
    let u_bb = a * a * u;
    let u_bl = -a * u - a * a * u1;
    let u_ll = a * (u1 + a * u2);

    let h_b = m * g1 * u_b;
    let h_l = m * g1 * u_l;
    let h_m = m * g0;
    let h_bb = m * (g2 * u_b * u_b + g1 * u_bb);
    let h_bl = m * (g2 * u_b * u_l + g1 * u_bl);
    let h_ll = m * (g2 * u_l * u_l + g1 * u_ll);

    let (p1, p2) = phi(m * g0, y);
    Deriv {
        g: [p1 * h_b, p1 * h_l, p1 * h_m],
        h: [
            p2 * h_b * h_b + p1 * h_bb,
            p2 * h_b * h_l + p1 * h_bl,
            p2 * h_b * h_m + p1 * h_b,
            p2 * h_l * h_l + p1 * h_ll,
            p2 * h_l * h_m + p1 * h_l,
            p2 * h_m * h_m + p1 * h_m,
        ],
    }
}

/// First and second derivative of the outcome term in `h = m * log P_step`.
fn phi(h: f64, y: u8) -> (f64, f64) {
    if y == CORRECT {
        (1.0, 0.0)
    } else {
        let a = h.exp();
        let om = -h.exp_m1();
        (-a / om, -a / (om * om))
    }
}

fn prior(p: &QParams, cfg: &Config, free_m: bool) -> (f64, [f64; 3], [f64; 3]) {
    let q = &cfg.question;
    let db = p.b - q.b_bar;
    let mut val = -0.5 * db * db / (q.s_b * q.s_b) - 0.5 * p.la * p.la / (q.s_a * q.s_a);
    let mut g = [-db / (q.s_b * q.s_b), -p.la / (q.s_a * q.s_a), 0.0];
    let mut h = [-1.0 / (q.s_b * q.s_b), -1.0 / (q.s_a * q.s_a), 0.0];
    if free_m {
        val -= 0.5 * p.lm * p.lm / (q.s_m * q.s_m);
        g[2] = -p.lm / (q.s_m * q.s_m);
        h[2] = -1.0 / (q.s_m * q.s_m);
    }
    (val, g, h)
}

fn clamp_params(p: &mut QParams, cfg: &Config, free_m: bool) {
    let n = &cfg.numeric;
    p.b = p.b.clamp(n.b_min, n.b_max);
    p.la = p.la.clamp(n.a_min.ln(), n.a_max.ln());
    p.lm = if free_m {
        p.lm.clamp(n.m_min.ln(), n.m_max.ln())
    } else {
        0.0
    };
}

fn objective(
    field: &Field,
    q: usize,
    sel: Option<u8>,
    p: &QParams,
    cfg: &Config,
    free_m: bool,
) -> f64 {
    let (a, m) = (p.a(), p.m());
    let mut v = 0.0;
    for &ti in &field.by_q[q] {
        let t = &field.teams[ti as usize];
        if sel.is_some_and(|s| t.half != s) {
            continue;
        }
        v += ll_from_u(intensity(&t.z, a, p.b, cfg), m, t.mask[q], cfg);
    }
    prior(p, cfg, free_m).0 + field.weight * v
}

fn init_params(field: &Field, q: usize, sel: Option<u8>, cfg: &Config) -> QParams {
    let (mut sum_s, mut n, mut hits) = (0.0f64, 0.0f64, 0.0f64);
    for &ti in &field.by_q[q] {
        let t = &field.teams[ti as usize];
        if sel.is_some_and(|s| t.half != s) {
            continue;
        }
        sum_s += intensity(&t.z, 1.0, 0.0, cfg).max(1e-12).ln();
        n += 1.0;
        if t.mask[q] == CORRECT {
            hits += 1.0;
        }
    }
    if n == 0.0 {
        return QParams {
            b: cfg.question.b_bar,
            la: 0.0,
            lm: 0.0,
        };
    }
    let phat = (hits / n).clamp(0.02, 0.98);
    let mut p = QParams {
        b: sum_s / n - (-(1.0 - phat).ln()).ln(),
        la: 0.0,
        lm: 0.0,
    };
    clamp_params(&mut p, cfg, false);
    p
}

/// Newton with step halving on the penalised log-likelihood of one question.
pub fn fit_question(field: &Field, q: usize, sel: Option<u8>, cfg: &Config) -> QParams {
    let free_m = cfg.question.s_m > 0.0;
    let dim = if free_m { 3 } else { 2 };
    let mut p = init_params(field, q, sel, cfg);
    if cfg.question.fit == QFit::TakeRate {
        return p;
    }
    let mut f = objective(field, q, sel, &p, cfg, free_m);
    for _ in 0..cfg.numeric.newton_iters {
        let (_, pg, ph) = prior(&p, cfg, free_m);
        let mut g = [0.0; 3];
        let mut h = [0.0; 6];
        for &ti in &field.by_q[q] {
            let t = &field.teams[ti as usize];
            if sel.is_some_and(|s| t.half != s) {
                continue;
            }
            let d = team_deriv(&t.z, t.mask[q], &p, cfg);
            for (k, v) in d.g.iter().enumerate() {
                g[k] += v;
            }
            for (k, v) in d.h.iter().enumerate() {
                h[k] += v;
            }
        }
        for (k, v) in g.iter_mut().enumerate() {
            *v = *v * field.weight + pg[k];
        }
        for v in h.iter_mut() {
            *v *= field.weight;
        }
        h[0] += ph[0];
        h[3] += ph[1];
        h[5] += ph[2];
        let Some(step) = solve(&g, &h, dim) else {
            break;
        };
        let mut scale = 1.0;
        let mut moved = 0.0;
        for _ in 0..8 {
            let mut cand = QParams {
                b: p.b + scale * step[0],
                la: p.la + scale * step[1],
                lm: p.lm + scale * step[2],
            };
            clamp_params(&mut cand, cfg, free_m);
            let fc = objective(field, q, sel, &cand, cfg, free_m);
            if fc > f {
                moved = (cand.b - p.b).abs() + (cand.la - p.la).abs() + (cand.lm - p.lm).abs();
                p = cand;
                f = fc;
                break;
            }
            scale *= 0.5;
        }
        if moved <= cfg.numeric.newton_tol {
            break;
        }
    }
    p
}

#[allow(clippy::needless_range_loop)]
/// Solves (-H + damping I) x = g for the ascent direction, raising the damping until it works.
fn solve(g: &[f64; 3], h: &[f64; 6], dim: usize) -> Option<[f64; 3]> {
    let mut lambda = 1e-8;
    for _ in 0..12 {
        let mut a = [
            [-h[0] + lambda, -h[1], -h[2]],
            [-h[1], -h[3] + lambda, -h[4]],
            [-h[2], -h[4], -h[5] + lambda],
        ];
        let mut x = [g[0], g[1], g[2]];
        let mut ok = true;
        for i in 0..dim {
            if a[i][i] <= 1e-12 {
                ok = false;
                break;
            }
            let piv = a[i][i];
            for j in i..dim {
                a[i][j] /= piv;
            }
            x[i] /= piv;
            for r in (i + 1)..dim {
                let f = a[r][i];
                for j in i..dim {
                    a[r][j] -= f * a[i][j];
                }
                x[r] -= f * x[i];
            }
        }
        if ok {
            for i in (0..dim).rev() {
                for j in (i + 1)..dim {
                    x[i] -= a[i][j] * x[j];
                }
            }
            for v in x.iter_mut().skip(dim) {
                *v = 0.0;
            }
            if x.iter().all(|v| v.is_finite() && v.abs() <= 20.0) {
                return Some(x);
            }
        }
        lambda *= 10.0;
    }
    None
}

pub fn fit_all(field: &Field, sel: Option<u8>, cfg: &Config) -> Vec<QParams> {
    (0..field.n_q)
        .into_par_iter()
        .map(|q| fit_question(field, q, sel, cfg))
        .collect()
}

/// Summed negative log-likelihood and observation count over the selected half.
pub fn score(field: &Field, params: &[QParams], sel: Option<u8>, cfg: &Config) -> (f64, u64) {
    (0..field.n_q)
        .into_par_iter()
        .map(|q| {
            let p = &params[q];
            let (a, m) = (p.a(), p.m());
            let mut s = 0.0;
            let mut n = 0u64;
            for &ti in &field.by_q[q] {
                let t = &field.teams[ti as usize];
                if sel.is_some_and(|h| t.half != h) {
                    continue;
                }
                s -= ll_from_u(intensity(&t.z, a, p.b, cfg), m, t.mask[q], cfg);
                n += 1;
            }
            (s, n)
        })
        .reduce(|| (0.0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
}

/// Gradient and observed Fisher information of the tournament log-likelihood in each theta_i,
/// summed per player and sorted by player index.
pub fn player_grads(field: &Field, params: &[QParams], cfg: &Config) -> Vec<(u32, f64, f64)> {
    if cfg.question.gradient == QGradient::Share {
        return share_grads(field, params, cfg);
    }
    let mut out: Vec<(u32, f64, f64)> = Vec::new();
    let mut lam: Vec<f64> = Vec::new();
    let mut acc: Vec<(f64, f64)> = Vec::new();
    for t in &field.teams {
        if t.z.is_empty() {
            continue;
        }
        acc.clear();
        acc.resize(t.z.len(), (0.0, 0.0));
        for (q, &y) in t.mask.iter().enumerate() {
            if y == SKIP {
                continue;
            }
            let p = &params[q];
            let (a, m) = (p.a(), p.m());
            lam.clear();
            let mut u = 0.0;
            for &zi in &t.z {
                let l = clamped_exp(a * (zi - p.b), cfg);
                lam.push(l);
                u += l;
            }
            let s = u_scale(u, cfg);
            u *= s;
            for l in lam.iter_mut() {
                *l *= s;
            }
            let tt = (-u).exp();
            let om = -(-u).exp_m1();
            let g1 = tt / om;
            let g2 = -tt / (om * om);
            let (p1, p2) = phi(m * log_p_step(u), y);
            let d1 = p1 * m * g1;
            let d2 = p2 * (m * g1) * (m * g1) + p1 * m * g2;
            for (k, &l) in lam.iter().enumerate() {
                let uz = a * l;
                acc[k].0 += d1 * uz;
                acc[k].1 -= d2 * uz * uz + d1 * a * uz;
            }
        }
        for (k, &i) in t.members.iter().enumerate() {
            out.push((i, acc[k].0, acc[k].1));
        }
    }
    merge(out)
}

fn merge(mut out: Vec<(u32, f64, f64)>) -> Vec<(u32, f64, f64)> {
    out.sort_unstable_by_key(|e| e.0);
    out.dedup_by(|b, a| {
        if a.0 == b.0 {
            a.1 += b.1;
            a.2 += b.2;
            true
        } else {
            false
        }
    });
    out
}

/// The spreadsheet gradient: each player of a lineup takes the team's error
/// `y - p` in proportion to their own chance of knowing the question.
fn share_grads(field: &Field, params: &[QParams], cfg: &Config) -> Vec<(u32, f64, f64)> {
    let mut out: Vec<(u32, f64, f64)> = Vec::new();
    let mut pi: Vec<f64> = Vec::new();
    let mut acc: Vec<(f64, f64)> = Vec::new();
    for t in &field.teams {
        if t.z.is_empty() {
            continue;
        }
        acc.clear();
        acc.resize(t.z.len(), (0.0, 0.0));
        for (q, &y) in t.mask.iter().enumerate() {
            if y == SKIP {
                continue;
            }
            let b = params[q].b;
            pi.clear();
            let (mut sum, mut miss) = (0.0, 1.0);
            for &zi in &t.z {
                let p = -(-clamped_exp(zi - b, cfg)).exp_m1();
                pi.push(p);
                sum += p;
                miss *= 1.0 - p;
            }
            if sum <= 0.0 {
                continue;
            }
            let p_team = 1.0 - miss;
            let d = ((y == CORRECT) as u8 as f64) - p_team;
            for (k, &p) in pi.iter().enumerate() {
                let share = p / sum;
                acc[k].0 += d * share;
                acc[k].1 += p_team * miss * share;
            }
        }
        for (k, &i) in t.members.iter().enumerate() {
            out.push((i, acc[k].0, acc[k].1));
        }
    }
    merge(out)
}

/// Builds the per-question view of a tournament, with the crossfit halves assigned.
pub fn build_field(t: &Tournament, players: &Players, cfg: &Config) -> Field {
    let vbar = players.editor_mean(t);
    let mut rng = crate::util::Rng::new(cfg.numeric.crossfit_seed ^ (t.id as u64));
    let mut teams = Vec::with_capacity(t.teams.len());
    for tr in &t.teams {
        let z = tr
            .members
            .iter()
            .map(|&i| players.theta(i as usize) + players.affinity(i as usize, &vbar))
            .collect::<Vec<_>>();
        teams.push(TeamObs {
            z,
            members: tr.members.clone(),
            mask: tr.mask.clone(),
            half: rng.bit(),
        });
    }
    let mut by_q = vec![Vec::new(); t.n_q];
    for (i, tm) in teams.iter().enumerate() {
        if tm.z.is_empty() {
            continue;
        }
        for (q, &v) in tm.mask.iter().enumerate() {
            if v != SKIP {
                by_q[q].push(i as u32);
            }
        }
    }
    Field {
        teams,
        by_q,
        n_q: t.n_q,
        weight: t.weight,
    }
}
