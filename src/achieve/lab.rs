//! The frozen values, the names and the rosters loaded once, each tournament
//! read as a field, and the team metric the knobs are scored by.

use super::frozen::{load_results, Names, Res, Seasons};
use crate::ndcg::{TEST, TUNE};
use crate::team_ndcg;
use crate::util::parse_date;
use std::collections::{HashMap, HashSet};

/// A field keeps its strongest lineups only, enough for any `field_k`.
const TOP_KEPT: usize = 50;

/// Each tournament as the weights see it.
pub(super) struct Field {
    /// The strongest lineups, best first, at most 50.
    pub top_q: Vec<f64>,
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
        if r.q.is_finite() {
            self.q.push(r.q);
        }
        self.n += 1;
        self.venues.insert(r.venue);
    }

    fn field(mut self) -> Field {
        let sum_all: f64 = self.q.iter().map(|x| x.max(0.0)).sum();
        self.q.sort_by(|a, b| b.total_cmp(a));
        self.q.truncate(TOP_KEPT);
        let counts = |m: HashMap<i64, usize>| {
            let mut v: Vec<usize> = m.into_values().collect();
            v.sort_by(|a, b| b.cmp(a));
            v
        };
        Field {
            top_q: self.q,
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
    pub fair: (f64, f64),
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
        let fair = ndcg(&rows, |r| r.fair_mu);
        Lab { rows, fields, names, seasons, ql: Vec::new(), last, fair }
    }

    /// A team's name, `?` when unknown.
    pub fn team_name(&self, team: i64) -> &str {
        self.names.teams.get(&team).map_or("?", |s| s.as_str())
    }
}

/// The team NDCG of DESIGN 7.3, weighted, over the tune and test windows.
pub(super) fn ndcg(rows: &[Res], mut score: impl FnMut(&Res) -> f64) -> (f64, f64) {
    let day = |s: &str| parse_date(s).unwrap();
    let win = [(day(TUNE.0), day(TUNE.1)), (day(TEST.0), day(TEST.1))];
    let mut by_t: HashMap<i64, (i32, f64, Vec<&Res>)> = HashMap::new();
    for r in rows.iter().filter(|r| r.format == 'i' && r.qual) {
        by_t.entry(r.tid).or_insert((r.day, r.tw, Vec::new())).2.push(r);
    }
    let mut acc = [(0.0, 0.0); 2];
    for (tid, (day, tw, v)) in &by_t {
        let Some(w) = win.iter().position(|&(a, b)| *day >= a && *day < b) else { continue };
        if let Some(x) = field_ndcg(*tid, v, &mut score) {
            acc[w].0 += tw * x;
            acc[w].1 += tw;
        }
    }
    let m = |a: (f64, f64)| if a.1 > 0.0 { a.0 / a.1 } else { f64::NAN };
    (m(acc[0]), m(acc[1]))
}

/// One field's NDCG over the results `score` reads.
fn field_ndcg(tid: i64, v: &[&Res], score: &mut impl FnMut(&Res) -> f64) -> Option<f64> {
    let (mut team, mut mu, mut place) = (Vec::new(), Vec::new(), Vec::new());
    for r in v {
        let s = score(r);
        if s.is_finite() {
            team.push(r.team);
            mu.push(s);
            place.push(r.place);
        }
    }
    team_ndcg::score(tid, &team, &mu, &place, usize::MAX)
}
