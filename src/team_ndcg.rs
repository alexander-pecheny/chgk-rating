//! Does the team number alone order an in-person field of base-roster teams?
//!
//! The metric of DESIGN 7.3: rank the qualifying teams of a tournament by the
//! rating as it stood that morning and score the order with NDCG, relevance
//! `1 / place` among the qualifying teams re-ranked. The headline is the mean
//! over tournaments weighted by `data/tournament_weights.csv`, so the fields
//! with the best teams count for more than a school event.
//!
//! `tools/team_ndcg.py` is the same arithmetic over a finished database, and
//! the two agree to the last digit. The qualifying set itself is that tool's
//! output: `out/team_ndcg/qualify.csv`.

pub use crate::ndcg::{weights, TEST, TUNE, WEIGHTS};
use crate::ndcg::ndcg_at;
use crate::util::{avg_ranks, csv_rows};
use std::collections::{HashMap, HashSet};

pub const QUALIFY: &str = "out/team_ndcg/qualify.csv";
/// `(tournament_id, team_id)` of every result that turned up in its own base
/// roster, by the MAII rule of Prilozhenie A.4.7.
pub fn qualify(path: &str) -> HashSet<(i64, i64)> {
    csv_rows(path)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| Some((r.first()?.parse().ok()?, r.get(1)?.parse().ok()?)))
        .collect()
}

/// A deterministic tie-break, the same number in `tools/team_ndcg.py`.
pub fn tiekey(tid: i64, team: i64) -> u64 {
    let mut x = (tid as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(team as u64);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// One tournament: NDCG of the order by `mu` against `place`, cut at `k`. None
/// when fewer than five teams qualify.
pub fn score(tid: i64, team: &[i64], mu: &[f64], place: &[f64], k: usize) -> Option<f64> {
    if team.len() < 5 {
        return None;
    }
    let ranks = avg_ranks(place);
    let mut ord: Vec<usize> = (0..team.len()).collect();
    ord.sort_by(|&a, &b| {
        mu[b]
            .total_cmp(&mu[a])
            .then(tiekey(tid, team[a]).cmp(&tiekey(tid, team[b])))
    });
    let rel: Vec<f64> = ord.iter().map(|&i| 1.0 / ranks[i]).collect();
    let v = ndcg_at(&rel, k);
    v.is_finite().then_some(v)
}

pub fn wmean(d: &HashMap<i64, f64>, w: &HashMap<i64, f64>) -> f64 {
    let (mut num, mut den) = (0.0, 0.0);
    for (t, v) in d {
        let x = *w.get(t).unwrap_or(&0.0);
        num += x * v;
        den += x;
    }
    if den > 0.0 {
        num / den
    } else {
        f64::NAN
    }
}

pub fn umean(d: &HashMap<i64, f64>) -> f64 {
    if d.is_empty() {
        f64::NAN
    } else {
        d.values().sum::<f64>() / d.len() as f64
    }
}
