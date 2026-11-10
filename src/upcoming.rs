//! Forecasts for tournaments not yet played whose lineups are already listed.
//! The place each lineup is expected to finish needs nothing but the players;
//! the number of questions it is expected to take needs questions, and a pack
//! not yet played has none. It borrows the fitted questions of one that has
//! been: the previous tournament of the same series, or failing that the
//! recent one whose trueDL is nearest the difficulty the organisers announced.

use crate::config::Config;
use crate::data::{Format, Tournament, CORRECT, SKIP};
use crate::question_model::{self as qm, Field, QParams};
use crate::util::spearman;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

/// A field with fewer answering lineups than this lends no questions: its take
/// rates are too few to say how hard a question was.
const MIN_TEAMS: usize = 8;
/// How far back the previous tournament of a series, and a pack of the
/// announced difficulty, may be.
const SERIES_DAYS: i32 = 4 * 365;
const NEAR_DAYS: i32 = 2 * 365;
/// A word in more tournament names than this ("кубок", "осень") names no
/// series on its own.
const RARE: usize = 25;
/// The share of the earlier name, each word weighted by its rarity, that the
/// later name must contain.
const CONTAINED: f64 = 0.5;
/// Shorter words are particles and abbreviations.
const MIN_WORD: usize = 3;

/// Why a pack was chosen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rule {
    /// The previous tournament of the same series.
    Series,
    /// The recent tournament whose trueDL is nearest the announced difficulty.
    Difficulty,
}

impl Rule {
    pub fn name(self) -> &'static str {
        match self {
            Rule::Series => "series",
            Rule::Difficulty => "difficulty",
        }
    }
}

/// A played tournament's questions, as the fit left them.
pub struct Pack {
    pub id: i64,
    pub day: i32,
    pub format: Format,
    pub truedl: Option<f64>,
    /// The questions some lineup played, in order.
    pub params: Vec<QParams>,
}

/// Every pack the pass has fitted so far, and what it takes to find one.
pub struct Refs {
    packs: Vec<Pack>,
    /// The words of every tournament's name, sorted, and how many names each
    /// word is in.
    words: HashMap<i64, Vec<String>>,
    df: HashMap<String, usize>,
    /// `tournaments.difficulty_forecast`, where the organisers gave one.
    announced: HashMap<i64, f64>,
    /// Which packs have each word in their name.
    by_word: HashMap<String, Vec<usize>>,
}

/// A name as the series rule reads it: its words in lower case, ё as е,
/// without digits or punctuation, so «Полифест-2025» and «Петербургские
/// сезоны: Осень. Полифест-2026» share «полифест».
pub fn words(name: &str) -> Vec<String> {
    let s = name.to_lowercase().replace('ё', "е");
    let mut v: Vec<String> =
        s.split(|c: char| !c.is_alphabetic()).filter(|w| w.chars().count() >= MIN_WORD).map(str::to_string).collect();
    v.sort();
    v.dedup();
    v
}

impl Refs {
    /// Names and announced difficulties of every tournament in the mirror.
    pub fn new(db: &Connection) -> rusqlite::Result<Refs> {
        let mut st = db.prepare("SELECT id, name, difficulty_forecast FROM tournaments")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<f64>>(2)?)))?;
        let (mut words_of, mut df, mut announced) = (HashMap::new(), HashMap::new(), HashMap::new());
        for row in rows {
            let (id, name, dl) = row?;
            let w = words(name.as_deref().unwrap_or(""));
            for x in &w {
                *df.entry(x.clone()).or_insert(0) += 1;
            }
            words_of.insert(id, w);
            if let Some(dl) = dl.filter(|d| d.is_finite()) {
                announced.insert(id, dl);
            }
        }
        Ok(Refs { packs: Vec::new(), words: words_of, df, announced, by_word: HashMap::new() })
    }

    /// The difficulty the organisers announced for a tournament.
    pub fn announced(&self, id: i64) -> Option<f64> {
        self.announced.get(&id).copied()
    }

    /// Keeps a played field's questions, when enough lineups answered them.
    /// Asynchronous packs are never kept: nothing upcoming is forecast there.
    pub fn absorb(&mut self, t: &Tournament, field: &Field, params: &[QParams], truedl: Option<f64>) {
        if t.format == Format::Async || answering(field) < MIN_TEAMS {
            return;
        }
        let params: Vec<QParams> = params.iter().zip(&field.by_q).filter(|(_, v)| !v.is_empty()).map(|(p, _)| *p).collect();
        if params.is_empty() {
            return;
        }
        let k = self.packs.len();
        for w in self.words.get(&t.id).into_iter().flatten() {
            self.by_word.entry(w.clone()).or_default().push(k);
        }
        self.packs.push(Pack { id: t.id, day: t.day, format: t.format, truedl, params });
    }

    /// The pack an unplayed tournament borrows its questions from, and why.
    pub fn pick(&self, id: i64, day: i32, format: Format) -> Option<(&Pack, Rule)> {
        self.series(id, day, format).map(|p| (p, Rule::Series)).or_else(|| self.nearest(id, day, format).map(|p| (p, Rule::Difficulty)))
    }

    /// The latest earlier tournament of the same format and series within
    /// `SERIES_DAYS`: one of the same name, or one that shares a rare word
    /// with this one's and most of whose name, by rarity, this one contains.
    /// The fullest containment wins, then the latest.
    pub fn series(&self, id: i64, day: i32, format: Format) -> Option<&Pack> {
        let mine = self.words.get(&id).filter(|w| !w.is_empty())?;
        let own: HashSet<&String> = mine.iter().collect();
        let n = self.words.len() as f64;
        let idf = |w: &String| (n / self.df.get(w).copied().unwrap_or(1).max(1) as f64).ln();
        let near: HashSet<usize> = mine.iter().filter_map(|w| self.by_word.get(w)).flatten().copied().collect();
        let mut best: Option<(f64, i32, &Pack)> = None;
        for p in near.into_iter().map(|k| &self.packs[k]) {
            if p.id == id || p.format != format || p.day >= day || p.day < day - SERIES_DAYS {
                continue;
            }
            let theirs = &self.words[&p.id];
            let shared: Vec<&String> = theirs.iter().filter(|w| own.contains(w)).collect();
            let total: f64 = theirs.iter().map(idf).sum();
            let contained = if total > 0.0 { shared.iter().map(|w| idf(w)).sum::<f64>() / total } else { 0.0 };
            let rare = shared.iter().any(|w| self.df.get(*w).is_some_and(|&d| d <= RARE));
            if !(theirs == mine || rare && contained >= CONTAINED) {
                continue;
            }
            let key = (if theirs == mine { 1.0 } else { contained }, p.day);
            if best.is_none_or(|b| key.0 > b.0 || key.0 == b.0 && key.1 > b.1) {
                best = Some((key.0, key.1, p));
            }
        }
        best.map(|b| b.2)
    }

    /// The pack of the same format within `NEAR_DAYS` whose trueDL is nearest
    /// the difficulty announced for this tournament, the latest of equals.
    pub fn nearest(&self, id: i64, day: i32, format: Format) -> Option<&Pack> {
        let target = self.announced(id)?;
        let from = self.packs.partition_point(|p| p.day < day - NEAR_DAYS);
        let to = self.packs.partition_point(|p| p.day < day);
        self.packs[from..to]
            .iter()
            .filter(|p| p.format == format && p.id != id)
            .filter_map(|p| p.truedl.map(|d| ((d - target).abs(), p)))
            .min_by(|a, b| a.0.total_cmp(&b.0).then(b.1.day.cmp(&a.1.day)))
            .map(|x| x.1)
    }
}

/// Lineups that answered at least one question.
fn answering(field: &Field) -> usize {
    field.teams.iter().filter(|tm| !tm.z.is_empty() && tm.mask.iter().any(|&y| y != SKIP)).count()
}

/// Each lineup's expected number of questions taken out of `n_q` on `pack`'s
/// questions, by team index; a result with no lineup has none. With `seat`,
/// a lineup listed longer than six is read as its strongest six at the format
/// given, since six play: the published forecast reads six seats too.
pub fn expected(field: &Field, pack: &Pack, n_q: usize, seat: Option<Format>, cfg: &Config) -> HashMap<u32, f64> {
    let n = if n_q > 0 { n_q } else { pack.params.len() } as f64;
    let share = |tm: &qm::TeamObs| match seat {
        Some(f) if tm.z.len() > SEATS => {
            let z = seated(&tm.z, f, cfg);
            qm::expected_share(&z, &qm::slopes(&z, cfg), &pack.params, cfg)
        }
        _ => qm::expected_share(&tm.z, &tm.k, &pack.params, cfg),
    };
    field.teams.iter().filter(|tm| !tm.z.is_empty()).map(|tm| (tm.team, n * share(tm))).collect()
}

/// Seats at a table.
const SEATS: usize = 6;

/// The strongest six of a longer lineup, each without the shift the question
/// model gives a lineup of its listed length and with the one it gives six.
fn seated(z: &[f64], format: Format, cfg: &Config) -> Vec<f64> {
    let shift = qm::size_shift(SEATS, format, cfg) - qm::size_shift(z.len(), format, cfg);
    let mut v: Vec<f64> = z.iter().map(|x| x + shift).collect();
    v.sort_by(|a, b| b.total_cmp(a));
    v.truncate(SEATS);
    v
}

/// One rule's errors, summed over the fields it forecast.
#[derive(Default, Clone, Copy)]
struct Tally {
    fields: usize,
    lineups: usize,
    /// Mean absolute error per lineup, absolute error of the field's mean,
    /// and the rank correlation, each summed over the fields.
    mae: f64,
    bias: f64,
    rho: f64,
}

/// A rule's tally per format: `(format, rule)`.
type Tallies = HashMap<(&'static str, &'static str), Tally>;

/// `fit --upcoming-backtest`: every played field since a date, forecast as if
/// it were upcoming, the questions taken against what each rule expected.
#[derive(Default)]
pub struct Backtest {
    by: Tallies,
    /// The same on the fields both rules could forecast.
    both: Tallies,
}

impl Backtest {
    /// One played field, before `refs` has absorbed it.
    pub fn field(&mut self, t: &Tournament, field: &Field, own: &[QParams], refs: &Refs, cfg: &Config) {
        if t.format == Format::Async || answering(field) < MIN_TEAMS {
            return;
        }
        let actual: HashMap<u32, (f64, usize)> = field
            .teams
            .iter()
            .filter(|tm| !tm.z.is_empty())
            .map(|tm| (tm.team, (tm.mask.iter().filter(|&&y| y == CORRECT).count() as f64, tm.mask.iter().filter(|&&y| y != SKIP).count())))
            .filter(|(_, (_, n))| *n > 0)
            .collect();
        let played: Vec<QParams> = own.iter().zip(&field.by_q).filter(|(_, v)| !v.is_empty()).map(|(p, _)| *p).collect();
        let me = Pack { id: t.id, day: t.day, format: t.format, truedl: None, params: played };
        let series = refs.series(t.id, t.day, t.format);
        let near = refs.nearest(t.id, t.day, t.format);
        let both = series.is_some() && near.is_some();
        for (name, pack) in [("own questions", Some(&me)), ("series", series), ("difficulty", near)] {
            let Some(pack) = pack else { continue };
            let e = expected(field, pack, 0, None, cfg);
            let (mut fc, mut ac) = (Vec::new(), Vec::new());
            for (team, &(score, n)) in &actual {
                if let Some(&x) = e.get(team) {
                    fc.push(x / pack.params.len() as f64 * n as f64);
                    ac.push(score);
                }
            }
            if fc.len() < 2 {
                continue;
            }
            let k = fc.len() as f64;
            let one = Tally {
                fields: 1,
                lineups: fc.len(),
                mae: fc.iter().zip(&ac).map(|(f, a)| (f - a).abs()).sum::<f64>() / k,
                bias: (fc.iter().sum::<f64>() - ac.iter().sum::<f64>()).abs() / k,
                rho: spearman(&fc, &ac).unwrap_or(0.0),
            };
            let add = |m: &mut Tallies| {
                let s = m.entry((t.format.name(), name)).or_default();
                *s = Tally { fields: s.fields + 1, lineups: s.lineups + one.lineups, mae: s.mae + one.mae, bias: s.bias + one.bias, rho: s.rho + one.rho };
            };
            add(&mut self.by);
            if both {
                add(&mut self.both);
            }
        }
    }

    pub fn print(&self) {
        for (title, m) in [("every field a rule reaches", &self.by), ("fields both rules reach", &self.both)] {
            println!("upcoming backtest, {title}: per field, mean |error| per lineup, |error| of the field mean, Spearman");
            for format in [Format::InPerson, Format::Sync, Format::StrictSync].map(Format::name) {
                for name in ["own questions", "series", "difficulty"] {
                    if let Some(x) = m.get(&(format, name)) {
                        let f = x.fields as f64;
                        println!("  {format:11} {name:14} {:5} fields {:6} lineups  {:.2}  {:.2}  {:.3}", x.fields, x.lineups, x.mae / f, x.bias / f, x.rho / f);
                    }
                }
            }
        }
    }
}
