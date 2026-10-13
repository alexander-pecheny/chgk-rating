//! From the counted results to a team's rating on a day and the table of
//! every listed team.

use super::knobs::Knobs;
use super::lab::{ndcg, Lab};
use super::value::{fade, overlap, values, Val};
use std::collections::{BTreeMap, HashMap};

/// The display map is fitted over the table's top, and only with enough
/// teams; its power is held to a range so the scale stays monotone and tame.
const MAP_TOP: usize = 100;
const MAP_MIN_TEAMS: f64 = 5.0;
const MAP_POWER: (f64, f64) = (0.05, 1.0);

/// One row of the table: the team, its published rating, and the results it
/// is made of with their values on the day.
struct Row {
    team: i64,
    rating: f64,
    counted: Vec<(Val, f64)>,
}

/// A team's place in the table: the published rating and how many results make it.
pub struct Standing {
    pub team: i64,
    pub rating: f64,
    pub counted: usize,
}

/// Every counted result of every team under one set of knobs.
pub struct Book<'a> {
    pub(super) lab: &'a Lab,
    pub(super) k: Knobs,
    /// Ordered by team, so every table written from it comes out the same.
    pub(super) vals: BTreeMap<i64, Vec<Val>>,
}

impl Lab {
    /// Every result valued under `k`.
    pub fn book(&self, k: &Knobs) -> Book<'_> {
        Book { lab: self, k: k.clone(), vals: values(self, k) }
    }
}

impl Book<'_> {
    /// The team metric of the rating on the morning of each result, tune and test.
    pub fn ndcg(&self) -> (f64, f64) {
        ndcg(&self.lab.rows, |r| self.rating(r.team, r.day).0)
    }

    /// The published number for a rating: squeezed so the lower table spreads.
    pub(super) fn shown(&self, x: f64) -> f64 {
        let r = self.k.squeeze_ref;
        if x > 0.0 { r * (x / r).powf(self.k.squeeze) } else { x }
    }

    /// The rating on the morning of `day`, and the results that make it, each
    /// with its value on that day: faded, and scaled by the roster overlap.
    pub(super) fn rating(&self, team: i64, day: i32) -> (f64, Vec<(Val, f64)>) {
        let (x, c, _) = self.rating_slots(team, day);
        (x, c)
    }

    /// The rating, its results with their values on `day`, and the divisor: the
    /// slots they held, never fewer than `min_slots`. Under `freeze` the results
    /// are chosen and the slots counted on the day after the last result, so
    /// until the team plays again every value only fades and nothing is divided
    /// by less: the rating can only fall.
    pub(super) fn rating_slots(&self, team: i64, day: i32) -> (f64, Vec<(Val, f64)>, f64) {
        let Some(v) = self.vals.get(&team) else { return (0.0, Vec::new(), 1.0) };
        let Some(last) = v.iter().filter(|x| x.day < day).map(|x| x.day).max() else {
            return (0.0, Vec::new(), 1.0);
        };
        let k = &self.k;
        let mut cur = self.current(team, day, if k.freeze { (last + 1).min(day) } else { day });
        cur.sort_by(|a, b| (b.0.base * b.1).total_cmp(&(a.0.base * a.1)));
        cur.truncate(k.top_x);
        // A faded result holds only its remaining share of a slot, so an old
        // result near zero never drags the average down by taking a whole one.
        let held: f64 = cur.iter().map(|x| x.1).sum();
        let slots = if k.min_slots == 0 { k.top_x as f64 } else { held.clamp(k.min_slots.min(k.top_x) as f64, k.top_x as f64) };
        let out: Vec<(Val, f64)> = cur.into_iter().filter(|x| x.2 > 0.0).map(|x| (x.0, x.2)).collect();
        (out.iter().map(|x| x.1).sum::<f64>() / slots, out, slots)
    }

    /// The team's results before `day` that still hold a share of a slot on
    /// `t0`: (result, its share on t0, its value on day).
    fn current(&self, team: i64, day: i32, t0: i32) -> Vec<(Val, f64, f64)> {
        let (lab, k) = (self.lab, &self.k);
        let now = if k.overlap_k != 0.0 { lab.seasons.current(team, day) } else { None };
        let mut ov: HashMap<i64, f64> = HashMap::new();
        self.vals[&team]
            .iter()
            .filter(|x| x.day < day)
            .map(|x| {
                let o = *ov.entry(x.season).or_insert_with(|| overlap(lab, team, x.season, now, k));
                (*x, fade(t0 - x.day, x, k) * o, x.base * fade(day - x.day, x, k) * o)
            })
            .filter(|x| x.1 > 0.0)
            .collect()
    }

    /// Whether a team is in the table on `day`: a base roster for the season (or
    /// the last one, early in a season) and a counted result recent enough.
    pub(super) fn listed(&self, team: i64, day: i32) -> bool {
        let k = &self.k;
        self.lab.seasons.listed(team, day, k)
            && (k.active_days <= 0
                || self.vals.get(&team).is_some_and(|v| v.iter().any(|x| x.day < day && day - x.day <= k.active_days)))
    }

    /// Every listed team with a positive rating on `day`, best first.
    pub fn table(&self, day: i32) -> Vec<Standing> {
        let rows = self.rows(day);
        rows.into_iter().map(|r| Standing { team: r.team, rating: r.rating, counted: r.counted.len() }).collect()
    }

    /// The table with every row's results, published through the display map
    /// `a·x^p`.
    fn rows(&self, day: i32) -> Vec<Row> {
        let mut v: Vec<Row> = self
            .vals
            .keys()
            .filter(|&&t| self.listed(t, day))
            .map(|&t| {
                let (rating, counted) = self.rating(t, day);
                Row { team: t, rating, counted }
            })
            .filter(|x| x.rating > 0.0)
            .collect();
        v.sort_by(|a, b| b.rating.total_cmp(&a.rating).then(a.team.cmp(&b.team)));
        let (a, p) = if self.k.scale > 0.0 { (self.k.scale, 1.0) } else { display_map(&v) };
        for x in v.iter_mut() {
            x.rating = a * x.rating.powf(p);
        }
        v
    }
}

/// The display map makes the numbers read as strengths: a·x^p, fitted in
/// logs over the top 100 so each team's number sits near the mean strength
/// its counted results demonstrated. It is monotone, so the order stands.
fn display_map(v: &[Row]) -> (f64, f64) {
    let point = |r: &Row| {
        let d: Vec<f64> = r.counted.iter().map(|(val, _)| val.by_score).filter(|x| x.is_finite() && *x > 0.0).collect();
        (!d.is_empty() && r.rating > 0.0).then(|| (r.rating.ln(), (d.iter().sum::<f64>() / d.len() as f64).ln()))
    };
    let pts: Vec<(f64, f64)> = v.iter().take(MAP_TOP).filter_map(point).collect();
    let n = pts.len() as f64;
    if n < MAP_MIN_TEAMS {
        return (1.0, 1.0);
    }
    let (mx, my) = (pts.iter().map(|q| q.0).sum::<f64>() / n, pts.iter().map(|q| q.1).sum::<f64>() / n);
    let sxy: f64 = pts.iter().map(|q| (q.0 - mx) * (q.1 - my)).sum();
    let sxx: f64 = pts.iter().map(|q| (q.0 - mx) * (q.0 - mx)).sum();
    let p = if sxx > 0.0 { (sxy / sxx).clamp(MAP_POWER.0, MAP_POWER.1) } else { 1.0 };
    ((my - p * mx).exp(), p)
}
