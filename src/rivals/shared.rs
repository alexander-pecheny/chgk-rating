//! What the rival engines share: the command line, id interning, read-only
//! databases, release-to-release changes and the empty event rows they fill.
use crate::output::PlayerEvent;
use crate::team_event::TeamEvent;
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;

/// Base of the decimal ids in a roster string.
const DECIMAL: i64 = 10;
/// The mean of two neighbouring places.
const HALF: f64 = 0.5;

/// A row of `player_ratings`: id, rating, sigma, games, change.
pub type PlayerRow = (i64, f64, f64, u32, f64);
/// A row of `team_ratings`: id, mu, sigma, core strength, evidence, change.
pub type TeamRow = (i64, f64, f64, f64, f64, f64);

/// `--flag value` pairs off the command line.
pub struct Args(Vec<String>);

impl Args {
    pub fn from_env() -> Args {
        Args(std::env::args().collect())
    }

    pub fn get(&self, name: &str) -> Option<String> {
        let i = self.0.iter().position(|x| x == name)?;
        self.0.get(i + 1).cloned()
    }

    pub fn or(&self, name: &str, default: &str) -> String {
        self.get(name).unwrap_or_else(|| default.into())
    }

    /// Whether a bare switch is present.
    pub fn has(&self, name: &str) -> bool {
        self.0.iter().any(|a| a == name)
    }
}

pub fn open_read_only(path: &str) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

/// Dense indices for external ids, in order of first sight.
#[derive(Default)]
pub struct Ids {
    map: HashMap<i64, u32>,
    pub list: Vec<i64>,
}

impl Ids {
    pub fn get(&mut self, id: i64) -> u32 {
        let n = self.list.len() as u32;
        *self.map.entry(id).or_insert_with(|| {
            self.list.push(id);
            n
        })
    }

    pub fn find(&self, id: i64) -> Option<u32> {
        self.map.get(&id).copied()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// Every run of digits in `s`, as numbers: the mirror's roster strings.
pub fn parse_ids(s: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let mut cur: Option<i64> = None;
    for c in s.bytes() {
        if c.is_ascii_digit() {
            cur = Some(cur.unwrap_or(0) * DECIMAL + (c - b'0') as i64);
        } else if let Some(v) = cur.take() {
            out.push(v);
        }
    }
    out.extend(cur);
    out
}

/// The value each entity had at the previous release it was listed in, for the
/// `change` column; NaN before the first.
pub struct Changes(pub Vec<f64>);

impl Changes {
    pub fn new(n: usize) -> Changes {
        Changes(vec![f64::NAN; n])
    }

    /// The change since the last call for `i`, recording `now` for the next.
    pub fn step(&mut self, i: usize, now: f64) -> f64 {
        let prev = std::mem::replace(&mut self.0[i], now);
        if prev.is_finite() {
            now - prev
        } else {
            0.0
        }
    }
}

/// A team event with every diagnostic empty, for an engine to fill what it has.
pub fn blank_event(team: u32) -> TeamEvent {
    TeamEvent {
        d_place: f64::NAN,
        d_pull: f64::NAN,
        team,
        c: 0.0,
        delta_l: 0.0,
        x: 0.0,
        forecast: 0.0,
        forecast_place: 0.0,
        expected_place: 0.0,
        actual_place: 0.0,
        pred_place: None,
        s_r: 0.0,
        s_pair: 0.0,
        i_pair: 0.0,
        mu_before: 0.0,
        mu_after: 0.0,
        sigma_before: 0.0,
        sigma_after: 0.0,
        evidence: 0.0,
        gap_days: 0,
        var_diff: 0.0,
        lost_prev: 0.0,
        rated: false,
        z: None,
    }
}

/// A player event of a system that keeps no format effect and no uncertainty:
/// the diagnostic columns stay empty.
pub fn bare_player_event(player: i64, before: f64, after: f64) -> PlayerEvent {
    PlayerEvent {
        player,
        theta_before: before,
        theta_after: after,
        sigma_after: 0.0,
        phi_before: 0.0,
        sigma_before: 0.0,
        games_ip: 0,
        days_ip: -1,
    }
}

/// Places from scores, highest first, ties sharing the mean of the range they span.
pub fn ranks_desc(key: &[f64]) -> Vec<f64> {
    let mut o: Vec<usize> = (0..key.len()).collect();
    o.sort_by(|&a, &b| key[b].total_cmp(&key[a]));
    let mut r = vec![0.0; key.len()];
    let mut i = 0;
    while i < o.len() {
        let mut j = i;
        while j + 1 < o.len() && key[o[j + 1]] == key[o[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 * HALF + 1.0;
        for &k in &o[i..=j] {
            r[k] = avg;
        }
        i = j + 1;
    }
    r
}
