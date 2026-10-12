//! The question model of DESIGN 3: every question gets a difficulty from the
//! lineups that played it, and every player a strength from the questions.

mod fit;
mod gradients;
mod likelihood;
mod pairs;
mod players;

pub use fit::fit_question;
pub use gradients::{lineup_fisher, player_grads, team_grads_by_id};
pub use players::{Pin, PlayerState, Players};

use crate::chemistry::Chemistry;
use crate::config::Config;
use crate::data::{Format, TeamResult, Tournament, SKIP};
use crate::util::Rng;
use likelihood::{clamped_exp, intensity, ll_from_u, log_p_step, slope, u_scale};
use rayon::prelude::*;

const HALF: f64 = 0.5;
/// trueDL reads a reference lineup of six equal players and reports ten times
/// the share of the package it would miss.
const REF_LINEUP: f64 = 6.0;
const TRUEDL_SCALE: f64 = 10.0;
/// `realised_strength` brackets the root this far either side of zero and
/// bisects it this many times.
const STRENGTH_BRACKET: f64 = 15.0;
const BISECTIONS: usize = 60;

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
    /// Each member's slope, read off `z`.
    pub k: Vec<f64>,
    pub members: Vec<u32>,
    /// The team id the lineup played under.
    pub team: u32,
    pub mask: Vec<u8>,
    pub half: u8,
    /// Finished first, sole or shared.
    pub first: bool,
}

pub struct Field {
    pub teams: Vec<TeamObs>,
    pub by_q: Vec<Vec<u32>>,
    pub n_q: usize,
    pub weight: f64,
    /// The global discrimination this tournament's questions are read at:
    /// `question.a_in_person` at an in-person table, 1 anywhere else.
    pub a_fix: f64,
    /// The global collaboration exponent, `question.m_fixed`.
    pub m_fix: f64,
}

/// `question.a_in_person` where it applies, 1 elsewhere.
pub fn a_of(f: Format, cfg: &Config) -> f64 {
    match f {
        Format::InPerson => cfg.question.a_in_person,
        _ => 1.0,
    }
}

/// Builds the per-question view of a tournament, with the crossfit halves assigned.
pub fn build_field(t: &Tournament, players: &Players, chem: &Chemistry, cfg: &Config) -> Field {
    let vbar = players.editor_mean(t);
    let a_fix = a_of(t.format, cfg);
    let mut rng = Rng::new(cfg.numeric.crossfit_seed ^ (t.id as u64));
    let best = t.teams.iter().map(|r| r.position).filter(|&p| p > 0.0).fold(f64::INFINITY, f64::min);
    let obs = |tr: &TeamResult| {
        let z = lineup(tr, players, &vbar, chem, a_fix, t.day, cfg);
        TeamObs {
        k: z.iter().map(|&zi| slope(zi, cfg)).collect(),
        z,
        members: tr.members.clone(),
        team: tr.team,
        mask: tr.mask.clone(),
        half: rng.bit(),
        first: tr.position == best,
    }};
    let teams: Vec<TeamObs> = t.teams.iter().map(obs).collect();
    let by_q = questions(&teams, t.n_q);
    Field { teams, by_q, n_q: t.n_q, weight: t.weight, a_fix, m_fix: cfg.question.m_fixed }
}

/// Each member's strength as this tournament reads it. Chemistry multiplies
/// the team's whole intensity by `exp(chem_u T)`; because `u = Σ exp(a(z_i − b))`,
/// that is exactly `chem_u T / a` added to every member's strength, so it needs
/// no new term anywhere else — and it reaches the difficulties and the player
/// gradients for free.
fn lineup(tr: &TeamResult, players: &Players, vbar: &[f64], chem: &Chemistry, a_fix: f64, day: i32, cfg: &Config) -> Vec<f64> {
    let bonus = if cfg.question.chem_u != 0.0 { cfg.question.chem_u * chem.t(&tr.members) / a_fix } else { 0.0 };
    let k = cfg.question.var_boost;
    let spread = |i: usize| if k != 0.0 { k * a_fix * players.sigma_at(i, day, cfg).powi(2) } else { 0.0 };
    let bonus = bonus + players.team_effect(tr.team);
    tr.members.iter().map(|&i| players.theta(i as usize) + players.affinity(i as usize, vbar) + bonus + spread(i as usize)).collect()
}

/// For every question, the lineups that answered or missed it.
fn questions(teams: &[TeamObs], n_q: usize) -> Vec<Vec<u32>> {
    let mut by_q = vec![Vec::new(); n_q];
    for (i, tm) in teams.iter().enumerate().filter(|(_, tm)| !tm.z.is_empty()) {
        for (q, _) in tm.mask.iter().enumerate().filter(|(_, &v)| v != SKIP) {
            by_q[q].push(i as u32);
        }
    }
    by_q
}

pub fn fit_all(field: &Field, sel: Option<u8>, cfg: &Config) -> Vec<QParams> {
    (0..field.n_q).into_par_iter().map(|q| fit_question(field, q, sel, cfg)).collect()
}

/// Summed negative log-likelihood and observation count over the selected half.
/// The questions are scored in parallel and added in order, so the sum is the
/// same on every run.
pub fn score(field: &Field, params: &[QParams], sel: Option<u8>, cfg: &Config) -> (f64, u64) {
    let one = |q: usize| {
        let p = &params[q];
        let (a, m) = (p.a(), p.m());
        let teams = field.by_q[q].iter().map(|&ti| &field.teams[ti as usize]);
        teams.filter(|t| sel.is_none_or(|h| t.half == h)).fold((0.0, 0), |(s, n), t| {
            (s - ll_from_u(intensity(&t.z, &t.k, a, p.b, cfg), m, t.mask[q], cfg), n + 1)
        })
    };
    let per_q: Vec<(f64, u64)> = (0..field.n_q).into_par_iter().map(one).collect();
    per_q.into_iter().fold((0.0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
}

/// DESIGN-adjacent: ten times the share of the package a reference lineup of six
/// players of strength `theta_ref` would miss. None when nothing was answered.
pub fn truedl(field: &Field, params: &[QParams], cfg: &Config) -> Option<f64> {
    let th = cfg.truedl.theta_ref;
    let asked = params.iter().enumerate().filter(|(q, _)| field.by_q.get(*q).is_some_and(|v| !v.is_empty()));
    let (sum, n) = asked.fold((0.0, 0), |(sum, n), (_, p)| {
        let u = REF_LINEUP * clamped_exp(p.a() * (th - p.b), cfg);
        (sum + (p.m() * log_p_step(u)).exp(), n + 1)
    });
    (n > 0).then(|| TRUEDL_SCALE * (1.0 - sum / n as f64))
}

/// Strength at which a lone player matches `target` correct answers on these
/// questions; the realised team strength used to calibrate `sigma_model`.
pub fn realised_strength(params: &[QParams], mask: &[u8], target: f64, cfg: &Config) -> Option<f64> {
    let expected = |s: f64| -> f64 {
        let asked = mask.iter().zip(params).filter(|(&y, _)| y != SKIP);
        asked
            .map(|(_, p)| {
                let u = clamped_exp(p.a() * (s - p.b), cfg);
                (p.m() * log_p_step(u * u_scale(u, cfg))).exp()
            })
            .sum()
    };
    let (mut lo, mut hi) = (-STRENGTH_BRACKET, STRENGTH_BRACKET);
    if expected(lo) >= target || expected(hi) <= target {
        return None;
    }
    for _ in 0..BISECTIONS {
        let mid = HALF * (lo + hi);
        if expected(mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(HALF * (lo + hi))
}
