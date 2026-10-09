//! One streaming pass over a ratings table: where each entity stood in the
//! published table at every release it appeared in, and its last published
//! rating.
//!
//! The rows come back in insertion order, which is release order, because `fit`
//! writes one release at a time and never revisits one. That is what makes this
//! a single pass over 18.7 million rows instead of a sort over them.

use crate::data::{col, columns, Display};
use crate::index::Ctx;
use rusqlite::{params, Row};
use std::collections::{HashMap, HashSet};

/// Which ratings table to read and how its values reach the published scale.
pub struct Listed {
    table: &'static str,
    id: &'static str,
    value: &'static str,
    /// A count shown beside the rating, when the table has one.
    extra: &'static str,
    a: f64,
    b: f64,
    provisional_sigma: f64,
}

impl Listed {
    pub fn players(d: &Display) -> Listed {
        let (a, b) = (d.a_player, d.b_player);
        let provisional_sigma = d.provisional_sigma;
        Listed { table: "player_ratings", id: "player_id", value: "theta", extra: "games_365", a, b, provisional_sigma }
    }

    pub fn teams(d: &Display) -> Listed {
        let (a, b) = (d.a_team, d.b_team);
        let provisional_sigma = d.provisional_sigma;
        Listed { table: "team_ratings", id: "team_id", value: "mu", extra: "NULL", a, b, provisional_sigma }
    }
}

/// An entity's rating at the last release it was listed in.
pub struct Current {
    release: i64,
    pub display: f64,
    /// The plus-minus on the display scale.
    pub sigma: f64,
    pub games_365: Option<i64>,
    pub provisional: bool,
}

pub struct History {
    /// One little-endian u32 place per release, in release order: the same
    /// order the chart reads its rating history in, so the two line up.
    places: HashMap<i64, Vec<u8>>,
    pub now: HashMap<i64, Current>,
}

// Positions in the query `scan` runs; the hot loop reads by index.
const REL: usize = 0;
const ID: usize = 1;
const RAW: usize = 2;
const SIGMA: usize = 3;
const DISPLAY: usize = 4;
const EXTRA: usize = 5;

/// One row of the table: who, the published value, whether it is provisional.
type Entry = (i64, f64, bool);

pub fn scan(c: &Ctx, l: Listed) -> History {
    let have = columns(c.r, l.table);
    let sql = format!(
        "select release_id, {}, {}, sigma, {}, {} from {}",
        l.id, l.value, col(&have, "display"), l.extra, l.table
    );
    let mut h = History { places: HashMap::new(), now: HashMap::new() };
    let mut cur = i64::MIN;
    let mut batch: Vec<Entry> = Vec::new();
    let mut done: HashSet<i64> = HashSet::new();
    crate::data::each(c.r, &sql, |x| {
        let rel: i64 = x.get(REL).unwrap();
        if rel != cur {
            h.flush(&mut batch);
            if !done.insert(cur) && cur != i64::MIN {
                eprintln!("warning: release {cur} is not contiguous; places may be wrong");
            }
            cur = rel;
        }
        batch.push(h.read(&l, rel, x));
    });
    h.flush(&mut batch);
    h
}

impl History {
    /// Reads one row, keeping it as the entity's current rating if it is the
    /// latest release seen for it.
    fn read(&mut self, l: &Listed, rel: i64, x: &Row) -> Entry {
        let id: i64 = x.get(ID).unwrap();
        let sigma = crate::data::f64_or(x, SIGMA, 0.0);
        let raw = crate::data::f64_or(x, RAW, 0.0);
        let display = crate::data::f64_or(x, DISPLAY, l.a + l.b * raw);
        let provisional = sigma > l.provisional_sigma;
        let now = Current { release: rel, display, sigma: l.b * sigma, games_365: x.get(EXTRA).unwrap(), provisional };
        match self.now.get(&id) {
            Some(n) if n.release > rel => {}
            _ => {
                self.now.insert(id, now);
            }
        }
        (id, display, provisional)
    }

    /// Places one release: settled ratings first, then by rating, the order
    /// the published table uses.
    fn flush(&mut self, batch: &mut Vec<Entry>) {
        batch.sort_by(|x, y| x.2.cmp(&y.2).then(y.1.total_cmp(&x.1)).then(x.0.cmp(&y.0)));
        for (i, (id, _, _)) in batch.iter().enumerate() {
            self.places.entry(*id).or_default().extend_from_slice(&(i as u32 + 1).to_le_bytes());
        }
        batch.clear();
    }
}

pub fn write(c: &Ctx, kind: i64, h: &History) {
    let mut ins = c.w.prepare("insert into place_hist values (?1,?2,?3)").unwrap();
    for (id, blob) in &h.places {
        ins.execute(params![kind, id, blob]).unwrap();
    }
    eprintln!("index: place history for {} entries", h.places.len());
}
