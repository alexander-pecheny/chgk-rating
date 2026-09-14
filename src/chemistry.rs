//! How long a lineup has played together: the one thing on the table that is a
//! property of the set of players rather than of any one of them.
//!
//! For every unordered pair of players the counter holds the number of rated
//! results the two shared, and a lineup's **togetherness** is
//!
//! ```text
//! T(L) = mean over pairs {i,j} in L of  ln(1 + games together)
//! ```
//!
//! A lineup of six strangers is at 0 and a settled team that has played a
//! hundred tournaments is near 4.6. The counter is advanced after a tournament
//! is scored, so what a field reads is the state of that morning.

use crate::data::Tournament;
use std::collections::HashMap;

pub struct Chemistry {
    pairs: HashMap<u64, u32>,
    on: bool,
}

fn key(a: u32, b: u32) -> u64 {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    ((lo as u64) << 32) | hi as u64
}

impl Chemistry {
    /// An `on = false` counter stays empty and answers 0, which is what every
    /// switch at its shipped default wants.
    pub fn new(on: bool) -> Chemistry {
        Chemistry {
            pairs: HashMap::new(),
            on,
        }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    /// `T(L)`; 0 for a lineup of fewer than two players, and 0 when off.
    pub fn t(&self, members: &[u32]) -> f64 {
        if !self.on || members.len() < 2 {
            return 0.0;
        }
        let mut sum = 0.0;
        let mut n = 0u32;
        for a in 0..members.len() {
            for b in a + 1..members.len() {
                sum += (self.pairs.get(&key(members[a], members[b])).copied().unwrap_or(0) as f64)
                    .ln_1p();
                n += 1;
            }
        }
        sum / n as f64
    }

    /// `ln(1 + games together)` for one pair, which is the term `t` averages.
    /// The feature table of plan 8 wants the spread of these and not only
    /// their mean.
    pub fn pair(&self, a: u32, b: u32) -> f64 {
        (self.pairs.get(&key(a, b)).copied().unwrap_or(0) as f64).ln_1p()
    }

    /// Counts one tournament's rated lineups. An unrated result is not a game
    /// the two players played together for this purpose, the same rule the team
    /// state follows.
    pub fn absorb(&mut self, t: &Tournament) {
        if !self.on {
            return;
        }
        for res in t.teams.iter().filter(|r| r.rated) {
            let m = &res.members;
            for a in 0..m.len() {
                for b in a + 1..m.len() {
                    *self.pairs.entry(key(m[a], m[b])).or_insert(0) += 1;
                }
            }
        }
    }
}
