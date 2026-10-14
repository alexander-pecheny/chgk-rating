//! chgk.quest, reimplemented from its published methodology page
//! (`data/external/chgk_quest/methodology.html`). Online noisy-OR SGD: one
//! gradient step per (team, question) observation, questions shuffled within a
//! tournament, tournaments in date order, a yearly gauge re-centering, and the
//! team "core strength" of the site's /teams page. Writes the DESIGN 6 schema.
use super::{blank_event, Args, Changes, Fallible, PlayerRow, Release, Rival, Schedule, TeamRow};
use crate::config::Config;
use crate::data::{Arena, Format, Tournament, CORRECT, SKIP};
use crate::gauge::Activity;
use crate::output::{TournamentMetrics, Writer};
use crate::team_event::TeamEvent;
use crate::util::{format_date, next_thursday, parse_date, spearman, Rng};
use std::collections::HashMap;
use std::time::Instant;

const ETA0: f64 = 0.15;
const ETA_DELTA: f64 = 0.001;
const ETA_TEAMMATE: f64 = 0.005;
const GAMES_OFFSET: f64 = 0.25;
const COLD_THETA: f64 = -1.5;
const REG_DELTA: f64 = 0.10;
const TOUR_LEN: usize = 12;
const SIZE_MAX: usize = 8;
const SIZE_ANCHOR: usize = 6;
const RECENTER_DAYS: i32 = 365;
const RECENTER_TARGET: f64 = -0.70;
const RECENTER_MIN_GAMES: u32 = 200;
const RECENTER_MIN_VETS: usize = 30;
const THETA_CLIP: f64 = 10.0;
const B_CLIP: f64 = 10.0;
const LA_CLIP: f64 = 3.0;
const Z_CLIP: f64 = 30.0;
const CORE_WINDOW: i32 = 365;
const CORE_TOP: usize = 6;
const CORE_MIN_GAMES: u32 = 2;
const CORE_MIN_SIZE: f64 = 3.0;
const CORE_MIN_TOURNAMENTS: u32 = 3;
/// A player's loyalty weight is full from this many games with the team.
const CORE_FULL_GAMES: f64 = 3.0;
const AGG_MIN_Q: usize = 100;
const AGG_MAX_RATE: f64 = 0.20;
/// Update weights of methodology 5.4: theta, b, log a, delta, per format.
const W_IN_PERSON: [f64; 4] = [1.0, 1.0, 1.0, 1.0];
const W_SYNC: [f64; 4] = [0.5, 0.5, 0.5, 1.0];
const W_ASYNC: [f64; 4] = [0.5, 0.15, 0.05, 0.5];
/// A question nobody played starts here.
const B_UNPLAYED: f64 = 1.15;
/// The take rate a question's start is fitted to is kept off 0 and 1.
const RATE_FLOOR: f64 = 0.02;
const RATE_CEIL: f64 = 0.98;
const INTENSITY_FLOOR: f64 = 1e-12;
const VAR_FLOOR: f64 = 1e-9;
const SHUFFLE_SEED: u64 = 0x0009_1657;
const CHECKPOINT_EVERY: i64 = 20;
const LOG_EVERY: usize = 1000;

fn weights(f: Format) -> [f64; 4] {
    match f {
        Format::InPerson => W_IN_PERSON,
        Format::Sync | Format::StrictSync => W_SYNC,
        Format::Async => W_ASYNC,
    }
}

/// Fisher-Yates with the tournament's own generator.
fn shuffle(v: &mut [usize], rng: &mut Rng) {
    for i in (1..v.len()).rev() {
        v.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
    }
}

/// The log-loss of one noisy-OR observation at total intensity `s`.
fn log_loss(y: u8, s: f64) -> f64 {
    if y == CORRECT {
        -(-(-s).exp_m1()).ln()
    } else {
        s
    }
}

#[derive(Clone)]
struct Model {
    theta: Vec<f64>,
    games: Vec<u32>,
    seen: Vec<bool>,
    last: Vec<i32>,
    fisher: Vec<f64>,
    d_size: [f64; SIZE_MAX + 1],
    d_pos: [f64; TOUR_LEN],
    shift_cum: f64,
}

impl Model {
    fn new(n: usize) -> Model {
        Model {
            theta: vec![COLD_THETA; n],
            games: vec![0; n],
            seen: vec![false; n],
            last: vec![i32::MIN / 2; n],
            fisher: vec![0.0; n],
            d_size: [0.0; SIZE_MAX + 1],
            d_pos: [0.0; TOUR_LEN],
            shift_cum: 0.0,
        }
    }

    fn eta(&self, i: usize) -> f64 {
        ETA0 / (GAMES_OFFSET + self.games[i] as f64).sqrt()
    }

    /// Methodology 7.2: pin the median of the active veterans to the target. A
    /// pure reparametrisation, so every fitted difficulty moves with it.
    fn recenter(&mut self, day: i32, qs: &mut [(i64, u32, f64, f64)]) -> Option<f64> {
        let mut vets: Vec<f64> = (0..self.theta.len())
            .filter(|&i| self.games[i] >= RECENTER_MIN_GAMES && day - self.last[i] <= RECENTER_DAYS)
            .map(|i| self.theta[i])
            .collect();
        if vets.len() < RECENTER_MIN_VETS {
            return None;
        }
        vets.sort_by(f64::total_cmp);
        let d = RECENTER_TARGET - vets[vets.len() / 2];
        self.theta.iter_mut().for_each(|t| *t += d);
        qs.iter_mut().for_each(|q| q.2 += q.3.exp() * d);
        self.shift_cum += d;
        Some(d)
    }

    /// The size and position offsets of question `q` for a lineup of `n`.
    fn delta(&self, n: usize, q: usize) -> f64 {
        self.d_size[n.min(SIZE_MAX)] + self.d_pos[q % TOUR_LEN]
    }

    /// Each player's intensity on a question into `lam`; returns their sum.
    fn intensities(&self, ls: &[u32], a: f64, b: f64, delta: f64, lam: &mut Vec<f64>) -> f64 {
        lam.clear();
        let mut s = 0.0;
        for &i in ls {
            let l = (a * self.theta[i as usize] - b - delta).clamp(-Z_CLIP, Z_CLIP).exp();
            lam.push(l);
            s += l;
        }
        s
    }

    /// The players' gradient step; returns sum of intensity times theta before it.
    fn step_players(&mut self, ls: &[u32], lam: &[f64], a: f64, (w, dl): (f64, f64), info: f64) -> f64 {
        let mut dot = 0.0;
        for (&i, &l) in ls.iter().zip(lam) {
            let i = i as usize;
            dot += l * self.theta[i];
            self.fisher[i] += info * (a * l) * (a * l);
            self.theta[i] = (self.theta[i] + self.eta(i) * w * dl * a * l).clamp(-THETA_CLIP, THETA_CLIP);
        }
        dot
    }

    /// The offsets' step, shrunk towards zero; the anchors stay at zero.
    fn step_offsets(&mut self, sz: usize, q: usize, step: f64) {
        let keep = 1.0 - ETA_DELTA * REG_DELTA;
        if sz != SIZE_ANCHOR {
            self.d_size[sz] = (self.d_size[sz] - step) * keep;
        }
        if q % TOUR_LEN != 0 {
            let j = q % TOUR_LEN;
            self.d_pos[j] = (self.d_pos[j] - step) * keep;
        }
    }

    /// Lineups pulled a little towards their own mean.
    fn shrink_teammates(&mut self, live: &[Vec<u32>]) {
        for ls in live.iter().filter(|ls| ls.len() >= 2) {
            let mean = ls.iter().map(|&i| self.theta[i as usize]).sum::<f64>() / ls.len() as f64;
            for &i in ls {
                self.theta[i as usize] -= ETA_TEAMMATE * (self.theta[i as usize] - mean);
            }
        }
    }

    /// A player first seen starts cold.
    fn lineup(&mut self, members: &[u32]) -> Vec<u32> {
        let mut v = members.to_vec();
        v.sort_unstable();
        v.dedup();
        for &i in &v {
            if !self.seen[i as usize] {
                self.seen[i as usize] = true;
                self.theta[i as usize] = COLD_THETA;
            }
        }
        v
    }
}

/// What one tournament leaves behind: its fitted questions, the expected score
/// of each lineup from the strengths it went in with, and the two log-losses.
struct Fit {
    params: Vec<(f64, f64)>,
    forecast: Vec<f64>,
    var: Vec<f64>,
    roster: Vec<f64>,
    online: (f64, u64),
    insample: (f64, u64),
}

fn seasonal_aggregate(t: &Tournament) -> bool {
    if !matches!(t.format, Format::InPerson | Format::Async) || t.n_q < AGG_MIN_Q {
        return false;
    }
    let (mut sum, mut n) = (0.0, 0.0);
    for s in t.teams.iter().filter_map(|r| r.score) {
        sum += s;
        n += 1.0;
    }
    n > 0.0 && sum / n / (t.n_q as f64) < AGG_MAX_RATE
}

/// Noisy-OR- and theta-aware start for every question: the difficulty at which
/// the lineups that played it would have matched its observed take rate.
fn init_questions(t: &Tournament, before: &[Vec<f64>]) -> Vec<(f64, f64)> {
    let mut acc = vec![(0.0f64, 0.0f64, 0.0f64); t.n_q];
    for (k, th) in before.iter().enumerate().filter(|(_, th)| !th.is_empty()) {
        let intensity = th.iter().map(|&x| x.clamp(-Z_CLIP, Z_CLIP).exp()).sum::<f64>().max(INTENSITY_FLOOR).ln();
        for (q, a) in acc.iter_mut().enumerate() {
            if let Some(&y) = t.teams[k].mask.get(q).filter(|&&y| y != SKIP) {
                a.0 += intensity;
                a.1 += 1.0;
                a.2 += (y == CORRECT) as u8 as f64;
            }
        }
    }
    acc.iter().map(|&(sum, n, hits)| start_of(sum, n, hits)).collect()
}

fn start_of(sum: f64, n: f64, hits: f64) -> (f64, f64) {
    if n == 0.0 {
        return (B_UNPLAYED, 0.0);
    }
    let p = (hits / n).clamp(RATE_FLOOR, RATE_CEIL);
    ((sum / n - (-(1.0 - p).ln()).ln()).clamp(-B_CLIP, B_CLIP), 0.0)
}

/// One tournament's online pass over shuffled questions and lineups.
struct Pass<'a> {
    t: &'a Tournament,
    live: &'a [Vec<u32>],
    w: [f64; 4],
    params: Vec<(f64, f64)>,
    online: (f64, u64),
    lam: Vec<f64>,
}

impl Pass<'_> {
    fn run(&mut self, m: &mut Model) {
        let mut order: Vec<usize> = (0..self.t.n_q).collect();
        let mut rng = Rng::new(SHUFFLE_SEED ^ self.t.id as u64);
        shuffle(&mut order, &mut rng);
        let mut tord: Vec<usize> = (0..self.live.len()).collect();
        for &q in &order {
            shuffle(&mut tord, &mut rng);
            for &k in &tord {
                match self.t.teams[k].mask.get(q) {
                    Some(&y) if y != SKIP && !self.live[k].is_empty() => self.observe(m, k, q, y),
                    _ => {}
                }
            }
        }
    }

    /// One gradient step on one (lineup, question) observation.
    fn observe(&mut self, m: &mut Model, k: usize, q: usize, y: u8) {
        let ls = &self.live[k];
        let (b, la) = self.params[q];
        let a = la.exp();
        let s = m.intensities(ls, a, b, m.delta(ls.len(), q), &mut self.lam);
        self.online.0 += log_loss(y, s);
        self.online.1 += 1;
        let dl = if y == CORRECT { 1.0 / s.exp_m1() } else { -1.0 };
        let dot = m.step_players(ls, &self.lam, a, (self.w[0], dl), 1.0 / s.exp_m1());
        self.params[q].0 = (b - ETA0 * self.w[1] * dl * s).clamp(-B_CLIP, B_CLIP);
        self.params[q].1 = (la + ETA0 * self.w[2] * dl * a * dot).clamp(-LA_CLIP, LA_CLIP);
        m.step_offsets(ls.len().min(SIZE_MAX), q, ETA_DELTA * self.w[3] * dl * s);
    }
}

impl Fit {
    fn new(params: Vec<(f64, f64)>, before: &[Vec<f64>], online: (f64, u64)) -> Fit {
        let roster = (before.iter())
            .map(|th| match th.iter().map(|&x| x.exp()).sum::<f64>() {
                s if s > 0.0 => s.ln(),
                _ => f64::NAN,
            })
            .collect();
        let n = before.len();
        Fit { params, forecast: vec![0.0; n], var: vec![0.0; n], roster, online, insample: (0.0, 0) }
    }

    /// The forecast from the strengths each lineup came with, and the in-sample
    /// loss from the strengths it leaves with.
    fn score(&mut self, t: &Tournament, m: &Model, live: &[Vec<u32>], before: &[Vec<f64>]) {
        for (k, ls) in live.iter().enumerate().filter(|(_, ls)| !ls.is_empty()) {
            for q in 0..t.n_q {
                let (b, la) = self.params[q];
                let (a, delta) = (la.exp(), m.delta(ls.len(), q));
                let inten = |th: f64| (a * th - b - delta).clamp(-Z_CLIP, Z_CLIP).exp();
                let p = -(-before[k].iter().map(|&x| inten(x)).sum::<f64>()).exp_m1();
                self.forecast[k] += p;
                self.var[k] += p * (1.0 - p);
                if let Some(&y) = t.teams[k].mask.get(q).filter(|&&y| y != SKIP) {
                    self.insample.0 += log_loss(y, ls.iter().map(|&i| inten(m.theta[i as usize])).sum());
                    self.insample.1 += 1;
                }
            }
        }
    }
}

fn run_tournament(t: &Tournament, m: &mut Model, live: &[Vec<u32>]) -> Fit {
    let before: Vec<Vec<f64>> = live.iter().map(|ls| ls.iter().map(|&i| m.theta[i as usize]).collect()).collect();
    let params = init_questions(t, &before);
    let mut pass = Pass { t, live, w: weights(t.format), params, online: (0.0, 0), lam: Vec::with_capacity(SIZE_MAX) };
    pass.run(m);
    m.shrink_teammates(live);
    let mut fit = Fit::new(pass.params, &before, pass.online);
    fit.score(t, m, live, &before);
    for &i in live.iter().flatten() {
        m.games[i as usize] += 1;
        m.last[i as usize] = t.day;
    }
    fit
}

/// The rolling 365-day book behind the site's team table: per player and per
/// (team, player) game counts within one universe of formats.
#[derive(Clone)]
struct Universe {
    per_team: Vec<HashMap<u32, u32>>,
    per_player: Vec<u32>,
    tournaments: Vec<u32>,
}

/// A count moved by one, never below zero.
fn bump(c: &mut u32, sign: i32) {
    *c = (*c as i32 + sign).max(0) as u32;
}

impl Universe {
    fn new(n_teams: usize, n_players: usize) -> Universe {
        Universe { per_team: vec![HashMap::new(); n_teams], per_player: vec![0; n_players], tournaments: vec![0; n_teams] }
    }

    fn add(&mut self, team: u32, ls: &[u32], sign: i32) {
        let t = team as usize;
        bump(&mut self.tournaments[t], sign);
        for &i in ls {
            let e = self.per_team[t].entry(i).or_insert(0);
            bump(e, sign);
            if *e == 0 {
                self.per_team[t].remove(&i);
            }
            bump(&mut self.per_player[i as usize], sign);
        }
    }

    /// Weighted mean theta over the six most loyal players, and the weight sum.
    fn core(&self, team: u32, theta: &[f64]) -> Option<(f64, f64)> {
        let mut v: Vec<(u32, u32)> =
            self.per_team[team as usize].iter().filter(|(_, &n)| n >= CORE_MIN_GAMES).map(|(&i, &n)| (i, n)).collect();
        v.sort_unstable_by_key(|&(i, n)| (std::cmp::Reverse(n), i));
        let (mut num, mut den) = (0.0, 0.0);
        for &(i, n) in v.iter().take(CORE_TOP) {
            let w = (n as f64 / self.per_player[i as usize].max(1) as f64) * (n as f64 / CORE_FULL_GAMES).min(1.0);
            num += w * theta[i as usize];
            den += w;
        }
        (den > 0.0).then_some((num / den, den))
    }
}

/// The two universes of the team table, all formats and over-the-shoulder only,
/// and the log that expires them.
struct Core {
    all: Universe,
    os: Universe,
    log: Vec<(i32, u32, Vec<u32>, bool)>,
    cursor: usize,
}

impl Core {
    fn new(n_teams: usize, n_players: usize) -> Core {
        let u = Universe::new(n_teams, n_players);
        Core { all: u.clone(), os: u, log: Vec::new(), cursor: 0 }
    }

    fn push(&mut self, day: i32, team: u32, ls: Vec<u32>, os: bool) {
        self.all.add(team, &ls, 1);
        if os {
            self.os.add(team, &ls, 1);
        }
        self.log.push((day, team, ls, os));
    }

    fn expire_before(&mut self, day: i32) {
        while self.cursor < self.log.len() && self.log[self.cursor].0 < day {
            let e = &mut self.log[self.cursor];
            let (team, ls, os) = (e.1, std::mem::take(&mut e.2), e.3);
            self.all.add(team, &ls, -1);
            if os {
                self.os.add(team, &ls, -1);
            }
            self.cursor += 1;
        }
    }

    /// A team's core strength, from its over-the-shoulder games when it has any.
    fn strength(&self, ti: usize, theta: &[f64]) -> Option<f64> {
        let u = if self.os.tournaments[ti] > 0 { &self.os } else { &self.all };
        let (th, size) = u.core(ti as u32, theta)?;
        (size >= CORE_MIN_SIZE).then_some(th)
    }
}

fn event(t: &Tournament, fit: &Fit, k: usize, place: f64) -> TeamEvent {
    let sd = fit.var[k].max(VAR_FLOOR).sqrt();
    let r = &t.teams[k];
    TeamEvent {
        x: fit.roster[k],
        forecast: fit.forecast[k],
        forecast_place: place,
        actual_place: r.position,
        pred_place: r.pred_pos,
        s_r: fit.roster[k],
        mu_before: fit.roster[k],
        mu_after: fit.roster[k],
        sigma_before: sd,
        sigma_after: sd,
        rated: r.rated,
        z: r.score.map(|s| (s - fit.forecast[k]) / sd),
        ..blank_event(r.team)
    }
}

fn write_events(w: &Writer, t: &Tournament, arena: &Arena, fit: &Fit) -> rusqlite::Result<()> {
    let mut order: Vec<usize> = (0..t.teams.len()).collect();
    order.sort_by(|&a, &b| fit.forecast[b].total_cmp(&fit.forecast[a]));
    let mut place = vec![0.0; t.teams.len()];
    for (rank, &k) in order.iter().enumerate() {
        place[k] = rank as f64 + 1.0;
    }
    let evs: Vec<TeamEvent> = (0..t.teams.len()).map(|k| event(t, fit, k, place[k])).collect();
    w.team_events(t.id, &format_date(t.day), &arena.team_ids, &evs)?;
    write_metrics(w, t, fit, &place, &evs)
}

fn write_metrics(w: &Writer, t: &Tournament, fit: &Fit, place: &[f64], evs: &[TeamEvent]) -> rusqlite::Result<()> {
    let actual: Vec<f64> = evs.iter().map(|e| e.actual_place).collect();
    let (po, pa): (Vec<f64>, Vec<f64>) = evs.iter().filter_map(|e| e.pred_place.map(|p| (p, e.actual_place))).unzip();
    let mean = |l: (f64, u64)| (l.1 > 0).then(|| l.0 / l.1 as f64);
    let date = format_date(t.day);
    let field = TournamentMetrics::field(t.id, &date, t.format.name(), weights(t.format)[0], t.teams.len(), t.n_q);
    let (insample, crossfit) = (mean(fit.insample), mean(fit.online));
    let (sp_forecast, sp_old) = (spearman(place, &actual), spearman(&po, &pa));
    w.tournament_metrics(&TournamentMetrics { n_obs: fit.online.1, insample, crossfit, sp_forecast, sp_old, ..field })
}

pub struct Quest {
    cfg: Config,
    arena: Arena,
    aggregate: Vec<bool>,
    m: Model,
    act: Activity,
    core: Core,
    qs: Vec<(i64, u32, f64, f64)>,
    prev_p: Changes,
    prev_t: Changes,
    from_day: i32,
    to_day: i32,
    next_recenter: i32,
    shift: f64,
    tot_on: (f64, u64),
    tot_in: (f64, u64),
    start: Instant,
}

impl Quest {
    pub fn open(args: &Args) -> Fallible<Quest> {
        let mut cfg = Config::load(&args.or("--config", "config_quest.toml"))?;
        let d = &mut cfg.data;
        for (flag, slot) in [("--from", &mut d.from), ("--to", &mut d.to), ("--out", &mut d.out_path), ("--forecast-from", &mut d.forecast_from)] {
            if let Some(v) = args.get(flag) {
                *slot = v;
            }
        }
        let start = Instant::now();
        let from_day = parse_date(&cfg.data.from).ok_or("bad --from")?;
        let to_day = parse_date(&cfg.data.to).ok_or("bad --to")?;
        let arena: Arena = crate::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
        let (np, nt) = (arena.n_players(), arena.n_teams());
        println!("loaded {} tournaments, {np} players, {nt} teams in {:.1}s", arena.tournaments.len(), start.elapsed().as_secs_f64());
        Ok(Quest {
            aggregate: arena.tournaments.iter().map(seasonal_aggregate).collect(),
            m: Model::new(np),
            act: Activity::new(np),
            core: Core::new(nt, np),
            qs: Vec::new(),
            prev_p: Changes::new(np),
            prev_t: Changes::new(nt),
            next_recenter: i32::MIN,
            shift: 0.0,
            tot_on: (0.0, 0),
            tot_in: (0.0, 0),
            cfg, arena, from_day, to_day, start,
        })
    }

    /// The first recentering comes a year after the first tournament, then a year
    /// after each one that found enough veterans.
    fn maybe_recenter(&mut self, day: i32) {
        if self.next_recenter == i32::MIN {
            self.next_recenter = day + RECENTER_DAYS;
        } else if day >= self.next_recenter {
            if let Some(d) = self.m.recenter(day, &mut self.qs) {
                self.shift += d;
                self.next_recenter = day + RECENTER_DAYS;
                println!("recenter {} delta {d:+.4}", format_date(day));
            }
        }
    }

    /// The tournament's lineups enter the activity window and the team book.
    fn record(&mut self, i: usize, live: Vec<Vec<u32>>) {
        let t = &self.arena.tournaments[i];
        let os = t.format != Format::Async;
        for (k, ls) in live.into_iter().enumerate().filter(|(_, ls)| !ls.is_empty()) {
            for &p in &ls {
                self.act.push(t.day, p, 1.0);
            }
            self.core.push(t.day, t.teams[k].team, ls, os);
        }
    }

    fn player_rows(&mut self, active: &[usize]) -> Vec<PlayerRow> {
        let (m, act) = (&self.m, &self.act);
        let row = |i: usize, change: f64| {
            (self.arena.player_ids[i], m.theta[i], 1.0 / m.fisher[i].max(VAR_FLOOR).sqrt(), act.count[i], change)
        };
        active.iter().map(|&i| row(i, self.prev_p.step(i, m.theta[i]))).collect()
    }

    fn team_rows(&mut self) -> Vec<TeamRow> {
        let mut rows = Vec::new();
        for ti in (0..self.arena.n_teams()).filter(|&ti| self.core.all.tournaments[ti] >= CORE_MIN_TOURNAMENTS) {
            if let Some(th) = self.core.strength(ti, &self.m.theta) {
                let change = self.prev_t.step(ti, th);
                rows.push((self.arena.team_ids[ti], th, 0.0, th, self.core.all.tournaments[ti] as f64, change));
            }
        }
        rows
    }
}

impl Rival for Quest {
    fn schedule(&self) -> Schedule {
        Schedule { first_day: next_thursday(self.from_day), last_day: Some(self.to_day), checkpoint_every: CHECKPOINT_EVERY }
    }

    fn out_path(&self) -> &str {
        &self.cfg.data.out_path
    }

    fn n_tournaments(&self) -> usize {
        self.arena.tournaments.len()
    }

    /// A seasonal aggregate is skipped without moving the calendar.
    fn after(&self, i: usize, day: i32) -> bool {
        !self.aggregate[i] && self.arena.tournaments[i].day >= day
    }

    fn rate(&mut self, w: &Writer, i: usize, _next: &Release) -> Fallible<()> {
        if self.aggregate[i] {
            return Ok(());
        }
        if self.arena.tournaments[i].forecast_only {
            // A copy of the model plays it, so nothing it learns stays.
            let (t, mut m) = (&self.arena.tournaments[i], self.m.clone());
            let live: Vec<Vec<u32>> = t.teams.iter().map(|r| m.lineup(&r.members)).collect();
            return Ok(write_events(w, t, &self.arena, &run_tournament(t, &mut m, &live))?);
        }
        self.maybe_recenter(self.arena.tournaments[i].day);
        let t = &self.arena.tournaments[i];
        let live: Vec<Vec<u32>> = t.teams.iter().map(|r| self.m.lineup(&r.members)).collect();
        let fit = run_tournament(t, &mut self.m, &live);
        (self.tot_on.0, self.tot_on.1) = (self.tot_on.0 + fit.online.0, self.tot_on.1 + fit.online.1);
        (self.tot_in.0, self.tot_in.1) = (self.tot_in.0 + fit.insample.0, self.tot_in.1 + fit.insample.1);
        self.qs.extend(fit.params.iter().enumerate().map(|(q, &(b, la))| (t.id, q as u32, b, la)));
        write_events(w, t, &self.arena, &fit)?;
        if i % LOG_EVERY == 0 {
            let n = self.arena.tournaments.len();
            println!("{i:6}/{n} {} {:.0}s", format_date(t.day), self.start.elapsed().as_secs_f64());
        }
        self.record(i, live);
        Ok(())
    }

    fn publish(&mut self, w: &Writer, r: &Release) -> Fallible<()> {
        self.act.expire_before(r.day - self.cfg.gauge.window_days);
        self.core.expire_before(r.day - CORE_WINDOW);
        let active: Vec<usize> = (0..self.arena.n_players()).filter(|&i| self.act.count[i] > 0).collect();
        w.release(r.id, &format_date(r.day), self.shift, self.m.shift_cum, active.len())?;
        self.shift = 0.0;
        if !self.cfg.data.write_rows(r.id, r.last) {
            return Ok(());
        }
        let rows = self.player_rows(&active);
        w.player_ratings(r.id, &crate::output::published(&rows), |t| t)?;
        let trows = self.team_rows();
        w.team_ratings(r.id, &trows, |mu| mu)?;
        Ok(())
    }

    fn finish(&mut self, w: &Writer, releases: i64) -> Fallible<()> {
        let mut st = w.conn.prepare("INSERT INTO question_params VALUES (?1,?2,?3,?4,1.0)")?;
        for &(tid, q, b, la) in &self.qs {
            st.execute(rusqlite::params![tid, q, b, la.exp()])?;
        }
        let skipped = self.aggregate.iter().filter(|&&a| a).count();
        println!("{skipped} seasonal aggregates skipped, {} questions", self.qs.len());
        let (on, ins) = (self.tot_on, self.tot_in);
        println!(
            "log-loss over {} observations: online {:.4}, in-sample {:.4}",
            on.1, on.0 / on.1.max(1) as f64, ins.0 / ins.1.max(1) as f64
        );
        println!(
            "{releases} releases, gauge shift total {:+.4}, done in {:.1}s",
            self.m.shift_cum, self.start.elapsed().as_secs_f64()
        );
        Ok(())
    }
}
