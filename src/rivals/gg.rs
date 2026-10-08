//! rating.chgk.gg, the current official rating of sports ChGK (the "b" model),
//! reimplemented from `chgk-gg/rating-b`. A.x tags name the clause of Приложение А
//! (`data/external/chgk_gg/prilozhenie_A.txt`) that the code below transcribes.
//! Bootstrapped from the official release of 2021-09-09, stepped weekly to the
//! present, written to the ratings.db schema of DESIGN 6.
//!
//! `--players DB` keeps the team rules and swaps the players: wherever a
//! technical rating reads a player, it reads that player's rating in DB (our
//! `player_ratings.theta` at the last release before the week), carried onto the
//! chgk.gg scale by matching quantiles against this week's chgk.gg players.
use super::gg_load;
use super::{open_read_only, ranks_desc, Args, Ids, PlayerRow, Release, Fallible, Rival, Schedule, TeamRow, WEEK};
use crate::output::Writer;
use crate::team_model::TeamEvent;
use crate::util::{format_date, parse_date, spearman};
use rusqlite::Connection;
use std::collections::HashMap;

/// A.4.8: the weekly decay of a player's bonus, and how many bonuses count.
pub(super) const BONUS_DECAY: f64 = 0.99;
const N_BEST: usize = 7;
const MAX_BONUS: f64 = 2300.0;
/// A.4.4.3: how many teams below a team its predicted bonus reads, each half the last.
const N_BP: usize = 15;
const BP_BASE: f64 = 2.0;
const NEW_TEAM_COEFF: f64 = 0.8;
const TOP_TEAMS_Q: usize = 100;
const MIN_BASE_FOR_Q: usize = 6;
const D2_MUL: f64 = 300.0;
const D2_DEN: f64 = 350.0;
const D1_NEG: f64 = 0.5;
const MIN_TR_RATIO: f64 = 0.5;
const MIN_LEGS: u32 = 3;
/// A.4.7: a base this large always inherits, and before 2021-08-28 one smaller
/// by one did when the name was kept.
const MIN_BASE_OLD: u32 = 4;
const MIN_BASE: u32 = 3;
/// A team with three or more legionnaires splits its bonus as 2 / n_legs.
const LEG_SPLIT: f64 = 2.0;
const TR_PLAYERS: usize = 6;
const BOOTSTRAP: &str = "2021-09-09";
/// A.3.2: the season a release looks back to, and how long into a season the
/// previous season's teams stay listed.
const PREV_SEASON_LOOKBACK: i32 = 180;
const PREV_SEASON_GRACE: i32 = 90;
const HALF: f64 = 0.5;
const EPS: f64 = 1e-12;
const CHECKPOINT_EVERY: i64 = 26;

/// Postgres's numeric -> integer cast: half away from zero.
fn round_half_up(x: f64) -> f64 {
    if x < 0.0 {
        -(-x + HALF).floor()
    } else {
        (x + HALF).floor()
    }
}

/// numpy's and Python's round: half to even.
pub(super) fn round_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// A.4.9.1
fn tech_rating(ratings: &mut [f64], q: f64) -> f64 {
    ratings.sort_by(|a, b| b.total_cmp(a));
    let mut s = 0.0;
    for (k, v) in ratings.iter().take(TR_PLAYERS).enumerate() {
        s += v * (TR_PLAYERS - k) as f64 / TR_PLAYERS as f64;
    }
    round_even(s) * q
}

/// The value at `t` on the piecewise-linear map from sorted `th` to sorted `pts`.
fn interpolate(th: &[f64], pts: &[f64], t: f64) -> f64 {
    let n = th.len();
    let i = th.partition_point(|&x| x < t);
    if i == 0 {
        return pts[0];
    }
    if i >= n {
        return pts[n - 1];
    }
    let f = (t - th[i - 1]) / (th[i] - th[i - 1]).max(EPS);
    pts[i - 1] + f.clamp(0.0, 1.0) * (pts[i] - pts[i - 1])
}

/// Our player ratings on the chgk.gg scale. `ours` holds (player index, theta)
/// at one release; the map from theta to points matches the quantiles of the
/// players rated by both, and anyone we rate gets a number, anyone we don't
/// keeps chgk.gg's.
fn swap_players(gg: &[f64], ours: &[(u32, f64)]) -> Vec<f64> {
    let mut out = gg.to_vec();
    let (mut th, mut pts): (Vec<f64>, Vec<f64>) = (ours.iter())
        .filter(|&&(p, _)| gg.get(p as usize).is_some_and(|&r| r > 0.0))
        .map(|&(p, t)| (t, gg[p as usize]))
        .unzip();
    if th.len() < 2 {
        return out;
    }
    th.sort_by(|a, b| a.total_cmp(b));
    pts.sort_by(|a, b| a.total_cmp(b));
    for &(p, t) in ours {
        if let Some(o) = out.get_mut(p as usize) {
            *o = interpolate(&th, &pts, t);
        }
    }
    out
}

/// A.4.4.3
fn bonus_predictions(rg_desc: &[f64], c: f64) -> Vec<f64> {
    let n = rg_desc.len();
    let mut out = vec![0.0; n];
    for (i, o) in out.iter_mut().enumerate() {
        let mut s = 0.0;
        for k in 0..N_BP.min(n - i) {
            s += rg_desc[i + k] * BP_BASE.powi(-(k as i32));
        }
        *o = round_even(s * c);
    }
    for i in 1..n {
        if rg_desc[i] == rg_desc[i - 1] {
            out[i] = out[i - 1];
        }
    }
    out
}

/// A.4.5
fn score_real(pred: &[f64], positions: &[f64]) -> Vec<f64> {
    let mut shared: HashMap<u64, usize> = HashMap::new();
    for p in positions {
        *shared.entry(p.to_bits()).or_insert(0) += 1;
    }
    let real = |p: &f64| {
        let pos = p - 1.0;
        let n = shared[&p.to_bits()] as f64;
        let lo = ((pos - (n - 1.0) * HALF) as i64).max(0) as usize;
        let hi = ((pos + (n - 1.0) * HALF) as i64).max(0) as usize + 1;
        let s = &pred[lo.min(pred.len())..hi.min(pred.len())];
        if s.is_empty() {
            0.0
        } else {
            round_even(s.iter().sum::<f64>() / s.len() as f64)
        }
    };
    positions.iter().map(real).collect()
}

/// A.4.7, in the three forms it has had.
pub(super) fn heredity(start: i32, rules: (i32, i32), n_base: u32, n_legs: u32, name_kept: bool) -> bool {
    if start < rules.0 {
        n_base >= MIN_BASE_OLD || (n_base == MIN_BASE && name_kept)
    } else if start < rules.1 {
        n_base >= MIN_BASE_OLD
    } else {
        n_base >= MIN_BASE && n_legs < n_base && n_legs <= MIN_LEGS
    }
}

pub(super) struct Res {
    pub team: u32,
    pub position: f64,
    pub members: Vec<u32>,
    pub base: Vec<u32>,
    pub n_legs: u32,
    pub heredity: bool,
    pub pred_pos: Option<f64>,
}

pub(super) struct Trn {
    pub id: i64,
    pub coeff: f64,
    pub maii: bool,
    pub end: i32,
    pub kind: String,
    pub teams: Vec<Res>,
}

pub(super) struct Bonus {
    pub tournament: i64,
    pub initial: f64,
    pub weeks: i32,
    pub cur: f64,
    pub raw: f64,
}

impl Bonus {
    fn new(tournament: i64, b: f64) -> Bonus {
        Bonus { tournament, initial: b, weeks: 0, cur: b, raw: b }
    }

    fn age(&mut self) {
        self.weeks += 1;
        self.raw = self.initial * BONUS_DECAY.powi(self.weeks);
        self.cur = round_even(self.raw);
    }
}

#[derive(Default)]
pub(super) struct Player {
    pub rating: f64,
    pub bonuses: Vec<Bonus>,
}

impl Player {
    /// A.4.8: the best seven bonuses, as they stand this week.
    fn settle(&mut self) {
        self.bonuses.sort_by(|a, b| b.raw.total_cmp(&a.raw).then(a.tournament.cmp(&b.tournament)));
        self.bonuses.truncate(N_BEST);
        self.rating = self.bonuses.iter().map(|b| b.cur).sum();
        if self.rating == 0.0 {
            self.bonuses.clear();
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Team {
    pub rating: f64,
    pub trb: f64,
}

pub(super) struct Roster {
    pub team: u32,
    pub player: u32,
    pub season: i32,
    pub start: Option<i32>,
    pub end: Option<i32>,
}

/// One tournament scored against the state at the start of its week: per team,
/// the A.4 quantities in the order the clauses define them.
struct Scored {
    rt: Vec<f64>,
    r: Vec<f64>,
    rb: Vec<f64>,
    rg: Vec<f64>,
    mp: Vec<f64>,
    bp: Vec<f64>,
    b: Vec<f64>,
    d1: Vec<f64>,
    d2: Vec<f64>,
    bonus: Vec<f64>,
}

/// A.4.4: the technical rating of the lineup, the inherited rating and base
/// technical rating, and the rating the prediction reads.
fn team_inputs(res: &Res, team: Team, pr: &[f64], q: f64) -> (f64, f64, f64, f64) {
    let mut rs: Vec<f64> = res.members.iter().map(|&p| pr[p as usize]).collect();
    let rt = tech_rating(&mut rs, q);
    let r = if res.heredity { team.rating } else { 0.0 };
    let rb = if res.heredity { team.trb } else { 0.0 };
    let g = if rb != 0.0 { r * rt / rb } else { rt };
    let rg = if rt < rb { g.max(MIN_TR_RATIO * r) } else { g.min(r.max(rt)) };
    (rt, r, rb, rg)
}

/// A.4.4.3 and A.4.5 in the order of `rg`, put back in the order of the results.
fn predicted_and_real(t: &Trn, rg: &[f64], c: f64) -> (Vec<f64>, Vec<f64>) {
    let n = rg.len();
    let mut ord: Vec<usize> = (0..n).collect();
    ord.sort_by(|&a, &b| rg[b].total_cmp(&rg[a]));
    let rg_desc: Vec<f64> = ord.iter().map(|&i| rg[i]).collect();
    let pos_desc: Vec<f64> = ord.iter().map(|&i| t.teams[i].position).collect();
    let bp_desc = bonus_predictions(&rg_desc, c);
    let b_desc = score_real(&bp_desc, &pos_desc);
    let (mut bp, mut b) = (vec![0.0; n], vec![0.0; n]);
    for (j, &i) in ord.iter().enumerate() {
        bp[i] = bp_desc[j];
        b[i] = b_desc[j];
    }
    (bp, b)
}

/// A.4.6: the two parts of the bonus, and the bonus.
fn bonus_of(t: &Trn, res: &Res, b: f64, bp: f64) -> (f64, f64, f64) {
    let mut d1 = b - bp;
    if d1 < 0.0 {
        d1 *= D1_NEG;
    }
    let d2 = D2_MUL * ((b - MAX_BONUS) / D2_DEN).exp();
    let mut bonus = t.coeff * (d1 + d2);
    if res.heredity && res.n_legs >= MIN_LEGS {
        bonus *= LEG_SPLIT / res.n_legs as f64;
    }
    (d1, d2, bonus)
}

fn score(t: &Trn, teams: &[Option<Team>], pr: &[f64], q: f64, c: f64) -> Scored {
    let (mut rt, mut r, mut rb, mut rg) = (vec![], vec![], vec![], vec![]);
    for res in &t.teams {
        let x = team_inputs(res, teams[res.team as usize].unwrap(), pr, q);
        rt.push(x.0);
        r.push(x.1);
        rb.push(x.2);
        rg.push(x.3);
    }
    let mp = ranks_desc(&rg);
    let (bp, b) = predicted_and_real(t, &rg, c);
    let (mut d1, mut d2, mut bonus) = (vec![], vec![], vec![]);
    for (k, res) in t.teams.iter().enumerate() {
        let x = bonus_of(t, res, b[k], bp[k]);
        d1.push(x.0);
        d2.push(x.1);
        bonus.push(x.2);
    }
    Scored { rt, r, rb, rg, mp, bp, b, d1, d2, bonus }
}

fn event(t: &Trn, s: &Scored, j: usize, res: &Res) -> TeamEvent {
    let moved = if res.heredity && t.maii { s.bonus[j] } else { 0.0 };
    TeamEvent {
        c: t.coeff,
        delta_l: s.d1[j],
        x: s.rt[j],
        forecast: s.rg[j],
        forecast_place: s.mp[j],
        expected_place: s.mp[j],
        actual_place: res.position,
        pred_place: res.pred_pos,
        s_r: s.b[j],
        s_pair: s.d2[j],
        i_pair: s.bp[j],
        mu_before: s.r[j],
        mu_after: s.r[j] + moved,
        sigma_before: s.rb[j],
        evidence: s.bonus[j],
        rated: res.heredity,
        ..super::blank_event(res.team)
    }
}

/// The seasons and their base rosters (A.2.3).
struct Seasons {
    seasons: Vec<(i32, i32, i32)>,
    rosters: Vec<Roster>,
    by_season: HashMap<i32, Vec<usize>>,
}

impl Seasons {
    fn at(&self, d: i32) -> i32 {
        self.seasons.iter().find(|s| s.1 <= d && d <= s.2).map(|s| s.0).unwrap_or(0)
    }

    fn start(&self, id: i32) -> i32 {
        self.seasons.iter().find(|s| s.0 == id).map(|s| s.1).unwrap_or(0)
    }

    fn rosters(&self, s: i32) -> impl Iterator<Item = &Roster> {
        self.by_season.get(&s).map(|v| v.as_slice()).unwrap_or(&[]).iter().map(|&i| &self.rosters[i])
    }
}

/// What one release week reads: everything is computed off the state as it stood
/// at the previous release, and only `publish` moves it.
struct Week {
    day: i32,
    release_id: i64,
    season: i32,
    sstart: i32,
    q: f64,
    c: f64,
    pr: Vec<f64>,
    scored: Vec<(usize, Scored)>,
}

pub struct Gg {
    conn: Connection,
    tournaments: Vec<Trn>,
    seasons: Seasons,
    players: Vec<Player>,
    teams: Vec<Option<Team>>,
    team_ids: Ids,
    player_ids: Ids,
    ours: Vec<(i32, Vec<(u32, f64)>)>,
    boot_day: i32,
    boot_release: i64,
    to_day: i32,
    rows_every: i64,
    out: String,
    week: Option<Week>,
}

impl Gg {
    pub fn open(args: &Args) -> Fallible<Gg> {
        let ext = args.or("--ext", "data/external/chgk_gg");
        let conn = open_read_only(&args.or("--db", "buff.db"))?;
        let (mut team_ids, mut player_ids) = (Ids::default(), Ids::default());
        let tournaments = gg_load::all_tournaments(&conn, &ext, &mut team_ids, &mut player_ids)?;
        let seasons = Self::seasons(&conn, &ext, &mut team_ids, &mut player_ids)?;
        let mut players = gg_load::bootstrap_players(&ext, &mut player_ids)?;
        let boot_day = parse_date(BOOTSTRAP).unwrap();
        let ours = match args.get("--players") {
            Some(path) => gg_load::ours(&path, boot_day, &mut player_ids)?,
            None => Vec::new(),
        };
        let boot_release: i64 = conn.query_row("SELECT id FROM releases WHERE date = ?1", [BOOTSTRAP], |r| r.get(0))?;
        let mut teams = gg_load::bootstrap_teams(&conn, boot_release, &mut team_ids)?;
        teams.resize(teams.len().max(team_ids.len()), None);
        players.resize_with(players.len().max(player_ids.len()), Player::default);
        let to_day = parse_date(&args.or("--to", "2026-09-10")).ok_or("bad --to")?;
        let rows_every = args.get("--rows-every").and_then(|s| s.parse().ok()).unwrap_or(1);
        let out = args.or("--out", "out/ext/gg.db");
        let mut gg = Gg { conn, tournaments, seasons, players, teams, team_ids, player_ids, ours, boot_day, boot_release, to_day, rows_every, out, week: None };
        gg.calendar();
        Ok(gg)
    }

    fn seasons(conn: &Connection, ext: &str, teams: &mut Ids, players: &mut Ids) -> Fallible<Seasons> {
        let rosters = gg_load::rosters(ext, teams, players)?;
        let mut by_season: HashMap<i32, Vec<usize>> = HashMap::new();
        for (i, r) in rosters.iter().enumerate() {
            by_season.entry(r.season).or_default().push(i);
        }
        Ok(Seasons { seasons: gg_load::seasons(conn)?, rosters, by_season })
    }

    /// Keep the tournaments some release covers, a week's in the order of their ids.
    fn calendar(&mut self) {
        let last = self.boot_day + WEEK * (self.to_day - self.boot_day).div_euclid(WEEK);
        let boot = self.boot_day;
        self.tournaments.retain(|t| t.end > boot && t.end <= last);
        self.tournaments.sort_by_key(|t| ((t.end - boot - 1).div_euclid(WEEK), t.id));
        eprintln!(
            "{} tournaments, {} roster rows; bootstrap {BOOTSTRAP}: {} teams, {} players",
            self.tournaments.len(),
            self.seasons.rosters.len(),
            self.teams.iter().filter(|t| t.is_some()).count(),
            self.players.iter().filter(|p| p.rating > 0.0).count()
        );
    }

    fn release_id(&self, r: &Release) -> i64 {
        let mut st = self.conn.prepare_cached("SELECT id FROM releases WHERE date = ?1").unwrap();
        st.query_row([format_date(r.day)], |row| row.get(0)).unwrap_or(self.boot_release + r.id)
    }

    /// The player ratings every technical rating of this week reads.
    fn week_players(&self, prev_day: i32) -> Vec<f64> {
        let gg: Vec<f64> = self.players.iter().map(|p| p.rating).collect();
        match self.ours.iter().rev().find(|o| o.0 <= prev_day) {
            Some(o) => swap_players(&gg, &o.1),
            None => gg,
        }
    }

    /// A.4.4.3's C, off the release table as published, before this week's changes.
    fn bonus_scale(&self) -> f64 {
        let mut all: Vec<f64> = self.teams.iter().flatten().map(|t| t.rating).collect();
        all.sort_by(|a, b| b.total_cmp(a));
        let top: f64 = (all.iter().take(N_BP).enumerate()).map(|(k, v)| v * BP_BASE.powi(-(k as i32))).sum();
        MAX_BONUS / top
    }

    /// A.2.3: each team's base players with a rating, on `day`.
    fn base_ratings(&self, season: i32, day: i32, pr: &[f64]) -> HashMap<u32, Vec<f64>> {
        let mut base_team: Vec<Option<u32>> = vec![None; self.players.len()];
        for r in self.seasons.rosters(season) {
            if r.start.is_some_and(|s| s <= day) && r.end.is_none_or(|e| e > day) {
                base_team[r.player as usize] = Some(r.team);
            }
        }
        let mut by_base: HashMap<u32, Vec<f64>> = HashMap::new();
        for (p, bt) in base_team.iter().enumerate() {
            if let (Some(t), true) = (bt, pr[p] > 0.0) {
                by_base.entry(*t).or_default().push(pr[p]);
            }
        }
        by_base
    }

    /// A.4.9.2: the mean ratio of rating to raw technical rating over the top teams.
    fn tech_scale(&self, by_base: &HashMap<u32, Vec<f64>>) -> f64 {
        let rating = |i: usize| self.teams[i].unwrap().rating;
        let mut order: Vec<usize> = (0..self.teams.len()).filter(|&i| self.teams[i].is_some()).collect();
        order.sort_by(|&a, &b| rating(b).total_cmp(&rating(a)).then(self.team_ids.list[a].cmp(&self.team_ids.list[b])));
        let (mut q_sum, mut q_n) = (0.0, 0.0);
        for &t in order.iter().take(TOP_TEAMS_Q) {
            let Some(rs) = by_base.get(&(t as u32)).filter(|rs| rs.len() >= MIN_BASE_FOR_Q) else { continue };
            let raw = tech_rating(&mut rs.clone(), 1.0);
            if raw != 0.0 {
                q_sum += rating(t) / raw;
                q_n += 1.0;
            }
        }
        q_sum / q_n
    }

    /// A.4.9 and A.4.2.3: base technical ratings, and the floor of a team whose
    /// base roster starts this week.
    fn set_base(&mut self, by_base: &HashMap<u32, Vec<f64>>, q: f64, w: &Week) {
        for t in self.teams.iter_mut().flatten() {
            t.trb = 0.0;
        }
        for (t, rs) in by_base.iter() {
            if let Some(Some(team)) = self.teams.get_mut(*t as usize) {
                team.trb = tech_rating(&mut rs.clone(), q);
            }
        }
        for r in self.seasons.rosters(w.season) {
            let eff = r.start.unwrap_or(w.sstart).max(w.sstart);
            if eff > w.day - WEEK && eff <= w.day {
                if let Some(Some(team)) = self.teams.get_mut(r.team as usize) {
                    team.rating = team.rating.max(team.trb * NEW_TEAM_COEFF);
                }
            }
        }
    }

    fn open_week(&mut self, r: &Release) {
        if self.week.is_some() {
            return;
        }
        let season = self.seasons.at(r.day);
        let pr = self.week_players(r.day - WEEK);
        let c = self.bonus_scale();
        let by_base = self.base_ratings(season, r.day, &pr);
        let q = self.tech_scale(&by_base);
        let sstart = self.seasons.start(season);
        let w = Week { day: r.day, release_id: self.release_id(r), season, sstart, q, c, pr, scored: Vec::new() };
        self.set_base(&by_base, q, &w);
        self.week = Some(w);
    }

    /// A team first seen this week starts at a share of its base technical rating.
    fn add_new_teams(&mut self, i: usize, pr: &[f64], q: f64) {
        for res in self.tournaments[i].teams.iter() {
            let slot = &mut self.teams[res.team as usize];
            if slot.is_none() {
                let mut rs: Vec<f64> = res.base.iter().map(|&p| pr[p as usize]).collect();
                let trb = tech_rating(&mut rs, q);
                *slot = Some(Team { rating: trb * NEW_TEAM_COEFF, trb });
            }
        }
    }

    /// The step: everything before read the old state, this writes it. Only MAII
    /// tournaments move it. Returns the team ratings and player ratings before.
    fn apply(&mut self, scored: &[(usize, Scored)]) -> (Vec<Option<f64>>, Vec<f64>) {
        let prev_team: Vec<Option<f64>> = self.teams.iter().map(|t| t.map(|t| t.rating)).collect();
        let prev_player: Vec<f64> = self.players.iter().map(|p| p.rating).collect();
        self.players.iter_mut().flat_map(|p| p.bonuses.iter_mut()).for_each(Bonus::age);
        for (ti, s) in scored {
            if self.tournaments[*ti].maii {
                self.award(*ti, s);
            }
        }
        for t in self.teams.iter_mut().flatten() {
            t.rating = t.rating.max(0.0);
        }
        self.players.iter_mut().for_each(Player::settle);
        (prev_team, prev_player)
    }

    fn award(&mut self, ti: usize, s: &Scored) {
        let t = &self.tournaments[ti];
        for (j, res) in t.teams.iter().enumerate() {
            if res.heredity {
                if let Some(team) = self.teams[res.team as usize].as_mut() {
                    team.rating += s.bonus[j];
                }
            }
            for &p in res.members.iter() {
                self.players[p as usize].bonuses.push(Bonus::new(t.id, s.b[j]));
            }
        }
    }

    /// A.3.2. The release table is also the state the next release starts from, so
    /// a team whose base roster lapses drops out and returns at a start rating.
    fn listed(&self, w: &Week) -> Vec<bool> {
        let mut listed: Vec<bool> = vec![false; self.teams.len()];
        let prev_season = self.seasons.at(w.day - PREV_SEASON_LOOKBACK);
        let include_prev = w.sstart + PREV_SEASON_GRACE >= w.day;
        let seasons = [Some(w.season), include_prev.then_some(prev_season)];
        for r in seasons.iter().flatten().flat_map(|&s| self.seasons.rosters(s)) {
            listed[r.team as usize] = true;
        }
        listed
    }

    fn team_rows(&self, listed: &[bool], prev: &[Option<f64>]) -> Vec<TeamRow> {
        let row = |i: usize, t: &Team| {
            let change = match prev[i] {
                Some(p) if p != 0.0 => round_half_up(t.rating - p),
                _ => 0.0,
            };
            (self.team_ids.list[i], round_half_up(t.rating), 0.0, round_half_up(t.trb), 0.0, change)
        };
        (self.teams.iter().enumerate())
            .filter_map(|(i, t)| t.as_ref().filter(|_| listed[i]).map(|t| row(i, t)))
            .collect()
    }

    fn player_rows(&self, prev: &[f64]) -> Vec<PlayerRow> {
        let row = |p: usize, pl: &Player| {
            let change = if prev[p] != 0.0 { pl.rating - prev[p] } else { 0.0 };
            (self.player_ids.list[p], pl.rating, 0.0, pl.bonuses.len() as u32, change)
        };
        (self.players.iter().enumerate()).filter(|(_, pl)| pl.rating > 0.0).map(|(p, pl)| row(p, pl)).collect()
    }

    fn write_tournament(&self, w: &Writer, ti: usize, s: &Scored) -> rusqlite::Result<()> {
        let t = &self.tournaments[ti];
        let evs: Vec<TeamEvent> = t.teams.iter().enumerate().map(|(j, res)| event(t, s, j, res)).collect();
        w.team_events(t.id, &format_date(t.end), &self.team_ids.list, &evs)?;
        let actual: Vec<f64> = t.teams.iter().map(|r| r.position).collect();
        let (theirs, act2): (Vec<f64>, Vec<f64>) =
            t.teams.iter().filter_map(|r| r.pred_pos.map(|p| (p, r.position))).unzip();
        w.tournament_metrics(
            t.id, &format_date(t.end), &t.kind, t.coeff, t.teams.len(), 0, 0,
            None, None, spearman(&s.mp, &actual), spearman(&theirs, &act2), None,
        )
    }

    /// Unlisted teams leave; the rest carry their rounded rating into next week.
    fn purge(&mut self, listed: &[bool]) {
        for (i, t) in self.teams.iter_mut().enumerate() {
            match t {
                Some(_) if !listed[i] => *t = None,
                Some(t) => t.rating = round_half_up(t.rating),
                None => {}
            }
        }
    }

    fn write_release(&self, w: &Writer, wk: &Week, listed: &[bool], prev: &(Vec<Option<f64>>, Vec<f64>)) -> rusqlite::Result<()> {
        let trows = self.team_rows(listed, &prev.0);
        let prows = self.player_rows(&prev.1);
        w.release(wk.release_id, &format_date(wk.day), 0.0, wk.q, prows.len())?;
        w.player_ratings(wk.release_id, &prows, |t| t)?;
        w.team_ratings(wk.release_id, &trows, |m| m)
    }
}

impl Rival for Gg {
    fn schedule(&self) -> Schedule {
        Schedule { first_day: self.boot_day + WEEK, last_day: Some(self.to_day), checkpoint_every: CHECKPOINT_EVERY }
    }

    fn out_path(&self) -> &str {
        &self.out
    }

    fn n_tournaments(&self) -> usize {
        self.tournaments.len()
    }

    fn after(&self, i: usize, day: i32) -> bool {
        self.tournaments[i].end > day
    }

    /// Every tournament of the week is scored against the state as it stood at the
    /// release; new teams join it as they are met.
    fn rate(&mut self, _w: &Writer, i: usize, next: &Release) -> Fallible<()> {
        self.open_week(next);
        let mut wk = self.week.take().unwrap();
        self.add_new_teams(i, &wk.pr, wk.q);
        wk.scored.push((i, score(&self.tournaments[i], &self.teams, &wk.pr, wk.q, wk.c)));
        self.week = Some(wk);
        Ok(())
    }

    fn publish(&mut self, w: &Writer, r: &Release) -> Fallible<()> {
        self.open_week(r);
        let wk = self.week.take().unwrap();
        let prev = self.apply(&wk.scored);
        let listed = self.listed(&wk);
        if (self.rows_every > 0 && wk.release_id % self.rows_every == 0) || r.last {
            self.write_release(w, &wk, &listed, &prev)?;
        }
        for (ti, s) in &wk.scored {
            self.write_tournament(w, *ti, s)?;
        }
        self.purge(&listed);
        if r.id % CHECKPOINT_EVERY == 0 {
            let live = self.teams.iter().filter(|t| t.is_some()).count();
            let players = self.players.iter().filter(|p| p.rating > 0.0).count();
            eprintln!("{} q={:.5} teams={live} players={players}", format_date(r.day), wk.q);
        }
        Ok(())
    }
}
