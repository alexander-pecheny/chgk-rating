//! The player state of DESIGN 3: a strength, its uncertainty and volatility,
//! the pinned format columns and the learned editor columns, and the Kalman
//! step that moves them after each tournament.

use crate::config::{AffStrength, Config, FormatEffect, PUpdate};
use crate::data::{Format, Tournament};
use crate::util::Rng;
use std::collections::HashMap;

const DAYS_PER_YEAR: f64 = 365.25;
/// Every affinity column is clamped this many prior widths out.
const CLAMP_WIDTHS: f64 = 5.0;
/// Seeds the editor vectors apart from the cross-fit halves.
const EDITOR_SALT: u64 = 0x5eed;
/// The surprise statistics count only players past their first few games.
const ZSTAT_MIN_GAMES: u32 = 6;
/// `player.update = "schedule"` has no variance; its sigma is a flag.
const SCHEDULE_ESTABLISHED: u32 = 5;
const SCHEDULE_SIGMA: (f64, f64) = (0.1, 0.5);

pub struct PlayerState {
    pub theta_raw: f64,
    pub var: f64,
    /// A second estimate of the same strength kept at a floor volatility of
    /// `player.v_fast`, so it forgets in a season what `theta_raw` forgets in a
    /// decade. Nothing but the forecast reads it; the question fit, the gauge
    /// and the published table are untouched. 0 leaves it pinned to `theta_raw`.
    pub theta_fast: f64,
    pub var_fast: f64,
    /// Per-player volatility: how fast strength may drift, learned from surprise.
    pub vol: f64,
    pub aff: Vec<f64>,
    pub aff_var: Vec<f64>,
    pub games: u32,
    pub last_day: i32,
    /// In-person tournaments absorbed, and the day of the last one. How much of
    /// a player's evidence is in person is what the format-aware quantile of
    /// DESIGN 4.1 asks, and this is the counter that answers it.
    pub games_ip: u32,
    pub last_ip: i32,
    pub seen: bool,
}

/// A column of the affinity state whose value is fixed by the tournament's
/// format rather than learned: DESIGN 3.7. The free columns after these are the
/// editor affinity of DESIGN 3.6.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pin {
    /// `phi`, read at an in-person table.
    InPerson,
    /// `psi`, read at a synchronous one.
    Sync,
}

pub struct Players {
    pub state: Vec<PlayerState>,
    pub shift_cum: f64,
    /// The gauge as published: `shift_cum` followed at a limited pace.
    pub shift_pub: f64,
    pub editors: Vec<f64>,
    pub rank: usize,
    /// The pinned columns, in order, followed by `free` learned editor columns.
    /// `rank` is the two together.
    pub pins: Vec<Pin>,
    pub free: usize,
    /// Which per-player format effect is in force; `None` is the shipped model.
    pub fmt: FormatEffect,
    /// Count, sum and sum of squares of the standardised surprise.
    pub zstat: (f64, f64, f64),
    /// Whether the editor affinity is a property of every player or of a strong
    /// one, and the `theta` the gate stands at; the gate is recomputed at each
    /// release from the established panel.
    pub aff_strength: AffStrength,
    pub aff_kappa: f64,
    pub aff_s_max: f64,
    pub aff_gate_theta: f64,
    /// The same cut point for the editor memory of the team side, which has its
    /// own share and is read by the forecast only.
    pub ed_gate_theta: f64,
}

/// The format indicator `phi` is multiplied by: DESIGN 3.7.
fn format_sign(f: Format, mode: FormatEffect) -> f64 {
    match (mode, f) {
        (FormatEffect::None, _) => 0.0,
        (_, Format::InPerson) => 1.0,
        (FormatEffect::InPerson, _) => 0.0,
        (FormatEffect::Signed, _) => -1.0,
    }
}

/// What a pinned column is read against today.
fn pin_value(p: Pin, f: Format, mode: FormatEffect) -> f64 {
    match p {
        Pin::InPerson => format_sign(f, mode),
        Pin::Sync => match f {
            Format::Sync | Format::StrictSync => 1.0,
            _ => 0.0,
        },
    }
}

/// A format effect is affinity against a fixed indicator, so it takes a column
/// of the same state; the editor columns follow it. Round 1 had room for only
/// one column and had to choose between them.
fn pins_of(cfg: &Config) -> Vec<Pin> {
    let mut pins = Vec::new();
    if cfg.question.format_effect != FormatEffect::None {
        pins.push(Pin::InPerson);
    }
    if cfg.question.sync_effect {
        pins.push(Pin::Sync);
    }
    pins
}

/// Years from `from` to `to`, never negative.
fn years(from: i32, to: i32) -> f64 {
    ((to - from).max(0) as f64) / DAYS_PER_YEAR
}

impl PlayerState {
    fn newcomer(rank: usize, aff_var: Vec<f64>, cfg: &Config) -> PlayerState {
        let var = cfg.player.sigma_new * cfg.player.sigma_new;
        PlayerState {
            theta_raw: cfg.player.mu_new,
            var,
            theta_fast: cfg.player.mu_new,
            var_fast: var,
            vol: cfg.player.tau_p,
            aff: vec![0.0; rank],
            aff_var,
            games: 0,
            last_day: 0,
            games_ip: 0,
            last_ip: 0,
            seen: false,
        }
    }

    /// Back to the start of the chronology on `day`: at the snapshot's strength
    /// for a player it holds, at the newcomer prior otherwise. Nothing
    /// snapshots the affinity, so a later pass must not inherit the value the
    /// first pass ended on: that is the future.
    fn restart(&mut self, warm: Option<f64>, day: i32, aff_prior: &[f64], cfg: &Config) {
        self.games = 0;
        self.last_day = day;
        self.games_ip = 0;
        self.last_ip = day;
        self.aff.iter_mut().for_each(|a| *a = 0.0);
        self.aff_var.copy_from_slice(aff_prior);
        let warm_var = cfg.player.warm_sigma * cfg.player.warm_sigma;
        let (theta, var) = warm.map_or((cfg.player.mu_new, cfg.player.sigma_new * cfg.player.sigma_new), |t| (t, warm_var));
        self.theta_raw = theta;
        self.var = var;
        self.seen = warm.is_some();
        self.theta_fast = self.theta_raw;
        self.var_fast = self.var;
        self.vol = cfg.player.tau_p;
    }
}

impl Players {
    pub fn new(n_players: usize, n_editors: usize, cfg: &Config) -> Players {
        let pins = pins_of(cfg);
        let free = cfg.question.aff_rank;
        let rank = pins.len() + free;
        let prior = Players::priors(&pins, free, cfg);
        let mut rng = Rng::new(cfg.numeric.crossfit_seed ^ EDITOR_SALT);
        let editors: Vec<f64> = (0..n_editors * free).map(|_| rng.normal() * cfg.question.s_aff).collect();
        Players {
            state: (0..n_players).map(|_| PlayerState::newcomer(rank, prior.clone(), cfg)).collect(),
            shift_cum: 0.0,
            shift_pub: 0.0,
            editors,
            rank,
            pins,
            free,
            fmt: cfg.question.format_effect,
            zstat: (0.0, 0.0, 0.0),
            aff_strength: cfg.question.aff_strength,
            aff_kappa: cfg.question.aff_kappa,
            aff_s_max: cfg.question.aff_s_max,
            // Nothing is above the gate until the first release sets it.
            aff_gate_theta: f64::INFINITY,
            ed_gate_theta: f64::NEG_INFINITY,
        }
    }

    /// How much of a player's editor affinity counts. The gauge centres the
    /// established panel at `theta = 0`, so `scale` is 1 for a player of average
    /// strength and grows with `theta`; `gate` is 1 above the panel's cut point
    /// and 0 below it. The format effect is never scaled: it is a property of
    /// the player, not a claim about how well they read an editor.
    pub fn aff_scale(&self, i: usize) -> f64 {
        match self.aff_strength {
            AffStrength::None => 1.0,
            AffStrength::Scale => (self.aff_kappa * self.theta(i)).exp().clamp(0.0, self.aff_s_max),
            AffStrength::Gate => (self.theta(i) >= self.aff_gate_theta) as u8 as f64,
        }
    }

    /// The `1 − share` quantile of `theta` over the established panel; `None`
    /// when there is no panel yet or no share is asked for.
    pub fn quantile(panel: &mut [f64], share: f64) -> Option<f64> {
        if panel.is_empty() || share <= 0.0 {
            return None;
        }
        panel.sort_by(f64::total_cmp);
        let k = ((1.0 - share) * (panel.len() - 1) as f64).round() as usize;
        Some(panel[k.min(panel.len() - 1)])
    }

    /// The affinity gate's cut point, recomputed at each release.
    pub fn set_gate(&mut self, panel: &mut [f64], share: f64) {
        if self.aff_strength != AffStrength::Gate {
            return;
        }
        self.aff_gate_theta = Players::quantile(panel, share).unwrap_or(f64::INFINITY);
    }

    /// The editor memory's cut point; below it a player contributes nothing.
    pub fn set_ed_gate(&mut self, panel: &mut [f64], share: f64) {
        self.ed_gate_theta = Players::quantile(panel, share).unwrap_or(f64::NEG_INFINITY);
    }

    /// Prior variance of every affinity column: each pinned one has its own
    /// width, the editor columns share `s_aff`.
    fn priors(pins: &[Pin], free: usize, cfg: &Config) -> Vec<f64> {
        let q = &cfg.question;
        let width = |p: &Pin| match p {
            Pin::InPerson => q.s_phi * q.s_phi,
            Pin::Sync => q.s_psi * q.s_psi,
        };
        let mut v: Vec<f64> = pins.iter().map(width).collect();
        v.extend(std::iter::repeat_n(q.s_aff * q.s_aff, free));
        v
    }

    /// Clamp of every affinity column, five prior widths out.
    fn clamps(&self, cfg: &Config) -> Vec<f64> {
        Players::priors(&self.pins, self.free, cfg).iter().map(|v| CLAMP_WIDTHS * v.sqrt()).collect()
    }

    pub fn theta(&self, i: usize) -> f64 {
        self.state[i].theta_raw - self.shift_cum
    }

    /// `theta` on the published scale; the same number unless
    /// `gauge.publish_step` holds the published scale back.
    pub fn published(&self, i: usize) -> f64 {
        self.state[i].theta_raw - self.shift_pub
    }

    /// The fast estimate of the same strength, on the same scale as `theta`.
    pub fn theta_fast(&self, i: usize) -> f64 {
        self.state[i].theta_fast - self.shift_cum
    }

    /// The format effect itself, whatever today's format: zero when none is in
    /// force, `phi_i` otherwise. `affinity` is what a tournament reads; this is
    /// what the diagnosis and the published table want.
    pub fn phi(&self, i: usize) -> f64 {
        match self.pins.iter().position(|p| *p == Pin::InPerson) {
            Some(k) => self.state[i].aff[k],
            None => 0.0,
        }
    }

    /// Uncertainty as of `day`, including the diffusion since the last game.
    pub fn sigma_at(&self, i: usize, day: i32, cfg: &Config) -> f64 {
        let s = &self.state[i];
        if cfg.player.update == PUpdate::Schedule {
            let established = s.games >= SCHEDULE_ESTABLISHED;
            return if established { SCHEDULE_SIGMA.0 } else { SCHEDULE_SIGMA.1 };
        }
        (s.var + s.vol * s.vol * years(s.last_day, day)).sqrt()
    }

    /// The vector a player's affinity is read against: the pinned format
    /// indicators first, then the mean editor vector of this tournament. Under
    /// the shipped switches the only column is the in-person indicator, so
    /// `affinity` returns `phi_i` in person and zero elsewhere.
    pub fn editor_mean(&self, t: &Tournament) -> Vec<f64> {
        let mut v: Vec<f64> = self.pins.iter().map(|p| pin_value(*p, t.format, self.fmt)).collect();
        v.resize(self.rank, 0.0);
        if self.free == 0 || t.editors.is_empty() {
            return v;
        }
        let cols = &mut v[self.pins.len()..];
        for &e in &t.editors {
            let s = e as usize * self.free;
            cols.iter_mut().zip(&self.editors[s..s + self.free]).for_each(|(x, e)| *x += e);
        }
        let n = t.editors.len() as f64;
        cols.iter_mut().for_each(|x| *x /= n);
        v
    }

    /// What a tournament adds to a player's strength: the pinned format columns
    /// as they stand, plus the editor columns times `aff_scale`.
    pub fn affinity(&self, i: usize, vbar: &[f64]) -> f64 {
        let s = &self.state[i];
        let n_pin = self.pins.len();
        let pinned: f64 = s.aff[..n_pin].iter().zip(vbar).map(|(a, v)| a * v).sum();
        if self.free == 0 {
            return pinned;
        }
        let free: f64 = s.aff[n_pin..].iter().zip(&vbar[n_pin..]).map(|(a, v)| a * v).sum();
        pinned + self.aff_scale(i) * free
    }

    /// Restarts the chronology with the players of `snap` already at strength,
    /// which is what removes the cold start of the first seasons. Players absent
    /// from `snap` enter later at the newcomer prior, so nothing after the
    /// snapshot date has seen the future.
    pub fn warm_restart(&mut self, snap: &[Option<f64>], day: i32, cfg: &Config) {
        let aff_prior = Players::priors(&self.pins, self.free, cfg);
        for (s, warm) in self.state.iter_mut().zip(snap) {
            s.restart(*warm, day, &aff_prior, cfg);
        }
        self.shift_cum = 0.0;
        self.shift_pub = 0.0;
        self.zstat = (0.0, 0.0, 0.0);
    }

    /// Published strength of every player who has already appeared.
    pub fn snapshot(&self) -> Vec<Option<f64>> {
        (0..self.state.len()).map(|i| self.state[i].seen.then(|| self.theta(i))).collect()
    }

    /// Pulls each lineup member towards their lineup's mean, a blunt answer to the
    /// noisy-OR habit of crediting one answer to whoever is already strongest.
    pub fn shrink_to_lineups(&mut self, t: &Tournament, cfg: &Config) {
        let f = cfg.player.teammate_shrink;
        if f <= 0.0 {
            return;
        }
        for team in t.teams.iter().filter(|team| team.members.len() >= 2) {
            let sum: f64 = team.members.iter().map(|&i| self.state[i as usize].theta_raw).sum();
            let mean = sum / team.members.len() as f64;
            for &i in &team.members {
                let s = &mut self.state[i as usize];
                s.theta_raw += f * (mean - s.theta_raw);
            }
        }
    }

    /// Gives players their first appearance a prior on the current gauge:
    /// `mu_new`, moved `seed_w` of the way to their rated teammates' mean.
    pub fn seed(&mut self, t: &Tournament, cfg: &Config) {
        for team in &t.teams {
            let start = self.newcomer_start(&team.members, cfg);
            for &i in &team.members {
                let s = &mut self.state[i as usize];
                if !s.seen {
                    s.seen = true;
                    s.theta_raw = start;
                    s.theta_fast = start;
                    s.last_day = t.day;
                }
            }
        }
    }

    fn newcomer_start(&self, members: &[u32], cfg: &Config) -> f64 {
        let flat = cfg.player.mu_new + self.shift_cum;
        if cfg.player.seed_w == 0.0 {
            return flat;
        }
        let rated: Vec<f64> = members.iter().map(|&i| &self.state[i as usize]).filter(|s| s.seen).map(|s| s.theta_raw).collect();
        if rated.is_empty() {
            return flat;
        }
        let mean = rated.iter().sum::<f64>() / rated.len() as f64;
        flat + cfg.player.seed_w * (mean - flat)
    }

    /// One Kalman step per player from this tournament's gradients, then the
    /// editor SGD. `ed_seen` counts each editor's tournaments so far.
    pub fn absorb(&mut self, t: &Tournament, grads: &[(u32, f64, f64)], ed_seen: &mut [f64], collect: bool, cfg: &Config) {
        let step = Step::new(self, t, cfg);
        // The affinity multiplier as the tournament read it, before the update.
        let aff_sc: HashMap<usize, f64> = if self.free > 0 {
            grads.iter().map(|&(i, _, _)| (i as usize, self.aff_scale(i as usize))).collect()
        } else {
            HashMap::new()
        };
        let mut ed_grad = vec![0.0; self.free];
        for &(i, g, info) in grads {
            let s = &mut self.state[i as usize];
            let obs = Obs { g, info, dt: years(s.last_day, t.day) };
            if let Some(z) = step.theta(s, &obs, cfg) {
                if collect && s.games > ZSTAT_MIN_GAMES {
                    self.zstat.0 += 1.0;
                    self.zstat.1 += z;
                    self.zstat.2 += z * z;
                }
            }
            step.fast(s, &obs, cfg);
            step.pinned(s, &obs, &self.pins, cfg);
            s.games += 1;
            s.last_day = t.day;
            if t.format == Format::InPerson {
                s.games_ip += 1;
                s.last_ip = t.day;
            }
            if self.free > 0 {
                // `theta` has already moved, so read the multiplier off the value
                // the tournament was scored at.
                let sc = aff_sc.get(&(i as usize)).copied().unwrap_or(1.0);
                step.editor_cols(s, &obs, sc, &mut ed_grad);
            }
        }
        if self.free > 0 && !t.editors.is_empty() {
            self.learn_editors(t, &ed_grad, ed_seen, cfg);
        }
    }

    /// The editor SGD: each of today's editors moves along the summed gradient
    /// of its columns, pulled towards zero by a prior spread over the editor's
    /// own tournaments rather than reapplied at each.
    fn learn_editors(&mut self, t: &Tournament, ed_grad: &[f64], ed_seen: &mut [f64], cfg: &Config) {
        let n_ed = t.editors.len() as f64;
        let lr = cfg.question.eta_v * t.weight;
        let shrink = 1.0 / (cfg.question.s_aff * cfg.question.s_aff);
        let clamp = CLAMP_WIDTHS * cfg.question.s_aff;
        for &e in &t.editors {
            ed_seen[e as usize] += 1.0;
            let pull = shrink / ed_seen[e as usize];
            let base = e as usize * self.free;
            for (v, g) in self.editors[base..base + self.free].iter_mut().zip(ed_grad) {
                *v = (*v + lr * (g / n_ed - *v * pull)).clamp(-clamp, clamp);
            }
        }
    }
}

/// One player's evidence from one tournament: the gradient, the information,
/// and the years since the player's last game.
struct Obs {
    g: f64,
    info: f64,
    dt: f64,
}

/// What every player's step in one tournament shares.
struct Step {
    weight: f64,
    day: i32,
    vbar: Vec<f64>,
    pin_clamp: Vec<f64>,
    n_pin: usize,
    free: usize,
    /// log volatility bounds and the rate it moves at.
    vol: (f64, f64, f64),
    /// The editor columns' diffusion per year and their clamp.
    tau_aff2: f64,
    aff_clamp: f64,
}

impl Step {
    fn new(players: &Players, t: &Tournament, cfg: &Config) -> Step {
        let p = &cfg.player;
        Step {
            weight: t.weight,
            day: t.day,
            vbar: players.editor_mean(t),
            pin_clamp: players.clamps(cfg),
            n_pin: players.pins.len(),
            free: players.free,
            vol: (p.v_min.ln(), p.v_max.ln(), p.v_eta),
            tau_aff2: p.tau_aff * p.tau_aff,
            aff_clamp: CLAMP_WIDTHS * cfg.question.s_aff,
        }
    }

    fn max_step(cfg: &Config) -> f64 {
        cfg.numeric.theta_max_step
    }

    /// `theta`: the schedule's fixed step, or the Kalman step with the
    /// volatility moved by the standardised surprise, which is returned.
    fn theta(&self, s: &mut PlayerState, o: &Obs, cfg: &Config) -> Option<f64> {
        let p = &cfg.player;
        if p.update == PUpdate::Schedule {
            let step = p.eta0 / (p.c0 + s.games as f64).sqrt();
            s.theta_raw += step * self.weight * o.g;
            return None;
        }
        s.var += s.vol * s.vol * o.dt;
        let prior = s.var;
        let ieff = self.weight * o.info * p.info_scale;
        let prec = 1.0 / prior + ieff.max(cfg.numeric.info_floor);
        s.theta_raw += (self.weight * p.info_scale * o.g / prec).clamp(-Step::max_step(cfg), Step::max_step(cfg));
        s.var = 1.0 / prec;
        if ieff <= cfg.numeric.info_floor {
            return None;
        }
        // Standardised surprise of this tournament against the prior.
        let z = (o.g / o.info) / (prior + 1.0 / ieff).sqrt();
        let (lo, hi, eta) = self.vol;
        let lv = s.vol.ln();
        s.vol = (lv + eta * (if z.abs() > 1.0 { hi } else { lo } - lv)).exp();
        Some(z)
    }

    /// The fast channel takes the same step against the same gradient, at a
    /// volatility floor of its own, so an established player's estimate can
    /// still move on a season of results. Only the forecast reads it.
    fn fast(&self, s: &mut PlayerState, o: &Obs, cfg: &Config) {
        let p = &cfg.player;
        if p.v_fast <= 0.0 || p.update == PUpdate::Schedule {
            return;
        }
        s.var_fast += p.v_fast * p.v_fast * o.dt;
        let ieff = self.weight * o.info * p.info_scale;
        let prec = 1.0 / s.var_fast + ieff.max(cfg.numeric.info_floor);
        s.theta_fast += (self.weight * p.info_scale * o.g / prec).clamp(-Step::max_step(cfg), Step::max_step(cfg));
        s.var_fast = 1.0 / prec;
    }

    /// A pinned column enters the likelihood exactly as `theta` does, times its
    /// format indicator, so it takes the same Kalman step against the same
    /// gradient — and only where the indicator is not zero, which is what keeps
    /// a format's evidence to its own column.
    fn pinned(&self, s: &mut PlayerState, o: &Obs, pins: &[Pin], cfg: &Config) {
        let p = &cfg.player;
        let tau2 = p.tau_phi * p.tau_phi;
        let max = Step::max_step(cfg);
        for k in (0..self.n_pin).filter(|&k| self.vbar[k] != 0.0) {
            let own = p.tau_phi_own && pins[k] == Pin::InPerson && s.games_ip > 0;
            let dt = if own { years(s.last_ip, self.day) } else { o.dt };
            let av = &mut s.aff_var[k];
            *av += tau2 * dt;
            let ieff = self.weight * o.info * p.info_scale;
            let prec = 1.0 / *av + ieff.max(cfg.numeric.info_floor);
            let step = self.vbar[k] * self.weight * p.info_scale * o.g / prec;
            s.aff[k] = (s.aff[k] + step.clamp(-max, max)).clamp(-self.pin_clamp[k], self.pin_clamp[k]);
            *av = 1.0 / prec;
        }
    }

    /// The editor columns enter the strength as `sc * aff * vbar`, so the chain
    /// rule puts `sc` in the gradient and `sc^2` in the information; a gated-out
    /// player has neither and never moves.
    fn editor_cols(&self, s: &mut PlayerState, o: &Obs, sc: f64, ed_grad: &mut [f64]) {
        let tau2 = self.tau_aff2;
        let clamp = self.aff_clamp;
        for (k, grad) in ed_grad.iter_mut().enumerate().take(self.free) {
            let j = self.n_pin + k;
            *grad += o.g * sc * s.aff[j];
            let av = &mut s.aff_var[j];
            *av += tau2 * o.dt;
            let p = 1.0 / *av + self.weight * o.info * (sc * self.vbar[j]) * (sc * self.vbar[j]);
            s.aff[j] = (s.aff[j] + self.weight * o.g * sc * self.vbar[j] / p).clamp(-clamp, clamp);
            *av = 1.0 / p;
        }
    }
}
