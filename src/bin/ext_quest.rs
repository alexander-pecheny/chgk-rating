//! chgk.quest, reimplemented from its published methodology page
//! (`data/external/chgk_quest/methodology.html`). Online noisy-OR SGD: one
//! gradient step per (team, question) observation, questions shuffled within a
//! tournament, tournaments in date order, a yearly gauge re-centering, and the
//! team "core strength" of the site's /teams page. Writes the DESIGN 6 schema.
use chgk_rating::config::Config;
use chgk_rating::data::{Arena, Format, Tournament, CORRECT, SKIP};
use chgk_rating::gauge::Activity;
use chgk_rating::output::Writer;
use chgk_rating::team_event::TeamEvent;
use chgk_rating::output::TournamentMetrics;
use chgk_rating::util::{format_date, next_thursday, parse_date, spearman, Rng};
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
const AGG_MIN_Q: usize = 100;
const AGG_MAX_RATE: f64 = 0.20;

/// Update weights of methodology 5.4: theta, b, log a, delta, per format.
fn weights(f: Format) -> [f64; 4] {
    match f {
        Format::InPerson => [1.0, 1.0, 1.0, 1.0],
        Format::Sync | Format::StrictSync => [0.5, 0.5, 0.5, 1.0],
        Format::Async => [0.5, 0.15, 0.05, 0.5],
    }
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

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
        for t in self.theta.iter_mut() {
            *t += d;
        }
        for q in qs.iter_mut() {
            q.2 += q.3.exp() * d;
        }
        self.shift_cum += d;
        Some(d)
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
    for r in &t.teams {
        if let Some(s) = r.score {
            sum += s;
            n += 1.0;
        }
    }
    n > 0.0 && sum / n / (t.n_q as f64) < AGG_MAX_RATE
}

/// Noisy-OR- and theta-aware start for every question: the difficulty at which
/// the lineups that played it would have matched its observed take rate.
fn init_questions(t: &Tournament, before: &[Vec<f64>]) -> Vec<(f64, f64)> {
    let mut acc = vec![(0.0f64, 0.0f64, 0.0f64); t.n_q];
    for (k, th) in before.iter().enumerate() {
        if th.is_empty() {
            continue;
        }
        let intensity = th
            .iter()
            .map(|&x| x.clamp(-Z_CLIP, Z_CLIP).exp())
            .sum::<f64>()
            .max(1e-12)
            .ln();
        for (q, a) in acc.iter_mut().enumerate() {
            if let Some(&y) = t.teams[k].mask.get(q) {
                if y != SKIP {
                    a.0 += intensity;
                    a.1 += 1.0;
                    a.2 += (y == CORRECT) as u8 as f64;
                }
            }
        }
    }
    acc.iter()
        .map(|&(sum, n, hits)| {
            if n == 0.0 {
                return (1.15, 0.0);
            }
            let p = (hits / n).clamp(0.02, 0.98);
            ((sum / n - (-(1.0 - p).ln()).ln()).clamp(-B_CLIP, B_CLIP), 0.0)
        })
        .collect()
}

fn run_tournament(t: &Tournament, m: &mut Model, live: &[Vec<u32>]) -> Fit {
    let w = weights(t.format);
    let before: Vec<Vec<f64>> = live
        .iter()
        .map(|ls| ls.iter().map(|&i| m.theta[i as usize]).collect())
        .collect();
    let mut params = init_questions(t, &before);
    let mut order: Vec<usize> = (0..t.n_q).collect();
    let mut rng = Rng::new(0x0009_1657 ^ t.id as u64);
    for i in (1..order.len()).rev() {
        order.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
    }

    let mut tord: Vec<usize> = (0..live.len()).collect();
    let mut online = (0.0, 0u64);
    let mut lam: Vec<f64> = Vec::with_capacity(8);
    for &q in &order {
        for i in (1..tord.len()).rev() {
            tord.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
        }
        for &k in &tord {
            let ls = &live[k];
            let y = match t.teams[k].mask.get(q) {
                Some(&v) if v != SKIP && !ls.is_empty() => v,
                _ => continue,
            };
            let (b, la) = params[q];
            let a = la.exp();
            let sz = ls.len().min(SIZE_MAX);
            let delta = m.d_size[sz] + m.d_pos[q % TOUR_LEN];
            lam.clear();
            let mut s = 0.0;
            for &i in ls {
                let l = (a * m.theta[i as usize] - b - delta)
                    .clamp(-Z_CLIP, Z_CLIP)
                    .exp();
                lam.push(l);
                s += l;
            }
            let dl = if y == CORRECT {
                online.0 -= (-(-s).exp_m1()).ln();
                1.0 / s.exp_m1()
            } else {
                online.0 += s;
                -1.0
            };
            online.1 += 1;
            let info = 1.0 / s.exp_m1();
            let mut dot = 0.0;
            for (&i, &l) in ls.iter().zip(&lam) {
                let i = i as usize;
                dot += l * m.theta[i];
                m.fisher[i] += info * (a * l) * (a * l);
                m.theta[i] =
                    (m.theta[i] + m.eta(i) * w[0] * dl * a * l).clamp(-THETA_CLIP, THETA_CLIP);
            }
            params[q].0 = (b - ETA0 * w[1] * dl * s).clamp(-B_CLIP, B_CLIP);
            params[q].1 = (la + ETA0 * w[2] * dl * a * dot).clamp(-LA_CLIP, LA_CLIP);
            let keep = 1.0 - ETA_DELTA * REG_DELTA;
            if sz != SIZE_ANCHOR {
                m.d_size[sz] = (m.d_size[sz] - ETA_DELTA * w[3] * dl * s) * keep;
            }
            if q % TOUR_LEN != 0 {
                let j = q % TOUR_LEN;
                m.d_pos[j] = (m.d_pos[j] - ETA_DELTA * w[3] * dl * s) * keep;
            }
        }
    }

    for ls in live {
        if ls.len() < 2 {
            continue;
        }
        let mean = ls.iter().map(|&i| m.theta[i as usize]).sum::<f64>() / ls.len() as f64;
        for &i in ls {
            m.theta[i as usize] -= ETA_TEAMMATE * (m.theta[i as usize] - mean);
        }
    }

    let mut fit = Fit {
        params,
        forecast: vec![0.0; live.len()],
        var: vec![0.0; live.len()],
        roster: before
            .iter()
            .map(|th| match th.iter().map(|&x| x.exp()).sum::<f64>() {
                s if s > 0.0 => s.ln(),
                _ => f64::NAN,
            })
            .collect(),
        online,
        insample: (0.0, 0),
    };
    for (k, ls) in live.iter().enumerate() {
        if ls.is_empty() {
            continue;
        }
        let dsz = m.d_size[ls.len().min(SIZE_MAX)];
        for q in 0..t.n_q {
            let (b, la) = fit.params[q];
            let a = la.exp();
            let delta = dsz + m.d_pos[q % TOUR_LEN];
            let inten = |th: f64| (a * th - b - delta).clamp(-Z_CLIP, Z_CLIP).exp();
            let p = -(-before[k].iter().map(|&x| inten(x)).sum::<f64>()).exp_m1();
            fit.forecast[k] += p;
            fit.var[k] += p * (1.0 - p);
            if let Some(&y) = t.teams[k].mask.get(q) {
                if y != SKIP {
                    let s: f64 = ls.iter().map(|&i| inten(m.theta[i as usize])).sum();
                    fit.insample.0 += if y == CORRECT {
                        -(-(-s).exp_m1()).ln()
                    } else {
                        s
                    };
                    fit.insample.1 += 1;
                }
            }
        }
    }

    for ls in live {
        for &i in ls {
            m.games[i as usize] += 1;
            m.last[i as usize] = t.day;
        }
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

impl Universe {
    fn new(n_teams: usize, n_players: usize) -> Universe {
        Universe {
            per_team: vec![HashMap::new(); n_teams],
            per_player: vec![0; n_players],
            tournaments: vec![0; n_teams],
        }
    }

    fn add(&mut self, team: u32, ls: &[u32], sign: i32) {
        let t = team as usize;
        self.tournaments[t] = (self.tournaments[t] as i32 + sign).max(0) as u32;
        for &i in ls {
            let e = self.per_team[t].entry(i).or_insert(0);
            *e = (*e as i32 + sign).max(0) as u32;
            if *e == 0 {
                self.per_team[t].remove(&i);
            }
            let p = &mut self.per_player[i as usize];
            *p = (*p as i32 + sign).max(0) as u32;
        }
    }

    /// Weighted mean theta over the six most loyal players, and the weight sum.
    fn core(&self, team: u32, theta: &[f64]) -> Option<(f64, f64)> {
        let mut v: Vec<(u32, u32)> = self.per_team[team as usize]
            .iter()
            .filter(|(_, &n)| n >= CORE_MIN_GAMES)
            .map(|(&i, &n)| (i, n))
            .collect();
        v.sort_unstable_by_key(|&(i, n)| (std::cmp::Reverse(n), i));
        let (mut num, mut den) = (0.0, 0.0);
        for &(i, n) in v.iter().take(CORE_TOP) {
            let w =
                (n as f64 / self.per_player[i as usize].max(1) as f64) * (n as f64 / 3.0).min(1.0);
            num += w * theta[i as usize];
            den += w;
        }
        (den > 0.0).then_some((num / den, den))
    }
}

struct Core {
    all: Universe,
    os: Universe,
    log: Vec<(i32, u32, Vec<u32>, bool)>,
    cursor: usize,
}

impl Core {
    fn new(n_teams: usize, n_players: usize) -> Core {
        let u = Universe::new(n_teams, n_players);
        Core {
            all: u.clone(),
            os: u,
            log: Vec::new(),
            cursor: 0,
        }
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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg_path = arg("--config").unwrap_or_else(|| "config_quest.toml".into());
    let mut cfg = Config::load(&cfg_path)?;
    for (flag, slot) in [
        ("--from", &mut cfg.data.from),
        ("--to", &mut cfg.data.to),
        ("--out", &mut cfg.data.out_path),
    ] {
        if let Some(v) = arg(flag) {
            *slot = v;
        }
    }
    let start = Instant::now();
    let from_day = parse_date(&cfg.data.from).ok_or("bad --from")?;
    let to_day = parse_date(&cfg.data.to).ok_or("bad --to")?;
    let arena: Arena = chgk_rating::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
    println!(
        "loaded {} tournaments, {} players, {} teams in {:.1}s",
        arena.tournaments.len(),
        arena.n_players(),
        arena.n_teams(),
        start.elapsed().as_secs_f64()
    );

    let w = Writer::create(&cfg.data.out_path)?;
    let mut m = Model::new(arena.n_players());
    let mut act = Activity::new(arena.n_players());
    let mut core = Core::new(arena.n_teams(), arena.n_players());
    let mut qs: Vec<(i64, u32, f64, f64)> = Vec::new();
    let mut prev_p = vec![f64::NAN; arena.n_players()];
    let mut prev_t = vec![f64::NAN; arena.n_teams()];
    let mut release_id = 0i64;
    let mut next_rel = next_thursday(from_day);
    let mut next_recenter = i32::MIN;
    let mut shift = 0.0;
    let mut skipped = 0usize;
    let (mut tot_on, mut tot_in) = ((0.0, 0u64), (0.0, 0u64));

    for (n, t) in arena.tournaments.iter().enumerate() {
        if seasonal_aggregate(t) {
            skipped += 1;
            continue;
        }
        while next_rel <= t.day {
            release_id += 1;
            let mut r = Release {
                w: &w,
                id: release_id,
                day: next_rel,
                shift,
                last: next_rel + 7 > to_day,
                prev_p: &mut prev_p,
                prev_t: &mut prev_t,
            };
            r.emit(&m, &mut act, &mut core, &arena, &cfg)?;
            shift = 0.0;
            next_rel += 7;
            if release_id % 20 == 0 {
                w.checkpoint()?;
            }
        }
        if next_recenter == i32::MIN {
            next_recenter = t.day + RECENTER_DAYS;
        } else if t.day >= next_recenter {
            if let Some(d) = m.recenter(t.day, &mut qs) {
                shift += d;
                next_recenter = t.day + RECENTER_DAYS;
                println!("recenter {} delta {d:+.4}", format_date(t.day));
            }
        }
        let live: Vec<Vec<u32>> = t
            .teams
            .iter()
            .map(|r| {
                let mut v = r.members.clone();
                v.sort_unstable();
                v.dedup();
                for &i in &v {
                    if !m.seen[i as usize] {
                        m.seen[i as usize] = true;
                        m.theta[i as usize] = COLD_THETA;
                    }
                }
                v
            })
            .collect();
        let fit = run_tournament(t, &mut m, &live);
        tot_on.0 += fit.online.0;
        tot_on.1 += fit.online.1;
        tot_in.0 += fit.insample.0;
        tot_in.1 += fit.insample.1;

        let os = t.format != Format::Async;
        for (k, ls) in live.iter().enumerate() {
            if ls.is_empty() {
                continue;
            }
            core.push(t.day, t.teams[k].team, ls.clone(), os);
            for &i in ls {
                act.push(t.day, i, 1.0);
            }
        }
        qs.extend(
            fit.params
                .iter()
                .enumerate()
                .map(|(q, &(b, la))| (t.id, q as u32, b, la)),
        );
        write_events(&w, t, &arena, &fit)?;
        if n % 1000 == 0 {
            println!(
                "{:6}/{} {} {:.0}s",
                n,
                arena.tournaments.len(),
                format_date(t.day),
                start.elapsed().as_secs_f64()
            );
        }
    }
    while next_rel <= to_day {
        release_id += 1;
        let mut r = Release {
            w: &w,
            id: release_id,
            day: next_rel,
            shift,
            last: next_rel + 7 > to_day,
            prev_p: &mut prev_p,
            prev_t: &mut prev_t,
        };
        r.emit(&m, &mut act, &mut core, &arena, &cfg)?;
        shift = 0.0;
        next_rel += 7;
    }
    let mut st = w
        .conn
        .prepare("INSERT INTO question_params VALUES (?1,?2,?3,?4,1.0)")?;
    for &(tid, q, b, la) in &qs {
        st.execute(rusqlite::params![tid, q, b, la.exp()])?;
    }
    drop(st);
    w.finish()?;
    println!("{skipped} seasonal aggregates skipped, {} questions", qs.len());
    println!(
        "log-loss over {} observations: online {:.4}, in-sample {:.4}",
        tot_on.1,
        tot_on.0 / tot_on.1.max(1) as f64,
        tot_in.0 / tot_in.1.max(1) as f64
    );
    println!(
        "{release_id} releases, gauge shift total {:+.4}, done in {:.1}s",
        m.shift_cum,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

fn write_events(w: &Writer, t: &Tournament, arena: &Arena, fit: &Fit) -> rusqlite::Result<()> {
    let mut order: Vec<usize> = (0..t.teams.len()).collect();
    order.sort_by(|&a, &b| fit.forecast[b].total_cmp(&fit.forecast[a]));
    let mut place = vec![0.0; t.teams.len()];
    for (rank, &k) in order.iter().enumerate() {
        place[k] = rank as f64 + 1.0;
    }
    let evs: Vec<TeamEvent> = (0..t.teams.len())
        .map(|k| {
            let sd = fit.var[k].max(1e-9).sqrt();
            TeamEvent {
                d_place: f64::NAN,
                d_pull: f64::NAN,
                team: t.teams[k].team,
                c: 0.0,
                delta_l: 0.0,
                x: fit.roster[k],
                forecast: fit.forecast[k],
                forecast_place: place[k],
                expected_place: 0.0,
                actual_place: t.teams[k].position,
                pred_place: t.teams[k].pred_pos,
                s_r: fit.roster[k],
                s_pair: 0.0,
                i_pair: 0.0,
                mu_before: fit.roster[k],
                mu_after: fit.roster[k],
                sigma_before: sd,
                sigma_after: sd,
                evidence: 0.0,
                gap_days: 0,
                var_diff: 0.0,
                lost_prev: 0.0,
                rated: t.teams[k].rated,
                z: t.teams[k].score.map(|s| (s - fit.forecast[k]) / sd),
            }
        })
        .collect();
    w.team_events(t.id, &format_date(t.day), &arena.team_ids, &evs)?;
    let actual: Vec<f64> = evs.iter().map(|e| e.actual_place).collect();
    let (po, pa): (Vec<f64>, Vec<f64>) = evs
        .iter()
        .filter_map(|e| e.pred_place.map(|p| (p, e.actual_place)))
        .unzip();
    let date = format_date(t.day);
    let field = TournamentMetrics::field(t.id, &date, t.format.name(), weights(t.format)[0], t.teams.len(), t.n_q);
    let mean = |(sum, n): (f64, u64)| (n > 0).then(|| sum / n as f64);
    w.tournament_metrics(&TournamentMetrics {
        n_obs: fit.online.1,
        insample: mean(fit.insample),
        crossfit: mean(fit.online),
        sp_forecast: spearman(&place, &actual),
        sp_old: spearman(&po, &pa),
        ..field
    })
}

struct Release<'a> {
    w: &'a Writer,
    id: i64,
    day: i32,
    shift: f64,
    last: bool,
    prev_p: &'a mut [f64],
    prev_t: &'a mut [f64],
}

impl Release<'_> {
    fn emit(
        &mut self,
        m: &Model,
        act: &mut Activity,
        core: &mut Core,
        arena: &Arena,
        cfg: &Config,
    ) -> rusqlite::Result<()> {
        act.expire_before(self.day - cfg.gauge.window_days);
        core.expire_before(self.day - CORE_WINDOW);
        let active: Vec<usize> = (0..arena.n_players()).filter(|&i| act.count[i] > 0).collect();
        self.w.release(
            self.id,
            &format_date(self.day),
            self.shift,
            m.shift_cum,
            active.len(),
        )?;
        if !cfg.data.write_rows(self.id, self.last) {
            return Ok(());
        }
        let rows: Vec<(i64, f64, f64, u32, f64)> = active
            .iter()
            .map(|&i| {
                let change = if self.prev_p[i].is_finite() {
                    m.theta[i] - self.prev_p[i]
                } else {
                    0.0
                };
                self.prev_p[i] = m.theta[i];
                (
                    arena.player_ids[i],
                    m.theta[i],
                    1.0 / m.fisher[i].max(1e-9).sqrt(),
                    act.count[i],
                    change,
                )
            })
            .collect();
        self.w.player_ratings(self.id, &rows, |t| t)?;
        let trows: Vec<(i64, f64, f64, f64, f64, f64)> = (0..arena.n_teams())
            .filter(|&ti| core.all.tournaments[ti] >= CORE_MIN_TOURNAMENTS)
            .filter_map(|ti| {
                let u = if core.os.tournaments[ti] > 0 {
                    &core.os
                } else {
                    &core.all
                };
                let (th, size) = u.core(ti as u32, &m.theta)?;
                (size >= CORE_MIN_SIZE).then(|| {
                    let change = if self.prev_t[ti].is_finite() {
                        th - self.prev_t[ti]
                    } else {
                        0.0
                    };
                    self.prev_t[ti] = th;
                    (
                        arena.team_ids[ti],
                        th,
                        0.0,
                        th,
                        core.all.tournaments[ti] as f64,
                        change,
                    )
                })
            })
            .collect();
        self.w.team_ratings(self.id, &trows, |mu| mu)
    }
}
