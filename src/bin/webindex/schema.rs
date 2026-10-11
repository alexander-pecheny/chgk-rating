//! The contract between this index and the site server in `web/`: the tables
//! `out/web.db` holds, the keys of its meta table, and the tables of
//! `out/ratings.db` the server reads beside it. The server checks
//! `index_version` at start, so any change here that it reads bumps VERSION and
//! `indexVersion` in web/store.go together.

use crate::data::{columns, table_exists};
use rusqlite::Connection;
use std::path::Path;

/// Bumped whenever the index grows a table or column the server reads, so a
/// deploy rebuilds it instead of serving a page with nothing behind it.
pub const VERSION: &str = "18";

/// `place_hist.kind`: whose places a blob holds.
pub const KIND_PLAYER: i64 = 0;
pub const KIND_TEAM: i64 = 1;

/// `player` and `team` carry the entity's last published rating, from the last
/// release it was listed in: the display value, the plus-minus on the display
/// scale and whether it is provisional. All three are NULL for an entity never
/// listed. `place_hist.places` is one little-endian u32 per release the entity
/// was listed in, in release order.
pub const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE release_ (id INTEGER PRIMARY KEY, date TEXT, shift REAL, shift_cum REAL,
                       n_players INTEGER, n_teams INTEGER);
CREATE TABLE player (id INTEGER PRIMARY KEY, name TEXT, games INTEGER, last_date TEXT,
                     display REAL, sigma REAL, games_365 INTEGER, provisional INTEGER);
CREATE TABLE team (id INTEGER PRIMARY KEY, name TEXT, town TEXT, games INTEGER, last_date TEXT,
                   display REAL, sigma REAL, provisional INTEGER);
CREATE TABLE tournament (id INTEGER PRIMARY KEY, name TEXT, date TEXT, type TEXT, weight REAL,
                         n_teams INTEGER, n_q INTEGER, n_obs INTEGER, ll_in REAL, ll_cf REAL,
                         sp_forecast REAL, sp_old REAL, truedl REAL);
CREATE TABLE player_game (player_id INTEGER, tournament_id INTEGER, team_id INTEGER,
                          position REAL, score INTEGER);
CREATE TABLE result_ (tournament_id INTEGER, team_id INTEGER, score INTEGER, name TEXT);
CREATE TABLE player_rank (rank INTEGER PRIMARY KEY, player_id INTEGER, display REAL, sigma REAL,
                          games INTEGER, change REAL, provisional INTEGER);
CREATE TABLE team_rank (rank INTEGER PRIMARY KEY, team_id INTEGER, display REAL, sigma REAL,
                        evidence REAL, change REAL, provisional INTEGER);
CREATE TABLE anomaly (rank INTEGER PRIMARY KEY, player_id INTEGER, k REAL, k_f REAL, k_m REAL,
                      f_gap REAL, f_on INTEGER, f_ip INTEGER, m_gap REAL, m_on INTEGER,
                      m_ip INTEGER);
CREATE TABLE venue (tournament_id INTEGER PRIMARY KEY, field_id INTEGER);
CREATE TABLE forecast_ndcg (tournament_id INTEGER, system TEXT, ndcg REAL, ndcg3 REAL, n_teams INTEGER, PRIMARY KEY (tournament_id, system)) WITHOUT ROWID;
CREATE TABLE field_score (tournament_id INTEGER, team_id INTEGER, score INTEGER, PRIMARY KEY (tournament_id, team_id)) WITHOUT ROWID;
CREATE TABLE base_roster (team_id INTEGER, player_id INTEGER, name TEXT, date_added TEXT);
CREATE TABLE place_hist (kind INTEGER, id INTEGER, places BLOB, PRIMARY KEY (kind, id)) WITHOUT ROWID;
CREATE VIRTUAL TABLE player_fts USING fts5(name, tokenize='unicode61 remove_diacritics 0');
CREATE VIRTUAL TABLE team_fts USING fts5(name, town, aliases, tokenize='unicode61 remove_diacritics 0');
CREATE VIRTUAL TABLE tournament_fts USING fts5(name, tokenize='unicode61 remove_diacritics 0');
";

pub const INDEXES: &str = "
CREATE INDEX idx_pg_player ON player_game(player_id);
CREATE INDEX idx_pg_tour ON player_game(tournament_id);
CREATE INDEX idx_res_tour ON result_(tournament_id, team_id);
CREATE INDEX idx_prank_player ON player_rank(player_id);
CREATE INDEX idx_trank_team ON team_rank(team_id);
CREATE INDEX idx_tour_date ON tournament(date);
CREATE INDEX idx_anom_player ON anomaly(player_id);
CREATE INDEX idx_base_team ON base_roster(team_id);
CREATE INDEX idx_venue_field ON venue(field_id);
";

/// The columns of `out/ratings.db` the server reads directly: the per-release
/// and per-result tables too large to copy here. A file that lacks one fails
/// the index build rather than a page.
const SITE_READS: &[(&str, &[&str])] = &[
    ("player_ratings", &["release_id", "player_id", "theta", "sigma", "vol"]),
    ("team_ratings", &["release_id", "team_id", "mu", "sigma"]),
    ("player_events", &["tournament_id", "player_id", "theta_before", "theta_after", "phi_before", "sigma_before"]),
    ("team_events", &["tournament_id", "team_id", "date", "forecast", "forecast_place", "actual_place",
                      "mu_before", "mu_after", "rated", "s_r"]),
    ("question_params", &["tournament_id", "q", "b"]),
];

/// The achievement rating's tables, read together when `achieve` writes them.
const ACHIEVE_READS: &[(&str, &[&str])] = &[
    ("achieve_meta", &["key", "value"]),
    ("achieve_team", &["team_id", "release_id", "rating", "divisor", "n_results"]),
    ("achieve_parts", &["team_id", "release_id", "slot", "tournament_id", "peak", "fade", "overlap", "contribution"]),
    ("achieve_values", &["tournament_id", "team_id", "date", "format", "place", "venue_place", "n_teams",
                         "n_venues", "strength", "w", "w_format", "w_field", "w_reach", "peak", "gain"]),
];

/// The columns of `reads` the database lacks, as `table.column`.
fn missing(db: &Connection, reads: &[(&str, &[&str])]) -> Vec<String> {
    let mut out = Vec::new();
    for (table, cols) in reads {
        let have = columns(db, table);
        out.extend(cols.iter().filter(|c| !have.contains(**c)).map(|c| format!("{table}.{c}")));
    }
    out
}

/// What the server may read from `out/ratings.db`, as meta flags: whether the
/// achievement tables are there, and whether `team_events` splits the step.
/// Panics on a file the server could not read at all.
pub fn site_reads(r: &Connection) -> (bool, bool) {
    let lacks = missing(r, SITE_READS);
    assert!(lacks.is_empty(), "ratings.db lacks what the site reads: {}", lacks.join(", "));
    let achieve = table_exists(r, "achieve_values");
    let lacks = missing(r, ACHIEVE_READS);
    assert!(!achieve || lacks.is_empty(), "achievement tables lack {}", lacks.join(", "));
    let te = columns(r, "team_events");
    (achieve, te.contains("d_place") && te.contains("d_pull"))
}

/// The ratings database is written once by `fit` and read for ever after, so a
/// missing index is worth adding in place rather than duplicating the table.
/// `fit` and `achieve` already create these; this covers a file written before
/// they did. An older single-column index the composite replaces is dropped, so
/// the file does not carry both.
pub fn ensure_ratings_indexes(path: &Path) {
    let db = Connection::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    // The running site reads this file while the indexes are written; wait
    // out its queries rather than fail on the first one.
    db.busy_timeout(std::time::Duration::from_secs(120)).expect("busy timeout");
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_player_events_pt ON player_events(player_id, tournament_id);
         CREATE INDEX IF NOT EXISTS idx_qp_tour ON question_params(tournament_id);
         CREATE INDEX IF NOT EXISTS idx_player_ratings_pr ON player_ratings(player_id, release_id);
         CREATE INDEX IF NOT EXISTS idx_player_ratings_rel ON player_ratings(release_id);
         CREATE INDEX IF NOT EXISTS idx_team_ratings_tr ON team_ratings(team_id, release_id);
         CREATE INDEX IF NOT EXISTS idx_team_events_team ON team_events(team_id);
         CREATE INDEX IF NOT EXISTS idx_team_events_tt ON team_events(tournament_id, team_id);
         DROP INDEX IF EXISTS idx_player_events_player;
         DROP INDEX IF EXISTS idx_player_ratings_player;
         DROP INDEX IF EXISTS idx_team_ratings_team;
         DROP INDEX IF EXISTS idx_team_events_tid;",
    )
    .expect("index out/ratings.db");
    if table_exists(&db, "achieve_values") {
        db.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_achieve_values_tt ON achieve_values(tournament_id, team_id);
             DROP INDEX IF EXISTS idx_achieve_values_tid;",
        )
        .expect("index achieve_values");
    }
}

/// The version the index at `path` was built with, or nothing if it cannot say.
pub fn version(path: &Path) -> Option<String> {
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    db.query_row("select value from meta where key = 'index_version'", [], |r| r.get(0)).ok()
}
