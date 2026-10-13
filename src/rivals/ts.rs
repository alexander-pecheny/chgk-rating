//! A TrueSkill-family rating on places alone: the Bayesian approximation of
//! Weng and Lin (JMLR 12, 2011), the method behind OpenSkill. Every player
//! carries a Gaussian `(mu, sigma^2)`; a lineup is the sum of its players; a
//! tournament is one ranking of the lineups that played it, read either as a
//! Plackett-Luce ranking (`--model pl`, the paper's Algorithm 4) or as every
//! pair of lineups under Bradley-Terry (`--model bt`, Algorithm 1), whose
//! steps shrink to `--opp` opponents' worth so a field of a thousand does not
//! count a thousand times. Questions are never read. Not a published system:
//! it stands for the family a rating built from places would come from.
use super::{bare_player_event, blank_event, Args, Changes, Fallible, PlayerRow, Release, Rival, Schedule};
use crate::config::Config;
use crate::data::{Arena, Format};
use crate::output::{TournamentMetrics, Writer};
use crate::team_event::TeamEvent;
use crate::util::{format_date, next_thursday, parse_date, spearman};
use rayon::prelude::*;

/// OpenSkill's defaults: `mu = 25`, `sigma = mu / 3`, `beta = sigma / 2`,
/// `tau = sigma / 100`, and the floor `kappa` on a variance's shrink.
const MU0: f64 = 25.0;
const SIGMA0: f64 = MU0 / 3.0;
const BETA: f64 = SIGMA0 / 2.0;
const TAU: f64 = SIGMA0 / 100.0;
const KAPPA: f64 = 1e-4;
const OPP: f64 = 10.0;
const W_ASYNC: f64 = 0.25;
const DAYS_PER_YEAR: f64 = 365.25;
const ACTIVE_DAYS: i32 = 365;
const CHECKPOINT_EVERY: i64 = 20;
const LOG_EVERY: usize = 2000;

#[derive(Clone, Copy, PartialEq)]
enum Model {
    PlackettLuce,
    BradleyTerry,
}

#[derive(Clone, Copy)]
struct Knobs {
    model: Model,
    mu0: f64,
    sigma0: f64,
    beta: f64,
    tau: f64,
    tau_year: f64,
    opp: f64,
    w_async: f64,
}

/// One lineup in a field: its place, mean and variance.
struct Side {
    place: f64,
    mu: f64,
    s2: f64,
}

/// Each lineup's `(Omega, Delta)` of Algorithm 4: Plackett-Luce over the
/// field's order, ties sharing a stage. Sorted by place, the sums over the
/// lineups placed at or below each stage are suffix sums, so the field costs
/// `n log n`.
fn plackett_luce(sides: &[Side], beta: f64) -> Vec<(f64, f64)> {
    let c = sides.iter().map(|s| s.s2 + beta * beta).sum::<f64>().sqrt();
    let mut o: Vec<usize> = (0..sides.len()).collect();
    o.sort_by(|&a, &b| sides[a].place.total_cmp(&sides[b].place));
    let top = sides.iter().map(|s| s.mu / c).fold(f64::MIN, f64::max);
    let e: Vec<f64> = sides.iter().map(|s| (s.mu / c - top).exp()).collect();
    // Stages: runs of equal place in `o`, with the suffix sum of e from each.
    let mut stages: Vec<(usize, usize, f64)> = Vec::new();
    let mut i = 0;
    while i < o.len() {
        let mut j = i;
        while j + 1 < o.len() && sides[o[j + 1]].place == sides[o[i]].place {
            j += 1;
        }
        stages.push((i, j + 1, 0.0));
        i = j + 1;
    }
    let mut tail = 0.0;
    for st in stages.iter_mut().rev() {
        tail += o[st.0..st.1].iter().map(|&k| e[k]).sum::<f64>();
        st.2 = tail;
    }
    let mut out = vec![(0.0, 0.0); sides.len()];
    let (mut inv, mut inv2) = (0.0, 0.0);
    for &(a, b, sum) in &stages {
        let ties = (b - a) as f64;
        inv += 1.0 / sum;
        inv2 += 1.0 / (sum * sum);
        for &k in &o[a..b] {
            let s = &sides[k];
            let own = 1.0 / ties;
            let omega = own - e[k] * inv;
            let delta = e[k] * inv - e[k] * e[k] * inv2;
            let gamma = s.s2.sqrt() / c;
            out[k] = (omega * s.s2 / c, delta * gamma * s.s2 / (c * c));
        }
    }
    out
}

/// Each lineup's `(Omega, Delta)` of Algorithm 1 over every opponent, scaled
/// to `opp` opponents' worth.
fn bradley_terry(sides: &[Side], beta: f64, opp: f64) -> Vec<(f64, f64)> {
    let scale = (opp / (sides.len() - 1).max(1) as f64).min(1.0);
    sides.par_iter().map(|a| {
        let (mut omega, mut delta) = (0.0, 0.0);
        for b in sides {
            if std::ptr::eq(a, b) {
                continue;
            }
            let c = (a.s2 + b.s2 + 2.0 * beta * beta).sqrt();
            let p = 1.0 / (1.0 + ((b.mu - a.mu) / c).exp());
            let s = if a.place < b.place { 1.0 } else if a.place > b.place { 0.0 } else { 0.5 };
            omega += a.s2 / c * (s - p);
            delta += a.s2.sqrt() / c * a.s2 / (c * c) * p * (1.0 - p);
        }
        (scale * omega, scale * delta)
    }).collect()
}

pub struct Ts {
    cfg: Config,
    arena: Arena,
    kn: Knobs,
    mu: Vec<f64>,
    s2: Vec<f64>,
    games: Vec<u32>,
    last: Vec<i32>,
    prev: Changes,
    from_day: i32,
    to_day: i32,
}

impl Ts {
    pub fn open(args: &Args) -> Fallible<Ts> {
        let mut cfg = Config::load(&args.or("--config", "config_elo.toml"))?;
        for (flag, slot) in [("--from", &mut cfg.data.from), ("--to", &mut cfg.data.to), ("--out", &mut cfg.data.out_path)] {
            if let Some(v) = args.get(flag) {
                *slot = v;
            }
        }
        let num = |flag: &str, default: f64| args.get(flag).and_then(|v| v.parse().ok()).unwrap_or(default);
        let model = match args.or("--model", "pl").as_str() {
            "pl" => Model::PlackettLuce,
            "bt" => Model::BradleyTerry,
            m => return Err(format!("unknown --model {m}").into()),
        };
        let kn = Knobs {
            model,
            mu0: num("--mu0", MU0),
            sigma0: num("--sigma0", SIGMA0),
            beta: num("--beta", BETA),
            tau: num("--tau", TAU),
            tau_year: num("--tau-year", 0.0),
            opp: num("--opp", OPP),
            w_async: num("--w-async", W_ASYNC),
        };
        let from_day = parse_date(&cfg.data.from).ok_or("bad --from")?;
        let to_day = parse_date(&cfg.data.to).ok_or("bad --to")?;
        let arena: Arena = crate::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
        let np = arena.n_players();
        Ok(Ts {
            mu: vec![kn.mu0; np],
            s2: vec![kn.sigma0 * kn.sigma0; np],
            games: vec![0; np],
            last: vec![i32::MIN; np],
            prev: Changes::new(np),
            cfg,
            arena,
            kn,
            from_day,
            to_day,
        })
    }
}

impl Rival for Ts {
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
        let kn = self.kn;
        let wt = if t.format == Format::Async { kn.w_async } else { 1.0 };
        let teams: Vec<(usize, Vec<usize>)> = t.teams.iter().enumerate().filter_map(|(k, r)| {
            let mut ms: Vec<usize> = r.members.iter().map(|&p| p as usize).collect();
            ms.sort_unstable();
            ms.dedup();
            (!ms.is_empty()).then_some((k, ms))
        }).collect();
        for (_, ms) in &teams {
            for &p in ms {
                let years = if self.last[p] == i32::MIN { 0.0 } else { (t.day - self.last[p]) as f64 / DAYS_PER_YEAR };
                self.s2[p] = (self.s2[p] + kn.tau * kn.tau + kn.tau_year * kn.tau_year * years).min(kn.sigma0 * kn.sigma0);
            }
        }
        let sides: Vec<Side> = teams.iter().map(|(k, ms)| Side {
            place: t.teams[*k].position,
            mu: ms.iter().map(|&p| self.mu[p]).sum(),
            s2: ms.iter().map(|&p| self.s2[p]).sum(),
        }).collect();
        let steps = match kn.model {
            Model::PlackettLuce => plackett_luce(&sides, kn.beta),
            Model::BradleyTerry => bradley_terry(&sides, kn.beta, kn.opp),
        };
        let mut evs = Vec::new();
        let mut pevs = Vec::new();
        for (((k, ms), side), &(omega, delta)) in teams.iter().zip(&sides).zip(&steps) {
            let r = &t.teams[*k];
            for &p in ms {
                let before = self.mu[p];
                let share = self.s2[p] / side.s2;
                self.mu[p] += wt * share * omega;
                self.s2[p] *= (1.0 - wt * share * delta).max(KAPPA);
                self.games[p] += 1;
                self.last[p] = t.day;
                if t.format == Format::InPerson {
                    pevs.push(bare_player_event(self.arena.player_ids[p], before, self.mu[p]));
                }
            }
            evs.push(TeamEvent {
                forecast: side.mu,
                x: side.mu,
                actual_place: r.position,
                pred_place: r.pred_pos,
                mu_before: side.mu,
                sigma_before: side.s2.sqrt(),
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
        if i.is_multiple_of(LOG_EVERY) {
            println!("{i:6}/{} {date}", self.arena.tournaments.len());
        }
        Ok(())
    }

    fn publish(&mut self, w: &Writer, r: &Release) -> Fallible<()> {
        let active: Vec<usize> = (0..self.arena.n_players()).filter(|&i| self.last[i] != i32::MIN && r.day - self.last[i] <= ACTIVE_DAYS).collect();
        w.release(r.id, &format_date(r.day), 0.0, 0.0, active.len())?;
        if !self.cfg.data.write_rows(r.id, r.last) {
            return Ok(());
        }
        let rows: Vec<PlayerRow> = active.iter().map(|&i| {
            let change = self.prev.step(i, self.mu[i]);
            (self.arena.player_ids[i], self.mu[i], self.s2[i].sqrt(), self.games[i], change)
        }).collect();
        w.player_ratings(r.id, &crate::output::published(&rows), |t| t)?;
        Ok(())
    }
}
