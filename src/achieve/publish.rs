//! The achievement rating written over the team side of a fit's database.

use super::book::Book;
use super::knobs::{Knobs, KNOBS};
use super::lab::Lab;
use crate::config::Config;
use crate::util::{format_date, parse_date};
use rayon::prelude::*;
use rusqlite::{params, Connection, Transaction};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// A divisor is never read as less than this.
const EPS: f64 = 1e-9;

const SCHEMA: &str = "DROP TABLE IF EXISTS achieve_values;
         CREATE TABLE achieve_values (tournament_id INTEGER, field_id INTEGER, team_id INTEGER, date TEXT,
           format TEXT, place REAL, venue_place REAL, n_teams INTEGER, n_venues INTEGER, field REAL,
           perf REAL, strength REAL, w REAL, w_format REAL, w_field REAL, w_reach REAL, n_towns INTEGER, peak REAL,
           gain REAL);
         DROP TABLE IF EXISTS achieve_parts;
         CREATE TABLE achieve_parts (team_id INTEGER, release_id INTEGER, slot INTEGER, tournament_id INTEGER,
           field_id INTEGER, peak REAL, fade REAL, overlap REAL, share REAL, contribution REAL,
           PRIMARY KEY (team_id, release_id, slot)) WITHOUT ROWID;
         DROP TABLE IF EXISTS achieve_team;
         CREATE TABLE achieve_team (team_id INTEGER, release_id INTEGER, rating REAL, divisor REAL,
           n_results INTEGER, empty_share REAL, date TEXT, PRIMARY KEY (team_id, release_id)) WITHOUT ROWID;
         DROP TABLE IF EXISTS team_ql;
         CREATE TABLE team_ql (release_id INTEGER, team_id INTEGER, ql REAL,
           PRIMARY KEY (release_id, team_id)) WITHOUT ROWID;
         DROP TABLE IF EXISTS achieve_meta;
         CREATE TABLE achieve_meta (key TEXT PRIMARY KEY, value TEXT);";

const INDEXES: &str = "CREATE INDEX IF NOT EXISTS idx_achieve_values_team ON achieve_values(team_id);
         CREATE INDEX IF NOT EXISTS idx_achieve_values_tt ON achieve_values(tournament_id, team_id);";

/// Everything computed before the database is touched for writing.
struct Computed {
    /// The releases the fit wrote team rows for, and every listed team's
    /// (rating, results) at each.
    releases: Vec<(i64, i32)>,
    per_release: Vec<Vec<(i64, f64, usize)>>,
    /// Each event's rating before and after, by `team_events` rowid.
    steps: Vec<(i64, f64, f64)>,
    /// What each (venue, team) result added to the rating.
    gains: HashMap<(i64, i64), f64>,
    teams: Vec<i64>,
}

/// Replace the team side of a fit's database with the achievement rating:
/// `team_ratings` at every release, each team's rating before and after every
/// event in `team_events`, and two tables the site explains a rating from —
/// `achieve_values`, what every counted result was worth on the day, and
/// `achieve_parts`, which of them make each rating at every release.
pub fn publish(lab: &Lab, k: &Knobs, db: &str, cfg: &Config, threads: usize) -> rusqlite::Result<()> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    let t0 = Instant::now();
    let b = lab.book(k);
    let mut conn = Connection::open(db)?;
    let (releases, events) = (releases(&conn)?, events(&conn)?);
    let c = pool.install(|| compute(&b, releases, &events, t0));
    let tx = conn.transaction()?;
    let scale = (cfg.display.a_team, cfg.display.b_team);
    write_ratings(&tx, &c, scale)?;
    write_steps(&tx, &c.steps, scale)?;
    tx.execute_batch(SCHEMA)?;
    write_values(&tx, &b, &c.gains)?;
    let parts: Vec<Vec<Part>> = pool.install(|| c.releases.par_iter().map(|&(_, d)| release_parts(&b, &c.teams, d)).collect());
    write_parts(&tx, &c.releases, &parts)?;
    write_ql(&tx, &c.releases, &lab.ql)?;
    write_meta(&tx, k)?;
    tx.execute_batch(INDEXES)?;
    tx.commit()?;
    eprintln!("publish: written to {db} [{:.0}s]", t0.elapsed().as_secs_f64());
    Ok(())
}

fn compute(b: &Book, releases: Vec<(i64, i32)>, events: &[(i64, i64, i32)], t0: Instant) -> Computed {
    let teams: Vec<i64> = b.vals.keys().copied().collect();
    let per_release = releases.par_iter().map(|&(_, d)| release_rows(b, &teams, d)).collect();
    eprintln!("publish: {} releases rated [{:.0}s]", releases.len(), t0.elapsed().as_secs_f64());
    // Every event's rating before and after, on the mornings of its day and the next.
    let steps: Vec<(i64, f64, f64)> =
        events.par_iter().map(|&(i, t, d)| (i, b.rating(t, d).0, b.rating(t, d + 1).0)).collect();
    eprintln!("publish: {} events stepped [{:.0}s]", steps.len(), t0.elapsed().as_secs_f64());
    let gains = teams.par_iter().flat_map_iter(|&t| gains(b, t)).collect();
    Computed { releases, per_release, steps, gains, teams }
}

/// Only the releases the fit wrote team rows for (every one in production,
/// the last one in a research config).
fn releases(conn: &Connection) -> rusqlite::Result<Vec<(i64, i32)>> {
    let all: Vec<(i64, i32)> = conn
        .prepare("SELECT id, date FROM releases ORDER BY date")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|x| x.ok())
        .filter_map(|(i, d)| Some((i, parse_date(&d)?)))
        .collect();
    let written: HashSet<i64> =
        conn.prepare("SELECT DISTINCT release_id FROM team_ratings")?.query_map([], |r| r.get(0))?.filter_map(|x| x.ok()).collect();
    Ok(all.into_iter().filter(|r| written.contains(&r.0)).collect())
}

fn events(conn: &Connection) -> rusqlite::Result<Vec<(i64, i64, i32)>> {
    let v = conn
        .prepare("SELECT rowid, team_id, date FROM team_events")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?
        .filter_map(|x| x.ok())
        .filter_map(|(i, t, d)| Some((i, t, parse_date(&d)?)))
        .collect();
    Ok(v)
}

/// Every listed team with a rating at a release: (team, rating, results).
fn release_rows(b: &Book, teams: &[i64], d: i32) -> Vec<(i64, f64, usize)> {
    let row = |&t: &i64| {
        let (x, c) = b.rating(t, d);
        (x > 0.0).then_some((t, x, c.len()))
    };
    teams.iter().filter(|&&t| b.listed(t, d)).filter_map(row).collect()
}

/// What each result added: the day's rise, credited only to the results
/// that entered the counted set the next morning, split by what they add.
fn gains(b: &Book, t: i64) -> Vec<((i64, i64), f64)> {
    let mut days: Vec<i32> = b.vals[&t].iter().map(|v| v.day).collect();
    days.sort_unstable();
    days.dedup();
    let mut out = Vec::new();
    for d in days {
        let x0 = b.rating(t, d).0;
        let (x1, c, slots) = b.rating_slots(t, d + 1);
        let rise = x1 - x0;
        let entered: Vec<(i64, f64)> = c.iter().filter(|(v, _)| v.day == d).map(|(v, now)| (v.venue, now / slots)).collect();
        let total: f64 = entered.iter().map(|e| e.1).sum();
        if rise > 0.0 && total > 0.0 {
            out.extend(entered.into_iter().map(|(venue, part)| ((venue, t), rise * part / total)));
        }
    }
    out
}

/// `team_ratings` at every release, with the change since the last one.
fn write_ratings(tx: &Transaction, c: &Computed, (a, bt): (f64, f64)) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM team_ratings", [])?;
    let mut ins = tx.prepare(
        "INSERT INTO team_ratings (release_id, team_id, mu, sigma, core_strength, evidence, display, listed, change)
             VALUES (?1, ?2, ?3, 0.1, NULL, ?4, ?5, 1, ?6)",
    )?;
    let mut prev: HashMap<i64, f64> = HashMap::new();
    for (&(rid, _), rows) in c.releases.iter().zip(&c.per_release) {
        let mut next = HashMap::with_capacity(rows.len());
        for &(t, x, n) in rows {
            let change = prev.get(&t).map(|p| x - p);
            ins.execute(params![rid, t, (x - a) / bt, n as i64, x, change])?;
            next.insert(t, x);
        }
        prev = next;
    }
    Ok(())
}

fn write_steps(tx: &Transaction, steps: &[(i64, f64, f64)], (a, bt): (f64, f64)) -> rusqlite::Result<()> {
    let mut up =
        tx.prepare("UPDATE team_events SET mu_before = ?2, mu_after = ?3, d_place = ?4, d_pull = NULL WHERE rowid = ?1")?;
    for &(i, x0, x1) in steps {
        // No rating yet reads as a blank, not as the map's zero.
        let m0 = (x0 > 0.0).then(|| (x0 - a) / bt);
        let m1 = (x1 > 0.0).then(|| (x1 - a) / bt);
        up.execute(params![i, m0, m1, (x1 - x0) / bt])?;
    }
    Ok(())
}

/// `team_ql`: each team's Q(L), its base roster's strength, at every release
/// the fit wrote team rows for.
fn write_ql(tx: &Transaction, releases: &[(i64, i32)], ql: &[(i32, i64, f64)]) -> rusqlite::Result<()> {
    let id: HashMap<i32, i64> = releases.iter().map(|&(i, d)| (d, i)).collect();
    let mut ins = tx.prepare("INSERT INTO team_ql VALUES (?1, ?2, ?3)")?;
    for &(d, t, q) in ql {
        if let Some(i) = id.get(&d) {
            ins.execute(params![i, t, q])?;
        }
    }
    Ok(())
}

/// `achieve_values`: every counted result and what it was worth on the day.
fn write_values(tx: &Transaction, b: &Book, gains: &HashMap<(i64, i64), f64>) -> rusqlite::Result<()> {
    let mut ins =
        tx.prepare("INSERT INTO achieve_values VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)")?;
    for (&t, vals) in &b.vals {
        for v in vals {
            let wf = v.w / (v.wfmt * v.geo).max(EPS);
            ins.execute(params![
                v.venue, v.tid, t, format_date(v.day), v.format.to_string(), v.place, v.vplace,
                v.nt as i64, v.nv as i64, v.fs, v.perf, v.shown, v.w, v.wfmt, wf, v.geo,
                v.n_towns as i64, v.base, gains.get(&(v.venue, t))
            ])?;
        }
    }
    Ok(())
}

/// One listed team's rating on a day taken apart: (team, rating, divisor,
/// share of the divisor no result fills, and per result (venue, field, value
/// on the day, fade, overlap, share, contribution)).
type Part = (i64, f64, f64, f64, Vec<(i64, i64, f64, f64, f64, f64, f64)>);

/// Which results make each listed rating on `d`, and what share of the
/// divisor they fill.
fn release_parts(b: &Book, teams: &[i64], d: i32) -> Vec<Part> {
    let mut out = Vec::new();
    for &t in teams.iter().filter(|&&t| b.listed(t, d)) {
        let (x, c, slots) = b.rating_slots(t, d);
        if x <= 0.0 {
            continue;
        }
        let rows: Vec<_> = c
            .iter()
            .map(|(v, now)| {
                let f = super::value::fade(d - v.day, v, &b.k);
                (v.venue, v.tid, v.base, f, now / (v.base * f).max(EPS), now / v.base.max(EPS) / slots, now / slots)
            })
            .collect();
        let empty = (1.0 - rows.iter().map(|r| r.5).sum::<f64>()).max(0.0);
        out.push((t, x, slots, empty, rows));
    }
    out
}

/// `achieve_parts` and `achieve_team` at every release, so a team's page as
/// of a past release can take its rating apart too.
fn write_parts(tx: &Transaction, releases: &[(i64, i32)], parts: &[Vec<Part>]) -> rusqlite::Result<()> {
    let mut ins = tx.prepare("INSERT INTO achieve_parts VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")?;
    let mut team = tx.prepare("INSERT INTO achieve_team VALUES (?1,?2,?3,?4,?5,?6,?7)")?;
    for (&(rid, d), teams) in releases.iter().zip(parts) {
        for (t, x, slots, empty, rows) in teams {
            for (i, &(venue, tid, base, f, ov, share, contrib)) in rows.iter().enumerate() {
                ins.execute(params![t, rid, i as i64 + 1, venue, tid, base, f, ov, share, contrib])?;
            }
            team.execute(params![t, rid, x, slots, rows.len() as i64, empty, format_date(d)])?;
        }
    }
    Ok(())
}

/// `achieve_meta`: the knobs the table was published under.
fn write_meta(tx: &Transaction, k: &Knobs) -> rusqlite::Result<()> {
    let mut ins = tx.prepare("INSERT INTO achieve_meta VALUES (?1, ?2)")?;
    for key in KNOBS {
        ins.execute(params![key, k.get(key)])?;
    }
    Ok(())
}
