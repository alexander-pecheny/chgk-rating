//! The Kalman team engine of DESIGN 4.2, which S2 replaced. It still runs from
//! the same config under `[team] engine = "kalman"`.

use crate::config::Config;
use crate::data::{TeamResult, Tournament, CORRECT};
use crate::team_event::TeamEvent;
use crate::question_model::{realised_strength, Players, QParams};
use rayon::prelude::*;

const DAYS_PER_YEAR: f64 = 365.25;
/// The package size `beta` is quoted for.
const BASE_PACKAGE: usize = 36;
/// Floors that keep a ratio finite.
const SMALL: f64 = 1e-9;
const TINY: f64 = 1e-12;
/// At most this share of a team's evidence leaves in one tournament.
const MAX_LOST: f64 = 0.9;
/// A player's evidence for a team is forgotten below this.
const MIN_EVIDENCE: f64 = 0.01;

/// Roster estimate of a lineup and its variance; DESIGN 4.1.
/// `z` holds theta_i + aff_i,t and `var` the matching player variances.
pub fn roster_estimate(z: &[f64], var: &[f64], cfg: &Config) -> (f64, f64) {
    if z.is_empty() {
        return (f64::NEG_INFINITY, f64::INFINITY);
    }
    let mx = z.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let sum: f64 = z.iter().map(|&x| (x - mx).exp()).sum();
    let s_r = mx + sum.ln();
    let sm = cfg.team.sigma_model;
    let v = z
        .iter()
        .zip(var)
        .map(|(&x, &vi)| {
            let p = (x - mx).exp() / sum;
            p * p * vi
        })
        .sum::<f64>()
        + sm * sm;
    (s_r, v)
}

/// Team state of DESIGN 4.2. `mu_raw` and `core_raw` are on the pre-gauge
/// scale, so a release shift costs nothing.
#[derive(Debug, Clone)]
pub struct TeamState {
    pub mu_raw: f64,
    pub var: f64,
    pub core_raw: f64,
    /// Per-team volatility, learned from the surprise of each result.
    pub vol: f64,
    /// Per player: evidence, and consecutive rated tournaments of this team missed.
    pub ev: Vec<(u32, f64, u32)>,
    pub last_day: i32,
    /// Share of evidence lost at the previous tournament, for the gamma check.
    pub lost_prev: f64,
}

impl TeamState {
    pub fn evidence(&self) -> f64 {
        self.ev.iter().map(|(_, e, _)| *e).sum()
    }
    pub fn sigma_at(&self, day: i32, _cfg: &Config) -> f64 {
        let dt = ((day - self.last_day).max(0) as f64) / DAYS_PER_YEAR;
        (self.var + self.vol * self.vol * dt).sqrt()
    }
}

/// What the pairwise stage needs, read off the pre-update states.
struct Entry {
    ri: usize,
    idx: usize,
    x: f64,
    c: f64,
    delta: f64,
    mu_before: f64,
    forecast: f64,
    sr: Option<(f64, f64)>,
    place: f64,
    score: Option<f64>,
    var: f64,
    evidence: f64,
    gap_days: i32,
    var_diff: f64,
    lost_prev: f64,
    rated: bool,
    /// Realised strength of the result and its standardised residual; DESIGN 4.3 step 6.
    real: Option<f64>,
    z: Option<f64>,
}

pub struct Teams {
    pub state: Vec<Option<TeamState>>,
}

impl Teams {
    pub fn new(n: usize) -> Teams {
        Teams {
            state: (0..n).map(|_| None).collect(),
        }
    }

    pub fn listed(&self, day: i32, window: i32) -> impl Iterator<Item = (usize, &TeamState)> {
        self.state
            .iter()
            .enumerate()
            .filter_map(move |(i, s)| s.as_ref().map(|s| (i, s)))
            .filter(move |(_, s)| s.last_day >= day - window)
    }

    /// One tournament: DESIGN 4.3. Player states must be the pre-tournament ones.
    pub fn step(&mut self, t: &Tournament, players: &Players, params: &[QParams], cfg: &Config) -> Vec<TeamEvent> {
        let vbar = players.editor_mean(t);
        let es: Vec<Entry> = t
            .teams
            .iter()
            .enumerate()
            .filter_map(|(ri, r)| self.enter(ri, r, t, players, &vbar, params, cfg))
            .collect();
        let n = es.len();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| es[b].forecast.total_cmp(&es[a].forecast));
        let mut fplace = vec![0.0; n];
        for (k, &i) in order.iter().enumerate() {
            fplace[i] = (k + 1) as f64;
        }
        let tm = &cfg.team;
        let n_q = if t.n_q == 0 { BASE_PACKAGE } else { t.n_q };
        let beta_t = tm.beta * (BASE_PACKAGE as f64 / n_q as f64).sqrt();
        let scale = if n > 1 { tm.nu.min((n - 1) as f64) / (n - 1) as f64 } else { 0.0 };
        let si = pair_stats(&es, beta_t, scale, cfg);
        let first_place = es.iter().map(|e| e.place).fold(f64::INFINITY, f64::min);
        let step = Step { t, shift: players.shift_cum, beta_t, scale, first_place };
        let mut out = Vec::with_capacity(n);
        for (k, e) in es.iter().enumerate() {
            let (s_pair, i_pair, _) = si[k];
            let mut ev = event(e, si[k], fplace[k], t, cfg);
            if let Some(st) = self.state[e.idx].as_mut() {
                ev.mu_after = update(st, e, s_pair, i_pair, &step, cfg);
                ev.sigma_after = st.var.sqrt();
            }
            out.push(ev);
        }
        out
    }

    /// One result as it stands in the field, opening the team's state on its
    /// first rated result. `None` for a team with neither a lineup nor a
    /// state, which cannot be placed on the scale.
    #[allow(clippy::too_many_arguments)]
    fn enter(
        &mut self,
        ri: usize,
        r: &TeamResult,
        t: &Tournament,
        players: &Players,
        vbar: &[f64],
        params: &[QParams],
        cfg: &Config,
    ) -> Option<Entry> {
        let idx = r.team as usize;
        let sr = (!r.members.is_empty()).then(|| lineup_estimate(r, players, vbar, cfg));
        let (gap_days, var_diff) = match &mut self.state[idx] {
            Some(s) => age(s, t.day),
            None => {
                let (s_r, v) = sr?;
                // An unrated result never opens a state; it stands in the field
                // at its lineup.
                if !r.rated {
                    return Some(Entry::unrated(ri, idx, r, sr, s_r + cfg.team.roster_bias, v));
                }
                self.state[idx] = Some(TeamState::open(s_r, v, players.shift_cum, t.day, cfg));
                (-1, 0.0)
            }
        };
        let s = self.state[idx].as_ref()?;
        let mut e = Entry::of(s, r, sr, players.shift_cum, cfg);
        (e.ri, e.idx, e.gap_days, e.var_diff) = (ri, idx, gap_days, var_diff);
        e.real = realised(r, params, cfg);
        // Against the team's own strength `x = mu + delta`, not the blended forecast.
        let x = e.mu_before + e.delta;
        e.z = e.real.map(|real| (real - x) / (s.var + cfg.team.sigma_model * cfg.team.sigma_model).sqrt());
        Some(e)
    }
}

/// The roster estimate of a lineup and its variance, from the players' state.
fn lineup_estimate(r: &TeamResult, players: &Players, vbar: &[f64], cfg: &Config) -> (f64, f64) {
    let z: Vec<f64> = r.members.iter().map(|&i| players.theta(i as usize) + players.affinity(i as usize, vbar)).collect();
    let var: Vec<f64> = r.members.iter().map(|&i| players.state[i as usize].var).collect();
    roster_estimate(&z, &var, cfg)
}

/// The variance a team gains for the time since its last game, as `(days,
/// variance added)`.
fn age(s: &mut TeamState, day: i32) -> (i32, f64) {
    let gap_days = (day - s.last_day).max(0);
    let var_diff = s.vol * s.vol * gap_days as f64 / DAYS_PER_YEAR;
    s.var += var_diff;
    (gap_days, var_diff)
}

/// The realised strength of a result, from the questions it took.
fn realised(r: &TeamResult, params: &[QParams], cfg: &Config) -> Option<f64> {
    if r.mask.is_empty() {
        return None;
    }
    let hits = r.mask.iter().filter(|&&y| y == CORRECT).count() as f64;
    realised_strength(params, &r.mask, hits, cfg)
}

impl TeamState {
    /// A new team at its first lineup, as uncertain as a new team is.
    fn open(s_r: f64, v: f64, shift: f64, day: i32, cfg: &Config) -> TeamState {
        let tm = &cfg.team;
        TeamState {
            mu_raw: s_r + tm.roster_bias + shift,
            var: v + tm.sigma_new_team * tm.sigma_new_team,
            core_raw: s_r + shift,
            ev: Vec::new(),
            last_day: day,
            lost_prev: 0.0,
            vol: tm.tau_t,
        }
    }
}

impl Entry {
    /// A result of a team with a state: how much of the team's evidence its
    /// lineup carries, the lineup offset, and the forecast that blends the two.
    /// The caller fills in the position in the field and the realised strength.
    fn of(s: &TeamState, r: &TeamResult, sr: Option<(f64, f64)>, shift: f64, cfg: &Config) -> Entry {
        let tm = &cfg.team;
        let e_tot = s.evidence();
        let own: f64 = r.members.iter().filter_map(|i| s.ev.iter().find(|(j, _, _)| j == i)).map(|(_, e, _)| *e).sum();
        let c = if e_tot <= 0.0 { 1.0 } else { own / e_tot };
        let mu = s.mu_raw - shift;
        let delta = sr.map_or(0.0, |(v, _)| v - (s.core_raw - shift));
        let x = mu + delta;
        let forecast = match sr {
            Some((v, vv)) => {
                let v = v + tm.roster_bias;
                (x / s.var + v / vv) / (1.0 / s.var + 1.0 / vv)
            }
            None => x,
        };
        Entry {
            ri: 0,
            idx: 0,
            // An unrated lineup stands in the field at its forecast strength.
            x: if r.rated { x } else { forecast },
            c,
            delta,
            mu_before: mu,
            forecast,
            sr,
            place: r.position,
            score: r.score,
            var: s.var,
            evidence: e_tot,
            gap_days: 0,
            var_diff: 0.0,
            lost_prev: s.lost_prev,
            rated: r.rated,
            real: None,
            z: None,
        }
    }

    fn unrated(ri: usize, idx: usize, r: &TeamResult, sr: Option<(f64, f64)>, x: f64, var: f64) -> Entry {
        Entry {
            ri,
            idx,
            x,
            c: 0.0,
            delta: 0.0,
            mu_before: f64::NAN,
            forecast: x,
            sr,
            place: r.position,
            score: r.score,
            var,
            evidence: 0.0,
            gap_days: -1,
            var_diff: 0.0,
            lost_prev: 0.0,
            rated: false,
            real: None,
            z: None,
        }
    }
}

/// The row of one result, as it stood before the update.
fn event(e: &Entry, (s_pair, i_pair, expected_place): (f64, f64, f64), forecast_place: f64, t: &Tournament, cfg: &Config) -> TeamEvent {
    TeamEvent {
        d_place: f64::NAN,
        d_pull: f64::NAN,
        team: e.idx as u32,
        c: e.c,
        delta_l: e.delta,
        x: e.x,
        forecast: e.forecast,
        forecast_place,
        expected_place,
        actual_place: e.place,
        pred_place: t.teams[e.ri].pred_pos,
        s_r: e.sr.map_or(f64::NAN, |p| p.0 + cfg.team.roster_bias),
        sigma_before: e.var.sqrt(),
        sigma_after: e.var.sqrt(),
        evidence: e.evidence,
        gap_days: e.gap_days,
        var_diff: e.var_diff,
        lost_prev: e.lost_prev,
        rated: e.rated,
        z: e.z,
        s_pair,
        i_pair,
        mu_before: e.mu_before,
        mu_after: f64::NAN,
    }
}

/// What every update in one field shares.
struct Step<'a> {
    t: &'a Tournament,
    shift: f64,
    beta_t: f64,
    scale: f64,
    first_place: f64,
}

/// Each team's `(score, information, expected place)` over the field. A shared
/// place says nothing about which team is better and counts for neither.
fn pair_stats(es: &[Entry], beta_t: f64, scale: f64, cfg: &Config) -> Vec<(f64, f64, f64)> {
    let tm = &cfg.team;
    let one = |(ia, a): (usize, &Entry)| {
        let (mut s, mut i, mut beaten) = (0.0, 0.0, 0.0);
        for (ib, b) in es.iter().enumerate() {
            if ia == ib {
                continue;
            }
            let p = 1.0 / (1.0 + ((b.x - a.x) / beta_t).exp());
            beaten += 1.0 - p;
            let y = match a.place.total_cmp(&b.place) {
                std::cmp::Ordering::Less => 1.0,
                std::cmp::Ordering::Equal => continue,
                std::cmp::Ordering::Greater => 0.0,
            };
            let m = match (a.score, b.score) {
                (Some(u), Some(v)) => (1.0 + (u - v).abs() / tm.kappa_m).min(tm.m_max),
                _ => 1.0,
            };
            s += (y - p) * m;
            i += p * (1.0 - p) * m;
        }
        (s * scale, i * scale, 1.0 + beaten)
    };
    es.par_iter().enumerate().map(one).collect()
}

/// The Kalman update of one team, then its volatility, its players' evidence
/// and its core. Returns the rating after, on the published scale.
fn update(st: &mut TeamState, e: &Entry, s_pair: f64, i_pair: f64, step: &Step, cfg: &Config) -> f64 {
    let (tm, t) = (&cfg.team, step.t);
    let beta_t = step.beta_t;
    if !(e.rated && e.c > 0.0) {
        if e.rated {
            st.last_day = t.day;
        }
        return e.mu_before;
    }
    st.var /= tm.forget.powf(t.weight * e.c);
    if tm.surprise_z0 > 0.0 {
        let i_raw = i_pair / step.scale.max(TINY);
        let z_raw = (s_pair * beta_t / i_pair.max(SMALL)) / (st.var + beta_t * beta_t / i_raw.max(SMALL)).sqrt();
        let down = !tm.surprise_down_only || z_raw < 0.0;
        if down && z_raw.abs() > tm.surprise_z0 {
            st.var *= (z_raw / tm.surprise_z0).powi(2).min(tm.surprise_max);
        }
    }
    let obs_prec = t.weight * e.c * i_pair / (beta_t * beta_t);
    let marg_sigma = tm.margin_sigma.unwrap_or(tm.sigma_model);
    // The score says how strong the team was but not who it beat, so it
    // sharpens the posterior without moving it: the winner rule is exact.
    let margin_prec = match e.real {
        Some(_) => tm.margin_prec_scale * t.weight * e.c / (marg_sigma * marg_sigma),
        None => 0.0,
    };
    let prec = 1.0 / st.var + obs_prec + margin_prec;
    let mut d = t.weight * e.c * (s_pair / beta_t) / prec;
    if d < 0.0 {
        d *= tm.k_down;
    }
    if tm.margin_mean {
        if let Some(real) = e.real {
            let cal = (real - tm.margin_intercept) / tm.margin_slope;
            d += margin_prec * (cal - e.x) / prec;
            if e.place <= step.first_place {
                d = d.max(0.0);
            }
        }
    }
    st.mu_raw += d;
    st.var = (1.0 / prec).max(tm.sigma_floor * tm.sigma_floor);
    let z = (s_pair * beta_t / i_pair.max(SMALL)) / (e.var + 1.0 / obs_prec.max(TINY)).sqrt();
    let (lo, hi) = (tm.v_min.ln(), tm.v_max.ln());
    let lv = st.vol.ln();
    st.vol = (lv + tm.v_eta * (if z.abs() > 1.0 { hi } else { lo } - lv)).exp();
    let mu_after = st.mu_raw - step.shift;
    absorb_lineup(st, e, &t.teams[e.ri].members, step, cfg);
    st.last_day = t.day;
    mu_after
}

/// The evidence each player has given the team, its decay for players who
/// stay away, and the core strength the lineup offset is measured against.
fn absorb_lineup(st: &mut TeamState, e: &Entry, members: &[u32], step: &Step, cfg: &Config) {
    let (tm, weight) = (&cfg.team, step.t.weight);
    let e_tot = st.evidence();
    for &i in members {
        match st.ev.iter_mut().find(|(j, _, _)| *j == i) {
            Some(p) => {
                p.1 = (p.1 + weight).min(tm.e_cap);
                p.2 = 0;
            }
            None => st.ev.push((i, weight.min(tm.e_cap), 0)),
        }
    }
    // A rotating roster is not a departure: evidence decays only after a player
    // has missed `miss_streak` of the team's tournaments in a row.
    let mut lost = 0.0;
    for (j, x, miss) in st.ev.iter_mut() {
        if !members.contains(j) {
            *miss += 1;
            if *miss >= tm.miss_streak {
                lost += *x * (1.0 - tm.rho);
                *x *= tm.rho;
            }
        }
    }
    st.lost_prev = 0.0;
    if e_tot > 0.0 {
        let l = (lost / e_tot).clamp(0.0, MAX_LOST);
        st.var /= (1.0 - l).powf(tm.gamma);
        st.lost_prev = l;
    }
    // After total turnover a team knows no more about itself than a new team of
    // the same lineup, so that is the ceiling on its uncertainty.
    if let Some((_, v)) = e.sr {
        st.var = st.var.min(v + tm.sigma_new_team * tm.sigma_new_team);
    }
    st.ev.retain(|(_, x, _)| *x > MIN_EVIDENCE);
    if let Some((v, _)) = e.sr {
        let w_add = weight * members.len() as f64;
        st.core_raw = (st.core_raw * e_tot + (v + step.shift) * w_add) / (e_tot + w_add);
    }
}
