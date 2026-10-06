use rusqlite::{Connection, OpenFlags};
use std::collections::HashSet;
use std::path::Path;

pub struct Display {
    pub a_team: f64,
    pub b_team: f64,
    pub a_player: f64,
    pub b_player: f64,
    pub provisional_sigma: f64,
    /// Model S2 publishes no team uncertainty, only the provisional flag, so the
    /// pages that would show a band or a plus-minus show rated games instead.
    pub s2: bool,
}

/// Whether a table exists, for pages that read an optional one.
pub fn table_exists(db: &Connection, table: &str) -> bool {
    !columns(db, table).is_empty()
}

/// Column names a table actually has, so the server survives the schema growing
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

pub fn read_display(path: &Path) -> Display {
    let txt = std::fs::read_to_string(path).unwrap_or_default();
    let v: toml::Value = txt.parse().unwrap_or(toml::Value::Table(Default::default()));
    let g = |k: &str, d: f64| {
        v.get("display")
            .and_then(|t| t.get(k))
            .and_then(|x| x.as_float())
            .unwrap_or(d)
    };
    Display {
        a_team: g("a_team", 0.0),
        b_team: g("b_team", 1.0),
        a_player: g("a_player", 0.0),
        b_player: g("b_player", 1.0),
        provisional_sigma: g("provisional_sigma", 0.3),
        s2: v.get("team").and_then(|t| t.get("engine")).and_then(|x| x.as_str()) == Some("s2"),
    }
}
