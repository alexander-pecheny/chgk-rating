//! The London Elo (https://elo-chgk.uk), reimplemented from its model page
//! (https://elo-chgk.uk/model). Players carry an Elo and a volatility; every
//! question gets a difficulty and a sport fitted to the lineups that played it,
//! calibrated so the expected takes match the observed ones; a lineup takes a
//! question by the hazard sum of its players.
//!
//! The model page stops at the forecast: it does not say how Elo and volatility
//! move. The update here is read off the site's tournament pages, where every
//! player's Elo before and change are printed (see `tools/fetch/elo_pages.py`):
//! a player moves by the format weight times `ALPHA + (vol / VOL_SCALE)^2` times the
//! team's surplus in questions, plus a small inflation, and never further than
//! the uniform shift of the whole lineup that would explain the surplus alone,
//! nor more than `DELTA_MAX` either way;
//! volatility shrinks by the information the tournament carried about the
//! player, `w * h^2 / SIGMA_V^2` in precision, where `h` is the derivative of
//! the team's expected score with respect to the player's Elo.
use super::{blank_event, bare_player_event, Args, Changes, Fallible, PlayerRow, Release, Rival, Schedule};
use crate::config::Config;
use crate::data::{Arena, Format, Tournament, CORRECT, SKIP};
use crate::output::{TournamentMetrics, Writer};
use crate::team_event::TeamEvent;
use crate::util::{format_date, next_thursday, parse_date, spearman};
use rayon::prelude::*;
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
/// `rho(mean Elo) = clip(0.70 + 0.0002 (mean - 2000), 0.70, 0.90)`; 0.65 for async.
const RHO_BASE: f64 = 0.70;
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
const ALPHA: f64 = 1.08;
const VOL_SCALE: f64 = 95.0;
const INFLATION: f64 = 0.5;
const SIGMA_V: f64 = 4.5;
/// No player moves further than this in one tournament; the site's pages show
/// newcomers stopped at exactly -100.
const DELTA_MAX: f64 = 100.0;
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
    (1.0 / (1.0 + kn.slope_per * (r - SLOPE_FROM).max(0.0))).max(SLOPE_MIN)
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
    inflation: f64,
    sigma_v: f64,
    lambda: f64,
    w_in_person: f64,
    w_sync: f64,
    w_async: f64,
    /// Split the surplus by each player's share of the team's derivative, not equally.
    credit_grad: bool,
    /// Cap a step at the lineup's uniform shift.
    cap_shift: bool,
    slope_per: f64,
    rho_base: f64,
    rho_per: f64,
    /// A2's `player.mu_young` in Elo: a player whose first game is in a young
    /// field starts this much below `ELO_NEW`.
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
        Lineup { slope: elo.iter().map(|&r| slope(r, kn)).collect(), elo, rho, exp_hazard: kn.exp_hazard }
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
        let h = sm.powf(self.rho);
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
        let (h, outer) = (sm.powf(self.rho), sm.powf(self.rho - 1.0));
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

fn rate_team(l: &Lineup, mask: &[u8], qs: &[(f64, f64)], w: f64, vols: &[f64], kn: &Knobs) -> TeamFit {
    let n = l.elo.len();
    let mut expected = 0.0;
    let (mut e_live, mut s_live, mut h) = (0.0, 0.0, vec![0.0; n]);
    let mut g = vec![0.0; n];
    let mut played: Vec<(f64, f64)> = Vec::new();
    for (q, &(d, s)) in qs.iter().enumerate() {
        if !s.is_finite() {
            continue;
        }
        let y = mask.get(q).copied().filter(|&y| y != SKIP);
        let p = if live(d) && y.is_some() { l.grad(d, s, &mut g) } else { l.take(d, s, 0.0).p };
        expected += p;
        if let (true, Some(y)) = (live(d), y) {
            e_live += p;
            s_live += (y == CORRECT) as u8 as f64;
            h.iter_mut().zip(&g).for_each(|(a, b)| *a += b);
            played.push((d, s));
        }
    }
    if played.is_empty() {
        return TeamFit { expected, deltas: vec![0.0; n], info: vec![0.0; n] };
    }
    let r = s_live - e_live;
    let shift = uniform_shift(l, &played, s_live);
    let hsum: f64 = h.iter().sum();
    let deltas = vols.iter().zip(&h).map(|(&v, &hi)| {
        let share = if kn.credit_grad && hsum > 0.0 { n as f64 * hi / hsum } else { 1.0 };
        let raw = w * (kn.alpha + (v / kn.vol_scale).powi(2)) * r * share;
        let cap = if kn.cap_shift { (w * shift.abs()).min(DELTA_MAX) } else { DELTA_MAX };
        raw.clamp(-cap, cap) + w * kn.inflation
    }).collect();
    let info = h.iter().map(|&x| w * x * x / (kn.sigma_v * kn.sigma_v)).collect();
    TeamFit { expected, deltas, info }
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
            inflation: num("--inflation", INFLATION),
            sigma_v: num("--sigma-v", SIGMA_V),
            lambda: num("--lambda", LAMBDA),
            w_in_person: num("--w-in-person", W_IN_PERSON),
            w_sync: num("--w-sync", W_SYNC),
            w_async: num("--w-async", W_ASYNC),
            credit_grad: args.has("--credit-grad"),
            cap_shift: !args.has("--no-cap"),
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
        Ok(Elo { cfg, arena, m, kn, prev: Changes::new(np), to_day, from_day, start, traced: false })
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
        let young = t.young && self.kn.young != 0.0;
        let seeded = |m: &Model, p: usize| if young && m.games[p] == 0 { ELO_NEW - self.kn.young } else { m.elo[p] };
        if young && !fc {
            for &p in members.iter().flatten() {
                self.m.elo[p as usize] = seeded(&self.m, p as usize);
            }
        }
        let ls: Vec<Option<Lineup>> = members.iter().map(|ms| {
            (!ms.is_empty()).then(|| Lineup::new(ms.iter().map(|&p| seeded(&self.m, p as usize)).collect(), t.format, &self.kn))
        }).collect();
        let qs = fit_questions(t, &ls, self.kn.lambda);
        let fresh = t.format == Format::InPerson && self.kn.ip_new != 0.0;
        let read: Vec<Option<Lineup>> = if fresh {
            members.iter().map(|ms| {
                (!ms.is_empty()).then(|| {
                    let at = |&p: &u32| seeded(&self.m, p as usize) - self.kn.ip_new / (1.0 + self.m.games_ip[p as usize] as f64);
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
            Some(rate_team(l, &r.mask, &qs, wt, &vols, &self.kn))
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
