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
//! For every `(player, editor)` pair the store holds the mean of
//! `(forecast place − actual place) / n` over the in-person fields that player
//! has played under that editor. A lineup's bonus is the mean of those over its
//! members and today's editors, counting only pairs with enough history, and it
//! enters the forecast and nothing else — no team state, no player strength and
//! no question difficulty reads it.

use crate::data::Tournament;
use std::collections::HashMap;

pub struct EditorMemory {
    acc: HashMap<u64, (f64, f64)>,
    on: bool,
    /// Pairs below this many past results are ignored: one result under an
    /// editor is noise, not a preference.
    min_n: f64,
}

fn key(player: u32, editor: u32) -> u64 {
    ((player as u64) << 32) | editor as u64
}

impl EditorMemory {
    pub fn new(on: bool, min_n: f64) -> EditorMemory {
        EditorMemory {
            acc: HashMap::new(),
            on,
            min_n,
        }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    /// The lineup's mean past residual under these editors; 0 when it has none.
    /// `gate` keeps a player out unless their strength is at least that, which is
    /// how "affinity is a property of a strong player" enters.
    pub fn bonus(&self, members: &[u32], editors: &[u32], theta: &[f64], gate: f64) -> f64 {
        if !self.on || editors.is_empty() {
            return 0.0;
        }
        let mut sum = 0.0;
        let mut n = 0u32;
        for (k, &p) in members.iter().enumerate() {
            if theta[k] < gate {
                continue;
            }
            for &e in editors {
                if let Some(&(s, c)) = self.acc.get(&key(p, e)) {
                    if c >= self.min_n {
                        sum += s / c;
                        n += 1;
                    }
                }
            }
        }
        if n == 0 {
            0.0
        } else {
            sum / n as f64
        }
    }

    /// Records one scored in-person field. `res` is `(team index in t.teams,
    /// residual)`; the residual joins the memory only after the field has been
    /// forecast, so a result never predicts itself.
    pub fn absorb(&mut self, t: &Tournament, res: &[(usize, f64)]) {
        if !self.on || t.editors.is_empty() {
            return;
        }
        for &(ri, r) in res {
            let Some(team) = t.teams.get(ri) else { continue };
            for &p in &team.members {
                for &e in &t.editors {
                    let a = self.acc.entry(key(p, e)).or_insert((0.0, 0.0));
                    a.0 += r;
                    a.1 += 1.0;
                }
            }
        }
    }
}
