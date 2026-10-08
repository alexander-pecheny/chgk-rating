//! Random search over the knobs, then local refinement of the best on the tune
//! window; every point scored on both windows and written as a row of a table.

use super::knobs::Knobs;
use super::lab::Lab;
use rayon::prelude::*;
use std::fmt::Write as _;
use std::time::Instant;

/// Marsaglia's xorshift64 shifts, and the 53 bits a uniform draw keeps.
const XORSHIFT: (u32, u32, u32) = (13, 7, 17);
const DROPPED_BITS: u32 = 11;
const TWO_53: f64 = (1u64 << 53) as f64;
/// Random points are scored and written in chunks of this many.
const CHUNK: usize = 200;
/// Each local round mutates up to three knobs of one of the best eight points,
/// 96 times.
const PARENTS: usize = 8;
const CHILDREN: usize = 96;
const MAX_MUTATIONS: f64 = 3.0;

/// How a knob is drawn: a whole number in [lo, hi], a number in [lo, hi) shown
/// to some decimals, e to such a number, zero with some probability and
/// otherwise a number, or one of a list (a repeat weighs a choice up).
#[derive(Clone, Copy)]
enum Draw {
    Int(f64, f64),
    Real(f64, f64, usize),
    Exp(f64, f64),
    OrZero(f64, f64, f64, usize),
    Pick(&'static [&'static str]),
}

struct Spec {
    key: &'static str,
    draw: Draw,
}

const fn int(key: &'static str, lo: f64, hi: f64) -> Spec {
    Spec { key, draw: Draw::Int(lo, hi) }
}
const fn real(key: &'static str, lo: f64, hi: f64, decimals: usize) -> Spec {
    Spec { key, draw: Draw::Real(lo, hi, decimals) }
}
const fn or_zero(key: &'static str, p: f64, lo: f64, hi: f64, decimals: usize) -> Spec {
    Spec { key, draw: Draw::OrZero(p, lo, hi, decimals) }
}
const fn pick(key: &'static str, opts: &'static [&'static str]) -> Spec {
    Spec { key, draw: Draw::Pick(opts) }
}

/// Every knob that can change the order of a field, with how to draw it. The
/// listing rules (grace, activity) and the display scale cannot, so they are
/// not searched.
const SEARCHED: [Spec; 36] = [
    int("top_x", 3.0, 15.0),
    int("min_slots", 0.0, 15.0),
    pick("perf", &["strength_place", "strength_place", "mix", "score", "place"]),
    real("place_pow", 0.0, 1.2, 2),
    real("mix", 0.0, 1.0, 2),
    real("w_sync", 0.0, 1.2, 2),
    real("w_strict", 0.0, 1.2, 2),
    pick("field_mode", &["sum", "world", "anchors"]),
    pick("field_k", &["5", "10", "20", "30"]),
    real("field_gamma", 0.0, 5.0, 2),
    real("field_zero", 0.0, 6000.0, 0),
    real("field_ref", 8000.0, 11000.0, 0),
    Spec { key: "sum_ref", draw: Draw::Exp(12.0, 14.6) },
    real("sum_gamma", 0.0, 1.5, 2),
    or_zero("country_pow", 0.4, 0.0, 1.0, 2),
    int("country_min", 1.0, 3.0),
    or_zero("town_pow", 0.4, 0.0, 0.8, 2),
    int("town_min", 1.0, 3.0),
    real("country_ref", 1.0, 10.0, 1),
    real("town_ref", 1.0, 25.0, 1),
    pick("geo_cap", &["0", "1", "1"]),
    pick("fade", &["yearly", "yearly", "exp", "linear", "none"]),
    real("loss_ip", 0.0, 0.7, 2),
    real("loss_sync", 0.0, 0.95, 2),
    int("full_days", 0.0, 730.0),
    int("zero_days", 400.0, 2000.0),
    int("half_life", 60.0, 1500.0),
    int("sync_full", 0.0, 365.0),
    int("sync_zero", 200.0, 1000.0),
    int("sync_half_life", 30.0, 700.0),
    int("tail_days", 730.0, 5000.0),
    real("slow_k", 0.0, 2.0, 2),
    or_zero("overlap_k", 0.3, 0.0, 2.0, 2),
    pick("overlap_by", &["strength", "count"]),
    pick("base_rule", &["0", "1"]),
    or_zero("win_bonus", 0.5, 0.0, 2000.0, 0),
];

/// xorshift, seeded, so a search is reproducible.
struct Rand(u64);

impl Rand {
    fn u(&mut self) -> f64 {
        self.0 ^= self.0 << XORSHIFT.0;
        self.0 ^= self.0 >> XORSHIFT.1;
        self.0 ^= self.0 << XORSHIFT.2;
        (self.0 >> DROPPED_BITS) as f64 / TWO_53
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.u()
    }
    fn index(&mut self, n: usize) -> usize {
        ((self.u() * n as f64) as usize).min(n - 1)
    }
    fn draw(&mut self, d: Draw) -> String {
        match d {
            Draw::Int(a, b) => (self.range(a, b + 1.0).floor()).to_string(),
            Draw::Real(a, b, n) => format!("{:.*}", n, self.range(a, b)),
            Draw::Exp(a, b) => format!("{:.0}", self.range(a, b).exp()),
            Draw::OrZero(p, a, b, n) => format!("{:.*}", n, if self.u() < p { 0.0 } else { self.range(a, b) }),
            Draw::Pick(v) => v[self.index(v.len())].into(),
        }
    }
}

/// What `achieve --search` was asked: points, local rounds, the seed, the
/// threads, the knobs held fixed on every point, and the table's path.
pub struct SearchOpts {
    pub n: usize,
    pub rounds: usize,
    pub seed: u64,
    pub threads: usize,
    pub fixed: String,
    pub path: String,
}

type Pt = (Vec<String>, f64, f64);

struct Search<'a> {
    lab: &'a Lab,
    o: &'a SearchOpts,
    pool: rayon::ThreadPool,
    out: String,
    all: Vec<Pt>,
    r: Rand,
    t0: Instant,
}

/// A drawn point, with the knobs `fixed` holds on top of it.
fn knobs_of(vals: &[String], fixed: &str) -> Knobs {
    let mut k = Knobs::default();
    for (s, v) in SEARCHED.iter().zip(vals) {
        let _ = k.set(s.key, v);
    }
    k.apply(fixed).unwrap_or(k)
}

impl Search<'_> {
    fn score(&self, pts: &[Vec<String>]) -> Vec<(f64, f64)> {
        let one = |v: &Vec<String>| self.lab.book(&knobs_of(v, &self.o.fixed)).ndcg();
        self.pool.install(|| pts.par_iter().map(one).collect())
    }

    /// The point as the knobs read it, `fixed` applied.
    fn shown(&self, v: &[String]) -> Vec<String> {
        let k = knobs_of(v, &self.o.fixed);
        SEARCHED.iter().map(|s| k.get(s.key)).collect()
    }

    /// Score `pts`, add them to the table under `phase`, and write it.
    fn record(&mut self, phase: &str, pts: Vec<Vec<String>>, shown: bool) -> std::io::Result<()> {
        let sc = self.score(&pts);
        let pts: Vec<Vec<String>> = if shown { pts.iter().map(|v| self.shown(v)).collect() } else { pts };
        for (v, (a, b)) in pts.into_iter().zip(sc) {
            let _ = writeln!(self.out, "{phase}\t{a:.5}\t{b:.5}\t{}", v.join("\t"));
            self.all.push((v, a, b));
        }
        std::fs::write(&self.o.path, &self.out)
    }

    fn random(&mut self) -> std::io::Result<()> {
        let n = self.o.n;
        for chunk in 0..n.div_ceil(CHUNK) {
            let pts: Vec<Vec<String>> =
                (0..CHUNK.min(n - chunk * CHUNK)).map(|_| SEARCHED.iter().map(|s| self.r.draw(s.draw)).collect()).collect();
            self.record("random", pts, true)?;
            let best = self.all.iter().map(|x| x.1).fold(f64::MIN, f64::max);
            eprintln!("random {} / {n}, best tune {best:.4}, {:.0}s", self.all.len() - 1, self.t0.elapsed().as_secs_f64());
        }
        Ok(())
    }

    /// Local refinement: mutate a few knobs of the best eight at a time.
    fn local(&mut self, round: usize) -> std::io::Result<()> {
        let mut best: Vec<&Pt> = self.all.iter().collect();
        best.sort_by(|a, b| b.1.total_cmp(&a.1));
        let parents: Vec<Vec<String>> = best.iter().take(PARENTS).map(|x| x.0.clone()).collect();
        let pts: Vec<Vec<String>> = (0..CHILDREN).map(|i| self.mutate(parents[i % parents.len()].clone())).collect();
        self.record("local", pts, true)?;
        let b = self.all.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        let rounds = self.o.rounds;
        eprintln!("local {}/{rounds}, best tune {:.4} (test {:.4}), {:.0}s", round + 1, b.1, b.2, self.t0.elapsed().as_secs_f64());
        Ok(())
    }

    fn mutate(&mut self, mut v: Vec<String>) -> Vec<String> {
        let m = 1 + (self.r.u() * MAX_MUTATIONS) as usize;
        for _ in 0..m {
            let j = self.r.index(SEARCHED.len());
            v[j] = self.r.draw(SEARCHED[j].draw);
        }
        v
    }
}

/// Random search, then local refinement of the best on the tune window; every
/// point scored on both windows and written as a row of `o.path`.
pub fn search(lab: &Lab, o: &SearchOpts) -> std::io::Result<()> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(o.threads).build().unwrap();
    let keys: Vec<&str> = SEARCHED.iter().map(|s| s.key).collect();
    let out = format!("phase\ttune\ttest\t{}\n", keys.join("\t"));
    let mut s = Search { lab, o, pool, out, all: Vec::new(), r: Rand(o.seed | 1), t0: Instant::now() };
    // The default first, as the reference row.
    let d = Knobs::default();
    s.record("default", vec![keys.iter().map(|k| d.get(k)).collect()], false)?;
    s.random()?;
    for round in 0..o.rounds {
        s.local(round)?;
    }
    Ok(())
}
