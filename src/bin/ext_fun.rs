//! rating.chgk.fun (A. Sirotkin) reimplemented from `AVSirotkin/ChGK`
//! `Ratings/rating.py`, and written to the DESIGN 6 schema so `eval` and
//! `tools/ndcg.py` read it unchanged. `out/ext/fun/report.md` is the method in
//! prose and the fidelity check against the site's own API.
use chgk_rating::data::Format;
use chgk_rating::output::Writer;
use chgk_rating::team_model::TeamEvent;
use chgk_rating::util::{civil_from_days, days_from_civil, format_date, parse_date, spearman};
use rayon::prelude::*;
use rusqlite::Connection;
use std::collections::HashMap;

const MIN_Q: f64 = 0.0;
const MAX_Q: f64 = 10000.0;
const DELTA_MULTIPLIER: f64 = 3.0;
const NON_RATE_DELTA_MULTIPLIER: f64 = 0.1;
const INDEPENDENT_SKILL_QUESTION: f64 = 2000.0;
const PLAYER_START_RATING: f64 = 1000.0;
const ELO_NORM: f64 = 400.0;
const EPS: f64 = 0.00001;

fn elo(r: f64, q: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf((q - r) / ELO_NORM))
}

fn independent_elo(rates: &[f64]) -> f64 {
    let mut p = 1.0f64;
    for &r in rates {
        p *= elo(INDEPENDENT_SKILL_QUESTION, r);
    }
    (1.0 / p - 1.0).ln() / 10f64.ln() * ELO_NORM + INDEPENDENT_SKILL_QUESTION
}

fn elo_estimate(r: f64, rates: &[f64]) -> f64 {
    rates.iter().map(|&q| elo(r, q)).sum()
}

/// Upstream's search, kept step for step: its stopping tolerance is 1e-5 points,
/// which is the floor on how closely any reimplementation can agree with it.
fn max_like(mut r: f64, rates: &[f64], gets: f64, is_team: bool) -> f64 {
    let n = rates.len() as f64;
    let result = if is_team { gets } else { n - gets };
    if result == n {
        return MAX_Q;
    }
    if result == 0.0 {
        return MIN_Q;
    }
    let mut step = r * 0.34;
    let mut up = false;
    let mut left = 10000;
    while step > EPS && left > 0 {
        left -= 1;
        if elo_estimate(r, rates) > result {
            if up {
                step /= 2.0;
            }
            up = false;
            r -= step;
        } else {
            if !up {
                step /= 2.0;
            }
            up = true;
            r += step;
        }
        if r > MAX_Q {
            return MAX_Q;
        }
        if r < MIN_Q {
            return MIN_Q;
        }
    }
    r
}

fn avg_ranks_desc(key: &[f64]) -> Vec<f64> {
    let mut o: Vec<usize> = (0..key.len()).collect();
    o.sort_by(|&a, &b| key[b].total_cmp(&key[a]));
    let mut r = vec![0.0; key.len()];
    let mut i = 0;
    while i < o.len() {
        let mut j = i;
        while j + 1 < o.len() && key[o[j + 1]] == key[o[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for &k in &o[i..=j] {
            r[k] = avg;
        }
        i = j + 1;
    }
    r
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

struct Res {
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
    res: Vec<Res>,
}

struct Arena {
    tours: Vec<Tour>,
    player_ids: Vec<i64>,
    team_ids: Vec<i64>,
    pidx: HashMap<i64, usize>,
}

fn ids(s: &str, index: &mut HashMap<i64, usize>, all: &mut Vec<i64>, out: &mut Vec<usize>) {
    let (mut cur, mut have) = (0i64, false);
    for c in s.bytes() {
        if c.is_ascii_digit() {
            cur = cur * 10 + (c - b'0') as i64;
            have = true;
        } else if have {
            let n = all.len();
            out.push(*index.entry(cur).or_insert_with(|| {
                all.push(cur);
                n
            }));
            cur = 0;
            have = false;
        }
    }
    if have {
        let n = all.len();
        out.push(*index.entry(cur).or_insert_with(|| {
            all.push(cur);
            n
        }));
    }
}

/// The system's own scope: everything rating.chgk.fun ingests, which is wider
/// than `data::load` — no `in_rating` filter, asynchronous tournaments included.
fn load(db: &str, max_end: &str) -> rusqlite::Result<Arena> {
    let conn = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT t.id, t.date_end, t.tournament_type, t.maii_rating, t.questions_by_tour,
                r.team_id, r.position, r.questions_total, r.mask, r.team_members
         FROM tournaments t JOIN tournament_results r ON r.id = t.id
         WHERE t.date_start > '2021-09-01'
           AND t.date_end > '2021-09-03' AND t.date_end < ?1
         ORDER BY t.date_end, t.id",
    )?;
    let mut rows = stmt.query([max_end])?;
    let (mut pidx, mut tidx) = (HashMap::new(), HashMap::new());
    let (mut player_ids, mut team_ids) = (Vec::new(), Vec::new());
    let mut tours: Vec<Tour> = Vec::new();

    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        if tours.last().map(|t| t.id) != Some(id) {
            let end: String = row.get(1)?;
            let day = parse_date(&end).unwrap_or(0);
            let declared: usize = row
                .get::<_, Option<String>>(4)?
                .map(|s| s.split(',').filter_map(|x| x.trim().parse::<usize>().ok()).sum())
                .unwrap_or(0);
            tours.push(Tour {
                id,
                end,
                day,
                kind: row
                    .get::<_, Option<String>>(2)?
                    .as_deref()
                    .and_then(Format::parse)
                    .map_or("other", Format::name),
                rated: row.get::<_, Option<i64>>(3)?.unwrap_or(0) != 0,
                n_declared: declared,
                res: Vec::new(),
            });
        }
        let mut members = Vec::new();
        if let Some(m) = row.get::<_, Option<String>>(9)? {
            ids(&m, &mut pidx, &mut player_ids, &mut members);
        }
        let team_id: i64 = row.get(5)?;
        let n = team_ids.len();
        let team = *tidx.entry(team_id).or_insert_with(|| {
            team_ids.push(team_id);
            n
        });
        tours.last_mut().unwrap().res.push(Res {
            team,
            position: row.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
            total: row
                .get::<_, Option<String>>(7)?
                .and_then(|s| s.trim().parse().ok()),
            mask: row.get::<_, Option<String>>(8)?.map(|s| s.into_bytes()),
            members,
        });
    }
    Ok(Arena {
        tours,
        player_ids,
        team_ids,
        pidx,
    })
}

struct Seat {
    team: i64,
    player: usize,
    from: i32,
    to: i32,
}

fn base_rosters(db: &str, pidx: &HashMap<i64, usize>) -> rusqlite::Result<Vec<Seat>> {
    let conn = Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT ts.team_id, ts.player_id, s.date_start, s.date_end, ts.date_added, ts.date_removed
         FROM team_seasons ts JOIN seasons s ON s.id = ts.season_id",
    )?;
    let mut out = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let Some(&player) = pidx.get(&r.get::<_, i64>(1)?) else {
            continue;
        };
        let day = |v: Option<String>| v.as_deref().and_then(parse_date);
        let (s0, s1) = (day(r.get(2)?), day(r.get(3)?));
        let (a, b) = (day(r.get(4)?), day(r.get(5)?));
        out.push(Seat {
            team: r.get(0)?,
            player,
            from: a.unwrap_or(i32::MIN / 2).max(s0.unwrap_or(i32::MIN / 2)),
            to: b.unwrap_or(i32::MAX / 2).min(s1.unwrap_or(i32::MAX / 2)),
        });
    }
    out.sort_by_key(|s| s.team);
    Ok(out)
}

struct State {
    pr: Vec<f64>,
    has: Vec<bool>,
    games: Vec<i64>,
    prev: Vec<f64>,
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
            prev: vec![f64::NAN; n],
            delta: vec![0.0; n],
            delta_rated: vec![0; n],
            touched: Vec::new(),
            dirty: vec![false; n],
            team_prev: HashMap::new(),
        }
    }

    fn add(&mut self, p: usize, d: f64, rated: bool) {
        if !self.dirty[p] {
            self.dirty[p] = true;
            self.touched.push(p);
        }
        self.delta[p] += d;
        self.delta_rated[p] += rated as i64;
    }

    fn finalize(
        &mut self,
        w: &Writer,
        arena: &Arena,
        seats: &[Seat],
        release: i64,
        day: i32,
    ) -> rusqlite::Result<()> {
        let mut newcomers: Vec<(i64, f64, f64, u32, f64)> = Vec::new();
        for &p in &self.touched {
            if !self.has[p] {
                self.has[p] = true;
                self.pr[p] = PLAYER_START_RATING;
                newcomers.push((arena.player_ids[p], PLAYER_START_RATING, 0.0, 0, 0.0));
            }
            self.pr[p] += self.delta[p];
            self.games[p] += self.delta_rated[p];
            self.delta[p] = 0.0;
            self.delta_rated[p] = 0;
            self.dirty[p] = false;
        }
        self.touched.clear();
        if release > 1 && !newcomers.is_empty() {
            w.player_ratings(release - 1, &newcomers, |t| t)?;
        }

        let live: Vec<usize> = (0..self.pr.len()).filter(|&p| self.has[p]).collect();
        let rows: Vec<(i64, f64, f64, u32, f64)> = live
            .iter()
            .map(|&p| {
                let change = if self.prev[p].is_finite() {
                    self.pr[p] - self.prev[p]
                } else {
                    0.0
                };
                self.prev[p] = self.pr[p];
                (
                    arena.player_ids[p],
                    self.pr[p],
                    0.0,
                    self.games[p] as u32,
                    change,
                )
            })
            .collect();
        w.release(release, &format_date(day), 0.0, 0.0, rows.len())?;
        w.player_ratings(release, &rows, |t| t)?;

        let mut trows: Vec<(i64, f64, f64, f64, f64, f64)> = Vec::new();
        let mut i = 0;
        while i < seats.len() {
            let mut j = i;
            let mut rates = Vec::new();
            while j < seats.len() && seats[j].team == seats[i].team {
                let s = &seats[j];
                if s.from <= day && day < s.to && self.has[s.player] {
                    rates.push(self.pr[s.player]);
                }
                j += 1;
            }
            if !rates.is_empty() {
                rates.sort_by(f64::total_cmp);
                let n = rates.len();
                let mu = independent_elo(&rates[n.saturating_sub(6)..]);
                let team = seats[i].team;
                let change = self.team_prev.get(&team).map_or(0.0, |p| mu - p);
                self.team_prev.insert(team, mu);
                trows.push((team, mu, 0.0, n as f64, 0.0, change));
            }
            i = j;
        }
        w.team_ratings(release, &trows, |m| m)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = arg("--db").unwrap_or_else(|| "buff.db".into());
    let out = arg("--out").unwrap_or_else(|| "out/ext/fun.db".into());
    let as_of = arg("--as-of").unwrap_or_else(|| "2026-09-04".into());
    let t0 = std::time::Instant::now();

    // Upstream reaches two release-weeks past the day it is run, so --as-of fixes
    // how many releases come out.
    let epoch = days_from_civil(2021, 9, 2);
    let actual = (parse_date(&as_of).ok_or("bad --as-of")? - (epoch + 1) + 6).div_euclid(7);
    let (y, m, d) = civil_from_days(epoch + 7 * (actual + 2) + 1);
    let max_end = format!("{y:04}-{m:02}-{d:02}");
    eprintln!("scope: date_end < {max_end}, releases through {}", actual + 2);

    let arena = load(&db, &max_end)?;
    let seats = base_rosters(&db, &arena.pidx)?;
    eprintln!(
        "{} tournaments, {} players, {} teams, {} base-roster seats, loaded in {:.1}s",
        arena.tours.len(),
        arena.player_ids.len(),
        arena.team_ids.len(),
        seats.len(),
        t0.elapsed().as_secs_f64()
    );

    let w = Writer::create(&out)?;
    let mut st = State::new(arena.player_ids.len());
    let mut release: i64 = 1;
    let mut boundary = epoch + 8;
    let mut cut = format!("{} 00:00:00", format_date(boundary));

    for t in &arena.tours {
        while t.end >= cut {
            st.finalize(&w, &arena, &seats, release, boundary - 1)?;
            release += 1;
            boundary += 7;
            cut = format!("{} 00:00:00", format_date(boundary));
            if release % 10 == 0 {
                w.checkpoint()?;
                eprintln!("release {release} at {:.0}s", t0.elapsed().as_secs_f64());
            }
        }
        run_tournament(t, &arena, &mut st, &w)?;
    }
    st.finalize(&w, &arena, &seats, release, boundary - 1)?;
    w.finish()?;

    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    eprintln!(
        "{out}: {:.2} GB, {} releases, {:.1}s",
        size as f64 / 1e9,
        release,
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}

fn run_tournament(t: &Tour, arena: &Arena, st: &mut State, w: &Writer) -> rusqlite::Result<()> {
    let mut local: HashMap<usize, usize> = HashMap::new();
    let mut order: Vec<usize> = Vec::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    for r in &t.res {
        if r.members.is_empty() {
            continue;
        }
        let n = order.len();
        let l = *local.entry(r.team).or_insert_with(|| {
            order.push(r.team);
            members.push(Vec::new());
            n
        });
        members[l].extend_from_slice(&r.members);
    }
    let n = order.len();
    let rating: Vec<f64> = members
        .iter()
        .map(|m| {
            let mut rs: Vec<f64> = m.iter().map(|&p| st.pr[p]).collect();
            if rs.len() > 6 {
                rs.sort_by(f64::total_cmp);
                rs.drain(..rs.len() - 6);
            }
            independent_elo(&rs)
        })
        .collect();

    let individual = t.res.iter().any(|r| r.mask.is_some());
    let mut gets: Vec<Option<f64>> = vec![None; n];
    let (mut q_gets, mut q_att): (Vec<f64>, Vec<Vec<f64>>) = (Vec::new(), Vec::new());
    let mut total_gets = 0.0;
    for r in &t.res {
        let Some(&l) = local.get(&r.team) else {
            continue;
        };
        if individual {
            let Some(m) = &r.mask else {
                gets[l] = None;
                continue;
            };
            if q_gets.len() < m.len() {
                q_gets.resize(m.len(), 0.0);
                q_att.resize(m.len(), Vec::new());
            }
            let mut g = 0.0;
            for (i, &c) in m.iter().enumerate() {
                if c == b'1' {
                    q_gets[i] += 1.0;
                    g += 1.0;
                    q_att[i].push(rating[l]);
                } else if c == b'0' || c == b'?' {
                    q_att[i].push(rating[l]);
                }
            }
            gets[l] = Some(g);
        } else {
            gets[l] = match r.total {
                None => None,
                Some(0) if r.position == 9999.0 => None,
                Some(v) => {
                    total_gets += v as f64;
                    Some(v as f64)
                }
            };
        }
    }

    let field: Vec<f64> = (0..n).filter(|&l| gets[l].is_some()).map(|l| rating[l]).collect();
    let (qv, hardness) = if individual {
        let qn = q_gets.len();
        if qn == 0 {
            (Vec::new(), MAX_Q)
        } else {
            let h = max_like(1000.0, &field, q_gets.iter().sum::<f64>() / qn as f64, false);
            let qv = (0..qn)
                .into_par_iter()
                .map(|i| max_like(1000.0, &q_att[i], q_gets[i], false))
                .collect();
            (qv, h)
        }
    } else if t.n_declared > 0 {
        let qval = max_like(1000.0, &field, total_gets / t.n_declared as f64, false);
        (vec![qval; t.n_declared], qval)
    } else {
        (Vec::new(), 0.0)
    };

    // Questions this team actually played: '-' is a question it never saw.
    let played = |mask: &Option<Vec<u8>>| -> Vec<f64> {
        match mask {
            Some(m) => qv
                .iter()
                .zip(m)
                .filter(|(_, &c)| c != b'-')
                .map(|(&q, _)| q)
                .collect(),
            None => qv.clone(),
        }
    };

    let weight = if t.rated {
        DELTA_MULTIPLIER
    } else {
        NON_RATE_DELTA_MULTIPLIER
    };
    let mut pdelta: HashMap<usize, f64> = HashMap::new();
    let mut qs_of: Vec<Vec<f64>> = vec![Vec::new(); n];
    for r in &t.res {
        if let Some(&l) = local.get(&r.team) {
            if qs_of[l].is_empty() {
                qs_of[l] = played(&r.mask);
            }
        }
    }
    for l in 0..n {
        let Some(g) = gets[l] else { continue };
        let d = (g - elo_estimate(rating[l], &qs_of[l])) * weight;
        let np = members[l].len() as f64;
        let base = if np > 6.0 { 6.0 / np } else { 1.0 };
        for &p in &members[l] {
            let wl = if t.rated {
                let c = st.games[p];
                base * if c < 10 {
                    10.0
                } else if c < 20 {
                    7.0
                } else if c < 30 {
                    5.0
                } else {
                    3.0
                } / weight
            } else {
                base
            };
            pdelta.insert(p, d * wl);
        }
    }

    let listed: Vec<(usize, usize)> = t
        .res
        .iter()
        .enumerate()
        .filter_map(|(i, r)| local.get(&r.team).map(|&l| (l, i)))
        .collect();
    let rows: Vec<(f64, Option<f64>)> = listed
        .par_iter()
        .map(|&(l, i)| {
            (
                elo_estimate(rating[l], &qs_of[l]),
                t.res[i]
                    .total
                    .map(|q| max_like(1000.0, &qs_of[l], q as f64, true)),
            )
        })
        .collect();
    let fplace = avg_ranks_desc(&listed.iter().map(|&(l, _)| rating[l]).collect::<Vec<_>>());

    let mut evs = Vec::with_capacity(rows.len());
    for (k, &(l, i)) in listed.iter().enumerate() {
        let (forecast, perf) = rows[k];
        let res = &t.res[i];
        evs.push(TeamEvent {
            team: order[l] as u32,
            c: 0.0,
            delta_l: 0.0,
            x: forecast,
            forecast: rating[l],
            forecast_place: fplace[k],
            expected_place: fplace[k],
            actual_place: res.position,
            pred_place: None,
            s_r: perf.unwrap_or(f64::NAN),
            s_pair: 0.0,
            i_pair: 1.0,
            mu_before: rating[l],
            mu_after: rating[l],
            sigma_before: 0.0,
            sigma_after: 0.0,
            evidence: members[l].len() as f64,
            gap_days: 0,
            var_diff: 0.0,
            lost_prev: 0.0,
            rated: t.rated,
            z: None,
        });
    }

    let mut prows: Vec<(i64, f64, f64, f64)> = Vec::new();
    for r in &t.res {
        for &p in &r.members {
            let d = pdelta.get(&p).copied();
            prows.push((
                arena.player_ids[p],
                st.pr[p],
                st.pr[p] + d.unwrap_or(0.0),
                0.0,
            ));
            if let Some(d) = d {
                st.add(p, d, t.rated);
            }
        }
    }

    let date = format_date(t.day);
    w.team_events(t.id, &date, &arena.team_ids, &evs)?;
    w.player_events(t.id, &prows)?;
    w.question_params(
        t.id,
        &qv.iter().map(|&b| (b, 1.0, 1.0)).collect::<Vec<_>>(),
    )?;
    let (fp, ap): (Vec<f64>, Vec<f64>) = evs
        .iter()
        .filter(|e| e.actual_place > 0.0 && e.actual_place != 9999.0)
        .map(|e| (e.forecast_place, e.actual_place))
        .unzip();
    let n_obs: u64 = qs_of.iter().map(|q| q.len() as u64).sum();
    w.tournament_metrics(
        t.id,
        &date,
        t.kind,
        weight,
        t.res.len(),
        qv.len(),
        n_obs,
        None,
        None,
        spearman(&fp, &ap),
        None,
        Some(hardness),
    )?;
    Ok(())
}
