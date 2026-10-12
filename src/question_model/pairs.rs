//! `player.pair_cov`, a prototype: one Kalman step per lineup on the lineup's
//! whole information matrix, with the covariance of every two players who
//! shared a lineup carried from one tournament to the next.
//!
//! The separate steps of `players.rs` read each shared game as evidence about
//! every member alone, so two partners who always play together both grow
//! certain while the split between them was never measured, and the split the
//! first games happened to leave stays. Here a shared game teaches the pair's
//! sum and leaves their difference uncertain, and a game one of them plays
//! apart moves that one by as much as the difference is still unknown.

use std::collections::HashMap;

use super::players::{drift, years, Players};
use crate::config::Config;
use crate::data::Tournament;

/// A stored covariance never claims more correlation than this.
const MAX_CORR: f64 = 0.95;
/// A pivot this small means the system cannot be solved.
const PIVOT_FLOOR: f64 = 1e-12;

/// Takes every lineup's joint step and returns each moved player's prior
/// variance, which the volatility update reads.
pub fn absorb(pl: &mut Players, t: &Tournament, grads: &[(u32, f64, f64)], blocks: &[(Vec<u32>, Vec<f64>)], cfg: &Config) -> HashMap<u32, f64> {
    let g: HashMap<u32, f64> = grads.iter().map(|&(i, g, _)| (i, g)).collect();
    let full = cfg.player.pair_cov == "full";
    let k = t.weight * cfg.player.info_scale;
    let mut priors = HashMap::new();
    for (members, f) in blocks {
        let n = members.len();
        let idx: Vec<usize> = (0..n).filter(|&a| g.contains_key(&members[a])).collect();
        let ids: Vec<u32> = idx.iter().map(|&a| members[a]).collect();
        let m = ids.len();
        if m == 0 {
            continue;
        }
        let mut p = vec![0.0; m * m];
        for (a, &i) in ids.iter().enumerate() {
            let s = &mut pl.state[i as usize];
            s.var += drift(s, years(s.last_day, t.day), cfg);
            p[a * m + a] = s.var;
            priors.insert(i, s.var);
        }
        if full {
            for a in 0..m {
                for b in a + 1..m {
                    let c = pl.cov[ids[a] as usize].get(&ids[b]).copied().unwrap_or(0.0);
                    let lim = MAX_CORR * (p[a * m + a] * p[b * m + b]).sqrt();
                    p[a * m + b] = c.clamp(-lim, lim);
                    p[b * m + a] = p[a * m + b];
                }
            }
        }
        let mut j = vec![0.0; m * m];
        for a in 0..m {
            for b in 0..m {
                if full || a == b {
                    j[a * m + b] = k * f[idx[a] * n + idx[b]];
                }
            }
            j[a * m + a] = j[a * m + a].max(cfg.numeric.info_floor);
        }
        let (p, post) = match posterior(&p, &j, m) {
            Some(post) => (p, post),
            // An unsolvable block falls back to separate steps.
            None => {
                let diag = |x: &[f64]| (0..m * m).map(|e| if e / m == e % m { x[e] } else { 0.0 }).collect::<Vec<f64>>();
                let (pd, jd) = (diag(&p), diag(&j));
                let post = posterior(&pd, &jd, m).expect("a diagonal block always solves");
                (pd, post)
            }
        };
        let steps = fair_steps(&ids, &post, &g, k, cfg.numeric.theta_max_step);
        for (a, &i) in ids.iter().enumerate() {
            let s = &mut pl.state[i as usize];
            s.theta_raw += steps[a];
            s.var = post[a * m + a];
        }
        if full {
            carry(pl, &ids, &p, &post, cfg.player.pair_prune);
        }
    }
    priors
}

/// Each member's step `P' k g` under the fair rule: a member moves only the
/// way their own steered gradient points. What the rule clips off is handed
/// to the members it lets move, in proportion to their steps, so the lineup's
/// total moves as the joint step says. Without that, a partner the joint step
/// would raise after a loss is held still while the other takes the whole
/// drop, and the split drifts one way game after game.
fn fair_steps(ids: &[u32], post: &[f64], g: &HashMap<u32, f64>, k: f64, max: f64) -> Vec<f64> {
    let m = ids.len();
    let raw: Vec<f64> = (0..m).map(|a| (0..m).map(|b| post[a * m + b] * k * g[&ids[b]]).sum()).collect();
    let mut steps: Vec<f64> = ids
        .iter()
        .zip(&raw)
        .map(|(i, &s)| match g[i] {
            gi if gi > 0.0 => s.max(0.0),
            gi if gi < 0.0 => s.min(0.0),
            _ => 0.0,
        })
        .collect();
    let clipped: f64 = raw.iter().sum::<f64>() - steps.iter().sum::<f64>();
    let moving: f64 = steps.iter().map(|s| s.abs()).sum();
    if moving > 0.0 {
        // The clipped share goes the way the moving members already move, or
        // shrinks their steps, never past zero.
        for s in &mut steps {
            let share = clipped * s.abs() / moving;
            let next = *s + share;
            *s = if next.signum() == s.signum() { next } else { 0.0 };
        }
    }
    steps.iter().map(|s| s.clamp(-max, max)).collect()
}

/// The posterior covariance `(I + P J)⁻¹ P`, symmetrised; `None` when the
/// system is singular or a variance comes out not positive or larger than
/// its prior.
fn posterior(p: &[f64], j: &[f64], m: usize) -> Option<Vec<f64>> {
    let mut lhs = vec![0.0; m * m];
    for a in 0..m {
        for b in 0..m {
            lhs[a * m + b] = (a == b) as u8 as f64 + (0..m).map(|c| p[a * m + c] * j[c * m + b]).sum::<f64>();
        }
    }
    let mut x = solve(lhs, p.to_vec(), m)?;
    for a in 0..m {
        for b in a + 1..m {
            let c = (x[a * m + b] + x[b * m + a]) / 2.0;
            x[a * m + b] = c;
            x[b * m + a] = c;
        }
        if !(x[a * m + a] > 0.0 && x[a * m + a] <= p[a * m + a] * (1.0 + 1e-9)) {
            return None;
        }
    }
    Some(x)
}

/// Stores the lineup's new covariances, and carries each member's
/// covariance with a partner who did not play through the step: a
/// partner's covariance row `c` against the lineup becomes `c A⁻¹ A'`.
/// The partner's own strength and variance stay where they are, so no
/// published number moves for a game its player did not play.
fn carry(pl: &mut Players, ids: &[u32], prior: &[f64], post: &[f64], prune: f64) {
    let m = ids.len();
    let inside: HashMap<u32, usize> = ids.iter().enumerate().map(|(a, &i)| (i, a)).collect();
    let mut rows: HashMap<u32, Vec<f64>> = HashMap::new();
    for (a, &i) in ids.iter().enumerate() {
        for (&o, &c) in &pl.cov[i as usize] {
            if !inside.contains_key(&o) {
                rows.entry(o).or_insert_with(|| vec![0.0; m])[a] = c;
            }
        }
    }
    let gain = if rows.is_empty() { None } else { solve(prior.to_vec(), post.to_vec(), m) };
    for &i in ids {
        let old: Vec<u32> = pl.cov[i as usize].keys().copied().collect();
        for o in old {
            pl.cov[o as usize].remove(&i);
        }
        pl.cov[i as usize].clear();
    }
    for a in 0..m {
        for b in a + 1..m {
            set(pl, ids[a], ids[b], post[a * m + b], prune);
        }
    }
    let Some(gain) = gain else { return };
    for (o, row) in rows {
        for b in 0..m {
            let c: f64 = (0..m).map(|a| row[a] * gain[a * m + b]).sum();
            set(pl, o, ids[b], c, prune);
        }
    }
}

/// Stores `cov(x, y) = c` on both sides when its correlation reaches `prune`.
fn set(pl: &mut Players, x: u32, y: u32, c: f64, prune: f64) {
    let corr = c / (pl.state[x as usize].var * pl.state[y as usize].var).sqrt();
    if corr.abs() >= prune && corr.is_finite() {
        pl.cov[x as usize].insert(y, c);
        pl.cov[y as usize].insert(x, c);
    }
}

/// Solves `A X = B` for square row-major `A` and `B` by elimination with
/// partial pivoting.
fn solve(mut a: Vec<f64>, mut b: Vec<f64>, m: usize) -> Option<Vec<f64>> {
    for col in 0..m {
        let piv = (col..m).max_by(|&x, &y| a[x * m + col].abs().total_cmp(&a[y * m + col].abs()))?;
        if a[piv * m + col].abs() < PIVOT_FLOOR {
            return None;
        }
        for c in 0..m {
            a.swap(col * m + c, piv * m + c);
            b.swap(col * m + c, piv * m + c);
        }
        for r in (0..m).filter(|&r| r != col) {
            let f = a[r * m + col] / a[col * m + col];
            if f == 0.0 {
                continue;
            }
            for c in 0..m {
                a[r * m + c] -= f * a[col * m + c];
                b[r * m + c] -= f * b[col * m + c];
            }
        }
    }
    for r in 0..m {
        let d = a[r * m + r];
        for c in 0..m {
            b[r * m + c] /= d;
        }
    }
    Some(b)
}
