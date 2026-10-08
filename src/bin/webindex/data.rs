//! Reading the two inputs: the config's display maps, and rows out of SQLite
//! with the defaults every table here wants for a missing value.

use chgk_rating::config::{Config, TeamEngine};
use rusqlite::{Connection, OpenFlags, Row, RowIndex};
use std::collections::HashSet;
use std::path::Path;

/// The published scale and what the site needs to know about the run.
pub struct Display {
    pub a_team: f64,
    pub b_team: f64,
    pub a_player: f64,
    pub b_player: f64,
    pub provisional_sigma: f64,
    /// How much of the format effect the published player number carries.
    pub player_phi: f64,
    /// Model S2 publishes no team uncertainty, only the provisional flag, so the
    /// pages that would show a band or a plus-minus show rated games instead.
    pub s2: bool,
}

/// The `[display]` section of the run's config, which `fit` published with.
pub fn read_display(path: &Path) -> Display {
    let c = Config::load(&path.to_string_lossy()).unwrap_or_else(|e| panic!("config {e}"));
    let d = c.display;
    Display {
        a_team: d.a_team,
        b_team: d.b_team,
        a_player: d.a_player,
        b_player: d.b_player,
        provisional_sigma: d.provisional_sigma,
        player_phi: d.player_phi,
        s2: c.team.engine == TeamEngine::S2,
    }
}

/// Whether a table exists, for pages that read an optional one.
pub fn table_exists(db: &Connection, table: &str) -> bool {
    !columns(db, table).is_empty()
}

/// Column names a table actually has, so the index survives the schema growing
/// `display`, `change` or `date` under it.
pub fn columns(db: &Connection, table: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(mut st) = db.prepare(&format!("pragma table_info({table})")) else {
        return out;
    };
    if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(1)) {
        out.extend(rows.flatten());
    }
    out
}

/// `name` when the table has it, the literal NULL when it does not, so one
/// projection string serves both schemas.
pub fn col<'a>(have: &HashSet<String>, name: &'a str) -> &'a str {
    if have.contains(name) {
        name
    } else {
        "NULL"
    }
}

pub fn open(path: &Path) -> Connection {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

/// Runs `sql` and hands every row to `f`.
pub fn each(db: &Connection, sql: &str, mut f: impl FnMut(&Row)) {
    let mut st = db.prepare(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    let mut rows = st.query([]).unwrap();
    while let Some(x) = rows.next().unwrap() {
        f(x);
    }
}

/// A nullable column, read as the type it must be.
pub fn opt_f64<I: RowIndex>(x: &Row, i: I) -> Option<f64> {
    x.get(i).unwrap()
}

pub fn f64_or<I: RowIndex>(x: &Row, i: I, d: f64) -> f64 {
    opt_f64(x, i).unwrap_or(d)
}

pub fn i64_or_zero<I: RowIndex>(x: &Row, i: I) -> i64 {
    x.get::<_, Option<i64>>(i).unwrap().unwrap_or(0)
}

pub fn text<I: RowIndex>(x: &Row, i: I) -> String {
    x.get::<_, Option<String>>(i).unwrap().unwrap_or_default()
}

/// A player's name as the site shows it: first name, then surname, or a
/// placeholder with the id when the source has neither.
pub fn full_name(id: i64, first: &str, sur: &str) -> String {
    let full = format!("{} {}", first.trim(), sur.trim()).trim().to_string();
    if full.is_empty() {
        format!("Игрок {id}")
    } else {
        full
    }
}

pub fn log(t0: std::time::Instant, what: &str) {
    eprintln!("index: {what} [{:.1}s]", t0.elapsed().as_secs_f64());
}
