//! The statistics behind both anomaly tools: the Markdown report of the
//! `anomaly` binary and the `/anomaly/` table `webindex` writes into web.db.
//! Each caller does its own reading and printing; the arithmetic is here.
//!
//! The two callers still apply different rules in a few places, kept as they
//! were and named where they differ:
//!
//! * an empty [`Acc`] has a NaN mean for the report and a zero mean for the
//!   index ([`Acc::mean`] against [`Acc::mean_or_zero`]);
//! * the report's standard error ([`Acc::se`]) gives fewer than three results
//!   the population's spread and does not cap the sample size; the index's
//!   ([`Acc::var_mean`]) does so below four and caps it at [`NU`];
//! * a player's profile builds its thirds from every question anybody answered
//!   ([`PROFILE_THIRDS`]), the scores only from well-answered ones
//!   ([`SCORE_THIRDS`]).

use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// Fewer in-person results than this and the comparison is against zero.
pub const MIN_IN_PERSON: f64 = 5.0;
/// Effective sample size ceiling. A player's results are not independent draws:
/// they are the same handful of teammates against the same circuit, week after
/// week, so the four hundredth result adds almost nothing the fiftieth did not.
/// DESIGN §4.3 caps the pairwise stage the same way and for the same reason.
pub const NU: f64 = 30.0;
/// A spread is never taken below this share of the population's.
const SD_FLOOR_SHARE: f64 = 0.5;
/// Fewer results than this and the report's standard error uses the
/// population's spread.
const FEW_REPORT: f64 = 3.0;
/// The same for the index, whose results are standardised to unit spread.
const FEW_INDEX: f64 = 4.0;

/// A mask shorter than this is no package.
pub const MIN_MASK: usize = 12;
/// Fewer teams than this and the field's take rate is no difficulty scale.
pub const MIN_TEAMS: usize = 15;
const THIRDS: usize = 3;
/// Easy-third edge cells: the edge in fiftieths, from −0.70 to +0.30.
const EDGE_STEPS: f64 = 50.0;
pub const EDGE_LO: i64 = -35;
const EDGE_HI: i64 = 15;

/// Fewer results in person than this, and a player adds nothing to the spread
/// of in-person means.
const BASELINE_MIN: f64 = 10.0;
/// Floors that keep a variance or a spread off zero.
const TINY: f64 = 1e-9;
const VAR_FLOOR: f64 = 1e-12;

/// Where EM starts: the bulk at half the excess spread, the rest at three
/// times it, holding a twentieth of the players.
const START_SPIKE: f64 = 0.25;
const START_SLAB: f64 = 9.0;
const START_SHARE: f64 = 0.05;
const EM_ROUNDS: usize = 200;
/// The second component's share stays within these.
const SHARE_MIN: f64 = 1e-4;
const SHARE_MAX: f64 = 0.5;
const HALF: f64 = 0.5;

/// Count, sum and sum of squares of a stream of values.
#[derive(Default, Clone, Copy)]
pub struct Acc {
    pub n: f64,
    pub sum: f64,
    pub sum2: f64,
}

impl Acc {
    pub fn push(&mut self, x: f64) {
        self.n += 1.0;
        self.sum += x;
        self.sum2 += x * x;
    }

    /// The mean, NaN when nothing was pushed: the report's rule.
    pub fn mean(&self) -> f64 {
        self.sum / self.n
    }

    /// The mean, zero when nothing was pushed: the index's rule, under which a
    /// player with no results of a kind sits at the population's centre.
    pub fn mean_or_zero(&self) -> f64 {
        if self.n > 0.0 {
            self.mean()
        } else {
            0.0
        }
    }

    pub fn sd(&self) -> f64 {
        if self.n < 2.0 {
            return f64::NAN;
        }
        (self.sum2 / self.n - self.mean() * self.mean()).max(0.0).sqrt()
    }

    /// The spread a mean's error is computed from: the population's for fewer
    /// than `few` results, else the own one floored at half the population's,
    /// so a player with a handful of results cannot look certain.
    fn floored_sd(&self, pop_sd: f64, few: f64) -> f64 {
        if self.n < few {
            pop_sd
        } else {
            self.sd().max(pop_sd * SD_FLOOR_SHARE)
        }
    }

    /// Standard error of the mean, as the report computes it.
    pub fn se(&self, pop_sd: f64) -> f64 {
        if self.n < 1.0 {
            return f64::INFINITY;
        }
        self.floored_sd(pop_sd, FEW_REPORT) / self.n.sqrt()
    }

    pub fn t(&self, pop_sd: f64) -> f64 {
        self.mean() / self.se(pop_sd)
    }

    /// Squared standard error of a standardised mean, as the index computes it:
    /// unit spread for fewer than four results and the sample capped at [`NU`].
    pub fn var_mean(&self) -> f64 {
        if self.n < 1.0 {
            return 1.0;
        }
        let sd = self.floored_sd(1.0, FEW_INDEX);
        sd * sd / self.n.min(NU)
    }
}

/// The t statistic of the difference between two means.
pub fn split_t(a: &Acc, b: &Acc, pop_sd: f64) -> f64 {
    (a.mean() - b.mean()) / (a.se(pop_sd).powi(2) + b.se(pop_sd).powi(2)).sqrt()
}

/// Every value pushed into the cell its key names.
pub fn cell_means<K: Hash + Eq>(items: impl Iterator<Item = (K, f64)>) -> HashMap<K, Acc> {
    let mut cells: HashMap<K, Acc> = HashMap::new();
    for (k, v) in items {
        cells.entry(k).or_default().push(v);
    }
    cells
}

/// The spread of values about the means of their cells.
pub fn centred_sd<K: Hash + Eq>(items: impl Iterator<Item = (K, f64)>, cells: &HashMap<K, Acc>) -> f64 {
    let mut all = Acc::default();
    for (k, v) in items {
        all.push(v - cells[&k].mean());
    }
    all.sd()
}

/// `ln(forecast place) − ln(actual place)`: how far a team finished above the
/// place its lineup's ratings predicted.
pub fn surprise(forecast: f64, actual: f64) -> Option<f64> {
    (forecast > 0.0 && actual > 0.0).then(|| forecast.ln() - actual.ln())
}

/// Hands every result in `results` to each player the roster seats at its
/// table, as (player, result).
pub fn attribute<V>(roster: &[(i64, i64, i64)], results: &HashMap<(i64, i64), V>, mut f: impl FnMut(i64, &V)) {
    for (pid, tid, team) in roster {
        if let Some(v) = results.get(&(*tid, *team)) {
            f(*pid, v);
        }
    }
}

/// (player, tournament, team) rows of `sql`, read once.
pub fn roster(db: &Connection, sql: &str) -> Vec<(i64, i64, i64)> {
    let mut st = db.prepare(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

// ── the mask test ────────────────────────────────────────────────────────────

/// Each team's answers, by tournament.
pub type Masks = BTreeMap<i64, Vec<(i64, Vec<u8>)>>;
/// Per question: teams that took it, teams that answered at all.
pub type Take = Vec<(f64, f64)>;

/// Every mask of [`MIN_MASK`] questions or more in buff.db, of the
/// tournaments `keep` names.
pub fn read_masks<T>(b: &Connection, keep: &HashMap<i64, T>) -> Masks {
    let mut by_tour = Masks::new();
    let mut st = b
        .prepare("select id, team_id, mask from tournament_results where mask is not null")
        .unwrap();
    let mut rows = st.query([]).unwrap();
    while let Some(x) = rows.next().unwrap() {
        let Ok(tid) = x.get::<_, i64>(0) else { continue };
        if !keep.contains_key(&tid) {
            continue;
        }
        let m: String = x.get::<_, Option<String>>(2).unwrap().unwrap_or_default();
        if m.len() >= MIN_MASK {
            by_tour.entry(tid).or_default().push((x.get(1).unwrap_or(0), m.into_bytes()));
        }
    }
    by_tour
}

/// The field's take rate on every question: the difficulty scale, no model
/// needed.
pub fn take_rates<'a>(masks: impl Iterator<Item = &'a [u8]> + Clone) -> Take {
    let n_q = masks.clone().map(|m| m.len()).max().unwrap_or(0);
    let mut take = vec![(0.0f64, 0.0f64); n_q];
    for (q, c) in masks.flat_map(|m| m.iter().enumerate()) {
        match c {
            b'1' => {
                take[q].0 += 1.0;
                take[q].1 += 1.0;
            }
            b'0' => take[q].1 += 1.0,
            _ => {}
        }
    }
    take
}

/// Which questions and teams a package's thirds are built from.
pub struct Rule {
    /// A question answered by fewer teams than this has no take rate.
    pub min_answered: f64,
    /// Fewer usable questions than this and the package has no thirds.
    pub min_questions: usize,
    /// A team that saw fewer questions of a third than this has no edge on it.
    pub min_seen: f64,
}

/// The scores': well-answered questions and teams that saw enough of them.
pub const SCORE_THIRDS: Rule = Rule { min_answered: 10.0, min_questions: 12, min_seen: 6.0 };
/// A profile's: every question anybody answered, every team that answered one.
pub const PROFILE_THIRDS: Rule = Rule { min_answered: 1.0, min_questions: THIRDS, min_seen: 1.0 };

/// A package ranked by the field's take rate.
pub struct Package {
    take: Take,
    /// The usable questions, hardest first.
    order: Vec<usize>,
}

impl Package {
    /// Nothing when too few questions are usable.
    pub fn new(take: Take, rule: &Rule) -> Option<Package> {
        let mut order: Vec<usize> = (0..take.len()).filter(|&q| take[q].1 >= rule.min_answered).collect();
        if order.len() < rule.min_questions {
            return None;
        }
        order.sort_by(|&a, &b| (take[a].0 / take[a].1).total_cmp(&(take[b].0 / take[b].1)));
        Some(Package { take, order })
    }

    /// How far a team's take rate sits above the field's on the easiest and
    /// the hardest third, as (easy, hard).
    pub fn edges(&self, m: &[u8], rule: &Rule) -> Option<(f64, f64)> {
        let third = self.order.len() / THIRDS;
        let hard = self.edge(m, &self.order[..third], rule)?;
        let easy = self.edge(m, &self.order[self.order.len() - third..], rule)?;
        Some((easy, hard))
    }

    /// The team's take rate on `qs` less the field's, per question it answered.
    fn edge(&self, m: &[u8], qs: &[usize], rule: &Rule) -> Option<f64> {
        let (mut me, mut field, mut n) = (0.0, 0.0, 0.0);
        for &q in qs {
            match m.get(q) {
                Some(b'1') => me += 1.0,
                Some(b'0') => {}
                _ => continue,
            }
            field += self.take[q].0 / self.take[q].1;
            n += 1.0;
        }
        (n >= rule.min_seen).then(|| (me - field) / n)
    }
}

/// One team's result on the mask test.
pub struct MaskEdge {
    pub tid: i64,
    pub team: i64,
    pub easy: f64,
    pub hard: f64,
}

/// Both edges of every team of every package fit for [`SCORE_THIRDS`].
pub fn mask_edges(masks: &Masks) -> Vec<MaskEdge> {
    let mut out = Vec::new();
    for (tid, rows) in masks.iter().filter(|(_, rows)| rows.len() >= MIN_TEAMS) {
        let Some(pkg) = Package::new(take_rates(rows.iter().map(|r| r.1.as_slice())), &SCORE_THIRDS) else { continue };
        for (team, m) in rows {
            if let Some((easy, hard)) = pkg.edges(m, &SCORE_THIRDS) {
                out.push(MaskEdge { tid: *tid, team: *team, easy, hard });
            }
        }
    }
    out
}

/// The cell a hard-third edge is compared within: the easy-third edge, binned.
pub fn edge_bin(easy: f64) -> i64 {
    ((easy * EDGE_STEPS).round() as i64).clamp(EDGE_LO, EDGE_HI)
}

// ── the index's score ────────────────────────────────────────────────────────

/// How far players' true in-person means spread. A player with no in-person
/// record is compared against zero, and that unknown baseline is worth this
/// much uncertainty — without it, having never played in person would make a
/// player look more certain than one who has.
pub fn baseline<'a>(in_person: impl Iterator<Item = &'a Acc>) -> f64 {
    let mut obs = Acc::default();
    let mut noise = 0.0;
    for ip in in_person.filter(|ip| ip.n >= BASELINE_MIN) {
        obs.push(ip.mean_or_zero());
        noise += ip.var_mean();
    }
    (obs.sd().powi(2) - noise / obs.n.max(1.0)).max(TINY).sqrt()
}

/// Online against in person, as (gap, variance), or against zero with the
/// baseline's uncertainty when there is too little in person to compare with.
pub fn gap(on: &Acc, ip: &Acc, baseline: f64) -> (f64, f64) {
    let compare = ip.n >= MIN_IN_PERSON;
    (
        on.mean_or_zero() - if compare { ip.mean_or_zero() } else { 0.0 },
        on.var_mean() + if compare { ip.var_mean() } else { baseline * baseline },
    )
}

/// The gap in the statistic's own units, as the table shows it.
pub fn shown_gap(on: &Acc, ip: &Acc) -> f64 {
    on.mean_or_zero() - if ip.n >= MIN_IN_PERSON { ip.mean_or_zero() } else { 0.0 }
}

/// What the population of true gaps looks like: a bulk sitting close to zero
/// and a small share genuinely far from it.
///
/// One normal will not do. Fitted to everybody it takes a spread of about 0.07
/// on the log-place scale, under which a gap of 1.03 is a fourteen-sigma event —
/// so a player with fifteen tournaments and an enormous gap is written off as
/// noise, while a player with seven hundred tournaments and a fifth of the gap
/// is believed. Two components fix that: a result far outside the bulk is
/// evidence of belonging to the small component, not of bad luck, and is barely
/// shrunk at all.
#[derive(Debug)]
pub struct Prior {
    /// Spread of the bulk.
    spike: f64,
    /// Spread of the rest.
    slab: f64,
    /// Share of players in the second component.
    share: f64,
}

/// One EM round's sums: the weight on each component, and the expected
/// squared true gap under each, weighted.
#[derive(Default)]
struct Sums {
    w: f64,
    w0: f64,
    t2: f64,
    s2: f64,
}

impl Prior {
    /// EM over the observed gaps and their variances.
    pub fn fit(obs: &[(f64, f64)]) -> Prior {
        let n = obs.len() as f64;
        let g2: f64 = obs.iter().map(|(g, _)| g * g).sum::<f64>() / n;
        let v: f64 = obs.iter().map(|(_, v)| v).sum::<f64>() / n;
        let excess = (g2 - v).max(VAR_FLOOR);
        let mut p = Prior { spike: (excess * START_SPIKE).sqrt(), slab: (excess * START_SLAB).sqrt(), share: START_SHARE };
        for _ in 0..EM_ROUNDS {
            p.step(obs, n);
        }
        p
    }

    fn step(&mut self, obs: &[(f64, f64)], n: f64) {
        let s = self.sums(obs);
        self.share = (s.w / n).clamp(SHARE_MIN, SHARE_MAX);
        self.slab = (s.t2 / s.w.max(TINY)).max(VAR_FLOOR).sqrt();
        self.spike = (s.s2 / s.w0.max(TINY)).max(VAR_FLOOR).sqrt();
        if self.slab < self.spike {
            std::mem::swap(&mut self.slab, &mut self.spike);
            self.share = 1.0 - self.share;
        }
    }

    fn sums(&self, obs: &[(f64, f64)]) -> Sums {
        let (s2, t2) = (self.spike * self.spike, self.slab * self.slab);
        let mut s = Sums::default();
        for &(g, v) in obs {
            let w = self.weight(g, v);
            s.w += w;
            s.w0 += 1.0 - w;
            s.t2 += w * second_moment(t2, g, v);
            s.s2 += (1.0 - w) * second_moment(s2, g, v);
        }
        s
    }

    /// Posterior probability that this gap belongs to the second component.
    fn weight(&self, g: f64, v: f64) -> f64 {
        let log_dens = |p2: f64, w: f64| {
            let var = p2 + v;
            w.ln() - HALF * var.ln() - HALF * g * g / var
        };
        let a = log_dens(self.slab * self.slab, self.share);
        let b = log_dens(self.spike * self.spike, 1.0 - self.share);
        1.0 / (1.0 + (b - a).exp())
    }

    /// The posterior mean of the true gap, in units of the second component's
    /// spread so the two statistics can be added.
    pub fn score(&self, (g, v): (f64, f64)) -> f64 {
        let (s2, t2) = (self.spike * self.spike, self.slab * self.slab);
        let w = self.weight(g, v);
        (w * g * t2 / (t2 + v) + (1.0 - w) * g * s2 / (s2 + v)) / self.slab
    }
}

/// E[θ²] of a gap `g` with variance `v`, under a component of prior variance `p2`.
fn second_moment(p2: f64, g: f64, v: f64) -> f64 {
    p2 * v / (p2 + v) + (g * p2 / (p2 + v)).powi(2)
}
