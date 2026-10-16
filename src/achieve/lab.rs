//! The frozen values, the names and the rosters loaded once, each tournament
//! read as a field, and the team metric the knobs are scored by.

use super::frozen::{load_results, Names, Res, Seasons};
use crate::ndcg::{method_metrics, TEST, TUNE};
use crate::util::{avg_ranks, parse_date};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A field keeps its strongest lineups only, enough for any `field_k`.
const TOP_KEPT: usize = 50;

/// Each tournament as the weights see it.
pub(super) struct Field {
    /// The strongest lineups, best first, at most 50.
    pub top_q: Vec<f64>,
    /// Every placed lineup's (place, strength, questions taken), strongest first.
    pub placed: Vec<(f64, f64, f64)>,
    /// The same two under the forecast's seat weights.
    pub top_q_fc: Vec<f64>,
    pub placed_fc: Vec<(f64, f64, f64)>,
    pub n_teams: usize,
    /// The tournaments merged into this field.
    pub n_venues: usize,
    /// The season's 20 strongest base rosters that week, summed.
    pub ref20: f64,
    /// Every lineup's strength, summed, negatives at 0.
    pub sum_all: f64,
    pub in_person: bool,
    /// Teams per country and per town, largest first.
    pub countries: Vec<usize>,
    pub towns: Vec<usize>,
}

/// One field as its results arrive: lineup strengths, results, venues, the
/// reference strength, the format, and teams per country and per town.
#[derive(Default)]
struct Gathered {
    q: Vec<f64>,
    placed: Vec<(f64, f64, f64)>,
    q_fc: Vec<f64>,
    placed_fc: Vec<(f64, f64, f64)>,
    n: usize,
    venues: HashSet<i64>,
    ref20: f64,
    in_person: bool,
    countries: HashMap<i64, usize>,
    towns: HashMap<i64, usize>,
}

impl Gathered {
    fn add(&mut self, r: &Res) {
        self.in_person = r.format == 'i';
        if r.country >= 0 {
            *self.countries.entry(r.country).or_default() += 1;
        }
        if r.town >= 0 {
            *self.towns.entry(r.town).or_default() += 1;
        }
        self.ref20 = r.ref20;
        for (q, top, placed) in [(r.q, &mut self.q, &mut self.placed), (r.q_fc, &mut self.q_fc, &mut self.placed_fc)] {
            if q.is_finite() {
                top.push(q);
                if r.place > 0.0 {
                    placed.push((r.place, q, r.taken));
                }
            }
        }
        self.n += 1;
        self.venues.insert(r.venue);
    }

    fn field(mut self) -> Field {
        let sum_all: f64 = self.q.iter().map(|x| x.max(0.0)).sum();
        for (top, placed) in [(&mut self.q, &mut self.placed), (&mut self.q_fc, &mut self.placed_fc)] {
            top.sort_by(|a, b| b.total_cmp(a));
            top.truncate(TOP_KEPT);
            placed.sort_by(|a, b| b.1.total_cmp(&a.1));
        }
        let counts = |m: HashMap<i64, usize>| {
            let mut v: Vec<usize> = m.into_values().collect();
            v.sort_by(|a, b| b.cmp(a));
            v
        };
        Field {
            top_q: self.q,
            placed: self.placed,
            top_q_fc: self.q_fc,
            placed_fc: self.placed_fc,
            n_teams: self.n,
            n_venues: self.venues.len(),
            ref20: self.ref20,
            sum_all,
            in_person: self.in_person,
            countries: counts(self.countries),
            towns: counts(self.towns),
        }
    }
}

/// The frozen values, the names and the rosters, loaded once.
pub struct Lab {
    /// Rated placed results only, the ones that can ever count.
    pub(super) rows: Vec<Res>,
    pub(super) fields: HashMap<i64, Field>,
    pub(super) names: Names,
    pub(super) seasons: Seasons,
    /// Each team's Q(L) at each release: (day, team, strength).
    pub(super) ql: Vec<(i32, i64, f64)>,
    /// The last day with a result.
    pub last: i32,
    /// The fair rule's weighted team NDCG, tune and test.
    pub fair: ([f64; 6], [f64; 6]),
}

impl Lab {
    /// The three files `achieve --build` writes into `dir`.
    pub fn load(dir: &str) -> std::io::Result<Lab> {
        let rs = load_results(&format!("{dir}/results.tsv"))?;
        let mut lab = Lab::from_results(rs, Names::load(&format!("{dir}/names.tsv")), Seasons::load(&format!("{dir}/rosters.tsv")));
        lab.ql = super::frozen::load_ql(&format!("{dir}/ql.tsv"));
        Ok(lab)
    }

    fn from_results(rs: Vec<Res>, names: Names, seasons: Seasons) -> Lab {
        let mut by_t: HashMap<i64, Gathered> = HashMap::new();
        for r in &rs {
            by_t.entry(r.tid).or_default().add(r);
        }
        let fields = by_t.into_iter().map(|(t, g)| (t, g.field())).collect();
        let last = rs.iter().map(|r| r.day).max().unwrap_or(0);
        let rows: Vec<Res> = rs.into_iter().filter(|r| r.rated && r.place > 0.0).collect();
        let fair = metrics(&rows, |r| r.fair_mu);
        Lab { rows, fields, names, seasons, ql: Vec::new(), last, fair }
    }

    /// A team's name, `?` when unknown.
    pub fn team_name(&self, team: i64) -> &str {
        self.names.teams.get(&team).map_or("?", |s| s.as_str())
    }
}

/// A lineup counts in the team metric when its strength is within this share
/// of its base roster's: the team that played is the team the rating rates.
const OWN_BAND: f64 = 0.05;

/// The six measures of /method/ for the rating `score` reads, weighted over the
/// in-person fields of the tune and test windows, among the lineups that played
/// at their base roster's strength, places re-ranked among them.
pub(super) fn metrics(rows: &[Res], score: impl FnMut(&Res) -> f64) -> ([f64; 6], [f64; 6]) {
    let f = scored_fields(rows, score);
    (mean(&f, 0), mean(&f, 1))
}

/// One field's six measures, its window (0 tune, 1 test) and its weight.
pub struct Scored {
    pub tid: i64,
    pub win: usize,
    pub tw: f64,
    pub m: [f64; 6],
}

pub(super) fn scored_fields(rows: &[Res], mut score: impl FnMut(&Res) -> f64) -> Vec<Scored> {
    let day = |s: &str| parse_date(s).unwrap();
    let wins = [(day(TUNE.0), day(TUNE.1)), (day(TEST.0), day(TEST.1))];
    let own = |r: &Res| r.q_base > 0.0 && (r.q_lineup / r.q_base - 1.0).abs() <= OWN_BAND;
    let mut by_t: BTreeMap<i64, (i32, f64, Vec<&Res>)> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.format == 'i' && own(r)) {
        by_t.entry(r.tid).or_insert((r.day, r.tw, Vec::new())).2.push(r);
    }
    let mut out = Vec::new();
    for (&tid, (day, tw, v)) in &by_t {
        let Some(win) = wins.iter().position(|&(a, b)| *day >= a && *day < b) else { continue };
        let scored: Vec<(f64, f64)> = v.iter().map(|r| (score(r), r.place)).filter(|x| x.0.is_finite()).collect();
        let places = avg_ranks(&scored.iter().map(|x| x.1).collect::<Vec<_>>());
        let field: Vec<(f64, f64)> = scored.iter().zip(places).map(|(x, p)| (x.0, p)).collect();
        if let Some(m) = method_metrics(tid, &field) {
            out.push(Scored { tid, win, tw: *tw, m });
        }
    }
    out
}

/// The weighted mean of each measure over the fields of window `win`.
pub fn mean(f: &[Scored], win: usize) -> [f64; 6] {
    std::array::from_fn(|i| {
        let (num, den) = f.iter().filter(|x| x.win == win && x.m[i].is_finite()).fold((0.0, 0.0), |a, x| (a.0 + x.tw * x.m[i], a.1 + x.tw));
        if den > 0.0 { num / den } else { f64::NAN }
    })
}

/// `b` less `a` on each measure over the fields both scored in window `win`,
/// with the standard error of a weighted mean of paired differences.
pub fn paired(a: &[Scored], b: &[Scored], win: usize) -> [(f64, f64); 6] {
    let by: HashMap<i64, &Scored> = a.iter().filter(|x| x.win == win).map(|x| (x.tid, x)).collect();
    std::array::from_fn(|i| {
        let d: Vec<(f64, f64)> = b
            .iter()
            .filter(|x| x.win == win)
            .filter_map(|x| by.get(&x.tid).map(|y| (x.tw, x.m[i] - y.m[i])))
            .filter(|x| x.1.is_finite())
            .collect();
        let w: f64 = d.iter().map(|x| x.0).sum();
        if w <= 0.0 {
            return (f64::NAN, f64::NAN);
        }
        let m = d.iter().map(|x| x.0 * x.1).sum::<f64>() / w;
        let var: f64 = d.iter().map(|x| (x.0 * (x.1 - m)).powi(2)).sum();
        (m, var.sqrt() / w)
    })
}
