//! How a player's lineups have fared under an editor before, read by the
//! forecast alone.
//!
//! `question.aff_rank` learns an editor affinity from the **question gradients**,
//! and three rounds have now measured that and found it negative. The residual
//! check of `tools/q_mech.py` finds the effect in a different place: a lineup's
//! mean past residual under today's editors predicts its residual here at
//! +0.098 among the strongest twentieth of lineups and +0.003 among the weakest
//! two fifths. That is an **outcome** memory, not a likelihood one, and this is
//! the object that carries it.
//!
//! For every key the store holds the mean of `(forecast place − actual place) / n`
//! over the in-person fields behind it. A lineup's bonus is the mean of those
//! over its members and today's editors, counting only keys with enough
//! history, and it enters the forecast and nothing else — no team state, no
//! player strength and no question difficulty reads it.
//!
//! Round 4 adds three things the key can be: the `(player, editor)` pair of
//! round 3, the `(team, editor)` pair, and the player against the whole set of
//! a tournament's editors. It also adds a half-life, so an entry can be allowed
//! to forget.

use crate::config::EdKey;
use crate::data::Tournament;
use std::collections::HashMap;

/// One key's history: the sum of the residuals, how many there were, and the
/// day it was last touched, which is what a half-life is measured from.
#[derive(Clone, Copy)]
struct Entry {
    sum: f64,
    n: f64,
    last: i32,
}

pub struct EditorMemory {
    acc: HashMap<u64, Entry>,
    on: bool,
    /// Keys below this many past results are ignored: one result under an
    /// editor is noise, not a preference.
    min_n: f64,
    key: EdKey,
    /// Years after which an entry counts for half; 0 never forgets.
    half_life: f64,
}

fn key(a: u32, b: u32) -> u64 {
    ((a as u64) << 32) | b as u64
}

/// One number for a whole set of editors, so a package written by two people is
/// its own author. Order-independent and stable across runs.
fn set_id(editors: &[u32]) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut v: Vec<u32> = editors.to_vec();
    v.sort_unstable();
    v.dedup();
    for e in v {
        h ^= e as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (h ^ (h >> 32)) as u32
}

impl EditorMemory {
    pub fn new(on: bool, min_n: f64, key: EdKey, half_life: f64) -> EditorMemory {
        EditorMemory {
            acc: HashMap::new(),
            on,
            min_n,
            key,
            half_life,
        }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    /// What `day` leaves of an entry last touched at `last`.
    fn decay(&self, last: i32, day: i32) -> f64 {
        if self.half_life <= 0.0 {
            return 1.0;
        }
        let dt = ((day - last).max(0) as f64) / 365.25;
        0.5f64.powf(dt / self.half_life)
    }

    /// The keys one member of one lineup is read under.
    fn keys(&self, player: u32, team: u32, editors: &[u32], out: &mut Vec<u64>) {
        out.clear();
        match self.key {
            EdKey::PlayerEditor => out.extend(editors.iter().map(|&e| key(player, e))),
            EdKey::TeamEditor => out.extend(editors.iter().map(|&e| key(team, e))),
            EdKey::PlayerSet => out.push(key(player, set_id(editors))),
        }
    }

    /// Adds one member's keys to a running `(sum, count)`: one term per key that
    /// clears `min_n`, so the lineup's bonus is the flat mean over the pairs
    /// that have a history, which is round 3's rule.
    fn collect(&self, player: u32, team: u32, editors: &[u32], day: i32, acc: &mut (f64, u32)) {
        let mut ks = Vec::new();
        self.keys(player, team, editors, &mut ks);
        for k in ks {
            if let Some(a) = self.acc.get(&k) {
                if a.n * self.decay(a.last, day) >= self.min_n {
                    acc.0 += a.sum / a.n;
                    acc.1 += 1;
                }
            }
        }
    }

    /// One member's own mean past residual under these editors, or `None` when
    /// they have no history that clears `min_n`.
    pub fn player_bonus(&self, player: u32, team: u32, editors: &[u32], day: i32) -> Option<f64> {
        if !self.on || editors.is_empty() {
            return None;
        }
        let mut acc = (0.0, 0u32);
        self.collect(player, team, editors, day, &mut acc);
        (acc.1 > 0).then(|| acc.0 / acc.1 as f64)
    }

    /// The lineup's mean past residual under these editors; 0 when it has none.
    /// `gate` keeps a player out unless their strength is at least that, which is
    /// how "affinity is a property of a strong player" enters. Under a key that
    /// is not a property of a player the gate is read off the lineup's best
    /// member instead, because there is one memory for the whole table.
    pub fn bonus(
        &self,
        members: &[u32],
        team: u32,
        editors: &[u32],
        theta: &[f64],
        gate: f64,
        day: i32,
    ) -> f64 {
        if !self.on || editors.is_empty() {
            return 0.0;
        }
        if self.key == EdKey::TeamEditor {
            let best = theta.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if best < gate {
                return 0.0;
            }
            return self.player_bonus(0, team, editors, day).unwrap_or(0.0);
        }
        let mut acc = (0.0, 0u32);
        for (k, &p) in members.iter().enumerate() {
            if theta[k] < gate {
                continue;
            }
            self.collect(p, team, editors, day, &mut acc);
        }
        if acc.1 == 0 {
            0.0
        } else {
            acc.0 / acc.1 as f64
        }
    }

    /// The `(sum of past residuals, count)` of one `(player, editor)` pair, or
    /// `None` when the two have no history that clears `min_n`. The feature
    /// table of plan 8 wants the count as well as the mean, and wants it
    /// without the gate. The count is the raw one: the reader has no day to
    /// measure a half-life from, and at the shipped `ed_half_life` of 0 nothing
    /// is forgotten anyway.
    pub fn pair(&self, player: u32, editor: u32) -> Option<(f64, f64)> {
        self.acc
            .get(&key(player, editor))
            .filter(|a| a.n >= self.min_n)
            .map(|a| (a.sum, a.n))
    }

    /// Records one scored in-person field. `res` is `(team index in t.teams,
    /// residual)`; the residual joins the memory only after the field has been
    /// forecast, so a result never predicts itself.
    pub fn absorb(&mut self, t: &Tournament, res: &[(usize, f64)]) {
        if !self.on || t.editors.is_empty() {
            return;
        }
        let mut ks = Vec::new();
        for &(ri, r) in res {
            let Some(team) = t.teams.get(ri) else { continue };
            // Under the team key the whole result is one entry; under the two
            // player keys it is one per member.
            let members: &[u32] = match self.key {
                EdKey::TeamEditor => &[0],
                _ => &team.members,
            };
            for &p in members {
                self.keys(p, team.team, &t.editors, &mut ks);
                let half = self.half_life;
                for &k in &ks {
                    let a = self.acc.entry(k).or_insert(Entry {
                        sum: 0.0,
                        n: 0.0,
                        last: t.day,
                    });
                    let f = if half <= 0.0 {
                        1.0
                    } else {
                        0.5f64.powf(((t.day - a.last).max(0) as f64) / (365.25 * half))
                    };
                    a.sum *= f;
                    a.n *= f;
                    a.sum += r;
                    a.n += 1.0;
                    a.last = t.day;
                }
            }
        }
    }
}
