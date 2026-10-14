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

/// A field is scored from five teams up, and NDCG@3 over its top three.
pub const MIN_FIELD: usize = 5;
const TOP: usize = 3;
/// The weight column of `data/tournament_weights.csv`.
const WEIGHT_COL: usize = 3;
const BOOTSTRAP_SEED: u64 = 0x_1CDC_2025;

/// One tournament's score: NDCG over the whole field and over the top three.
#[derive(Debug, Clone, Copy)]
pub struct Scored {
    pub tid: i64,
    pub ndcg: f64,
    pub ndcg3: f64,
}

/// DCG of `rel` in the order given over the ideal order, cut at `k`, with the
/// usual `1 / log2(rank + 1)` discount; NaN when nothing is relevant.
pub fn ndcg_at(rel: &[f64], k: usize) -> f64 {
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
    if field.len() < MIN_FIELD {
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
        ndcg3: ndcg_at(&rel, TOP),
    })
}

/// The six measures of /method/ on one field, as `tools/rival_metrics.py`
/// computes them: NDCG, NDCG@3 and NDCG@10 with relevance
/// `1/(|position - place| + 1)`, Pearson's r of the forecast rank against the
/// place, and the pairwise score over every pair and over the best tenth of the
/// field (at least five teams). `field` is (forecast, higher better; actual
/// place); `None` under five teams, NaN where a measure is undefined.
pub fn method_metrics(tid: i64, field: &[(f64, f64)]) -> Option<[f64; 6]> {
    if field.len() < MIN_FIELD {
        return None;
    }
    let mut ord: Vec<usize> = (0..field.len()).collect();
    let mut rng = Rng::new(tid as u64);
    for i in (1..ord.len()).rev() {
        ord.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
    }
    ord.sort_by(|&a, &b| field[b].0.total_cmp(&field[a].0));
    let place = |i: usize| field[i].1;
    let hits = |k: usize| {
        let dcg = |o: &[usize]| -> f64 {
            o.iter().take(k).enumerate().map(|(p, &i)| 1.0 / (((p + 1) as f64 - place(i)).abs() + 1.0) / ((p + 2) as f64).log2()).sum()
        };
        let mut ideal = ord.clone();
        ideal.sort_by(|&a, &b| place(a).total_cmp(&place(b)));
        dcg(&ord) / dcg(&ideal)
    };
    let all: Vec<usize> = (0..field.len()).collect();
    let mut places: Vec<f64> = all.iter().map(|&i| place(i)).collect();
    places.sort_by(f64::total_cmp);
    let cut = places[(field.len() as f64 * TOP_SHARE).ceil().max(MIN_FIELD as f64) as usize - 1];
    let best: Vec<usize> = all.iter().copied().filter(|&i| place(i) <= cut).collect();
    let fplace = ranks(&all.iter().map(|&i| -field[i].0).collect::<Vec<_>>());
    Some([
        hits(field.len()),
        hits(TOP),
        hits(TOP10),
        pearson(&fplace, &all.iter().map(|&i| place(i)).collect::<Vec<_>>()),
        pairwise(field, &all),
        pairwise(field, &best),
    ])
}

/// The best tenth of a field the pairwise score reads, and the NDCG cut of /method/.
const TOP_SHARE: f64 = 0.1;
const TOP10: usize = 10;

/// 1-based ranks of `v` ascending, ties sharing the mean rank.
fn ranks(v: &[f64]) -> Vec<f64> {
    let mut ord: Vec<usize> = (0..v.len()).collect();
    ord.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    let mut r = vec![0.0; v.len()];
    let mut i = 0;
    while i < ord.len() {
        let mut j = i;
        while j + 1 < ord.len() && v[ord[j + 1]] == v[ord[i]] {
            j += 1;
        }
        for &t in &ord[i..=j] {
            r[t] = (i + j) as f64 / 2.0 + 1.0;
        }
        i = j + 1;
    }
    r
}

fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let sxy: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
    let sxx: f64 = x.iter().map(|a| (a - mx).powi(2)).sum();
    let syy: f64 = y.iter().map(|b| (b - my).powi(2)).sum();
    if sxx > 0.0 && syy > 0.0 { sxy / (sxx * syy).sqrt() } else { f64::NAN }
}

/// Share of the maximum over the pairs of `teams`: 1 for a pair ordered as the
/// result, a tie included, 0.5 when exactly one side ties.
fn pairwise(field: &[(f64, f64)], teams: &[usize]) -> f64 {
    let sign = |x: f64| (x > 0.0) as i8 - (x < 0.0) as i8;
    let (mut got, mut n) = (0.0, 0);
    for (k, &a) in teams.iter().enumerate() {
        for &b in &teams[k + 1..] {
            let (f, r) = (sign(field[a].0 - field[b].0), sign(field[b].1 - field[a].1));
            got += if f == r { 1.0 } else if f == 0 || r == 0 { 0.5 } else { 0.0 };
            n += 1;
        }
    }
    if n > 0 { got / n as f64 } else { f64::NAN }
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
        .filter_map(|r| Some((r.first()?.trim().parse().ok()?, r.get(WEIGHT_COL)?.trim().parse().ok()?)))
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
    let mut rng = Rng::new(BOOTSTRAP_SEED);
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
