//! Top-weighted forecast quality: NDCG of the forecast order against the actual
//! places, the metric plan 3 tunes on. Relevance is `1/place`, so the first
//! places carry almost all the weight and a swap deep in the field is free.
//!
//! The headline number is the mean over tournaments weighted by
//! `data/tournament_weights.csv`: how strong the field's ten best lineups were
//! under the shipped model, less the 10th percentile of that, clipped at zero.
//! The weights are frozen, so every candidate and every external system is
//! scored on the same footing. `tools/ndcg.py` is the same metric in Python.

use crate::util::Rng;
use rusqlite::Connection;
use std::collections::HashMap;

/// The two windows of plan 3: every decision is made on `TUNE`, `TEST` is read
/// once per candidate.
pub const TUNE: (&str, &str) = ("2018-09-01", "2024-08-29");
pub const TEST: (&str, &str) = ("2024-08-30", "2026-08-27");
pub const WEIGHTS: &str = "data/tournament_weights.csv";

/// One tournament's score: NDCG over the whole field and over the top three.
#[derive(Debug, Clone, Copy)]
pub struct Scored {
    pub tid: i64,
    pub ndcg: f64,
    pub ndcg3: f64,
}

fn ndcg_at(rel: &[f64], k: usize) -> f64 {
    let k = k.min(rel.len());
    let dcg: f64 = rel[..k]
        .iter()
        .enumerate()
        .map(|(i, r)| r / ((i + 2) as f64).log2())
        .sum();
    let mut best = rel.to_vec();
    best.sort_by(|a, b| b.total_cmp(a));
    let idcg: f64 = best[..k]
        .iter()
        .enumerate()
        .map(|(i, r)| r / ((i + 2) as f64).log2())
        .sum();
    if idcg > 0.0 {
        dcg / idcg
    } else {
        f64::NAN
    }
}

/// One field's score from (forecast, actual place) pairs; `None` under five
/// teams. The parameter sweep scores its replays with this same function.
pub fn score_field(tid: i64, field: &[(f64, f64)]) -> Option<Scored> {
    if field.len() < 5 {
        return None;
    }
    let mut ord: Vec<usize> = (0..field.len()).collect();
    let mut rng = Rng::new(tid as u64);
    for i in (1..ord.len()).rev() {
        ord.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
    }
    ord.sort_by(|&a, &b| field[b].0.total_cmp(&field[a].0));
    let rel: Vec<f64> = ord.iter().map(|&i| 1.0 / field[i].1).collect();
    Some(Scored {
        tid,
        ndcg: ndcg_at(&rel, rel.len()),
        ndcg3: ndcg_at(&rel, 3),
    })
}

/// Per-tournament NDCG of the `forecast` column over in-person tournaments with
/// at least five teams. Ties in the forecast are broken by a shuffle seeded from
/// the tournament id, so the stored finishing order never leaks into the rank.
pub fn per_tournament(conn: &Connection, from: &str, to: &str) -> rusqlite::Result<Vec<Scored>> {
    let mut st = conn.prepare(
        "SELECT e.tournament_id, e.forecast, e.actual_place
         FROM team_events e JOIN tournament_metrics m ON m.tournament_id = e.tournament_id
         WHERE m.type = 'in-person' AND m.date >= ?1 AND m.date < ?2
           AND e.actual_place > 0 AND e.forecast IS NOT NULL
         ORDER BY e.tournament_id",
    )?;
    let mut rows = st.query([from, to])?;
    let mut out = Vec::new();
    let mut cur = i64::MIN;
    let mut field: Vec<(f64, f64)> = Vec::new();
    let flush = |tid: i64, field: &mut Vec<(f64, f64)>, out: &mut Vec<Scored>| {
        out.extend(score_field(tid, field));
        field.clear();
    };
    while let Some(r) = rows.next()? {
        let tid: i64 = r.get(0)?;
        if tid != cur {
            if cur != i64::MIN {
                flush(cur, &mut field, &mut out);
            }
            cur = tid;
        }
        field.push((r.get(1)?, r.get(2)?));
    }
    if cur != i64::MIN {
        flush(cur, &mut field, &mut out);
    }
    out.retain(|s| s.ndcg.is_finite() && s.ndcg3.is_finite());
    Ok(out)
}

/// The frozen per-tournament weights. An absent file leaves every weight at 1.
pub fn weights(path: &str) -> HashMap<i64, f64> {
    crate::util::csv_rows(path)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| Some((r.first()?.trim().parse().ok()?, r.get(3)?.trim().parse().ok()?)))
        .collect()
}

/// Weighted mean of `f` over the scored tournaments. A tournament missing from
/// the weight table weighs nothing, which is also what a bottom-decile field does.
pub fn mean(v: &[Scored], w: Option<&HashMap<i64, f64>>, f: impl Fn(&Scored) -> f64) -> f64 {
    let (mut num, mut den) = (0.0, 0.0);
    for s in v {
        let wt = w.map_or(1.0, |m| m.get(&s.tid).copied().unwrap_or(0.0));
        num += wt * f(s);
        den += wt;
    }
    if den > 0.0 {
        num / den
    } else {
        f64::NAN
    }
}

/// Both numbers of one window: `(NDCG, NDCG@3)`.
pub fn summary(v: &[Scored], w: Option<&HashMap<i64, f64>>) -> (f64, f64) {
    (mean(v, w, |s| s.ndcg), mean(v, w, |s| s.ndcg3))
}

/// Paired bootstrap over the tournaments both databases forecast: the standard
/// error of the difference of the weighted means, from `n` resamples.
pub fn paired_se(
    a: &[Scored],
    b: &[Scored],
    w: Option<&HashMap<i64, f64>>,
    n: usize,
    f: impl Fn(&Scored) -> f64,
) -> (f64, f64) {
    let bi: HashMap<i64, &Scored> = b.iter().map(|s| (s.tid, s)).collect();
    let pairs: Vec<(f64, f64, f64)> = a
        .iter()
        .filter_map(|s| {
            let o = bi.get(&s.tid)?;
            let wt = w.map_or(1.0, |m| m.get(&s.tid).copied().unwrap_or(0.0));
            Some((wt, f(s), f(o)))
        })
        .collect();
    let diff = |idx: &[usize]| -> f64 {
        let (mut num, mut den) = (0.0, 0.0);
        for &i in idx {
            num += pairs[i].0 * (pairs[i].1 - pairs[i].2);
            den += pairs[i].0;
        }
        if den > 0.0 {
            num / den
        } else {
            f64::NAN
        }
    };
    let all: Vec<usize> = (0..pairs.len()).collect();
    let point = diff(&all);
    if pairs.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    let mut rng = Rng::new(0x_1CDC_2025);
    let (mut s1, mut s2) = (0.0, 0.0);
    let mut idx = vec![0usize; pairs.len()];
    for _ in 0..n {
        for x in idx.iter_mut() {
            *x = (rng.next_u64() % pairs.len() as u64) as usize;
        }
        let d = diff(&idx);
        s1 += d;
        s2 += d * d;
    }
    let m = s1 / n as f64;
    ((s2 / n as f64 - m * m).max(0.0).sqrt(), point)
}
