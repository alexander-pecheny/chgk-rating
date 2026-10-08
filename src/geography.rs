//! Where a lineup comes from and where it is playing.
//!
//! The mirror carries the town a tournament is played in (`tournaments.town_id`)
//! and the town a team was registered in on the day
//! (`tournament_results.team_current_town`); it carries nothing about a player.
//! A player's town is therefore the town they have played out of most often in
//! the rated results **strictly before** today, which is a running count the
//! same shape as the editor memory: advanced only once a field has been
//! forecast, so no result enters its own forecast.
//!
//! The residual says the sign is the opposite of the obvious one. Over the tune
//! window a team at home beats its forecast by −0.0054 ± 0.0017 of a field and a
//! visitor by +0.0047 ± 0.0015, and the gap holds in the strongest fifth of
//! fields and among the four highest forecasts. Travelling is a choice a team in
//! form makes; turning out at home is what everyone does.

use crate::data::Tournament;
use std::collections::HashMap;

pub struct Geography {
    on: bool,
    /// `(player, town) -> results`, and the town each player has most of.
    cnt: HashMap<u64, u32>,
    best: HashMap<u32, (u32, u32)>,
}

/// The two load windows, in days.
const WEEK: i32 = 7;
const MONTH: i32 = 30;

fn key(player: u32, town: u32) -> u64 {
    crate::util::pack(player, town)
}

impl Geography {
    pub fn new(on: bool) -> Geography {
        Geography {
            on,
            cnt: HashMap::new(),
            best: HashMap::new(),
        }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    /// Share of the lineup that has played out of `town` more than out of
    /// anywhere else. Players with no record at all are left out of both sides,
    /// and a lineup with no record anywhere counts as away.
    pub fn home_share(&self, members: &[u32], town: u32) -> f64 {
        if !self.on || town == u32::MAX {
            return 0.0;
        }
        let (mut hit, mut seen): (u32, u32) = (0, 0);
        for &p in members {
            if let Some(&(t, _)) = self.best.get(&p) {
                seen += 1;
                hit += (t == town) as u32;
            }
        }
        if seen == 0 {
            0.0
        } else {
            hit as f64 / seen as f64
        }
    }

    /// One field, once it has been forecast.
    pub fn absorb(&mut self, t: &Tournament) {
        if !self.on {
            return;
        }
        for res in &t.teams {
            if res.town == u32::MAX {
                continue;
            }
            for &p in &res.members {
                let c = self.cnt.entry(key(p, res.town)).or_insert(0);
                *c += 1;
                let c = *c;
                let b = self.best.entry(p).or_insert((res.town, 0));
                if c > b.1 {
                    *b = (res.town, c);
                }
            }
        }
    }
}

/// How much each player has already played: the in-person fields behind them in
/// the trailing 7 and 30 days. A championship weekend is often three fields, and
/// the question is whether the third one is worse than the first.
pub struct PlayerLoad {
    on: bool,
    days: HashMap<u32, Vec<i32>>,
}

impl PlayerLoad {
    pub fn new(on: bool) -> PlayerLoad {
        PlayerLoad {
            on,
            days: HashMap::new(),
        }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    /// `(mean over the lineup of the last 7 days, the same over 30)`.
    pub fn counts(&self, members: &[u32], day: i32) -> (f64, f64) {
        if !self.on || members.is_empty() {
            return (0.0, 0.0);
        }
        let (mut a, mut b) = (0.0, 0.0);
        for &p in members {
            if let Some(v) = self.days.get(&p) {
                a += v.iter().filter(|&&d| d >= day - WEEK && d < day).count() as f64;
                b += v.iter().filter(|&&d| d >= day - MONTH && d < day).count() as f64;
            }
        }
        let n = members.len() as f64;
        (a / n, b / n)
    }

    /// One in-person field, once it has been forecast. Days older than the
    /// longest window are dropped, so the store stays the size of a month.
    pub fn absorb(&mut self, t: &Tournament) {
        if !self.on {
            return;
        }
        for res in &t.teams {
            for &p in &res.members {
                let v = self.days.entry(p).or_default();
                v.retain(|&d| d > t.day - MONTH);
                v.push(t.day);
            }
        }
    }
}
