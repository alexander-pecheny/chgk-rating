//! The frozen values and the lists beside them, as the three files of
//! `out/achieve/`: `results.tsv`, `names.tsv` and `rosters.tsv`.

use super::knobs::Knobs;
use crate::util::{format_date as show_day, parse_date as parse_day};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// One result, every strength on the published team scale. Nothing here
/// changes after the day it was played.
#[derive(Clone)]
pub struct Res {
    pub tid: i64,
    pub day: i32,
    /// 'i' in person, 's' synchronous, 'S' strictly synchronous, 'a' asynchronous.
    pub format: char,
    pub team: i64,
    pub place: f64,
    pub rated: bool,
    pub n: u32,
    pub n_base: u32,
    /// The lineup's strength as the forecast reads it; NaN with no lineup.
    pub q: f64,
    /// Performance from the questions taken, before `shift`.
    pub perf_s: f64,
    /// Performance from the place against the field's lineups.
    pub perf_p: f64,
    pub taken: f64,
    pub n_q: f64,
    /// The shipped fair rule's rating that morning, for the comparison.
    pub fair_mu: f64,
    /// Where it was played and the place there, when `tid` is a merged field.
    pub venue: i64,
    pub vplace: f64,
    /// In the team metric's qualifying set, and the tournament's metric weight.
    pub qual: bool,
    pub tw: f64,
    /// The combined strength of the season's 20 strongest base rosters that
    /// week, read the way this tournament's lineups are.
    pub ref20: f64,
    /// The team's town and country on the day, -1 where unknown.
    pub town: i64,
    pub country: i64,
}

/// The columns added after `fair_mu`, read with a default when a file lacks them.
const VENUE: usize = 14;
const VPLACE: usize = 15;
const QUAL: usize = 16;
const TW: usize = 17;
const REF20: usize = 18;
const TOWN: usize = 19;
const COUNTRY: usize = 20;
const UNKNOWN: i64 = -1;
/// A `names.tsv` line: kind, id, name.
const NAME_FIELDS: usize = 3;

const HEADER: &str = "tid\tday\tformat\tteam\tplace\trated\tn\tn_base\tq\tperf_s\tperf_p\ttaken\tn_q\tfair_mu\tvenue\tvplace\tqual\ttw\tref20\ttown\tcountry";

pub fn save_results(rs: &[Res], path: &str) -> std::io::Result<()> {
    let mut s = String::from(HEADER);
    s.push('\n');
    for r in rs {
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.1}\t{:.1}\t{:.1}\t{}\t{}\t{:.1}\t{}\t{}\t{}\t{}\t{:.0}\t{}\t{}",
            r.tid, show_day(r.day), r.format, r.team, r.place, r.rated as u8, r.n, r.n_base, r.q, r.perf_s, r.perf_p,
            r.taken, r.n_q, r.fair_mu, r.venue, r.vplace, r.qual as u8, r.tw, r.ref20, r.town, r.country
        );
    }
    std::fs::write(path, s)
}

pub fn load_results(path: &str) -> std::io::Result<Vec<Res>> {
    let s = std::fs::read_to_string(path)?;
    Ok(s.lines().skip(1).filter_map(parse_res).collect())
}

/// One line of `results.tsv`; the columns after `fair_mu` came later and
/// default when missing.
fn parse_res(l: &str) -> Option<Res> {
    let f = |x: &str| x.parse::<f64>().unwrap_or(f64::NAN);
    let v: Vec<&str> = l.split('\t').collect();
    let int = |i: usize, d: i64| v.get(i).and_then(|x| x.parse().ok()).unwrap_or(d);
    let tid: i64 = v[0].parse().ok()?;
    Some(Res {
        tid,
        day: parse_day(v[1])?,
        format: v[2].chars().next()?,
        team: v[3].parse().ok()?,
        place: f(v[4]),
        rated: v[5] == "1",
        n: v[6].parse().ok()?,
        n_base: v[7].parse().ok()?,
        q: f(v[8]),
        perf_s: f(v[9]),
        perf_p: f(v[10]),
        taken: f(v[11]),
        n_q: f(v[12]),
        fair_mu: f(v[13]),
        venue: int(VENUE, tid),
        vplace: v.get(VPLACE).map_or(f(v[4]), |x| f(x)),
        qual: v.get(QUAL).is_some_and(|x| *x == "1"),
        tw: v.get(TW).map_or(0.0, |x| f(x)),
        ref20: v.get(REF20).map_or(f64::NAN, |x| f(x)),
        town: int(TOWN, UNKNOWN),
        country: int(COUNTRY, UNKNOWN),
    })
}

/// Team and tournament names: `names.tsv` holds `T id name` and `U id name` lines.
pub struct Names {
    pub teams: HashMap<i64, String>,
    pub tours: HashMap<i64, String>,
}

impl Names {
    pub fn load(path: &str) -> Names {
        let mut n = Names { teams: HashMap::new(), tours: HashMap::new() };
        for l in std::fs::read_to_string(path).unwrap_or_default().lines() {
            let mut it = l.splitn(NAME_FIELDS, '\t');
            let (Some(k), Some(id), Some(name)) = (it.next(), it.next(), it.next()) else { continue };
            let Ok(id) = id.parse() else { continue };
            match k {
                "T" => n.teams.insert(id, name.to_string()),
                "U" => n.tours.insert(id, name.to_string()),
                _ => None,
            };
        }
        n
    }

    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let mut s = String::new();
        for (id, name) in &self.teams {
            let _ = writeln!(s, "T\t{id}\t{}", name.replace(['\t', '\n'], " "));
        }
        for (id, name) in &self.tours {
            let _ = writeln!(s, "U\t{id}\t{}", name.replace(['\t', '\n'], " "));
        }
        std::fs::write(path, s)
    }
}

/// Seasons, which teams had a base roster in which, the rosters, and each
/// player's strength: `rosters.tsv` holds `S id first last`, `R team season`,
/// `Q player rating` and `P team season player` lines.
pub struct Seasons {
    pub list: Vec<(i64, i32, i32)>,
    pub has: HashSet<(i64, i64)>,
    /// The base roster of each (team, season), player ids sorted.
    pub players: HashMap<(i64, i64), Vec<i64>>,
    /// Each player's last published rating, clipped at 0.
    pub strength: HashMap<i64, f64>,
}

impl Seasons {
    pub fn load(path: &str) -> Seasons {
        let mut s = Seasons { list: Vec::new(), has: HashSet::new(), players: HashMap::new(), strength: HashMap::new() };
        for l in std::fs::read_to_string(path).unwrap_or_default().lines() {
            s.read_line(&l.split('\t').collect::<Vec<_>>());
        }
        s.list.sort();
        s.dedup();
        s
    }

    fn read_line(&mut self, v: &[&str]) {
        match v {
            ["S", id, a, b] => {
                if let (Ok(id), Some(a), Some(b)) = (id.parse(), parse_day(a), parse_day(b)) {
                    self.list.push((id, a, b));
                }
            }
            ["R", t, x] => {
                if let (Ok(t), Ok(x)) = (t.parse(), x.parse()) {
                    self.has.insert((t, x));
                }
            }
            ["Q", pl, r] => {
                if let (Ok(pl), Ok(r)) = (pl.parse::<i64>(), r.parse::<f64>()) {
                    self.strength.insert(pl, r.max(0.0));
                }
            }
            ["P", t, x, pl] => {
                if let (Ok(t), Ok(x), Ok(pl)) = (t.parse(), x.parse(), pl.parse()) {
                    self.players.entry((t, x)).or_default().push(pl);
                }
            }
            _ => {}
        }
    }

    /// Every roster sorted, each player once.
    pub fn dedup(&mut self) {
        for v in self.players.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
    }

    /// The season a day falls in.
    pub fn season_of(&self, day: i32) -> Option<i64> {
        self.list.iter().find(|x| day >= x.1 && day <= x.2).map(|x| x.0)
    }

    /// The roster a team is read at on `day`: this season's, or last season's
    /// while this one has none.
    pub fn current(&self, team: i64, day: i32) -> Option<&Vec<i64>> {
        let s = self.season_of(day)?;
        self.players.get(&(team, s)).or_else(|| self.players.get(&(team, s - 1)))
    }

    pub fn save(&self, path: &str) -> std::io::Result<()> {
        let mut o = String::new();
        for (id, a, b) in &self.list {
            let _ = writeln!(o, "S\t{id}\t{}\t{}", show_day(*a), show_day(*b));
        }
        for (t, x) in &self.has {
            let _ = writeln!(o, "R\t{t}\t{x}");
        }
        for (pl, r) in &self.strength {
            let _ = writeln!(o, "Q\t{pl}\t{r:.0}");
        }
        for ((t, x), v) in &self.players {
            for pl in v {
                let _ = writeln!(o, "P\t{t}\t{x}\t{pl}");
            }
        }
        std::fs::write(path, o)
    }

    /// Whether the team is listed on `day`: a base roster for the season, or
    /// one last season while the new one is younger than `grace` days.
    pub fn listed(&self, team: i64, day: i32, k: &Knobs) -> bool {
        if k.grace < 0 {
            return true;
        }
        let Some(&(s, start, _)) = self.list.iter().find(|x| day >= x.1 && day <= x.2) else {
            return true;
        };
        self.has.contains(&(team, s)) || (day - start <= k.grace && self.has.contains(&(team, s - 1)))
    }
}
