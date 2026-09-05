use crate::config::Config;
use crate::data::{Tournament, CORRECT};
use crate::question_model::{realised_strength, Players, QParams};
use rayon::prelude::*;

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
        let dt = ((day - self.last_day).max(0) as f64) / 365.25;
        (self.var + self.vol * self.vol * dt).sqrt()
    }
}

/// One row of `team_events`.
pub struct TeamEvent {
    pub team: u32,
    pub c: f64,
    pub delta_l: f64,
    pub x: f64,
    pub forecast: f64,
    pub forecast_place: f64,
    pub actual_place: f64,
    /// The old system's `predictedPosition`, carried through for DESIGN 7.2.
    pub pred_place: Option<f64>,
    pub s_r: f64,
    pub s_pair: f64,
    pub i_pair: f64,
    pub mu_before: f64,
    pub mu_after: f64,
    pub sigma_before: f64,
    pub sigma_after: f64,
    pub evidence: f64,
    pub gap_days: i32,
    pub var_diff: f64,
    pub lost_prev: f64,
    pub rated: bool,
    /// (realised strength - x) / sqrt(sigma_T^2 + sigma_model^2); should be unit variance.
    pub z: Option<f64>,
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
    pub fn step(
        &mut self,
        t: &Tournament,
        players: &Players,
        params: &[QParams],
        cfg: &Config,
    ) -> Vec<TeamEvent> {
        let shift = players.shift_cum;
        let vbar = players.editor_mean(t);
        let tm = &cfg.team;

        let mut es: Vec<Entry> = Vec::with_capacity(t.teams.len());
        let mut zs: Vec<Option<f64>> = Vec::with_capacity(t.teams.len());
        for (ri, r) in t.teams.iter().enumerate() {
            let idx = r.team as usize;
            let sr = (!r.members.is_empty()).then(|| {
                let z: Vec<f64> = r
                    .members
                    .iter()
                    .map(|&i| players.theta(i as usize) + players.affinity(i as usize, &vbar))
                    .collect();
                let var: Vec<f64> = r
                    .members
                    .iter()
                    .map(|&i| players.state[i as usize].var)
                    .collect();
                roster_estimate(&z, &var, cfg)
            });
            let mut gap_days = -1;
            let mut var_diff = 0.0;
            match &mut self.state[idx] {
                Some(s) => {
                    gap_days = (t.day - s.last_day).max(0);
                    var_diff = s.vol * s.vol * gap_days as f64 / 365.25;
                    s.var += var_diff;
                }
                None => {
                    // A team with neither a lineup nor a state cannot be placed on the
                    // scale; an unrated result never opens one.
                    let Some((s_r, v)) = sr else { continue };
                    if !r.rated {
                        es.push(Entry {
                            ri,
                            idx,
                            x: s_r + tm.roster_bias,
                            c: 0.0,
                            delta: 0.0,
                            mu_before: f64::NAN,
                            forecast: s_r + tm.roster_bias,
                            sr,
                            place: r.position,
                            score: r.score,
                            var: v,
                            evidence: 0.0,
                            gap_days: -1,
                            var_diff: 0.0,
                            lost_prev: 0.0,
                            rated: false,
                        });
                        zs.push(None);
                        continue;
                    }
                    self.state[idx] = Some(TeamState {
                        mu_raw: s_r + tm.roster_bias + shift,
                        var: v + tm.sigma_new_team * tm.sigma_new_team,
                        core_raw: s_r + shift,
                        ev: Vec::new(),
                        last_day: t.day,
                        lost_prev: 0.0,
                        vol: tm.tau_t,
                    });
                }
            }
            let s = self.state[idx].as_ref().unwrap();
            let e_tot = s.evidence();
            let c = if e_tot <= 0.0 {
                1.0
            } else {
                r.members
                    .iter()
                    .filter_map(|i| s.ev.iter().find(|(j, _, _)| j == i))
                    .map(|(_, e, _)| *e)
                    .sum::<f64>()
                    / e_tot
            };
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
            let z = (!r.mask.is_empty())
                .then(|| {
                    let hits = r.mask.iter().filter(|&&y| y == CORRECT).count() as f64;
                    realised_strength(params, &r.mask, hits, cfg)
                })
                .flatten()
                .map(|real| (real - x) / (s.var + tm.sigma_model * tm.sigma_model).sqrt());
            es.push(Entry {
                ri,
                idx,
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
                gap_days,
                var_diff,
                lost_prev: s.lost_prev,
                rated: r.rated,
            });
            zs.push(z);
        }

        let n = es.len();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| es[b].forecast.total_cmp(&es[a].forecast));
        let mut fplace = vec![0.0; n];
        for (k, &i) in order.iter().enumerate() {
            fplace[i] = (k + 1) as f64;
        }

        let n_q = if t.n_q == 0 { 36 } else { t.n_q };
        let beta_t = tm.beta * (36.0 / n_q as f64).sqrt();
        let scale = if n > 1 {
            tm.nu.min((n - 1) as f64) / (n - 1) as f64
        } else {
            0.0
        };
        let si: Vec<(f64, f64)> = es
            .par_iter()
            .enumerate()
            .map(|(ia, a)| {
                let (mut s, mut i) = (0.0, 0.0);
                for (ib, b) in es.iter().enumerate() {
                    if ia == ib {
                        continue;
                    }
                    // A shared place says nothing about which team is better.
                    let y = match a.place.total_cmp(&b.place) {
                        std::cmp::Ordering::Less => 1.0,
                        std::cmp::Ordering::Equal => continue,
                        std::cmp::Ordering::Greater => 0.0,
                    };
                    let p = 1.0 / (1.0 + ((b.x - a.x) / beta_t).exp());
                    let m = match (a.score, b.score) {
                        (Some(u), Some(v)) => (1.0 + (u - v).abs() / tm.kappa_m).min(tm.m_max),
                        _ => 1.0,
                    };
                    s += (y - p) * m;
                    i += p * (1.0 - p) * m;
                }
                (s * scale, i * scale)
            })
            .collect();

        let mut out = Vec::with_capacity(n);
        for (k, e) in es.iter().enumerate() {
            let (s_pair, i_pair) = si[k];
            let members = &t.teams[e.ri].members;
            let Some(st) = self.state[e.idx].as_mut() else {
                out.push(TeamEvent {
                    team: e.idx as u32,
                    c: 0.0,
                    delta_l: 0.0,
                    x: e.x,
                    forecast: e.forecast,
                    forecast_place: fplace[k],
                    actual_place: e.place,
                    pred_place: t.teams[e.ri].pred_pos,
                    s_r: e.sr.map_or(f64::NAN, |p| p.0 + tm.roster_bias),
                    sigma_before: e.var.sqrt(),
                    sigma_after: e.var.sqrt(),
                    evidence: 0.0,
                    gap_days: -1,
                    var_diff: 0.0,
                    lost_prev: 0.0,
                    rated: false,
                    z: None,
                    s_pair,
                    i_pair,
                    mu_before: f64::NAN,
                    mu_after: f64::NAN,
                });
                continue;
            };
            let mut mu_after = e.mu_before;
            if e.rated && e.c > 0.0 {
                let obs_prec = t.weight * e.c * i_pair / (beta_t * beta_t);
                // The score says how strong the team was but not who it beat, so it
                // sharpens the posterior without moving it: the winner rule is exact.
                let margin_prec = match zs[k] {
                    Some(_) => t.weight * e.c / (tm.sigma_model * tm.sigma_model),
                    None => 0.0,
                };
                let prec = 1.0 / st.var + obs_prec + margin_prec;
                st.mu_raw += t.weight * e.c * (s_pair / beta_t) / prec;
                st.var = 1.0 / prec;
                let z = (s_pair * beta_t / i_pair.max(1e-9))
                    / (e.var + 1.0 / obs_prec.max(1e-12)).sqrt();
                let (lo, hi) = (tm.v_min.ln(), tm.v_max.ln());
                let lv = st.vol.ln();
                st.vol = (lv + tm.v_eta * (if z.abs() > 1.0 { hi } else { lo } - lv)).exp();
                mu_after = st.mu_raw - shift;

                let e_tot = st.evidence();
                for &i in members {
                    match st.ev.iter_mut().find(|(j, _, _)| *j == i) {
                        Some(p) => {
                            p.1 = (p.1 + t.weight).min(tm.e_cap);
                            p.2 = 0;
                        }
                        None => st.ev.push((i, t.weight.min(tm.e_cap), 0)),
                    }
                }
                // A rotating roster is not a departure: evidence decays only after
                // a player has missed `miss_streak` of the team's tournaments in a row.
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
                    let l = (lost / e_tot).clamp(0.0, 0.9);
                    st.var /= (1.0 - l).powf(tm.gamma);
                    st.lost_prev = l;
                }
                // After total turnover a team knows no more about itself than a new
                // team of the same lineup, so that is the ceiling on its uncertainty.
                if let Some((_, v)) = e.sr {
                    st.var = st.var.min(v + tm.sigma_new_team * tm.sigma_new_team);
                }
                st.ev.retain(|(_, x, _)| *x > 0.01);
                if let Some((v, _)) = e.sr {
                    let w_add = t.weight * members.len() as f64;
                    st.core_raw = (st.core_raw * e_tot + (v + shift) * w_add) / (e_tot + w_add);
                }
            }
            if e.rated {
                st.last_day = t.day;
            }
            out.push(TeamEvent {
                team: e.idx as u32,
                c: e.c,
                delta_l: e.delta,
                x: e.x,
                forecast: e.forecast,
                forecast_place: fplace[k],
                actual_place: e.place,
                pred_place: t.teams[e.ri].pred_pos,
                s_r: e.sr.map_or(f64::NAN, |p| p.0 + tm.roster_bias),
                sigma_before: e.var.sqrt(),
                sigma_after: self.state[e.idx].as_ref().unwrap().var.sqrt(),
                evidence: e.evidence,
                gap_days: e.gap_days,
                var_diff: e.var_diff,
                lost_prev: e.lost_prev,
                rated: e.rated,
                z: zs[k],
                s_pair,
                i_pair,
                mu_before: e.mu_before,
                mu_after,
            });
        }
        out
    }
}
