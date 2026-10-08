//! The wide per-result feature table of plan 8: everything the pipeline can
//! know about a lineup **on the morning of its tournament**, one row per result
//! of every in-person field.
//!
//! This module is a research instrument. Nothing in the shipped model reads it,
//! and turning it on changes no number the model produces: it is handed the
//! same state the forecast was handed, after that field has been scored and
//! before any memory has absorbed it.
//!
//! **Leakage is the one way the table fails**, so every quantity here obeys one
//! rule: it is computed from tournaments strictly earlier than this one, or
//! from the calendar. In particular nothing reads this tournament's masks, its
//! scores or anyone's finishing place. The three histories the module keeps of
//! its own — a player's recent games, a team's past surprises, a team's record
//! under an editor — are advanced in `absorb_players` and `absorb_teams`, which
//! `fit` calls only after the row has been written, exactly as
//! `editor_memory::EditorMemory` is advanced. `tools/features_check.py` is the
//! test of that claim.
//!
//! The columns whose names begin `y_` are the target and `w_weight` is the
//! metric's own weight; `m_` is what the shipped model forecast, kept so the
//! ranker can be scored against it. `ext_` columns are filled in later, from
//! the external systems' own databases, and are for diagnosis only.

use crate::chemistry::Chemistry;
use crate::config::{Config, S2Lineup, S2};
use crate::data::{Arena, Format, Tournament};
use crate::editor_memory::EditorMemory;
use crate::question_model::Players;
use crate::team_event::TeamEvent;
use crate::team_s2::{lineup_strength, player_rating, roster_sd, TeamsS2};
use crate::util::civil_from_days;
use rusqlite::types::Value;
use rusqlite::Connection;
use std::collections::{HashMap, VecDeque};

/// How far back the per-player and per-team histories are kept.
const KEEP_DAYS: i32 = 420;
const YEAR: i32 = 365;

/// One row under construction. The names are built by the same code that builds
/// the values, on the first row only, so the header and the row can never drift
/// apart.
struct Row {
    names: Vec<String>,
    vals: Vec<f64>,
    want: bool,
}

impl Row {
    fn new(want: bool) -> Row {
        Row { names: Vec::new(), vals: Vec::new(), want }
    }
    fn p<F: FnOnce() -> String>(&mut self, f: F, v: f64) {
        if self.want {
            self.names.push(f());
        }
        self.vals.push(v);
    }
}

/// The summaries a per-player quantity is reduced to over a lineup.
#[derive(Clone, Copy)]
enum St {
    Mean,
    Min,
    Max,
    Sd,
    Top3,
    Bot2,
}

const ST_ALL: &[St] = &[St::Mean, St::Min, St::Max, St::Sd, St::Top3, St::Bot2];
const ST_MMMS: &[St] = &[St::Mean, St::Min, St::Max, St::Sd];
const ST_MMM: &[St] = &[St::Mean, St::Min, St::Max];
const ST_M: &[St] = &[St::Mean];

fn st_name(s: St) -> &'static str {
    match s {
        St::Mean => "mean",
        St::Min => "min",
        St::Max => "max",
        St::Sd => "sd",
        St::Top3 => "top3",
        St::Bot2 => "bot2",
    }
}

/// Every summary is taken over the members whose value is known, so one player
/// with no in-person record does not blank the whole lineup's column.
fn st_val(s: St, v: &[f64]) -> f64 {
    let mut f: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if f.is_empty() {
        return f64::NAN;
    }
    match s {
        St::Mean => f.iter().sum::<f64>() / f.len() as f64,
        St::Min => f.iter().copied().fold(f64::INFINITY, f64::min),
        St::Max => f.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        St::Sd => {
            let m = f.iter().sum::<f64>() / f.len() as f64;
            (f.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / f.len() as f64).sqrt()
        }
        St::Top3 => {
            f.sort_by(|a, b| b.total_cmp(a));
            f.iter().take(3).sum::<f64>()
        }
        St::Bot2 => {
            f.sort_by(f64::total_cmp);
            let k = 2.min(f.len());
            f.iter().take(k).sum::<f64>() / k as f64
        }
    }
}

fn agg(r: &mut Row, name: &str, v: &[f64], sts: &[St]) {
    for &s in sts {
        r.p(|| format!("agg_{name}_{}", st_name(s)), st_val(s, v));
    }
}

fn key2(a: u32, b: u32) -> u64 {
    ((a as u64) << 32) | b as u64
}

/// The team state as it stood before the field was scored. `fit` takes this
/// snapshot before the engine's step and hands it back afterwards, so the row
/// carries the rating the forecast was made with rather than the one the result
/// produced.
pub struct Pre {
    states: Vec<Option<TeamState>>,
    rank: Vec<f64>,
}

struct TeamHist {
    /// `(day, surprise)` over in-person rated results: how far the lineup
    /// forecast put the team from where it finished, in fields of that size.
    surp: VecDeque<(i32, f64)>,
    /// `(day, place / n)` over the same results.
    place: VecDeque<(i32, f64)>,
    last_rated: Vec<u32>,
    last_ip: Vec<u32>,
}

impl TeamHist {
    fn new() -> TeamHist {
        TeamHist {
            surp: VecDeque::new(),
            place: VecDeque::new(),
            last_rated: Vec::new(),
            last_ip: Vec::new(),
        }
    }
}

/// Everything the table needs that no other module keeps.
pub struct Features {
    conn: Connection,
    cols: usize,
    /// Written once, when the first row is built.
    header: Vec<String>,
    /// The day a player was first seen in any format, and their recent record.
    p_first: Vec<i32>,
    p_ip_days: Vec<VecDeque<i32>>,
    p_theta: Vec<VecDeque<(i32, f64)>>,
    p_sign: Vec<VecDeque<f64>>,
    teams: HashMap<u32, TeamHist>,
    /// `(team, editor)` to the sum and the count of past residuals, the team
    /// analogue of `editor_memory`'s player pairs.
    te: HashMap<u64, (f64, f64)>,
    /// How many past in-person fields a team has played under an editor.
    te_n: HashMap<u64, f64>,
    weights: HashMap<i64, f64>,
    /// In-person tournaments per calendar day: a multi-stage event is several.
    same_day: HashMap<i32, f64>,
    rank_day: i32,
    rank: Vec<f64>,
    rows: u64,
}

impl Features {
    pub fn create(path: &str, arena: &Arena) -> rusqlite::Result<Features> {
        if let Some(dir) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::remove_file(path);
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;")?;
        let mut same_day: HashMap<i32, f64> = HashMap::new();
        for t in &arena.tournaments {
            if t.format == Format::InPerson && !t.train_only {
                *same_day.entry(t.day).or_insert(0.0) += 1.0;
            }
        }
        let n = arena.n_players();
        Ok(Features {
            conn,
            cols: 0,
            header: Vec::new(),
            p_first: vec![i32::MIN; n],
            p_ip_days: (0..n).map(|_| VecDeque::new()).collect(),
            p_theta: (0..n).map(|_| VecDeque::new()).collect(),
            p_sign: (0..n).map(|_| VecDeque::new()).collect(),
            teams: HashMap::new(),
            te: HashMap::new(),
            te_n: HashMap::new(),
            weights: crate::ndcg::weights(crate::ndcg::WEIGHTS),
            same_day,
            rank_day: i32::MIN / 2,
            rank: Vec::new(),
            rows: 0,
        })
    }

    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// The team states of this field, before the engine touches them. The
    /// rating rank is over every team with a rated game in the trailing year,
    /// recomputed at most once a week because nothing in it moves faster.
    pub fn pre(&mut self, t: &Tournament, teams: &TeamsS2) -> Pre {
        if t.day - self.rank_day >= 7 {
            let mut v: Vec<(usize, f64)> = (0..teams.n_teams())
                .filter_map(|i| {
                    teams
                        .state_of(i)
                        .filter(|s| s.3 >= t.day - YEAR)
                        .map(|s| (i, s.0))
                })
                .collect();
            v.sort_by(|a, b| b.1.total_cmp(&a.1));
            self.rank = vec![f64::NAN; teams.n_teams()];
            for (k, (i, _)) in v.iter().enumerate() {
                self.rank[*i] = (k + 1) as f64;
            }
            self.rank_day = t.day;
        }
        Pre {
            states: t
                .teams
                .iter()
                .map(|r| teams.state_of(r.team as usize))
                .collect(),
            rank: t
                .teams
                .iter()
                .map(|r| self.rank.get(r.team as usize).copied().unwrap_or(f64::NAN))
                .collect(),
        }
    }

    /// One in-person field, one row per result. Called after the engine's step
    /// — which only moves team state, and `pre` has already saved that — and
    /// before any of the forecast memories absorbs the day.
    #[allow(clippy::too_many_arguments)]
    pub fn emit(
        &mut self,
        t: &Tournament,
        arena: &Arena,
        players: &Players,
        chem: &Chemistry,
        edmem: &EditorMemory,
        pre: &Pre,
        events: &[TeamEvent],
        cfg: &Config,
    ) -> rusqlite::Result<()> {
        if t.format != Format::InPerson || t.train_only {
            return Ok(());
        }
        let field = FieldCtx::new(t, players, cfg, self.weights.get(&t.id).copied().unwrap_or(0.0));
        let ev: HashMap<u32, &TeamEvent> = events.iter().map(|e| (e.team, e)).collect();
        let mut batch: Vec<(i64, i64, String, Vec<f64>)> = Vec::with_capacity(t.teams.len());
        for (ri, res) in t.teams.iter().enumerate() {
            let want = self.header.is_empty() && batch.is_empty();
            let mut r = Row::new(want);
            field.lineup_cols(&mut r, ri, &cfg.team.s2);
            let lin = self.player_cols(&mut r, &res.members, players, t.day, cfg);
            self.shape_cols(&mut r, res, &lin, chem, pre.states[ri], field.q_ship[ri], cfg);
            self.team_cols(&mut r, res.team, pre.states[ri], pre.rank[ri], t.day);
            self.editor_cols(&mut r, res, &lin.theta, edmem, players.ed_gate_theta, &t.editors);
            field.field_cols(&mut r, ri);
            self.calendar_cols(&mut r, t, res);
            // The external systems, for diagnosis only.
            r.p(|| "ext_old_pred".into(), res.pred_pos.unwrap_or(f64::NAN));
            target_cols(&mut r, res, ev.get(&res.team).copied(), events.len(), &field);
            if want {
                self.create_table(std::mem::take(&mut r.names))?;
            }
            assert_eq!(r.vals.len(), self.cols, "feature row width drifted");
            let date = crate::util::format_date(t.day);
            batch.push((t.id, arena.team_ids[res.team as usize], date, r.vals));
        }
        self.insert(batch)
    }

    fn create_table(&mut self, header: Vec<String>) -> rusqlite::Result<()> {
        self.header = header;
        self.cols = self.header.len();
        let cols: String = self.header.iter().map(|c| format!(", \"{c}\" REAL")).collect();
        self.conn.execute_batch(&format!(
            "CREATE TABLE features (tournament_id INTEGER, team_id INTEGER, date TEXT{cols});
             BEGIN"
        ))
    }

    fn insert(&mut self, batch: Vec<(i64, i64, String, Vec<f64>)>) -> rusqlite::Result<()> {
        let holes: String = (4..self.cols + 4).map(|i| format!(",?{i}")).collect();
        let mut stmt = self.conn.prepare_cached(&format!("INSERT INTO features VALUES (?1,?2,?3{holes})"))?;
        for (tid, team, date, vals) in batch {
            let mut v: Vec<Value> = Vec::with_capacity(self.cols + 3);
            v.push(Value::Integer(tid));
            v.push(Value::Integer(team));
            v.push(Value::Text(date));
            v.extend(vals.into_iter().map(|x| if x.is_finite() { Value::Real(x) } else { Value::Null }));
            stmt.execute(rusqlite::params_from_iter(v))?;
            self.rows += 1;
        }
        drop(stmt);
        if self.rows % 20_000 < 200 {
            self.conn.execute_batch("COMMIT; BEGIN")?;
        }
        Ok(())
    }

    /// The per-player aggregates over the lineup, and the per-player values
    /// the later groups read again.
    fn player_cols(&self, r: &mut Row, members: &[u32], players: &Players, day: i32, cfg: &Config) -> LineupHistory {
        let mut h = LineupHistory::default();
        let since = |d: i32, last: i32, n: u32| if n > 0 { (d - last) as f64 } else { f64::NAN };
        for &m in members {
            let i = m as usize;
            let st = &players.state[i];
            let (a, f) = (players.theta(i), players.phi(i));
            h.theta.push(a);
            h.phi.push(f);
            h.thph.push(a + f);
            h.sigma.push(players.sigma_at(i, day, cfg));
            h.games.push(st.games as f64);
            h.games_ip.push(st.games_ip as f64);
            h.games_ip365.push(count_since(&self.p_ip_days[i], day - YEAR));
            h.days_last.push(since(day, st.last_day, st.games));
            h.days_ip.push(since(day, st.last_ip, st.games_ip));
            h.vol.push(st.vol);
            h.first_age.push(if self.p_first[i] > i32::MIN { (day - self.p_first[i]) as f64 } else { f64::NAN });
            h.share_ip.push(if st.games > 0 { st.games_ip as f64 / st.games as f64 } else { f64::NAN });
            let s = &self.p_sign[i];
            h.sign5.push(if s.is_empty() { f64::NAN } else { s.iter().sum::<f64>() / s.len() as f64 });
            h.d90.push(theta_change(&self.p_theta[i], day - 90, a));
            h.d365.push(theta_change(&self.p_theta[i], day - YEAR, a));
        }
        agg(r, "theta", &h.theta, ST_ALL);
        agg(r, "thetaphi", &h.thph, ST_ALL);
        agg(r, "sigma", &h.sigma, ST_ALL);
        agg(r, "games_ip", &h.games_ip, ST_MMMS);
        agg(r, "days_ip", &h.days_ip, ST_MMMS);
        agg(r, "phi", &h.phi, ST_MMM);
        agg(r, "games", &h.games, ST_MMM);
        agg(r, "games_ip365", &h.games_ip365, ST_MMM);
        agg(r, "days_last", &h.days_last, ST_MMM);
        agg(r, "vol", &h.vol, ST_MMM);
        agg(r, "first_age", &h.first_age, ST_MMM);
        agg(r, "share_ip", &h.share_ip, ST_MMM);
        agg(r, "dtheta90", &h.d90, ST_MMM);
        agg(r, "dtheta365", &h.d365, ST_MMM);
        agg(r, "sign5", &h.sign5, ST_M);
        h
    }

    /// Who the lineup is made of: its size, how new its players are, how much
    /// of it played the team's last games, and how long its pairs have played
    /// together.
    #[allow(clippy::too_many_arguments)]
    fn shape_cols(
        &self,
        r: &mut Row,
        res: &crate::data::TeamResult,
        lin: &LineupHistory,
        chem: &Chemistry,
        state: Option<TeamState>,
        q: f64,
        cfg: &Config,
    ) {
        let members = &res.members;
        let n_m = members.len();
        let hist = self.teams.get(&res.team);
        let in_last = |prev: Option<&Vec<u32>>| match prev {
            Some(v) if !v.is_empty() => members.iter().filter(|m| v.contains(m)).count() as f64,
            _ => f64::NAN,
        };
        let count = |v: &[f64], f: &dyn Fn(f64) -> bool| v.iter().filter(|x| f(**x)).count() as f64;
        let n_last_rated = in_last(hist.map(|h| &h.last_rated));
        let n_last_ip = in_last(hist.map(|h| &h.last_ip));
        let per_member = |x: f64| if n_m == 0 { f64::NAN } else { x / n_m as f64 };
        let prov = cfg.display.provisional_sigma;
        r.p(|| "shp_size".into(), n_m as f64);
        r.p(|| "shp_newcomers".into(), count(&lin.sigma, &|s| s > prov));
        r.p(|| "shp_few_ip".into(), count(&lin.games_ip, &|g| g < 5.0));
        r.p(|| "shp_sync_only".into(), per_member(count(&lin.games_ip, &|g| g == 0.0)));
        r.p(|| "shp_no_history".into(), count(&lin.games, &|g| g == 0.0));
        r.p(|| "shp_n_last_rated".into(), n_last_rated);
        r.p(|| "shp_share_last_rated".into(), per_member(n_last_rated));
        r.p(|| "shp_n_last_ip".into(), n_last_ip);
        let (c_mean, c_min, c_max) = pair_stats(chem, members);
        r.p(|| "shp_chem_mean".into(), c_mean);
        r.p(|| "shp_chem_min".into(), c_min);
        r.p(|| "shp_chem_max".into(), c_max);
        let gap = match state {
            Some((_, qbar, _, _)) if q.is_finite() => q - qbar,
            _ => f64::NAN,
        };
        r.p(|| "shp_q_minus_qbar".into(), gap);
    }

    /// The team's state before the field, and its recent surprises and places.
    fn team_cols(&self, r: &mut Row, team: u32, st: Option<TeamState>, rank: f64, day: i32) {
        let hist = self.teams.get(&team);
        let field = |f: fn(&TeamState) -> f64| st.as_ref().map_or(f64::NAN, f);
        r.p(|| "tm_has_state".into(), st.is_some() as u8 as f64);
        r.p(|| "tm_r".into(), field(|s| s.0));
        r.p(|| "tm_qbar".into(), field(|s| s.1));
        r.p(|| "tm_r_minus_qbar".into(), field(|s| s.0 - s.1));
        r.p(|| "tm_games".into(), field(|s| s.2));
        r.p(|| "tm_days_since".into(), st.map_or(f64::NAN, |s| (day - s.3) as f64));
        let surp: Vec<f64> = hist.map(|h| h.surp.iter().rev().take(5).map(|x| x.1).collect()).unwrap_or_default();
        r.p(|| "tm_surp5_mean".into(), st_val(St::Mean, &surp));
        r.p(|| "tm_surp5_sd".into(), st_val(St::Sd, &surp));
        let year = |q: &VecDeque<(i32, f64)>| -> Vec<f64> { q.iter().filter(|x| x.0 >= day - YEAR).map(|x| x.1).collect() };
        let surp365 = hist.map(|h| year(&h.surp)).unwrap_or_default();
        r.p(|| "tm_surp365_mean".into(), st_val(St::Mean, &surp365));
        r.p(|| "tm_surp365_n".into(), surp365.len() as f64);
        let pl365 = hist.map(|h| year(&h.place)).unwrap_or_default();
        r.p(|| "tm_best_place365".into(), st_val(St::Min, &pl365));
        r.p(|| "tm_worst_place365".into(), st_val(St::Max, &pl365));
        r.p(|| "tm_rank".into(), rank);
    }

    /// The record of the lineup's players, and of the team, under today's editors.
    fn editor_cols(
        &self,
        r: &mut Row,
        res: &crate::data::TeamResult,
        theta: &[f64],
        edmem: &EditorMemory,
        gate: f64,
        editors: &[u32],
    ) {
        let (ed_g, ng) = ed_pairs(edmem, &res.members, editors, theta, gate);
        let (ed_u, nu) = ed_pairs(edmem, &res.members, editors, theta, f64::NEG_INFINITY);
        r.p(|| "ed_player_gated".into(), ed_g);
        r.p(|| "ed_player_ungated".into(), ed_u);
        r.p(|| "ed_pairs_gated".into(), ng);
        r.p(|| "ed_pairs_ungated".into(), nu);
        let (mut ts, mut tn, mut tc) = (0.0, 0.0, 0.0);
        for &e in editors {
            if let Some(&(s, c)) = self.te.get(&key2(res.team, e)) {
                ts += s / c;
                tn += 1.0;
            }
            tc += self.te_n.get(&key2(res.team, e)).copied().unwrap_or(0.0);
        }
        r.p(|| "ed_team_resid".into(), if tn > 0.0 { ts / tn } else { f64::NAN });
        r.p(|| "ed_team_n".into(), tn);
        r.p(|| "ed_team_tournaments".into(), tc);
        r.p(|| "ed_n_editors".into(), editors.len() as f64);
    }

    fn calendar_cols(&self, r: &mut Row, t: &Tournament, res: &crate::data::TeamResult) {
        let (year, month, dom) = civil_from_days(t.day);
        // A ChGK season runs from September, so September is month 0.
        let season_month = ((month as i32 - 9) + 12) % 12;
        let season_start = crate::util::days_from_civil(if month >= 9 { year } else { year - 1 }, 9, 1);
        // 1970-01-01 was a Thursday.
        let dow = (t.day.rem_euclid(7) + 4) % 7;
        r.p(|| "cal_month".into(), month as f64);
        r.p(|| "cal_season_month".into(), season_month as f64);
        r.p(|| "cal_dow".into(), dow as f64);
        r.p(|| "cal_dom".into(), dom as f64);
        r.p(|| "cal_same_date".into(), self.same_day.get(&t.day).copied().unwrap_or(1.0));
        r.p(|| "cal_season_day".into(), (t.day - season_start) as f64);
        let home = if t.town == u32::MAX || res.town == u32::MAX { f64::NAN } else { (t.town == res.town) as u8 as f64 };
        r.p(|| "cal_home".into(), home);
    }

    /// The player half of the histories, advanced once the row is written.
    /// `grads` is `(player, gradient, information)` of this tournament, so the
    /// sign of the gradient is the day's surprise for that player.
    pub fn absorb_players(
        &mut self,
        t: &Tournament,
        grads: &[(u32, f64, f64)],
        players: &Players,
    ) {
        let mut seen: Vec<u32> = t
            .teams
            .iter()
            .flat_map(|r| r.members.iter().copied())
            .collect();
        seen.sort_unstable();
        seen.dedup();
        for m in seen {
            let i = m as usize;
            if self.p_first[i] == i32::MIN {
                self.p_first[i] = t.day;
            }
            let h = &mut self.p_theta[i];
            h.push_back((t.day, players.theta(i)));
            while h.len() > 1 && h[1].0 < t.day - KEEP_DAYS {
                h.pop_front();
            }
            if t.format == Format::InPerson {
                let d = &mut self.p_ip_days[i];
                d.push_back(t.day);
                while d.front().is_some_and(|x| *x < t.day - YEAR) {
                    d.pop_front();
                }
            }
        }
        for &(m, g, _) in grads {
            let s = &mut self.p_sign[m as usize];
            s.push_back(g.signum());
            if s.len() > 5 {
                s.pop_front();
            }
        }
    }

    /// The team half: the surprises, the places, the lineups and the record
    /// under today's editors. In-person rated results only, which is what every
    /// other outcome memory in the engine counts.
    pub fn absorb_teams(&mut self, t: &Tournament, events: &[TeamEvent]) {
        if t.format != Format::InPerson || t.train_only {
            return;
        }
        let n = events.len().max(1) as f64;
        let by_team: HashMap<u32, &TeamEvent> = events.iter().map(|e| (e.team, e)).collect();
        for res in &t.teams {
            let Some(e) = by_team.get(&res.team) else {
                continue;
            };
            if !e.rated || e.actual_place <= 0.0 {
                continue;
            }
            let h = self.teams.entry(res.team).or_insert_with(TeamHist::new);
            h.surp
                .push_back((t.day, (e.forecast_place - e.actual_place) / n));
            h.place.push_back((t.day, e.actual_place / n));
            while h.surp.front().is_some_and(|x| x.0 < t.day - KEEP_DAYS) && h.surp.len() > 5 {
                h.surp.pop_front();
            }
            while h.place.front().is_some_and(|x| x.0 < t.day - KEEP_DAYS) {
                h.place.pop_front();
            }
            if !res.members.is_empty() {
                h.last_rated = res.members.clone();
                h.last_ip = res.members.clone();
            }
            let resid = (e.forecast_place - e.actual_place) / n;
            for &ed in &t.editors {
                let a = self.te.entry(key2(res.team, ed)).or_insert((0.0, 0.0));
                a.0 += resid;
                a.1 += 1.0;
                *self.te_n.entry(key2(res.team, ed)).or_insert(0.0) += 1.0;
            }
        }
    }

    pub fn finish(self) -> rusqlite::Result<()> {
        self.conn.execute_batch("COMMIT")?;
        self.conn
            .execute_batch("CREATE INDEX idx_features_t ON features(tournament_id)")?;
        Ok(())
    }
}

/// A team's `(rating, usual lineup, rated games, day of the last rated game)`.
type TeamState = (f64, f64, f64, i32);

/// The per-player values of one lineup that more than one group reads.
#[derive(Default)]
struct LineupHistory {
    theta: Vec<f64>,
    phi: Vec<f64>,
    thph: Vec<f64>,
    sigma: Vec<f64>,
    games: Vec<f64>,
    games_ip: Vec<f64>,
    games_ip365: Vec<f64>,
    days_last: Vec<f64>,
    days_ip: Vec<f64>,
    vol: Vec<f64>,
    first_age: Vec<f64>,
    share_ip: Vec<f64>,
    sign5: Vec<f64>,
    d90: Vec<f64>,
    d365: Vec<f64>,
}

/// One field read once: every lineup on the team scale, the shipped rule's
/// strength of each, and the summaries of those the field columns read.
struct FieldCtx {
    base: Vec<Vec<f64>>,
    sigmas: Vec<Vec<f64>>,
    q_ship: Vec<f64>,
    known: Vec<f64>,
    n_teams: f64,
    weight: f64,
    mean: f64,
    sd: f64,
    top10: f64,
    first: f64,
    fourth: f64,
}

impl FieldCtx {
    fn new(t: &Tournament, players: &Players, cfg: &Config, weight: f64) -> FieldCtx {
        let p = &cfg.team.s2;
        let vbar = players.editor_mean(t);
        let seat = |&m: &u32| player_rating(players.theta(m as usize) + players.affinity(m as usize, &vbar), cfg);
        let base: Vec<Vec<f64>> = t.teams.iter().map(|res| res.members.iter().map(seat).collect()).collect();
        let sigmas: Vec<Vec<f64>> = t
            .teams
            .iter()
            .map(|res| res.members.iter().map(|&m| players.sigma_at(m as usize, t.day, cfg)).collect())
            .collect();
        // The shipped rule at the shipped quantile is the field's own ordering,
        // and the field-context features are read off it.
        let ship = |(b, s): (&Vec<f64>, &Vec<f64>)| {
            if b.is_empty() {
                f64::NAN
            } else {
                lineup_strength(&discount(b, s, p.sigma_c, p.player_scale), p)
            }
        };
        let q_ship: Vec<f64> = base.iter().zip(&sigmas).map(ship).collect();
        let known: Vec<f64> = q_ship.iter().copied().filter(|x| x.is_finite()).collect();
        let mut sorted = known.clone();
        sorted.sort_by(|a, b| b.total_cmp(a));
        let (mean, sd) = if known.is_empty() {
            (f64::NAN, f64::NAN)
        } else {
            let m = known.iter().sum::<f64>() / known.len() as f64;
            (m, (known.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / known.len() as f64).sqrt())
        };
        let k = 10.min(sorted.len());
        let top10 = if k == 0 { f64::NAN } else { sorted[..k].iter().sum::<f64>() / k as f64 };
        FieldCtx {
            first: sorted.first().copied().unwrap_or(f64::NAN),
            fourth: sorted.get(3).copied().unwrap_or(f64::NAN),
            base,
            sigmas,
            q_ship,
            known,
            n_teams: t.teams.len() as f64,
            weight,
            mean,
            sd,
            top10,
        }
    }

    /// The lineup's strength under every rule of DESIGN 4.1 at four quantiles.
    fn lineup_cols(&self, r: &mut Row, ri: usize, p: &S2) {
        let (b, sg) = (&self.base[ri], &self.sigmas[ri]);
        for (rn, rule, tau) in [
            ("mean", S2Lineup::Mean, 1.0),
            ("best6", S2Lineup::Best6, 1.0),
            ("rost", S2Lineup::Roster, 1.0),
            ("t05", S2Lineup::Tempered, 0.5),
            ("t07", S2Lineup::Tempered, 0.7),
        ] {
            let mut pr = p.clone();
            pr.lineup = rule;
            pr.tau = tau;
            for sc in [0.0, 0.75, 1.5, 3.0] {
                let v =
                    if b.is_empty() { f64::NAN } else { lineup_strength(&discount(b, sg, sc, p.player_scale), &pr) };
                r.p(|| format!("lin_{rn}_sc{:03}", (sc * 100.0) as i32), v);
            }
        }
        let sd = if b.is_empty() { f64::NAN } else { roster_sd(&discount(b, sg, p.sigma_c, p.player_scale), sg, p) };
        r.p(|| "lin_roster_sd".into(), sd);
    }

    /// Where the lineup stands in its field.
    fn field_cols(&self, r: &mut Row, ri: usize) {
        let q = self.q_ship[ri];
        r.p(|| "fld_n_teams".into(), self.n_teams);
        r.p(|| "fld_weight".into(), self.weight);
        let rank = if q.is_finite() { 1.0 + self.known.iter().filter(|x| **x > q).count() as f64 } else { f64::NAN };
        r.p(|| "fld_rank_q".into(), rank);
        r.p(|| "fld_rank_frac".into(), rank / self.n_teams);
        r.p(|| "fld_gap_first".into(), q - self.first);
        r.p(|| "fld_gap_fourth".into(), q - self.fourth);
        r.p(|| "fld_z".into(), if self.sd > 0.0 { (q - self.mean) / self.sd } else { f64::NAN });
        let near = if q.is_finite() {
            self.known.iter().filter(|x| (*x - q).abs() <= 100.0).count() as f64 - 1.0
        } else {
            f64::NAN
        };
        r.p(|| "fld_within100".into(), near);
        r.p(|| "fld_mean_q".into(), self.mean);
        r.p(|| "fld_top10_q".into(), self.top10);
        r.p(|| "fld_sd_q".into(), self.sd);
        r.p(|| "fld_spread14".into(), self.first - self.fourth);
    }
}

/// The target, the metric's weight, and what the model said.
fn target_cols(r: &mut Row, res: &crate::data::TeamResult, e: Option<&TeamEvent>, n_events: usize, field: &FieldCtx) {
    r.p(|| "m_forecast".into(), e.map_or(f64::NAN, |e| e.forecast));
    // The field size the outcome memories divide a residual by, which is the
    // count of entries the engine kept and not always the count of results.
    // `tools/features_check.py` needs it to re-derive them.
    r.p(|| "m_n_events".into(), n_events as f64);
    r.p(|| "m_forecast_place".into(), e.map_or(f64::NAN, |e| e.forecast_place));
    r.p(|| "w_weight".into(), field.weight);
    r.p(|| "y_n_teams".into(), field.n_teams);
    r.p(|| "y_rated".into(), res.rated as u8 as f64);
    r.p(|| "y_place".into(), res.position);
}

/// Each member read at a pessimistic quantile: the affine map of DESIGN 4.1 is
/// linear in `theta`, so a different `sigma_c` is a shift of the same numbers.
fn discount(base: &[f64], sigmas: &[f64], sc: f64, scale: f64) -> Vec<f64> {
    base.iter()
        .zip(sigmas)
        .map(|(b, s)| b - scale * sc * s)
        .collect()
}

fn count_since(d: &VecDeque<i32>, from: i32) -> f64 {
    d.iter().filter(|x| **x >= from).count() as f64
}

/// How much a player's `theta` has moved since the last record on or before
/// `when`. `None` when the history does not reach back that far.
fn theta_change(h: &VecDeque<(i32, f64)>, when: i32, now: f64) -> f64 {
    match h.iter().rev().find(|x| x.0 <= when) {
        Some(&(_, old)) => now - old,
        None => f64::NAN,
    }
}

/// The mean and the count of the qualifying `(player, editor)` pairs, which is
/// `editor_memory::bonus` with the count kept.
fn ed_pairs(
    edmem: &EditorMemory,
    members: &[u32],
    editors: &[u32],
    theta: &[f64],
    gate: f64,
) -> (f64, f64) {
    let (mut sum, mut n) = (0.0, 0.0);
    for (k, &m) in members.iter().enumerate() {
        if theta.get(k).copied().unwrap_or(f64::NEG_INFINITY) < gate {
            continue;
        }
        for &e in editors {
            if let Some((s, c)) = edmem.pair(m, e) {
                sum += s / c;
                n += 1.0;
            }
        }
    }
    (if n > 0.0 { sum / n } else { f64::NAN }, n)
}

/// `(mean, min, max)` of `ln(1 + games together)` over a lineup's pairs.
fn pair_stats(chem: &Chemistry, members: &[u32]) -> (f64, f64, f64) {
    if members.len() < 2 {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let mut v = Vec::new();
    for a in 0..members.len() {
        for b in a + 1..members.len() {
            v.push(chem.pair(members[a], members[b]));
        }
    }
    (
        st_val(St::Mean, &v),
        st_val(St::Min, &v),
        st_val(St::Max, &v),
    )
}
