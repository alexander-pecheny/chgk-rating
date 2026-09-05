//! The serving index: everything a page needs that `out/ratings.db` and
//! `buff.db` cannot answer with an index they already have.
//!
//! Two things are missing there. Names and rosters live in `buff.db` behind a
//! JSON blob and no index at all, so finding one player's games means scanning
//! 940 000 results. And the two ranking tables need the last release ordered by
//! rating, which `player_ratings` has no index for and would not want one for:
//! 18.7 million rows to serve 26 thousand. Both are computed once here.

use crate::data::{col, columns, Display};
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;

/// Bumped whenever the index grows a table or column the server reads, so a
/// deploy rebuilds it instead of serving a page with nothing behind it.
pub const VERSION: &str = "5";

const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE release_ (id INTEGER PRIMARY KEY, date TEXT, shift REAL, shift_cum REAL,
                       n_players INTEGER, n_teams INTEGER);
CREATE TABLE player (id INTEGER PRIMARY KEY, name TEXT, games INTEGER, last_date TEXT);
CREATE TABLE team (id INTEGER PRIMARY KEY, name TEXT, town TEXT, games INTEGER, last_date TEXT);
CREATE TABLE tournament (id INTEGER PRIMARY KEY, name TEXT, date TEXT, type TEXT, weight REAL,
                         n_teams INTEGER, n_q INTEGER, n_obs INTEGER, ll_in REAL, ll_cf REAL,
                         sp_forecast REAL, sp_old REAL, truedl REAL);
CREATE TABLE player_game (player_id INTEGER, tournament_id INTEGER, team_id INTEGER,
                          position REAL, score INTEGER);
CREATE TABLE result_ (tournament_id INTEGER, team_id INTEGER, score INTEGER);
CREATE TABLE player_rank (rank INTEGER PRIMARY KEY, player_id INTEGER, display REAL, sigma REAL,
                          games INTEGER, change REAL, provisional INTEGER);
CREATE TABLE team_rank (rank INTEGER PRIMARY KEY, team_id INTEGER, display REAL, sigma REAL,
                        evidence REAL, change REAL, provisional INTEGER);
CREATE TABLE anomaly (rank INTEGER PRIMARY KEY, player_id INTEGER, k REAL, k_f REAL, k_m REAL,
                      f_gap REAL, f_on INTEGER, f_ip INTEGER, m_gap REAL, m_on INTEGER,
                      m_ip INTEGER);
CREATE TABLE place_hist (kind INTEGER, id INTEGER, places BLOB, PRIMARY KEY (kind, id)) WITHOUT ROWID;
CREATE VIRTUAL TABLE player_fts USING fts5(name, tokenize='unicode61 remove_diacritics 0');
CREATE VIRTUAL TABLE team_fts USING fts5(name, town, tokenize='unicode61 remove_diacritics 0');
";

const INDEXES: &str = "
CREATE INDEX idx_pg_player ON player_game(player_id);
CREATE INDEX idx_pg_tour ON player_game(tournament_id);
CREATE INDEX idx_res_tour ON result_(tournament_id);
CREATE INDEX idx_prank_player ON player_rank(player_id);
CREATE INDEX idx_trank_team ON team_rank(team_id);
CREATE INDEX idx_tour_date ON tournament(date);
CREATE INDEX idx_anom_player ON anomaly(player_id);
";

/// The ratings database is written once by `fit` and read for ever after, so a
/// missing index is worth adding in place rather than duplicating the table.
pub fn ensure_ratings_indexes(path: &Path) {
    let db = Connection::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_qp_tour ON question_params(tournament_id);
         CREATE INDEX IF NOT EXISTS idx_player_ratings_player ON player_ratings(player_id);
         CREATE INDEX IF NOT EXISTS idx_team_ratings_team ON team_ratings(team_id);
         CREATE INDEX IF NOT EXISTS idx_team_events_team ON team_events(team_id);
         CREATE INDEX IF NOT EXISTS idx_team_events_tid ON team_events(tournament_id);",
    )
    .expect("index out/ratings.db");
}

pub fn build(ratings: &Path, buff: &Path, out: &Path, d: &Display) {
    let t0 = std::time::Instant::now();
    let r = crate::data::open(ratings);
    let b = crate::data::open(buff);
    let _ = std::fs::remove_file(out);
    let w = Connection::open(out).unwrap_or_else(|e| panic!("create {}: {e}", out.display()));
    w.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;").unwrap();
    w.execute_batch(SCHEMA).unwrap();
    w.execute_batch("BEGIN").unwrap();

    // ── releases ─────────────────────────────────────────────────────────────
    let teams_per_release: HashMap<i64, i64> = {
        let mut st = r
            .prepare("select release_id, count(*) from team_ratings where listed = 1 group by release_id")
            .unwrap();
        let v = st.query_map([], |x| Ok((x.get(0)?, x.get(1)?))).unwrap();
        v.map(|x| x.unwrap()).collect()
    };
    let mut last_release = (0i64, String::new());
    {
        let mut st = r
            .prepare("select id, date, shift, shift_cum, n_players from releases order by date, id")
            .unwrap();
        let mut ins = w.prepare("insert into release_ values (?1,?2,?3,?4,?5,?6)").unwrap();
        let mut rows = st.query([]).unwrap();
        while let Some(x) = rows.next().unwrap() {
            let id: i64 = x.get(0).unwrap();
            let date: String = x.get::<_, Option<String>>(1).unwrap().unwrap_or_default();
            ins.execute(params![
                id,
                date,
                x.get::<_, Option<f64>>(2).unwrap().unwrap_or(0.0),
                x.get::<_, Option<f64>>(3).unwrap().unwrap_or(0.0),
                x.get::<_, Option<i64>>(4).unwrap().unwrap_or(0),
                teams_per_release.get(&id).copied().unwrap_or(0),
            ])
            .unwrap();
            last_release = (id, date);
        }
    }
    assert!(last_release.0 > 0, "no releases in {}", ratings.display());

    // ── tournaments ──────────────────────────────────────────────────────────
    let mut tour_date: HashMap<i64, String> = HashMap::new();
    // Whether a tournament could be played from home, which is what the anomaly
    // score of `/anomaly/` is a comparison across.
    let mut tour_online: HashMap<i64, bool> = HashMap::new();
    {
        let names: HashMap<i64, String> = {
            let mut st = b.prepare("select id, name from tournaments").unwrap();
            let v = st
                .query_map([], |x| {
                    Ok((x.get(0)?, x.get::<_, Option<String>>(1)?.unwrap_or_default()))
                })
                .unwrap();
            v.map(|x| x.unwrap()).collect()
        };
        let mut st = r
            .prepare(
                "select tournament_id, date, type, weight, n_teams, n_q, n_obs, logloss_insample, \
                 logloss_crossfit, spearman_forecast, spearman_old_system, truedl \
                 from tournament_metrics",
            )
            .unwrap();
        let mut ins = w
            .prepare("insert into tournament values (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)")
            .unwrap();
        let mut rows = st.query([]).unwrap();
        while let Some(x) = rows.next().unwrap() {
            let id: i64 = x.get(0).unwrap();
            let date: String = x.get::<_, Option<String>>(1).unwrap().unwrap_or_default();
            let kind: String = x.get::<_, Option<String>>(2).unwrap().unwrap_or_default();
            tour_date.insert(id, date.clone());
            tour_online.insert(id, kind != "in-person");
            ins.execute(params![
                id,
                names.get(&id).cloned().unwrap_or_default(),
                date,
                kind,
                x.get::<_, Option<f64>>(3).unwrap().unwrap_or(0.0),
                x.get::<_, Option<i64>>(4).unwrap().unwrap_or(0),
                x.get::<_, Option<i64>>(5).unwrap().unwrap_or(0),
                x.get::<_, Option<i64>>(6).unwrap().unwrap_or(0),
                x.get::<_, Option<f64>>(7).unwrap(),
                x.get::<_, Option<f64>>(8).unwrap(),
                x.get::<_, Option<f64>>(9).unwrap(),
                x.get::<_, Option<f64>>(10).unwrap(),
                x.get::<_, Option<f64>>(11).unwrap(),
            ])
            .unwrap();
        }
    }
    eprintln!("index: {} tournaments [{:.1}s]", tour_date.len(), t0.elapsed().as_secs_f64());

    // ── one pass over the results: rosters, team names, scores ───────────────
    // No index on `tournament_results`, so this is a single scan and everything
    // that needs it is collected in the same loop.
    let mut team_last: HashMap<i64, (String, String, String, i64)> = HashMap::new();
    let mut player_last: HashMap<i64, (String, i64)> = HashMap::new();
    {
        let mut st = b
            .prepare(
                "select id, team_id, team_current_name, team_current_town, questions_total, \
                 position, team_members from tournament_results",
            )
            .unwrap();
        let mut pg = w.prepare("insert into player_game values (?1,?2,?3,?4,?5)").unwrap();
        let mut rs = w.prepare("insert into result_ values (?1,?2,?3)").unwrap();
        let mut rows = st.query([]).unwrap();
        while let Some(x) = rows.next().unwrap() {
            let tid: i64 = match x.get(0) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let Some(date) = tour_date.get(&tid) else { continue };
            let team_id: i64 = match x.get(1) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let name: String = x.get::<_, Option<String>>(2).unwrap().unwrap_or_default();
            let town: String = x.get::<_, Option<String>>(3).unwrap().unwrap_or_default();
            let score = x
                .get::<_, Option<String>>(4)
                .unwrap()
                .and_then(|s| s.trim().parse::<i64>().ok());
            let position: Option<f64> = x.get(5).unwrap_or(None);
            if !name.is_empty() {
                let e = team_last.entry(team_id).or_insert_with(|| {
                    (name.clone(), town.clone(), date.clone(), 0)
                });
                if *date >= e.2 {
                    *e = (name, town, date.clone(), e.3);
                }
                e.3 += 1;
            }
            rs.execute(params![tid, team_id, score]).unwrap();
            if let Some(members) = x.get::<_, Option<String>>(6).unwrap() {
                if members.len() > 2 {
                    if let Ok(ids) = serde_json::from_str::<Vec<i64>>(&members) {
                        for pid in ids {
                            pg.execute(params![pid, tid, team_id, position, score]).unwrap();
                            let e = player_last.entry(pid).or_insert((String::new(), 0));
                            if *date >= e.0 {
                                e.0 = date.clone();
                            }
                            e.1 += 1;
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "index: {} players with games, {} teams [{:.1}s]",
        player_last.len(),
        team_last.len(),
        t0.elapsed().as_secs_f64()
    );

    // ── names ────────────────────────────────────────────────────────────────
    {
        let mut st = b.prepare("select id, name, surname from players").unwrap();
        let mut ins = w.prepare("insert into player values (?1,?2,?3,?4)").unwrap();
        let mut fts = w.prepare("insert into player_fts(rowid, name) values (?1,?2)").unwrap();
        let mut rows = st.query([]).unwrap();
        while let Some(x) = rows.next().unwrap() {
            let id: i64 = x.get(0).unwrap();
            let Some((last, games)) = player_last.get(&id) else { continue };
            let first: String = x.get::<_, Option<String>>(1).unwrap().unwrap_or_default();
            let sur: String = x.get::<_, Option<String>>(2).unwrap().unwrap_or_default();
            let full = format!("{} {}", sur.trim(), first.trim()).trim().to_string();
            let full = if full.is_empty() { format!("Игрок {id}") } else { full };
            ins.execute(params![id, full, games, last]).unwrap();
            fts.execute(params![id, full]).unwrap();
        }
        let mut ins = w.prepare("insert into team values (?1,?2,?3,?4,?5)").unwrap();
        let mut fts = w.prepare("insert into team_fts(rowid, name, town) values (?1,?2,?3)").unwrap();
        for (id, (name, town, last, games)) in &team_last {
            ins.execute(params![id, name, town, games, last]).unwrap();
            fts.execute(params![id, name, town]).unwrap();
        }
    }

    // ── the two ranking tables of the last release ───────────────────────────
    let pf = change_factor(&r, "player_ratings", last_release.0, d.b_player);
    let tf = change_factor(&r, "team_ratings", last_release.0, d.b_team);
    let ranked = |table: &str, id_col: &str, value: &str, extra: &str, factor: f64, b: f64, a: f64|
     -> Vec<(i64, f64, f64, f64, f64, bool)> {
        let have = columns(&r, table);
        let sql = format!(
            "select {id_col}, {value}, sigma, {extra}, {}, {} from {table} where release_id = ?1",
            col(&have, "change"),
            col(&have, "display"),
        );
        let mut st = r.prepare(&sql).unwrap();
        let mut rows = st.query([last_release.0]).unwrap();
        let mut out = Vec::new();
        while let Some(x) = rows.next().unwrap() {
            let id: i64 = x.get(0).unwrap();
            let raw: f64 = x.get::<_, Option<f64>>(1).unwrap().unwrap_or(0.0);
            let sigma: f64 = x.get::<_, Option<f64>>(2).unwrap().unwrap_or(0.0);
            let extra: f64 = x.get::<_, Option<f64>>(3).unwrap().unwrap_or(0.0);
            let change: Option<f64> = x.get(4).unwrap();
            let display: f64 = x.get::<_, Option<f64>>(5).unwrap().unwrap_or(a + b * raw);
            out.push((
                id,
                display,
                b.abs() * sigma,
                extra,
                factor * change.unwrap_or(0.0),
                sigma > d.provisional_sigma,
            ));
        }
        // Settled ratings first, then by rating: the published order.
        out.sort_by(|x, y| x.5.cmp(&y.5).then(y.1.partial_cmp(&x.1).unwrap()));
        out
    };

    let prows = ranked(
        "player_ratings", "player_id", "theta", "games_365", pf, d.b_player, d.a_player,
    );
    let mut ins = w.prepare("insert into player_rank values (?1,?2,?3,?4,?5,?6,?7)").unwrap();
    for (i, x) in prows.iter().enumerate() {
        ins.execute(params![i as i64 + 1, x.0, x.1, x.2, x.3 as i64, x.4, x.5 as i64]).unwrap();
    }
    drop(ins);
    let trows = ranked(
        "team_ratings", "team_id", "mu", "evidence", tf, d.b_team, d.a_team,
    );
    let mut ins = w.prepare("insert into team_rank values (?1,?2,?3,?4,?5,?6,?7)").unwrap();
    for (i, x) in trows.iter().enumerate() {
        ins.execute(params![i as i64 + 1, x.0, x.1, x.2, x.3, x.4, x.5 as i64]).unwrap();
    }
    drop(ins);

    for (k, v) in [
        ("index_version", VERSION.to_string()),
        ("release_id", last_release.0.to_string()),
        ("release_date", last_release.1.clone()),
        ("n_player_rank", prows.len().to_string()),
        ("n_team_rank", trows.len().to_string()),
        ("a_team", d.a_team.to_string()),
        ("b_team", d.b_team.to_string()),
        ("a_player", d.a_player.to_string()),
        ("b_player", d.b_player.to_string()),
    ] {
        w.execute("insert into meta values (?1,?2)", params![k, v]).unwrap();
    }

    let rows = crate::anomaly::build(&r, &b, &w, &tour_online);
    crate::anomaly::write(&w, &rows);
    eprintln!("index: anomaly scores [{:.1}s]", t0.elapsed().as_secs_f64());

    place_history(&r, &w, "player_ratings", "player_id", "theta", 0, d.a_player, d.b_player, d);
    place_history(&r, &w, "team_ratings", "team_id", "mu", 1, d.a_team, d.b_team, d);

    w.execute_batch("COMMIT").unwrap();
    w.execute_batch(INDEXES).unwrap();
    eprintln!(
        "index: release {} ({}), {} teams, {} players ranked [{:.1}s]",
        last_release.0,
        last_release.1,
        trows.len(),
        prows.len(),
        t0.elapsed().as_secs_f64()
    );
}

/// `change` is a delta of the internal value, so it needs the display factor —
/// but a future schema could publish it already scaled. Deltas on the internal
/// scale never reach 1; on a published scale of thousands they routinely do, so
/// the largest one in the table says which this is.
fn change_factor(db: &Connection, table: &str, release: i64, b: f64) -> f64 {
    if !columns(db, table).contains("change") || b <= 10.0 {
        return 1.0;
    }
    let biggest: Option<f64> = db
        .query_row(
            &format!("select max(abs(change)) from {table} where release_id = ?1"),
            [release],
            |r| r.get(0),
        )
        .unwrap_or(None);
    match biggest {
        Some(x) if x > 1.0 => 1.0,
        _ => b,
    }
}

/// Where each entity stood in the published table at every release it appeared
/// in, stored as one blob per entity in release order — the same order the
/// chart reads its rating history in, so the two series line up by index.
///
/// The rows come back in insertion order, which is release order, because `fit`
/// writes one release at a time and never revisits one. That is what makes this
/// a single streaming pass over 18.7 million rows instead of a sort over them.
#[allow(clippy::too_many_arguments)]
fn place_history(
    r: &Connection,
    w: &Connection,
    table: &str,
    id_col: &str,
    value: &str,
    kind: i64,
    a: f64,
    b: f64,
    d: &Display,
) {
    let have = columns(r, table);
    let sql = format!(
        "select release_id, {id_col}, {value}, sigma, {} from {table}",
        col(&have, "display")
    );
    let mut st = r.prepare(&sql).unwrap();
    let mut rows = st.query([]).unwrap();

    let mut out: HashMap<i64, Vec<u8>> = HashMap::new();
    let mut cur = i64::MIN;
    let mut batch: Vec<(i64, f64, bool)> = Vec::new();
    let mut done: std::collections::HashSet<i64> = std::collections::HashSet::new();
    // Settled ratings first, then by rating: the order the published table uses.
    let flush = |batch: &mut Vec<(i64, f64, bool)>, out: &mut HashMap<i64, Vec<u8>>| {
        batch.sort_by(|x, y| x.2.cmp(&y.2).then(y.1.partial_cmp(&x.1).unwrap()));
        for (i, (id, _, _)) in batch.iter().enumerate() {
            out.entry(*id).or_default().extend_from_slice(&(i as u32 + 1).to_le_bytes());
        }
        batch.clear();
    };
    while let Some(x) = rows.next().unwrap() {
        let rel: i64 = x.get(0).unwrap();
        if rel != cur {
            flush(&mut batch, &mut out);
            if !done.insert(cur) && cur != i64::MIN {
                eprintln!("warning: release {cur} is not contiguous; places may be wrong");
            }
            cur = rel;
        }
        let raw: f64 = x.get::<_, Option<f64>>(2).unwrap().unwrap_or(0.0);
        let sigma: f64 = x.get::<_, Option<f64>>(3).unwrap().unwrap_or(0.0);
        let display: f64 = x.get::<_, Option<f64>>(4).unwrap().unwrap_or(a + b * raw);
        batch.push((x.get(1).unwrap(), display, sigma > d.provisional_sigma));
    }
    flush(&mut batch, &mut out);

    let mut ins = w.prepare("insert into place_hist values (?1,?2,?3)").unwrap();
    for (id, blob) in &out {
        ins.execute(params![kind, id, blob]).unwrap();
    }
    eprintln!("index: place history for {} entries of {table}", out.len());
}

/// The version the index at `path` was built with, or nothing if it cannot say.
pub fn version(path: &Path) -> Option<String> {
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    db.query_row("select value from meta where key = 'index_version'", [], |r| r.get(0)).ok()
}
