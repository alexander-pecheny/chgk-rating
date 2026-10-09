//! rating.chgk.fun (A. Sirotkin) reimplemented from `AVSirotkin/ChGK`
//! `Ratings/rating.py`, and written to the DESIGN 6 schema so `eval` and
//! `tools/ndcg.py` read it unchanged. `out/ext/fun/report.md` is the method in
//! prose and the fidelity check against the site's own API.
use super::{
    bare_player_event, blank_event, open_read_only, parse_ids, ranks_desc, Args, Changes, Ids,
    PlayerRow, Release, Fallible, Rival, Schedule, TeamRow, WEEK,
};
use crate::data::Format;
use crate::output::{PlayerEvent, TournamentMetrics, Writer};
use crate::team_event::TeamEvent;
use crate::util::{format_date, parse_date, spearman};
use rayon::prelude::*;
use std::collections::HashMap;

const MIN_Q: f64 = 0.0;
const MAX_Q: f64 = 10000.0;
const DELTA_MULTIPLIER: f64 = 3.0;
const NON_RATE_DELTA_MULTIPLIER: f64 = 0.1;
const INDEPENDENT_SKILL_QUESTION: f64 = 2000.0;
const PLAYER_START_RATING: f64 = 1000.0;
const ELO_NORM: f64 = 400.0;
const ELO_BASE: f64 = 10.0;
const EPS: f64 = 0.00001;
/// Upstream's search: first step as a share of the start, the factor a step
/// shrinks by on every turn, the cap on iterations, and where every search starts.
const FIRST_STEP: f64 = 0.34;
const STEP_SHRINK: f64 = 2.0;
const MAX_STEPS: u32 = 10000;
const SEARCH_START: f64 = 1000.0;
/// A team plays as its six strongest; a larger roster splits the delta.
const TEAM_TOP: usize = 6;
/// The newcomer boost by rated games played: below each bound, the multiplier.
const BOOST: [(i64, f64); 3] = [(10, 10.0), (20, 7.0), (30, 5.0)];
const BOOST_VETERAN: f64 = 3.0;
/// The position the mirror gives a team that did not finish.
const NOT_PLACED: f64 = 9999.0;
/// Upstream's first release-week, and how far past its run day it reaches.
const EPOCH: &str = "2021-09-02";
const LOOKAHEAD_WEEKS: i32 = 2;
const DEFAULT_AS_OF: &str = "2026-09-04";
const CHECKPOINT_EVERY: i64 = 10;
/// The system's own scope: everything rating.chgk.fun ingests, which is wider
/// than `data::load`: no `in_rating` filter, asynchronous tournaments included.
const SCOPE: &str = "SELECT t.id, t.date_end, t.tournament_type, t.maii_rating, t.questions_by_tour,
            r.team_id, r.position, r.questions_total, r.mask, r.team_members
     FROM tournaments t JOIN tournament_results r ON r.id = t.id
     WHERE t.date_start > '2021-09-01'
       AND t.date_end > '2021-09-03' AND t.date_end < ?1
     ORDER BY t.date_end, t.id";
const SEATS: &str = "SELECT ts.team_id, ts.player_id, s.date_start, s.date_end, ts.date_added, ts.date_removed
     FROM team_seasons ts JOIN seasons s ON s.id = ts.season_id";

fn elo(r: f64, q: f64) -> f64 {
    1.0 / (1.0 + ELO_BASE.powf((q - r) / ELO_NORM))
}

fn independent_elo(rates: &[f64]) -> f64 {
    let mut p: f64 = 1.0;
    for &r in rates {
        p *= elo(INDEPENDENT_SKILL_QUESTION, r);
    }
    (1.0 / p - 1.0).ln() / ELO_BASE.ln() * ELO_NORM + INDEPENDENT_SKILL_QUESTION
}

fn elo_estimate(r: f64, rates: &[f64]) -> f64 {
    rates.iter().map(|&q| elo(r, q)).sum()
}

/// Upstream's search, kept step for step: its stopping tolerance is 1e-5 points,
/// which is the floor on how closely any reimplementation can agree with it.
fn max_like(r: f64, rates: &[f64], gets: f64, is_team: bool) -> f64 {
    let n = rates.len() as f64;
    let result = if is_team { gets } else { n - gets };
    if result == n {
        return MAX_Q;
    }
    if result == 0.0 {
        return MIN_Q;
    }
    search(r, rates, result)
}

fn search(mut r: f64, rates: &[f64], result: f64) -> f64 {
    let mut step = r * FIRST_STEP;
    let mut up = false;
    let mut left = MAX_STEPS;
    while step > EPS && left > 0 {
        left -= 1;
        // Upstream turns on `>`, so a tie keeps climbing.
        let go_down = elo_estimate(r, rates) > result;
        if go_down == up {
            step /= STEP_SHRINK;
        }
        up = !go_down;
        r += if up { step } else { -step };
        if r > MAX_Q || r < MIN_Q {
            return r.clamp(MIN_Q, MAX_Q);
        }
    }
    r
}

/// The player multiplier of a rated tournament, by games played before it.
fn boost(games: i64) -> f64 {
    BOOST.iter().find(|b| games < b.0).map_or(BOOST_VETERAN, |b| b.1)
}

fn top_rates(mut rs: Vec<f64>) -> f64 {
    if rs.len() > TEAM_TOP {
        rs.sort_by(f64::total_cmp);
        rs.drain(..rs.len() - TEAM_TOP);
    }
    independent_elo(&rs)
}

struct Placing {
    team: usize,
    position: f64,
    total: Option<i64>,
    mask: Option<Vec<u8>>,
    members: Vec<usize>,
}

struct Tour {
    id: i64,
    end: String,
    day: i32,
    kind: &'static str,
    rated: bool,
    n_declared: usize,
    res: Vec<Placing>,
}

struct Seat {
    team: i64,
    player: usize,
    from: i32,
    to: i32,
}

fn new_tour(row: &rusqlite::Row) -> rusqlite::Result<Tour> {
    let end: String = row.get("date_end")?;
    let declared: usize = row
        .get::<_, Option<String>>("questions_by_tour")?
        .map(|s| s.split(',').filter_map(|x| x.trim().parse::<usize>().ok()).sum())
        .unwrap_or(0);
    let kind = row.get::<_, Option<String>>("tournament_type")?;
    Ok(Tour {
        id: row.get("id")?,
        day: parse_date(&end).unwrap_or(0),
        end,
        kind: kind.as_deref().and_then(Format::parse).map_or("other", Format::name),
        rated: row.get::<_, Option<i64>>("maii_rating")?.unwrap_or(0) != 0,
        n_declared: declared,
        res: Vec::new(),
    })
}

fn new_res(row: &rusqlite::Row, teams: &mut Ids, players: &mut Ids) -> rusqlite::Result<Placing> {
    let roster = row.get::<_, Option<String>>("team_members")?.unwrap_or_default();
    Ok(Placing {
        team: teams.get(row.get("team_id")?) as usize,
        position: row.get::<_, Option<f64>>("position")?.unwrap_or(0.0),
        total: row.get::<_, Option<String>>("questions_total")?.and_then(|s| s.trim().parse().ok()),
        mask: row.get::<_, Option<String>>("mask")?.map(|s| s.into_bytes()),
        members: parse_ids(&roster).into_iter().map(|p| players.get(p) as usize).collect(),
    })
}

fn load(db: &str, max_end: &str, teams: &mut Ids, players: &mut Ids) -> rusqlite::Result<Vec<Tour>> {
    let conn = open_read_only(db)?;
    let mut stmt = conn.prepare(SCOPE)?;
    let mut rows = stmt.query([max_end])?;
    let mut tours: Vec<Tour> = Vec::new();
    while let Some(row) = rows.next()? {
        if tours.last().map(|t| t.id) != Some(row.get("id")?) {
            tours.push(new_tour(row)?);
        }
        let res = new_res(row, teams, players)?;
        tours.last_mut().unwrap().res.push(res);
    }
    Ok(tours)
}

fn seat(r: &rusqlite::Row, player: usize) -> rusqlite::Result<Seat> {
    let day = |v: Option<String>| v.as_deref().and_then(parse_date);
    let (s0, s1) = (day(r.get("date_start")?), day(r.get("date_end")?));
    let (a, b) = (day(r.get("date_added")?), day(r.get("date_removed")?));
    Ok(Seat {
        team: r.get("team_id")?,
        player,
        from: a.unwrap_or(i32::MIN / 2).max(s0.unwrap_or(i32::MIN / 2)),
        to: b.unwrap_or(i32::MAX / 2).min(s1.unwrap_or(i32::MAX / 2)),
    })
}

/// Base rosters of the players this scope knows, grouped by team.
fn base_rosters(db: &str, players: &Ids) -> rusqlite::Result<Vec<Seat>> {
    let conn = open_read_only(db)?;
    let mut stmt = conn.prepare(SEATS)?;
    let mut out = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        if let Some(p) = players.find(r.get("player_id")?) {
            out.push(seat(r, p as usize)?);
        }
    }
    out.sort_by_key(|s| s.team);
    Ok(out)
}

struct State {
    pr: Vec<f64>,
    has: Vec<bool>,
    games: Vec<i64>,
    prev: Changes,
    delta: Vec<f64>,
    delta_rated: Vec<i64>,
    touched: Vec<usize>,
    dirty: Vec<bool>,
    team_prev: HashMap<i64, f64>,
}

impl State {
    fn new(n: usize) -> State {
        State {
            pr: vec![PLAYER_START_RATING; n],
            has: vec![false; n],
            games: vec![0; n],
            prev: Changes::new(n),
            delta: vec![0.0; n],
            delta_rated: vec![0; n],
            touched: Vec::new(),
            dirty: vec![false; n],
            team_prev: HashMap::new(),
        }
    }

    /// Deltas wait for the release, as upstream applies them weekly.
    fn add(&mut self, p: usize, d: f64, rated: bool) {
        if !self.dirty[p] {
            self.dirty[p] = true;
            self.touched.push(p);
        }
        self.delta[p] += d;
        self.delta_rated[p] += rated as i64;
    }

    /// Apply the week's deltas; returns the players who appear for the first time.
    fn settle(&mut self) -> Vec<usize> {
        let mut newcomers = Vec::new();
        for p in std::mem::take(&mut self.touched) {
            if !self.has[p] {
                self.has[p] = true;
                self.pr[p] = PLAYER_START_RATING;
                newcomers.push(p);
            }
            self.pr[p] += self.delta[p];
            self.games[p] += self.delta_rated[p];
            self.delta[p] = 0.0;
            self.delta_rated[p] = 0;
            self.dirty[p] = false;
        }
        newcomers
    }

    fn player_rows(&mut self, ids: &[i64]) -> Vec<PlayerRow> {
        let live: Vec<usize> = (0..self.pr.len()).filter(|&p| self.has[p]).collect();
        live.iter()
            .map(|&p| {
                let change = self.prev.step(p, self.pr[p]);
                (ids[p], self.pr[p], 0.0, self.games[p] as u32, change)
            })
            .collect()
    }

    /// A team's rating at a release: its base roster's top six on that day.
    fn team_row(&mut self, seats: &[Seat], day: i32) -> Option<TeamRow> {
        let mut rates: Vec<f64> = seats
            .iter()
            .filter(|s| s.from <= day && day < s.to && self.has[s.player])
            .map(|s| self.pr[s.player])
            .collect();
        if rates.is_empty() {
            return None;
        }
        rates.sort_by(f64::total_cmp);
        let n = rates.len();
        let mu = independent_elo(&rates[n.saturating_sub(TEAM_TOP)..]);
        let team = seats[0].team;
        let change = self.team_prev.get(&team).map_or(0.0, |p| mu - p);
        self.team_prev.insert(team, mu);
        Some((team, mu, 0.0, n as f64, 0.0, change))
    }

    fn team_rows(&mut self, seats: &[Seat], day: i32) -> Vec<TeamRow> {
        seats
            .chunk_by(|a, b| a.team == b.team)
            .filter_map(|g| self.team_row(g, day))
            .collect()
    }
}

/// The lineups of one tournament: each team once, its members merged, and its
/// strength from the ratings it came with.
struct Lineups {
    local: HashMap<usize, usize>,
    order: Vec<usize>,
    members: Vec<Vec<usize>>,
    rating: Vec<f64>,
}

impl Lineups {
    fn gather(t: &Tour, pr: &[f64]) -> Lineups {
        let mut local: HashMap<usize, usize> = HashMap::new();
        let (mut order, mut members) = (Vec::new(), Vec::<Vec<usize>>::new());
        for r in t.res.iter().filter(|r| !r.members.is_empty()) {
            let n = order.len();
            let l = *local.entry(r.team).or_insert_with(|| {
                order.push(r.team);
                members.push(Vec::new());
                n
            });
            members[l].extend_from_slice(&r.members);
        }
        let rating = members.iter().map(|m| top_rates(m.iter().map(|&p| pr[p]).collect())).collect();
        Lineups { local, order, members, rating }
    }

    fn len(&self) -> usize {
        self.order.len()
    }
}

/// What the field took: per lineup, and per question for a tournament with masks.
struct Takes {
    individual: bool,
    gets: Vec<Option<f64>>,
    q_gets: Vec<f64>,
    q_att: Vec<Vec<f64>>,
    total: f64,
}

impl Takes {
    fn count(t: &Tour, ls: &Lineups) -> Takes {
        let individual = t.res.iter().any(|r| r.mask.is_some());
        let mut k = Takes { individual, gets: vec![None; ls.len()], q_gets: vec![], q_att: vec![], total: 0.0 };
        for r in &t.res {
            let Some(&l) = ls.local.get(&r.team) else { continue };
            k.gets[l] = if individual {
                r.mask.as_ref().map(|m| k.mask(m, ls.rating[l]))
            } else {
                k.total_of(r)
            };
        }
        k
    }

    fn mask(&mut self, m: &[u8], rating: f64) -> f64 {
        if self.q_gets.len() < m.len() {
            self.q_gets.resize(m.len(), 0.0);
            self.q_att.resize(m.len(), Vec::new());
        }
        let mut g = 0.0;
        for (i, &c) in m.iter().enumerate() {
            if c == b'1' {
                self.q_gets[i] += 1.0;
                g += 1.0;
            }
            if matches!(c, b'1' | b'0' | b'?') {
                self.q_att[i].push(rating);
            }
        }
        g
    }

    fn total_of(&mut self, r: &Placing) -> Option<f64> {
        match r.total {
            None => None,
            Some(0) if r.position == NOT_PLACED => None,
            Some(v) => {
                self.total += v as f64;
                Some(v as f64)
            }
        }
    }

    /// Question values and the tournament's hardness, fitted against the field.
    fn questions(&self, t: &Tour, ls: &Lineups) -> (Vec<f64>, f64) {
        let field: Vec<f64> = (0..ls.len()).filter(|&l| self.gets[l].is_some()).map(|l| ls.rating[l]).collect();
        if self.individual {
            return self.per_question(&field);
        }
        if t.n_declared == 0 {
            return (Vec::new(), 0.0);
        }
        let qval = max_like(SEARCH_START, &field, self.total / t.n_declared as f64, false);
        (vec![qval; t.n_declared], qval)
    }

    fn per_question(&self, field: &[f64]) -> (Vec<f64>, f64) {
        let qn = self.q_gets.len();
        if qn == 0 {
            return (Vec::new(), MAX_Q);
        }
        let h = max_like(SEARCH_START, field, self.q_gets.iter().sum::<f64>() / qn as f64, false);
        let qv = (0..qn)
            .into_par_iter()
            .map(|i| max_like(SEARCH_START, &self.q_att[i], self.q_gets[i], false))
            .collect();
        (qv, h)
    }
}

/// Questions a team actually played: '-' is a question it never saw.
fn played(qv: &[f64], mask: &Option<Vec<u8>>) -> Vec<f64> {
    match mask {
        Some(m) => qv.iter().zip(m).filter(|(_, &c)| c != b'-').map(|(&q, _)| q).collect(),
        None => qv.to_vec(),
    }
}

/// One tournament from rating to rows.
struct Rated<'a> {
    t: &'a Tour,
    ls: Lineups,
    takes: Takes,
    qv: Vec<f64>,
    hardness: f64,
    qs_of: Vec<Vec<f64>>,
    weight: f64,
}

impl<'a> Rated<'a> {
    fn new(t: &'a Tour, pr: &[f64]) -> Rated<'a> {
        let ls = Lineups::gather(t, pr);
        let takes = Takes::count(t, &ls);
        let (qv, hardness) = takes.questions(t, &ls);
        let mut qs_of: Vec<Vec<f64>> = vec![Vec::new(); ls.len()];
        for r in &t.res {
            if let Some(&l) = ls.local.get(&r.team) {
                if qs_of[l].is_empty() {
                    qs_of[l] = played(&qv, &r.mask);
                }
            }
        }
        let weight = if t.rated { DELTA_MULTIPLIER } else { NON_RATE_DELTA_MULTIPLIER };
        Rated { t, ls, takes, qv, hardness, qs_of, weight }
    }

    /// Each player's delta, the last lineup listing them winning.
    fn player_deltas(&self, games: &[i64]) -> HashMap<usize, f64> {
        let mut out = HashMap::new();
        for l in 0..self.ls.len() {
            let Some(g) = self.takes.gets[l] else { continue };
            let d = (g - elo_estimate(self.ls.rating[l], &self.qs_of[l])) * self.weight;
            let np = self.ls.members[l].len() as f64;
            let base = if np > TEAM_TOP as f64 { TEAM_TOP as f64 / np } else { 1.0 };
            for &p in &self.ls.members[l] {
                let wl = if self.t.rated { base * boost(games[p]) / self.weight } else { base };
                out.insert(p, d * wl);
            }
        }
        out
    }

    fn events(&self) -> Vec<TeamEvent> {
        let listed: Vec<(usize, usize)> = (self.t.res.iter().enumerate())
            .filter_map(|(i, r)| self.ls.local.get(&r.team).map(|&l| (l, i)))
            .collect();
        let rows: Vec<(f64, Option<f64>)> = listed.par_iter().map(|&(l, i)| self.forecast(l, i)).collect();
        let fplace = ranks_desc(&listed.iter().map(|&(l, _)| self.ls.rating[l]).collect::<Vec<_>>());
        (listed.iter().zip(rows).zip(fplace))
            .map(|((&(l, i), (x, perf)), place)| self.event(l, i, x, perf, place))
            .collect()
    }

    fn forecast(&self, l: usize, i: usize) -> (f64, Option<f64>) {
        let qs = &self.qs_of[l];
        let perf = self.t.res[i].total.map(|q| max_like(SEARCH_START, qs, q as f64, true));
        (elo_estimate(self.ls.rating[l], qs), perf)
    }

    fn event(&self, l: usize, i: usize, x: f64, perf: Option<f64>, place: f64) -> TeamEvent {
        let rating = self.ls.rating[l];
        TeamEvent {
            x,
            forecast: rating,
            forecast_place: place,
            expected_place: place,
            actual_place: self.t.res[i].position,
            s_r: perf.unwrap_or(f64::NAN),
            i_pair: 1.0,
            mu_before: rating,
            mu_after: rating,
            evidence: self.ls.members[l].len() as f64,
            rated: self.t.rated,
            ..blank_event(self.ls.order[l] as u32)
        }
    }

    fn write(&self, w: &Writer, team_ids: &[i64], evs: &[TeamEvent], prows: &[PlayerEvent]) -> rusqlite::Result<()> {
        let date = format_date(self.t.day);
        w.team_events(self.t.id, &date, team_ids, evs)?;
        w.player_events(self.t.id, prows)?;
        w.question_params(self.t.id, &self.qv.iter().map(|&b| (b, 1.0, 1.0)).collect::<Vec<_>>())?;
        self.metrics(w, &date, evs)
    }

    fn metrics(&self, w: &Writer, date: &str, evs: &[TeamEvent]) -> rusqlite::Result<()> {
        let (fp, ap): (Vec<f64>, Vec<f64>) = (evs.iter())
            .filter(|e| e.actual_place > 0.0 && e.actual_place != NOT_PLACED)
            .map(|e| (e.forecast_place, e.actual_place))
            .unzip();
        let n_obs: u64 = self.qs_of.iter().map(|q| q.len() as u64).sum();
        let t = self.t;
        let field = TournamentMetrics::field(t.id, date, t.kind, self.weight, t.res.len(), self.qv.len());
        let sp_forecast = spearman(&fp, &ap);
        w.tournament_metrics(&TournamentMetrics { n_obs, sp_forecast, truedl: Some(self.hardness), ..field })
    }
}

pub struct Fun {
    tours: Vec<Tour>,
    seats: Vec<Seat>,
    players: Ids,
    teams: Ids,
    st: State,
    epoch: i32,
    out: String,
}

impl Fun {
    /// Upstream reaches two release-weeks past the day it is run, so `--as-of`
    /// fixes how many releases come out.
    pub fn open(args: &Args) -> Fallible<Fun> {
        let db = args.or("--db", "buff.db");
        let as_of = args.or("--as-of", DEFAULT_AS_OF);
        let epoch = parse_date(EPOCH).unwrap();
        let actual = (parse_date(&as_of).ok_or("bad --as-of")? - (epoch + 1) + WEEK - 1).div_euclid(WEEK);
        let max_end = format_date(epoch + WEEK * (actual + LOOKAHEAD_WEEKS) + 1);
        eprintln!("scope: date_end < {max_end}, releases through {}", actual + LOOKAHEAD_WEEKS);
        let (mut teams, mut players) = (Ids::default(), Ids::default());
        let tours = load(&db, &max_end, &mut teams, &mut players)?;
        let seats = base_rosters(&db, &players)?;
        eprintln!(
            "{} tournaments, {} players, {} teams, {} base-roster seats",
            tours.len(), players.len(), teams.len(), seats.len()
        );
        let st = State::new(players.len());
        Ok(Fun { tours, seats, players, teams, st, epoch, out: args.or("--out", "out/ext/fun.db") })
    }

    fn player_events(&mut self, t: &Tour, deltas: &HashMap<usize, f64>) -> Vec<PlayerEvent> {
        let mut prows = Vec::new();
        for &p in t.res.iter().flat_map(|r| &r.members) {
            let d = deltas.get(&p).copied();
            prows.push(bare_player_event(self.players.list[p], self.st.pr[p], self.st.pr[p] + d.unwrap_or(0.0)));
            if let Some(d) = d {
                self.st.add(p, d, t.rated);
            }
        }
        prows
    }
}

impl Rival for Fun {
    fn schedule(&self) -> Schedule {
        Schedule { first_day: self.epoch + WEEK, last_day: None, checkpoint_every: CHECKPOINT_EVERY }
    }

    fn out_path(&self) -> &str {
        &self.out
    }

    fn n_tournaments(&self) -> usize {
        self.tours.len()
    }

    /// Upstream cuts a week at midnight, on the timestamp as the mirror holds it.
    fn after(&self, i: usize, day: i32) -> bool {
        self.tours[i].end >= format!("{} 00:00:00", format_date(day + 1))
    }

    fn rate(&mut self, w: &Writer, i: usize, _next: &Release) -> Fallible<()> {
        let tours = std::mem::take(&mut self.tours);
        let t = &tours[i];
        let rated = Rated::new(t, &self.st.pr);
        let evs = rated.events();
        let prows = self.player_events(t, &rated.player_deltas(&self.st.games));
        rated.write(w, &self.teams.list, &evs, &prows)?;
        self.tours = tours;
        Ok(())
    }

    /// Newcomers also enter the release before at the start rating, as upstream lists them.
    fn publish(&mut self, w: &Writer, r: &Release) -> Fallible<()> {
        let ids = &self.players.list;
        let newcomers: Vec<PlayerRow> =
            self.st.settle().into_iter().map(|p| (ids[p], PLAYER_START_RATING, 0.0, 0, 0.0)).collect();
        if r.id > 1 && !newcomers.is_empty() {
            w.player_ratings(r.id - 1, &crate::output::published(&newcomers), |t| t)?;
        }
        let rows = self.st.player_rows(ids);
        w.release(r.id, &format_date(r.day), 0.0, 0.0, rows.len())?;
        w.player_ratings(r.id, &crate::output::published(&rows), |t| t)?;
        w.team_ratings(r.id, &self.st.team_rows(&self.seats, r.day), |m| m)?;
        Ok(())
    }
}
