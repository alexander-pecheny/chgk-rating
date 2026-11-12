//! The London Elo (https://elo-chgk.uk), reimplemented from its model page
//! (https://elo-chgk.uk/model). Players carry an Elo and a volatility; every
//! question gets a difficulty and a sport fitted to the lineups that played it,
//! calibrated so the expected takes match the observed ones; a lineup takes a
//! question by the hazard sum of its players.
//!
//! The model page stops at the forecast: it does not say how Elo and volatility
//! move, and its author says the rules have grown since (2026-11). Everything
//! below is read off what the site prints: the tournament pages give every
//! player's Elo before and change and every lineup's expected score, the
//! player list gives every volatility (`tools/fetch/elo_pages.py`,
//! `tools/elo_check.py`). Fitted in November 2026 on 300 tournaments and
//! 37 000 players; none of it is the author's code.
//!
//! The forecast departs from the page in three places: `rho` starts at 0.68,
//! the slope rises below 2000, and a lineup of more than six sums its hazards
//! at `(6 / n)^BENCH` (the site's rule for those is not this one; this is the
//! closest one-number fit).
//!
//! A player moves by the weight times `ALPHA + (vol / VOL_SCALE)^VOL_POW` times
//! the team's surplus in questions, never further than `SHIFT_CAP` of the
//! uniform shift of the whole lineup that would explain the surplus alone, nor
//! more than `DELTA_MAX` either way. The weight is the format's, divided by
//! `1 + WEEK_K (n - 1)` for a player's `n` synchronous (or asynchronous) games
//! of the week, and lowered for a pack that is easy for the lineup. On top
//! come `INFLATION_Q` per question the team missed, weighted by the week
//! alone, and a bonus for the first three places in person. Precision gains
//! `INFO_SCALE` of the weighted Fisher information of the tournament, at most
//! `INFO_CAP` of what it was. A newcomer starts where the flag of their first
//! team puts them.
//!
//! Known gaps: the site keeps covariances between players and this does not;
//! the site's scale sits some 60 Elo above this replay's, a gap that opens
//! before 2010 and holds since; tournaments are rated on their first day here
//! and on their last there.
use super::{blank_event, bare_player_event, Args, Changes, Fallible, PlayerRow, Release, Rival, Schedule};
use crate::config::Config;
use crate::data::{Arena, Format, TeamResult, Tournament, CORRECT, SKIP};
use crate::output::{TournamentMetrics, Writer};
use crate::team_event::TeamEvent;
use crate::util::{format_date, next_thursday, parse_date, spearman};
use rayon::prelude::*;
use std::collections::HashMap;
use std::f64::consts::LN_10;
use std::time::Instant;

/// A new player's Elo and volatility.
const ELO_NEW: f64 = 2000.0;
const VOL_NEW: f64 = 1000.0;
/// The standard sport, `ln 10 / 400`, and its bounds as ratios.
const SPORT0: f64 = LN_10 / 400.0;
const SPORT_MIN: f64 = 1.0 / 3.0;
const SPORT_MAX: f64 = 2.0;
/// Strength of the prior `(ln sport - ln sport0)^2`; the page does not give it.
const LAMBDA: f64 = 2.0;
/// `slope(R) = max(SLOPE_MIN, 1 / (1 + 2 max(R - 2000, 0) / 1000))`.
const SLOPE_MIN: f64 = 0.40;
const SLOPE_FROM: f64 = 2000.0;
const SLOPE_PER: f64 = 2.0 / 1000.0;
/// Below `SLOPE_FROM` the slope rises, `1 + SLOPE_LOW (2000 - R)`: the page says 1
/// there, the site's expected scores say otherwise.
const SLOPE_LOW: f64 = 2.0 / 1000.0;
/// `rho(mean Elo) = clip(0.68 + 0.0002 (mean - 2000), 0.68, 0.90)`; 0.65 for async.
/// The page gives 0.70; the site's expected scores are those of 0.68.
const RHO_BASE: f64 = 0.68;
const RHO_PER: f64 = 0.00020;
const RHO_MAX: f64 = 0.90;
const RHO_ASYNC: f64 = 0.65;
/// What the site shows for a question everybody or nobody took.
const DIFF_ALL: f64 = 0.0;
const DIFF_NONE: f64 = 5000.0;
/// Format weights, as the site's tournament list prints them.
const W_IN_PERSON: f64 = 1.0;
const W_STRICT_SYNC: f64 = 1.0;
const W_SYNC: f64 = 0.85;
const W_ASYNC: f64 = 0.25;
/// The update read off the published deltas; `--alpha` and friends override.
/// A player moves by `ALPHA + (vol / VOL_SCALE)^VOL_POW` per question of surplus.
const ALPHA: f64 = 1.0;
const VOL_SCALE: f64 = 53.0;
const VOL_POW: f64 = 1.75;
/// Inflation: a fixed amount per tournament, and an amount per question the team missed.
const INFLATION: f64 = 0.0;
const INFLATION_Q: f64 = 0.010;
/// `--h2-info` reads the information as `w h^2 / SIGMA_V^2`, the rule before November 2026.
const SIGMA_V: f64 = 4.5;
/// The share of the Fisher information of a tournament a player's precision
/// gains, and the most it gains, as a multiple of the precision it had.
const INFO_SCALE: f64 = 0.5;
const INFO_CAP: f64 = 1.1;
/// A player's k-th synchronous (or asynchronous) game of a calendar week weighs
/// `1 / (1 + WEEK_K (n - 1))` of its format's weight, `n` the games of that format that week.
const WEEK_K: f64 = 0.3;
/// A pack whose median difficulty is less than `EASY_X` above the lineup's mean
/// Elo weighs less, down to `EASY_Y` where the two meet.
const EASY_X: f64 = 300.0;
const EASY_Y: f64 = 0.75;
/// The first three places of an in-person tournament add `PRIZE_TOP - PRIZE_STEP * place`.
const PRIZE_TOP: f64 = 1.65;
const PRIZE_STEP: f64 = 0.4;
const PRIZE_PLACES: f64 = 3.5;
/// A lineup of more than `SEATS` sums its hazards at `(SEATS / n)^BENCH`.
const SEATS: f64 = 6.0;
const BENCH: f64 = 0.6;
/// Where a newcomer starts by the flag of their first team: student, school or
/// senior school, middle school, junior school (the rating site's flag ids).
const FLAG_STARTS: [(i64, f64); 5] = [(4, 1950.0), (3, 1900.0), (38, 1900.0), (37, 1850.0), (39, 1800.0)];
/// Thursday 1970-01-01 is day 0; this brings a day onto weeks that start on Monday.
const MONDAY_SHIFT: i32 = 3;
const DAYS_PER_WEEK: i32 = 7;
/// No player moves further than this in one tournament; the site's pages show
/// newcomers stopped at exactly -100.
const DELTA_MAX: f64 = 100.0;
/// Nor further than this share of the lineup's uniform shift, which is where
/// the site's high-volatility players stop.
const SHIFT_CAP: f64 = 0.6;
/// `--exp-hazard` clamps its exponent here.
const EXP_MAX: f64 = 30.0;
/// Search ranges and iteration counts of the numerics.
const DIFF_LO: f64 = -3000.0;
const DIFF_HI: f64 = 8000.0;
/// Brent's search on log-sport stops within 0.1% of the sport.
const BRENT_ITERS: usize = 40;
const BRENT_TOL: f64 = 1e-4;
const BRENT_EPS: f64 = 1e-3;
/// `(3 - sqrt 5) / 2`, the share of the interval a golden step takes.
const GOLDEN_SECTION: f64 = 0.381_966_011_250_105_1;
const NEWTON_ITERS: usize = 40;
const CALIB_TOL: f64 = 1e-6;
const SHIFT_MAX: f64 = 1000.0;
const SHIFT_TOL: f64 = 1e-6;
const P_EPS: f64 = 1e-12;
const VOL_FLOOR: f64 = 1e-6;
const ACTIVE_DAYS: i32 = 365;
/// The fixed pack of the forecast that reads nothing of the tournament itself:
/// standard-sport questions from `PACK_FROM` to `PACK_TO` Elo every `PACK_STEP`.
const PACK_FROM: f64 = 1600.0;
const PACK_TO: f64 = 3600.0;
const PACK_STEP: f64 = 50.0;
const CHECKPOINT_EVERY: i64 = 20;
const LOG_EVERY: usize = 1000;

fn weight(f: Format, kn: &Knobs) -> f64 {
    match f {
        Format::InPerson => kn.w_in_person,
        Format::StrictSync => W_STRICT_SYNC,
        Format::Sync => kn.w_sync,
        Format::Async => kn.w_async,
    }
}

fn slope(r: f64, kn: &Knobs) -> f64 {
    if r < SLOPE_FROM {
        return 1.0 + kn.slope_low * (SLOPE_FROM - r);
    }
    (1.0 / (1.0 + kn.slope_per * (r - SLOPE_FROM))).max(SLOPE_MIN)
}

/// The Monday-based week of a day.
fn week(day: i32) -> i32 {
    (day + MONDAY_SHIFT).div_euclid(DAYS_PER_WEEK)
}

fn rho(elos: &[f64], f: Format, kn: &Knobs) -> f64 {
    if f == Format::Async {
        return RHO_ASYNC;
    }
    let mean = elos.iter().sum::<f64>() / elos.len() as f64;
    (kn.rho_base + kn.rho_per * (mean - SLOPE_FROM)).clamp(kn.rho_base, RHO_MAX.max(kn.rho_base))
}

/// The tunable part of the update.
#[derive(Clone, Copy)]
struct Knobs {
    alpha: f64,
    vol_scale: f64,
    vol_pow: f64,
    inflation: f64,
    inflation_q: f64,
    sigma_v: f64,
    /// Information as `w h^2 / sigma_v^2`, not the Fisher sum over questions.
    h2_info: bool,
    info_scale: f64,
    info_cap: f64,
    week_k: f64,
    easy_x: f64,
    easy_y: f64,
    prize_top: f64,
    prize_step: f64,
    bench: f64,
    slope_low: f64,
    /// Every newcomer starts at `ELO_NEW`, whatever the flag of their first team.
    flat_start: bool,
    lambda: f64,
    w_in_person: f64,
    w_sync: f64,
    w_async: f64,
    /// Split the surplus by each player's share of the team's derivative, not equally.
    credit_grad: bool,
    /// Cap a step at `shift_cap` of the lineup's uniform shift.
    cap_shift: bool,
    shift_cap: f64,
    slope_per: f64,
    rho_base: f64,
    rho_per: f64,
    /// A2's `player.mu_young` in Elo: a player whose first game is in a young
    /// field, on a team with no school or student flag, starts this much below `ELO_NEW`.
    young: f64,
    /// A2's hazard: a player's `u_i` is `e^z`, not `softplus(z)`.
    exp_hazard: bool,
    /// A2's `team.s2.k_ip_new` in Elo: at an in-person table the forecast reads
    /// each player `ip_new / (1 + in-person games)` lower; the update does not.
    ip_new: f64,
}

/// A lineup as the hazard model reads it: each player's Elo and slope, and rho.
struct Lineup {
    elo: Vec<f64>,
    slope: Vec<f64>,
    rho: f64,
    /// What the hazard sum is multiplied by: 1 up to `SEATS` players.
    scale: f64,
    exp_hazard: bool,
}

/// One question for one lineup: the team probability, and the pieces the
/// derivatives need.
struct Take {
    p: f64,
    /// d P_team / d diff.
    dp_dd: f64,
}

impl Lineup {
    fn new(elo: Vec<f64>, f: Format, kn: &Knobs) -> Lineup {
        let rho = rho(&elo, f, kn);
        let scale = (SEATS / (elo.len() as f64).max(SEATS)).powf(kn.bench);
        Lineup { slope: elo.iter().map(|&r| slope(r, kn)).collect(), elo, rho, scale, exp_hazard: kn.exp_hazard }
    }

    /// A player's hazard `u` at `z` and its derivative `du/dz`.
    fn hazard(&self, z: f64) -> (f64, f64) {
        if self.exp_hazard {
            let u = z.min(EXP_MAX).exp();
            (u, u)
        } else {
            softplus_sigmoid(z)
        }
    }

    /// `P_team = 1 - exp(-(sum u_i^(1/rho))^rho)`, `u_i = -ln(1 - P_i)`, shifted by `shift` Elo.
    fn take(&self, d: f64, s: f64, shift: f64) -> Take {
        let (inv, mut sm, mut dsm) = (1.0 / self.rho, 0.0, 0.0);
        for (&r, &k) in self.elo.iter().zip(&self.slope) {
            // u_i = -ln(1 - P_i) = softplus(k s (r - d)), computed stably.
            let (u, p) = self.hazard(k * s * (r + shift - d));
            let ui = u.max(P_EPS).powf(inv);
            sm += ui;
            // d u / d d = -k s P_i, and d u^(1/rho) = (1/rho) u^(1/rho - 1) du.
            dsm += inv * ui / u.max(P_EPS) * (-k * s * p);
        }
        let h = (self.scale * sm).powf(self.rho);
        let dh = if sm > 0.0 { self.rho * h / sm * dsm } else { 0.0 };
        let e = (-h).exp();
        Take { p: (1.0 - e).clamp(P_EPS, 1.0 - P_EPS), dp_dd: e * dh }
    }

    /// Expected score on the fixed pack.
    fn pack(&self) -> f64 {
        let n = ((PACK_TO - PACK_FROM) / PACK_STEP).round() as usize;
        (0..=n).map(|i| self.take(PACK_FROM + i as f64 * PACK_STEP, SPORT0, 0.0).p).sum()
    }

    /// d P_team / d Elo_i for every player, slopes held fixed.
    fn grad(&self, d: f64, s: f64, out: &mut [f64]) -> f64 {
        let inv = 1.0 / self.rho;
        let ups: Vec<(f64, f64)> = self.elo.iter().zip(&self.slope).map(|(&r, &k)| self.hazard(k * s * (r - d))).collect();
        let sm: f64 = ups.iter().map(|&(u, _)| u.max(P_EPS).powf(inv)).sum();
        let (h, outer) = ((self.scale * sm).powf(self.rho), self.scale.powf(self.rho) * sm.powf(self.rho - 1.0));
        let e = (-h).exp();
        for (i, &k) in self.slope.iter().enumerate() {
            let (u, p) = ups[i];
            out[i] = e * outer * u.max(P_EPS).powf(inv - 1.0) * p * k * s;
        }
        1.0 - e
    }
}

/// `ln(1 + e^z)` and `1 / (1 + e^-z)` from one exponential.
fn softplus_sigmoid(z: f64) -> (f64, f64) {
    let e = (-z.abs()).exp();
    let sp = z.max(0.0) + e.ln_1p();
    let sg = if z >= 0.0 { 1.0 / (1.0 + e) } else { e / (1.0 + e) };
    (sp, sg)
}

/// The question's observations: lineup index and whether it took it.
type Obs = Vec<(usize, bool)>;

/// The difficulty at which the lineups' expected takes equal the observed ones,
/// for sport `s`: safeguarded Newton on a decreasing function. Also returns the
/// penalised log-loss there, from the same pass.
fn calibrate(ls: &[&Lineup], obs: &Obs, s: f64, start: f64, lambda: f64) -> (f64, f64) {
    let target = obs.iter().filter(|o| o.1).count() as f64;
    let (mut lo, mut hi, mut d) = (DIFF_LO, DIFF_HI, start.clamp(DIFF_LO, DIFF_HI));
    let mut data = 0.0;
    for _ in 0..NEWTON_ITERS {
        let (mut f, mut df) = (-target, 0.0);
        data = 0.0;
        for &(k, y) in obs {
            let t = ls[k].take(d, s, 0.0);
            f += t.p;
            df += t.dp_dd;
            data -= if y { t.p.ln() } else { (1.0 - t.p).ln() };
        }
        if f.abs() < CALIB_TOL {
            break;
        }
        if f > 0.0 {
            lo = d;
        } else {
            hi = d;
        }
        let next = d - f / df;
        d = if df < 0.0 && next > lo && next < hi { next } else { 0.5 * (lo + hi) };
    }
    (d, data + lambda * (s / SPORT0).ln().powi(2))
}

/// Sport by Brent's minimisation over its log within the bounds, difficulty by
/// the calibration at each candidate.
fn fit_question(ls: &[&Lineup], obs: &Obs, lambda: f64) -> (f64, f64) {
    let takes = obs.iter().filter(|o| o.1).count();
    if takes == obs.len() {
        return (DIFF_ALL, SPORT0);
    }
    if takes == 0 {
        return (DIFF_NONE, SPORT0);
    }
    let mut d = 2000.0;
    let x = brent((SPORT0 * SPORT_MIN).ln(), (SPORT0 * SPORT_MAX).ln(), |x| {
        let (nd, l) = calibrate(ls, obs, x.exp(), d, lambda);
        d = nd;
        l
    });
    (calibrate(ls, obs, x.exp(), d, lambda).0, x.exp())
}

/// The minimum of `f` on `[a, b]`: golden section with parabolic steps
/// (Brent 1973, as in Numerical Recipes' `brent`).
// sloplint: ignore[magic-numbers] the halves and doubles are the textbook algorithm's own
fn brent(mut a: f64, mut b: f64, mut f: impl FnMut(f64) -> f64) -> f64 {
    let g = GOLDEN_SECTION;
    let (mut x, mut d, mut e) = (a + g * (b - a), 0.0f64, 0.0f64);
    let (mut w, mut v) = (x, x);
    let mut fx = f(x);
    let (mut fw, mut fv) = (fx, fx);
    for _ in 0..BRENT_ITERS {
        let m = 0.5 * (a + b);
        let tol = BRENT_TOL * x.abs() + BRENT_EPS;
        if (x - m).abs() <= 2.0 * tol - 0.5 * (b - a) {
            break;
        }
        let mut golden = true;
        if e.abs() > tol {
            let r = (x - w) * (fx - fv);
            let q = (x - v) * (fx - fw);
            let (mut p, mut q) = ((x - v) * q - (x - w) * r, 2.0 * (q - r));
            if q > 0.0 {
                p = -p;
            }
            q = q.abs();
            if p.abs() < (0.5 * q * e).abs() && p > q * (a - x) && p < q * (b - x) {
                e = d;
                d = p / q;
                let u = x + d;
                if u - a < 2.0 * tol || b - u < 2.0 * tol {
                    d = tol.copysign(m - x);
                }
                golden = false;
            }
        }
        if golden {
            e = if x >= m { a - x } else { b - x };
            d = g * e;
        }
        let u = if d.abs() >= tol { x + d } else { x + tol.copysign(d) };
        let fu = f(u);
        if fu <= fx {
            if u >= x { a = x } else { b = x }
            (v, fv, w, fw, x, fx) = (w, fw, x, fx, u, fu);
        } else {
            if u < x { a = u } else { b = u }
            if fu <= fw || w == x {
                (v, fv, w, fw) = (w, fw, u, fu);
            } else if fu <= fv || v == x || v == w {
                (v, fv) = (u, fu);
            }
        }
    }
    x
}

/// Whether a question's parameters move ratings: not everybody and not nobody.
fn live(d: f64) -> bool {
    d != DIFF_ALL && d != DIFF_NONE
}

struct Model {
    elo: Vec<f64>,
    vol: Vec<f64>,
    games: Vec<u32>,
    games_ip: Vec<u32>,
    last: Vec<i32>,
}

/// One team's outcome: expected score over every fitted question, and the
/// changes of its players.
struct TeamFit {
    expected: f64,
    deltas: Vec<f64>,
    info: Vec<f64>,
}

/// What a lineup has at stake in a tournament: the format's weight, each
/// player's own weight and volatility, and the bonus of the place.
struct Stake<'a> {
    format: f64,
    weights: &'a [f64],
    vols: &'a [f64],
    prize: f64,
}

/// What a tournament adds to a player's precision.
fn information(kn: &Knobs, w: f64, h: f64, fisher: f64, vol: f64) -> f64 {
    if kn.h2_info {
        return w * h * h / (kn.sigma_v * kn.sigma_v);
    }
    (kn.info_scale * w * fisher).min(kn.info_cap / (vol * vol))
}

fn rate_team(l: &Lineup, r: &TeamResult, qs: &[(f64, f64)], st: &Stake, kn: &Knobs) -> TeamFit {
    let (ws, vols) = (st.weights, st.vols);
    let n = l.elo.len();
    let mut expected = 0.0;
    let (mut e_live, mut s_live, mut h, mut fisher) = (0.0, 0.0, vec![0.0; n], vec![0.0; n]);
    let mut g = vec![0.0; n];
    let mut played: Vec<(f64, f64)> = Vec::new();
    let mut missed = 0.0;
    for (q, &(d, s)) in qs.iter().enumerate() {
        if !s.is_finite() {
            continue;
        }
        let y = r.mask.get(q).copied().filter(|&y| y != SKIP);
        let p = if live(d) && y.is_some() { l.grad(d, s, &mut g) } else { l.take(d, s, 0.0).p };
        expected += p;
        missed += y.is_some_and(|y| y != CORRECT) as u8 as f64;
        if let (true, Some(y)) = (live(d), y) {
            e_live += p;
            s_live += (y == CORRECT) as u8 as f64;
            let v = (p * (1.0 - p)).max(P_EPS);
            for i in 0..n {
                h[i] += g[i];
                fisher[i] += g[i] * g[i] / v;
            }
            played.push((d, s));
        }
    }
    if played.is_empty() {
        return TeamFit { expected, deltas: vec![0.0; n], info: vec![0.0; n] };
    }
    let surplus = s_live - e_live;
    let shift = uniform_shift(l, &played, s_live);
    let easy = easy_pack(l, &mut played, kn);
    let hsum: f64 = h.iter().sum();
    let deltas = (0..n).map(|i| {
        let w = ws[i] * easy;
        let share = if kn.credit_grad && hsum > 0.0 { n as f64 * h[i] / hsum } else { 1.0 };
        let raw = w * (kn.alpha + (vols[i] / kn.vol_scale).powf(kn.vol_pow)) * surplus * share;
        let cap = if kn.cap_shift { (kn.shift_cap * w * shift.abs()).min(DELTA_MAX) } else { DELTA_MAX };
        // Inflation is not weighted by the format, only by how busy the week was.
        raw.clamp(-cap, cap) + ws[i] / st.format * (kn.inflation + kn.inflation_q * missed) + w * st.prize
    }).collect();
    let info = (0..n).map(|i| information(kn, ws[i] * easy, h[i], fisher[i], vols[i])).collect();
    TeamFit { expected, deltas, info }
}

/// What an easy pack is worth to this lineup: 1 when the median difficulty of
/// the questions it played is `easy_x` or more above its mean Elo, `easy_y`
/// when it is at or below it.
fn easy_pack(l: &Lineup, played: &mut [(f64, f64)], kn: &Knobs) -> f64 {
    if kn.easy_y >= 1.0 || kn.easy_x <= 0.0 {
        return 1.0;
    }
    played.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mid = played.len() / 2;
    let median = if played.len() % 2 == 1 { played[mid].0 } else { 0.5 * (played[mid - 1].0 + played[mid].0) };
    let mean = l.elo.iter().sum::<f64>() / l.elo.len() as f64;
    (kn.easy_y + (1.0 - kn.easy_y) * (median - mean) / kn.easy_x).clamp(kn.easy_y, 1.0)
}

/// The bonus of a prize place at an in-person tournament.
fn prize(place: f64, f: Format, kn: &Knobs) -> f64 {
    if f != Format::InPerson || !(place > 0.0 && place <= PRIZE_PLACES) {
        return 0.0;
    }
    (kn.prize_top - kn.prize_step * place).max(0.0)
}

/// The shift of every player's Elo at which the lineup's expected score over
/// `played` equals `score`: safeguarded Newton, since moving every player up
/// by `x` is moving every question down by `x`.
fn uniform_shift(l: &Lineup, played: &[(f64, f64)], score: f64) -> f64 {
    let (mut lo, mut hi, mut x) = (-SHIFT_MAX, SHIFT_MAX, 0.0);
    for _ in 0..NEWTON_ITERS {
        let (mut f, mut df) = (-score, 0.0);
        for &(d, s) in played {
            let t = l.take(d, s, x);
            f += t.p;
            df -= t.dp_dd;
        }
        if f.abs() < SHIFT_TOL {
            break;
        }
        if f < 0.0 {
            lo = x;
        } else {
            hi = x;
        }
        let next = x - f / df;
        x = if df > 0.0 && next > lo && next < hi { next } else { 0.5 * (lo + hi) };
    }
    x
}

/// Every question's (difficulty, sport), or NaN for one nobody played.
fn fit_questions(t: &Tournament, ls: &[Option<Lineup>], lambda: f64) -> Vec<(f64, f64)> {
    let lineups: Vec<&Lineup> = ls.iter().flatten().collect();
    let index: Vec<Option<usize>> = {
        let mut i = 0;
        ls.iter().map(|l| l.as_ref().map(|_| { i += 1; i - 1 })).collect()
    };
    (0..t.n_q).into_par_iter().map(|q| {
        let obs: Obs = t.teams.iter().zip(&index).filter_map(|(r, &k)| {
            let k = k?;
            r.mask.get(q).filter(|&&y| y != SKIP).map(|&y| (k, y == CORRECT))
        }).collect();
        if obs.is_empty() { (f64::NAN, f64::NAN) } else { fit_question(&lineups, &obs, lambda) }
    }).collect()
}

pub struct Elo {
    cfg: Config,
    arena: Arena,
    m: Model,
    kn: Knobs,
    prev: Changes,
    to_day: i32,
    from_day: i32,
    start: Instant,
    /// Whether `elo_trace` exists yet in the output.
    traced: bool,
    /// Where a newcomer of this (tournament, team) starts, by the team's flags.
    starts: HashMap<(i64, i64), f64>,
    /// The week each tournament is counted in: that of its last day.
    weeks: Vec<i32>,
    /// How many games of a format a player has in a week: (player, week, async).
    busy: HashMap<(u32, i32, bool), u32>,
}

/// The flag starts of every result and the last day of every tournament, read
/// from the mirror; the shared loader carries neither.
fn side_tables(cfg: &Config) -> Fallible<(HashMap<(i64, i64), f64>, HashMap<i64, i32>)> {
    let conn = rusqlite::Connection::open_with_flags(&cfg.data.db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut starts = HashMap::new();
    let mut st = conn.prepare("SELECT id, team_id, flags FROM tournament_results WHERE flags IS NOT NULL AND flags != '[]'")?;
    let mut rows = st.query([])?;
    while let Some(row) = rows.next()? {
        let (id, team, flags): (i64, i64, String) = (row.get(0)?, row.get(1)?, row.get(2)?);
        let Ok(serde_json::Value::Array(list)) = serde_json::from_str(&flags) else { continue };
        let ids: Vec<i64> = list.iter().filter_map(|f| f.get("id").and_then(serde_json::Value::as_i64)).collect();
        let start = FLAG_STARTS.iter().filter(|(f, _)| ids.contains(f)).map(|&(_, e)| e).fold(f64::INFINITY, f64::min);
        if start.is_finite() {
            starts.insert((id, team), start);
        }
    }
    let mut ends = HashMap::new();
    let mut st = conn.prepare("SELECT id, substr(date_end,1,10) FROM tournaments WHERE date_end IS NOT NULL")?;
    let mut rows = st.query([])?;
    while let Some(row) = rows.next()? {
        let (id, end): (i64, String) = (row.get(0)?, row.get(1)?);
        if let Some(day) = parse_date(&end) {
            ends.insert(id, day);
        }
    }
    Ok((starts, ends))
}

impl Elo {
    pub fn open(args: &Args) -> Fallible<Elo> {
        let mut cfg = Config::load(&args.or("--config", "config_elo.toml"))?;
        let d = &mut cfg.data;
        for (flag, slot) in [("--from", &mut d.from), ("--to", &mut d.to), ("--out", &mut d.out_path), ("--forecast-from", &mut d.forecast_from)] {
            if let Some(v) = args.get(flag) {
                *slot = v;
            }
        }
        let num = |flag: &str, default: f64| args.get(flag).and_then(|v| v.parse().ok()).unwrap_or(default);
        let kn = Knobs {
            alpha: num("--alpha", ALPHA),
            vol_scale: num("--vol-scale", VOL_SCALE),
            vol_pow: num("--vol-pow", VOL_POW),
            inflation: num("--inflation", INFLATION),
            inflation_q: num("--inflation-q", INFLATION_Q),
            sigma_v: num("--sigma-v", SIGMA_V),
            h2_info: args.has("--h2-info"),
            info_scale: num("--info-scale", INFO_SCALE),
            info_cap: num("--info-cap", INFO_CAP),
            week_k: num("--week-k", WEEK_K),
            easy_x: num("--easy-x", EASY_X),
            easy_y: num("--easy-y", EASY_Y),
            prize_top: num("--prize-top", PRIZE_TOP),
            prize_step: num("--prize-step", PRIZE_STEP),
            bench: num("--bench", BENCH),
            slope_low: num("--slope-low", SLOPE_LOW),
            flat_start: args.has("--flat-start"),
            lambda: num("--lambda", LAMBDA),
            w_in_person: num("--w-in-person", W_IN_PERSON),
            w_sync: num("--w-sync", W_SYNC),
            w_async: num("--w-async", W_ASYNC),
            credit_grad: args.has("--credit-grad"),
            cap_shift: !args.has("--no-cap"),
            shift_cap: num("--shift-cap", SHIFT_CAP),
            slope_per: num("--slope-per", SLOPE_PER),
            rho_base: num("--rho-base", RHO_BASE),
            rho_per: num("--rho-per", RHO_PER),
            young: num("--young", 0.0),
            exp_hazard: args.has("--exp-hazard"),
            ip_new: num("--ip-new", 0.0),
        };
        let start = Instant::now();
        let from_day = parse_date(&cfg.data.from).ok_or("bad --from")?;
        let to_day = parse_date(&cfg.data.to).ok_or("bad --to")?;
        let arena: Arena = crate::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
        let np = arena.n_players();
        println!("loaded {} tournaments, {np} players in {:.1}s", arena.tournaments.len(), start.elapsed().as_secs_f64());
        let m = Model { elo: vec![ELO_NEW; np], vol: vec![VOL_NEW; np], games: vec![0; np], games_ip: vec![0; np], last: vec![i32::MIN / 2; np] };
        let (starts, ends) = side_tables(&cfg)?;
        let weeks: Vec<i32> = arena.tournaments.iter().map(|t| week(ends.get(&t.id).copied().unwrap_or(t.day).max(t.day))).collect();
        let mut busy: HashMap<(u32, i32, bool), u32> = HashMap::new();
        for (t, &wk) in arena.tournaments.iter().zip(&weeks) {
            if !matches!(t.format, Format::Sync | Format::Async) {
                continue;
            }
            for r in &t.teams {
                let mut ms = r.members.clone();
                ms.sort_unstable();
                ms.dedup();
                for p in ms {
                    *busy.entry((p, wk, t.format == Format::Async)).or_insert(0) += 1;
                }
            }
        }
        Ok(Elo { cfg, arena, m, kn, prev: Changes::new(np), to_day, from_day, start, traced: false, starts, weeks, busy })
    }
}

impl Elo {
    /// Where a newcomer of each team of `t` starts: where the flag of the team
    /// puts them, and with no flag `--young` lower in a young field.
    fn first_elos(&self, t: &Tournament) -> Vec<f64> {
        let young = t.young && self.kn.young != 0.0;
        let unflagged = ELO_NEW - if young { self.kn.young } else { 0.0 };
        t.teams.iter().map(|r| {
            let flag = if self.kn.flat_start { None } else { self.starts.get(&(t.id, self.arena.team_ids[r.team as usize])) };
            flag.copied().unwrap_or(unflagged)
        }).collect()
    }

    /// Each member's weight at tournament `i`: the format's, less for every
    /// other game of that format the player has that week.
    fn weights(&self, i: usize, members: &[u32]) -> Vec<f64> {
        let t = &self.arena.tournaments[i];
        let wt = weight(t.format, &self.kn);
        if !matches!(t.format, Format::Sync | Format::Async) || self.kn.week_k <= 0.0 {
            return vec![wt; members.len()];
        }
        let key = (self.weeks[i], t.format == Format::Async);
        members.iter().map(|&p| {
            let n = self.busy.get(&(p, key.0, key.1)).copied().unwrap_or(1).max(1);
            wt / (1.0 + self.kn.week_k * (n - 1) as f64)
        }).collect()
    }
}

impl Rival for Elo {
    fn schedule(&self) -> Schedule {
        Schedule { first_day: next_thursday(self.from_day), last_day: Some(self.to_day), checkpoint_every: CHECKPOINT_EVERY }
    }

    fn out_path(&self) -> &str {
        &self.cfg.data.out_path
    }

    fn n_tournaments(&self) -> usize {
        self.arena.tournaments.len()
    }

    fn after(&self, i: usize, day: i32) -> bool {
        self.arena.tournaments[i].day >= day
    }

    fn rate(&mut self, w: &Writer, i: usize, _next: &Release) -> Fallible<()> {
        let t = &self.arena.tournaments[i];
        let fc = t.forecast_only;
        let members: Vec<Vec<u32>> = t.teams.iter().map(|r| {
            let mut v = r.members.clone();
            v.sort_unstable();
            v.dedup();
            v
        }).collect();
        let first = self.first_elos(t);
        let seeded = |m: &Model, k: usize, p: usize| if m.games[p] == 0 { first[k] } else { m.elo[p] };
        if !fc {
            for (k, ms) in members.iter().enumerate() {
                for &p in ms {
                    self.m.elo[p as usize] = seeded(&self.m, k, p as usize);
                }
            }
        }
        let ls: Vec<Option<Lineup>> = members.iter().enumerate().map(|(k, ms)| {
            (!ms.is_empty()).then(|| Lineup::new(ms.iter().map(|&p| seeded(&self.m, k, p as usize)).collect(), t.format, &self.kn))
        }).collect();
        let qs = fit_questions(t, &ls, self.kn.lambda);
        let fresh = t.format == Format::InPerson && self.kn.ip_new != 0.0;
        let read: Vec<Option<Lineup>> = if fresh {
            members.iter().enumerate().map(|(k, ms)| {
                (!ms.is_empty()).then(|| {
                    let at = |&p: &u32| seeded(&self.m, k, p as usize) - self.kn.ip_new / (1.0 + self.m.games_ip[p as usize] as f64);
                    Lineup::new(ms.iter().map(at).collect(), t.format, &self.kn)
                })
            }).collect()
        } else {
            Vec::new()
        };
        let wt = weight(t.format, &self.kn);
        let fits: Vec<Option<TeamFit>> = ls.par_iter().zip(&t.teams).zip(&members).map(|((l, r), ms)| {
            let l = l.as_ref()?;
            let vols: Vec<f64> = ms.iter().map(|&p| self.m.vol[p as usize]).collect();
            let ws = self.weights(i, ms);
            let st = Stake { format: wt, weights: &ws, vols: &vols, prize: prize(r.position, t.format, &self.kn) };
            Some(rate_team(l, r, &qs, &st, &self.kn))
        }).collect();
        if !self.traced {
            // Every move of every player, for a fit that blends player ratings with Elo's.
            w.conn.execute_batch("CREATE TABLE IF NOT EXISTS elo_trace (player_id INTEGER, day INTEGER, elo REAL)")?;
            self.traced = true;
        }
        let mut trace = w.conn.prepare_cached("INSERT INTO elo_trace VALUES (?1, ?2, ?3)")?;
        let mut evs = Vec::new();
        let mut pevs = Vec::new();
        for (k, f) in fits.iter().enumerate() {
            let r = &t.teams[k];
            let Some(f) = f else { continue };
            let l = ls[k].as_ref().unwrap();
            for (j, &p) in members[k].iter().enumerate().filter(|_| !fc) {
                let p = p as usize;
                let (before, vol_before) = (self.m.elo[p], self.m.vol[p]);
                self.m.elo[p] += f.deltas[j];
                let prec = 1.0 / self.m.vol[p].powi(2) + f.info[j];
                self.m.vol[p] = prec.max(VOL_FLOOR).powf(-0.5);
                self.m.games[p] += 1;
                self.m.games_ip[p] += (t.format == Format::InPerson) as u32;
                self.m.last[p] = t.day;
                trace.execute(rusqlite::params![self.arena.player_ids[p], t.day, self.m.elo[p]])?;
                if t.format == Format::InPerson {
                    let mut e = bare_player_event(self.arena.player_ids[p], before, self.m.elo[p]);
                    (e.sigma_before, e.sigma_after) = (vol_before, self.m.vol[p]);
                    e.games_ip = self.m.games[p] as i64 - 1;
                    pevs.push(e);
                }
            }
            let mean = l.elo.iter().sum::<f64>() / l.elo.len() as f64;
            let (expected, x) = match read.get(k).and_then(Option::as_ref) {
                Some(lf) => (qs.iter().filter(|q| q.1.is_finite()).map(|&(d, s)| lf.take(d, s, 0.0).p).sum(), lf.pack()),
                None => (f.expected, l.pack()),
            };
            evs.push(TeamEvent {
                forecast: expected,
                x,
                actual_place: r.position,
                pred_place: r.pred_pos,
                mu_before: mean,
                mu_after: mean,
                rated: r.rated,
                ..blank_event(r.team)
            });
        }
        let date = format_date(t.day);
        w.team_events(t.id, &date, &self.arena.team_ids, &evs)?;
        if !pevs.is_empty() {
            w.player_events(t.id, &pevs)?;
        }
        let place = super::ranks_desc(&evs.iter().map(|e| e.forecast).collect::<Vec<_>>());
        let actual: Vec<f64> = evs.iter().map(|e| e.actual_place).collect();
        let field = TournamentMetrics::field(t.id, &date, t.format.name(), wt, t.teams.len(), t.n_q);
        w.tournament_metrics(&TournamentMetrics { sp_forecast: spearman(&place, &actual), ..field })?;
        let mut st = w.conn.prepare_cached("INSERT INTO question_params VALUES (?1,?2,?3,?4,1.0)")?;
        for (q, &(d, s)) in qs.iter().enumerate().filter(|(_, p)| p.0.is_finite()) {
            st.execute(rusqlite::params![t.id, q as i64, d, s])?;
        }
        if i % LOG_EVERY == 0 {
            let n = self.arena.tournaments.len();
            println!("{i:6}/{n} {date} {:.0}s", self.start.elapsed().as_secs_f64());
        }
        Ok(())
    }

    fn publish(&mut self, w: &Writer, r: &Release) -> Fallible<()> {
        let active: Vec<usize> = (0..self.arena.n_players()).filter(|&i| r.day - self.m.last[i] <= ACTIVE_DAYS).collect();
        w.release(r.id, &format_date(r.day), 0.0, 0.0, active.len())?;
        if !self.cfg.data.write_rows(r.id, r.last) {
            return Ok(());
        }
        let rows: Vec<PlayerRow> = active.iter().map(|&i| {
            let change = self.prev.step(i, self.m.elo[i]);
            (self.arena.player_ids[i], self.m.elo[i], self.m.vol[i], self.m.games[i], change)
        }).collect();
        w.player_ratings(r.id, &crate::output::published(&rows), |t| t)?;
        Ok(())
    }

    fn finish(&mut self, _w: &Writer, releases: i64) -> Fallible<()> {
        println!("{releases} releases, done in {:.1}s", self.start.elapsed().as_secs_f64());
        Ok(())
    }
}
