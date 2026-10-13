use crate::config::Config;
use crate::util::parse_date;
use rusqlite::Connection;
use std::collections::HashMap;

pub const CORRECT: u8 = 1;
pub const WRONG: u8 = 0;
pub const SKIP: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    InPerson,
    Sync,
    StrictSync,
    Async,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "Обычный" | "Региональный" => Some(Format::InPerson),
            "Синхрон" => Some(Format::Sync),
            "Строго синхронный" => Some(Format::StrictSync),
            "Асинхрон" => Some(Format::Async),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Format::InPerson => "in-person",
            Format::Sync => "sync",
            Format::StrictSync => "strict-sync",
            Format::Async => "async",
        }
    }
    pub fn weight(self, c: &Config) -> f64 {
        match self {
            Format::InPerson => c.data.w_in_person,
            Format::Sync => c.data.w_sync,
            Format::StrictSync => c.data.w_strict_sync,
            Format::Async => c.data.w_async,
        }
    }
}

pub struct TeamResult {
    pub team: u32,
    pub position: f64,
    pub score: Option<f64>,
    /// `predictedPosition` of the old system, for the forecast comparison.
    pub pred_pos: Option<f64>,
    /// The old system's per-result `inRating`; false means a pickup lineup that
    /// earns its team no rating, no evidence and no place in the table.
    pub rated: bool,
    /// The town the team was registered in on the day, as an index into
    /// `Arena.towns`; `u32::MAX` when the mirror carries none. `fit --features`
    /// also compares it with the tournament's own town.
    pub town: u32,
    pub members: Vec<u32>,
    /// Whether each member was on the team's base roster that day, by
    /// `base_rosters`; all false when neither source knows the team.
    pub base: Vec<bool>,
    pub mask: Vec<u8>,
    /// The tournament the result was played at and its place there, which
    /// `merge_cities` keeps while it re-ranks `position` over the merged field.
    pub venue: i64,
    pub venue_position: f64,
}

pub struct Tournament {
    pub id: i64,
    pub day: i32,
    pub format: Format,
    pub weight: f64,
    /// A tournament the official system leaves out, loaded under
    /// `data.train_unrated`: it moves the players and nothing else. No team
    /// state opens or moves on it, it writes no team event and it never enters
    /// the metric's tournament set.
    pub train_only: bool,
    /// Where the tournament was played, as an index into `Arena.towns`;
    /// `u32::MAX` when the mirror carries none. `tournaments.town_id` is a town
    /// id and `tournament_results.team_current_town` a town name, so the two are
    /// brought onto one index through `data/towns.csv`.
    pub town: u32,
    pub editors: Vec<u32>,
    pub teams: Vec<TeamResult>,
    pub n_q: usize,
    pub n_q_declared: usize,
}

/// Everything the online pass reads, loaded once and kept in memory.
pub struct Arena {
    pub tournaments: Vec<Tournament>,
    pub player_ids: Vec<i64>,
    pub editor_ids: Vec<i64>,
    pub team_ids: Vec<i64>,
    /// Town names in index order, so a town is one `u32` everywhere downstream.
    pub towns: Vec<String>,
}

impl Arena {
    pub fn n_players(&self) -> usize {
        self.player_ids.len()
    }
    pub fn n_editors(&self) -> usize {
        self.editor_ids.len()
    }
    pub fn n_teams(&self) -> usize {
        self.team_ids.len()
    }
    pub fn n_towns(&self) -> usize {
        self.towns.len()
    }
}

/// The town id to town name map of the old system, saved by
/// `tools/fetch/towns.py`. An absent file leaves every tournament townless,
/// which turns the geography switches off rather than failing the run.
pub const TOWNS: &str = "data/towns.csv";

pub fn town_names(path: &str) -> HashMap<i64, String> {
    crate::util::csv_rows(path)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            Some((
                r.first()?.trim().parse().ok()?,
                r.get(1)?.trim().to_string(),
            ))
        })
        .collect()
}

/// Ids of one kind numbered in order of first appearance, so an entity is
/// one `u32` everywhere downstream.
struct Interner<K> {
    index: HashMap<K, u32>,
    ids: Vec<K>,
}

impl<K: std::hash::Hash + Eq + Clone> Interner<K> {
    fn new() -> Interner<K> {
        Interner { index: HashMap::new(), ids: Vec::new() }
    }

    fn get(&mut self, k: K) -> u32 {
        let n = self.ids.len() as u32;
        *self.index.entry(k.clone()).or_insert_with(|| {
            self.ids.push(k);
            n
        })
    }
}

/// Every run of digits in `s` as an id, read through `alias` and interned.
fn parse_ids(s: &str, ids: &mut Interner<i64>, alias: &HashMap<i64, i64>) -> Vec<u32> {
    let id = |v: i64| alias.get(&v).copied().unwrap_or(v);
    s.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).filter_map(|x| x.parse().ok()).map(|v| ids.get(id(v))).collect()
}

/// `alias_id -> player_id` from `data.player_aliases`; empty when none is named.
fn aliases(path: &str) -> HashMap<i64, i64> {
    if path.is_empty() {
        return HashMap::new();
    }
    let pair = |r: &Vec<String>| Some((r.first()?.trim().parse().ok()?, r.get(1)?.trim().parse().ok()?));
    let m: HashMap<i64, i64> = crate::util::csv_rows(path).unwrap_or_default().iter().filter_map(pair).collect();
    eprintln!("{} player aliases from {path}", m.len());
    m
}

/// Pulls one numeric field out of the old system's per-result JSON blob.
fn json_number(s: &str, key: &str) -> Option<f64> {
    let i = s.find(key)? + key.len();
    let rest = s[i..].trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn json_true(s: &str, key: &str) -> bool {
    match s.find(key) {
        Some(i) => s[i + key.len()..]
            .trim_start()
            .strip_prefix(':')
            .is_some_and(|r| r.trim_start().starts_with("true")),
        None => false,
    }
}

fn parse_mask(s: &str, out: &mut Vec<u8>) {
    for c in s.bytes() {
        out.push(match c {
            b'1' => CORRECT,
            b'0' => WRONG,
            _ => SKIP,
        });
    }
}

/// `data.merge_cities`: the in-person fields of one day (or `days` apart) that share their
/// editors and question count, and that rating.chgk.info links as played on
/// one package (`links`), are one field. Each group is folded into its lowest
/// id, provided every result carries a score, and re-ranked by it, ties
/// sharing the mean place.
fn merge_cities(ts: &mut Vec<Tournament>, days: i32, links: &Links, venue_drop: bool) {
    let into = merge_targets(ts, days, links);
    let mut merged = vec![false; ts.len()];
    for (k, first) in into.iter().enumerate() {
        let Some(first) = *first else { continue };
        let teams = std::mem::take(&mut ts[k].teams);
        ts[first].teams.extend(teams);
        ts[first].id = ts[first].id.min(ts[k].id);
        merged[first] = true;
    }
    for (t, _) in ts.iter_mut().zip(&merged).filter(|(_, &m)| m) {
        rescore(t, venue_drop);
    }
    let folded = into.iter().filter(|x| x.is_some()).count();
    let groups = merged.iter().filter(|&&m| m).count();
    ts.retain(|t| !t.teams.is_empty());
    eprintln!("merge_cities: {folded} fields folded into {groups} tournaments");
}

/// Pairs of tournament ids rating.chgk.info lists as sharing questions, each
/// pair both ways round.
pub type Links = std::collections::HashSet<(i64, i64)>;

/// `buff.db`'s mirror of the official intersections; empty on a mirror that
/// has none yet.
fn links(conn: &Connection) -> Links {
    let Ok(mut st) = conn.prepare("select tournament_id, other_id from tournament_intersections") else {
        return Links::new();
    };
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

/// For every tournament, the earlier one of its group it folds into, if any:
/// the first group of its editors and question count, within `days`, that
/// holds a tournament linked to it.
fn merge_targets(ts: &[Tournament], days: i32, links: &Links) -> Vec<Option<usize>> {
    let mut groups: HashMap<(Vec<u32>, usize), Vec<(i32, Vec<usize>)>> = HashMap::new();
    let mut into = vec![None; ts.len()];
    for (k, t) in ts.iter().enumerate() {
        let mergeable = t.format == Format::InPerson && !t.train_only && !t.editors.is_empty() && t.n_q > 0;
        if !mergeable || t.teams.iter().any(|r| r.score.is_none()) {
            continue;
        }
        let mut ed = t.editors.clone();
        ed.sort_unstable();
        let same = groups.entry((ed, t.n_q)).or_default();
        let linked = |m: &usize| links.contains(&(ts[*m].id, t.id));
        match same.iter_mut().find(|(d, members)| (t.day - d).abs() <= days && members.iter().any(linked)) {
            Some((_, members)) => {
                into[k] = Some(members[0]);
                members.push(k);
            }
            None => same.push((t.day, vec![k])),
        }
    }
    into
}

/// A question dropped at any venue is dropped at all of them, and the scores
/// are recounted and re-ranked, so every venue plays for the same questions.
/// Under `venue_drop` a question is dropped only where a whole venue skipped
/// it; otherwise any team's skip drops it for everyone.
fn rescore(t: &mut Tournament, venue_drop: bool) {
    let nq = t.teams.iter().map(|r| r.mask.len()).max().unwrap_or(0);
    let drop = if venue_drop { venue_drops(t, nq) } else { any_drops(t, nq) };
    for r in t.teams.iter_mut().filter(|r| !r.mask.is_empty()) {
        r.mask.resize(nq, SKIP);
        r.mask.iter_mut().zip(&drop).filter(|(_, &d)| d).for_each(|(y, _)| *y = SKIP);
        r.score = Some(r.mask.iter().filter(|&&y| y == CORRECT).count() as f64);
    }
    let s: Vec<f64> = t.teams.iter().map(|r| r.score.unwrap_or(0.0)).collect();
    for (r, &x) in t.teams.iter_mut().zip(&s) {
        let above = s.iter().filter(|&&y| y > x).count() as f64;
        let level = s.iter().filter(|&&y| y == x).count() as f64;
        r.position = above + (level + 1.0) * HALF;
    }
}

fn skipped(r: &TeamResult, q: usize) -> bool {
    r.mask.get(q).is_none_or(|&y| y == SKIP)
}

fn any_drops(t: &Tournament, nq: usize) -> Vec<bool> {
    let masked: Vec<&TeamResult> = t.teams.iter().filter(|r| !r.mask.is_empty()).collect();
    (0..nq).map(|q| masked.iter().any(|r| skipped(r, q))).collect()
}

/// The questions some venue as a whole did not play.
fn venue_drops(t: &Tournament, nq: usize) -> Vec<bool> {
    let mut by_venue: HashMap<i64, Vec<&TeamResult>> = HashMap::new();
    for r in t.teams.iter().filter(|r| !r.mask.is_empty()) {
        by_venue.entry(r.venue).or_default().push(r);
    }
    (0..nq).map(|q| by_venue.values().any(|v| v.iter().all(|r| skipped(r, q)))).collect()
}

/// The roster history `tools/fetch/prep_gg_inputs.py` writes from chgk.gg's dump, and
/// the season calendar that closes an open entry.
pub const BASE_ROSTERS: &str = "data/external/chgk_gg/base_rosters.csv";
pub const SEASONS: &str = "data/external/chgk_gg/seasons.csv";

/// `(team, player, season, added, removed)` of one base-roster entry.
type RosterEntry = (i64, i64, i64, Option<i32>, Option<i32>);
type Rosters = HashMap<(i64, i64), Vec<(i32, i32)>>;

/// For each (team, player) the day ranges they were on its base roster: the
/// chgk.gg history, joined with `buff.db`'s `team_seasons`, which holds the two
/// current seasons and so keeps the daily update current. An entry with no end
/// runs to the end of its season.
fn base_rosters(conn: &Connection) -> Rosters {
    let season_end = season_ends();
    let mut m = Rosters::new();
    for (t, pl, s, a, b) in roster_csv().into_iter().chain(roster_db(conn)) {
        if let Some(a) = a {
            let b = b.or(season_end.get(&s).copied()).unwrap_or(i32::MAX);
            m.entry((t, pl)).or_default().push((a, b));
        }
    }
    m
}

/// The comma-separated fields of every line of `path` after its header.
fn csv_fields(path: &str) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines().skip(1).map(|l| l.trim().split(',').map(str::to_string).collect()).collect()
}

fn season_ends() -> HashMap<i64, i32> {
    let end = |f: &Vec<String>| Some((f.first()?.parse().ok()?, parse_date(f.get(SEASON_END)?)?));
    csv_fields(SEASONS).iter().filter_map(end).collect()
}

fn roster_csv() -> Vec<RosterEntry> {
    let rows = csv_fields(BASE_ROSTERS);
    if rows.is_empty() {
        eprintln!("warning: no {BASE_ROSTERS}; base rosters from buff.db alone");
    }
    let day = |f: &Vec<String>, i: usize| f.get(i).and_then(|x| parse_date(x));
    let entry = |f: &Vec<String>| {
        Some((f.first()?.parse().ok()?, f.get(1)?.parse().ok()?, f.get(2)?.parse().ok()?, day(f, ROSTER_ADDED), day(f, ROSTER_REMOVED)))
    };
    rows.iter().filter_map(entry).collect()
}

fn roster_db(conn: &Connection) -> Vec<RosterEntry> {
    let sql = "SELECT team_id, player_id, season_id, substr(date_added,1,10), substr(date_removed,1,10) FROM team_seasons";
    let day = |s: Option<String>| s.as_deref().and_then(parse_date);
    let read = |r: &rusqlite::Row| Ok((r.get(0)?, r.get(1)?, r.get(2)?, day(r.get(ROSTER_ADDED)?), day(r.get(ROSTER_REMOVED)?)));
    conn.prepare(sql).and_then(|mut st| st.query_map([], read)?.collect()).unwrap_or_default()
}

/// The columns of the results query in `load`.
const C_ID: usize = 0;
const C_DATE: usize = 1;
const C_TYPE: usize = 2;
const C_EDITORS: usize = 3;
const C_TEAM: usize = 4;
const C_POSITION: usize = 5;
const C_SCORE: usize = 6;
const C_MASK: usize = 7;
const C_MEMBERS: usize = 8;
const C_RATING: usize = 9;
const C_QUESTIONS: usize = 10;
const C_IN_RATING: usize = 11;
const C_TOWN: usize = 12;
const C_TEAM_TOWN: usize = 13;
/// The date columns of the roster files and of `team_seasons`.
const SEASON_END: usize = 2;
const ROSTER_ADDED: usize = 3;
const ROSTER_REMOVED: usize = 4;
const HALF: f64 = 0.5;
/// A correlation of take rates needs this many questions to mean anything.

pub fn load(cfg: &Config, from: &str, to: &str) -> Result<Arena, rusqlite::Error> {
    let conn = Connection::open_with_flags(&cfg.data.db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(&results_sql(cfg))?;
    let mut rows = stmt.query([from, to])?;
    let mut ld = Loader::new(cfg);
    let mut skip_id = i64::MIN;
    while let Some(row) = rows.next()? {
        let tid: i64 = row.get(C_ID)?;
        if tid == skip_id {
            continue;
        }
        if ld.tournaments.last().map(|t| t.id) != Some(tid) {
            match ld.tournament(row)? {
                Some(t) => ld.tournaments.push(t),
                None => {
                    skip_id = tid;
                    continue;
                }
            }
        }
        ld.result(row)?;
    }
    Ok(ld.finish(&conn))
}

/// Every result of the window, the official system's unrated tournaments in
/// or out as `data` says.
fn results_sql(cfg: &Config) -> String {
    let rated = if cfg.data.include_unrated || cfg.data.train_unrated {
        ""
    } else if cfg.data.unrated_in_person {
        "(t.in_rating = 1 OR t.tournament_type IN ('Обычный', 'Региональный')) AND"
    } else if cfg.data.w_async > 0.0 {
        "(t.in_rating = 1 OR t.tournament_type = 'Асинхрон') AND"
    } else {
        "t.in_rating = 1 AND"
    };
    format!(
        "SELECT t.id, substr(t.date_start,1,10), t.tournament_type, t.editors,
                r.team_id, r.position, r.questions_total, r.mask, r.team_members, r.rating,
                t.questions_by_tour, t.in_rating, t.town_id, r.team_current_town
         FROM tournaments t JOIN tournament_results r ON r.id = t.id
         WHERE {rated} substr(t.date_start,1,10) >= ?1 AND substr(t.date_start,1,10) < ?2
         ORDER BY substr(t.date_start,1,10), t.id"
    )
}

/// The arena as it is read, one result row at a time.
struct Loader<'a> {
    cfg: &'a Config,
    players: Interner<i64>,
    editors: Interner<i64>,
    teams: Interner<i64>,
    /// One index per town name, shared by the tournament's own town and by the
    /// towns the teams are registered in.
    towns: Interner<String>,
    names: HashMap<i64, String>,
    alias: HashMap<i64, i64>,
    no_alias: HashMap<i64, i64>,
    tournaments: Vec<Tournament>,
}

impl<'a> Loader<'a> {
    fn new(cfg: &'a Config) -> Loader<'a> {
        Loader {
            cfg,
            players: Interner::new(),
            editors: Interner::new(),
            teams: Interner::new(),
            towns: Interner::new(),
            names: town_names(TOWNS),
            alias: aliases(&cfg.data.player_aliases),
            no_alias: HashMap::new(),
            tournaments: Vec::new(),
        }
    }

    fn town(&mut self, name: Option<String>) -> u32 {
        name.filter(|s| !s.is_empty()).map_or(u32::MAX, |n| self.towns.get(n))
    }

    /// The tournament a row opens, or `None` for one the window leaves out:
    /// no date, an unknown format, or a weight of zero. `train_unrated` keeps
    /// the unrated tournaments for the player side only, at a weight of their
    /// own; `include_unrated` treats them as ordinary tournaments, which is the
    /// switch round 2 measured.
    fn tournament(&mut self, row: &rusqlite::Row) -> rusqlite::Result<Option<Tournament>> {
        let (cfg, id) = (self.cfg, row.get(C_ID)?);
        let date: String = row.get(C_DATE)?;
        let kind: Option<String> = row.get(C_TYPE)?;
        let (Some(day), Some(format)) = (parse_date(&date), kind.as_deref().and_then(Format::parse)) else {
            return Ok(None);
        };
        let in_rating: i64 = row.get::<_, Option<i64>>(C_IN_RATING)?.unwrap_or(0);
        // The official system rates no asynchronous tournament, so a positive
        // `w_async` reads them for the players alone, at that weight.
        let unrated_async = format == Format::Async && in_rating == 0 && !cfg.data.include_unrated;
        let train_only = unrated_async || (cfg.data.train_unrated && !cfg.data.include_unrated && in_rating == 0);
        let weight = format.weight(cfg) * if train_only && !unrated_async { cfg.data.w_unrated } else { 1.0 };
        if weight <= 0.0 {
            return Ok(None);
        }
        let editors = row.get::<_, Option<String>>(C_EDITORS)?.map_or(Vec::new(), |e| parse_ids(&e, &mut self.editors, &self.no_alias));
        // Tournaments whose results carry no mask still need a question count
        // for the pairwise scale; the tournament row declares it.
        let declared = row.get::<_, Option<String>>(C_QUESTIONS)?;
        let n_q_declared = declared.map_or(0, |s| s.split(',').filter_map(|x| x.trim().parse::<usize>().ok()).sum());
        let town_name = row.get::<_, Option<i64>>(C_TOWN)?.and_then(|i| self.names.get(&i).cloned());
        let town = self.town(town_name);
        Ok(Some(Tournament { id, day, format, weight, train_only, town, editors, teams: Vec::new(), n_q: 0, n_q_declared }))
    }

    /// One result of the current tournament.
    fn result(&mut self, row: &rusqlite::Row) -> rusqlite::Result<()> {
        let members = row.get::<_, Option<String>>(C_MEMBERS)?.map_or(Vec::new(), |m| parse_ids(&m, &mut self.players, &self.alias));
        let mut mask = Vec::new();
        if let Some(m) = row.get::<_, Option<String>>(C_MASK)? {
            parse_mask(&m, &mut mask);
        }
        let score: Option<f64> = row.get::<_, Option<String>>(C_SCORE)?.and_then(|s| s.trim().parse().ok());
        let rating_json: Option<String> = row.get(C_RATING)?;
        let pred_pos = rating_json.as_deref().and_then(|s| json_number(s, "\"predictedPosition\""));
        let rated = rating_json.as_deref().is_some_and(|s| json_true(s, "\"inRating\""));
        let team = self.teams.get(row.get(C_TEAM)?);
        let town = self.town(row.get(C_TEAM_TOWN)?);
        let position = row.get::<_, Option<f64>>(C_POSITION)?.unwrap_or(0.0);
        let Some(t) = self.tournaments.last_mut() else { return Ok(()) };
        t.n_q = t.n_q.max(mask.len());
        let venue = t.id;
        let base = Vec::new();
        t.teams.push(TeamResult { team, position, venue, venue_position: position, score, pred_pos, rated, town, base, members, mask });
        Ok(())
    }

    fn finish(mut self, conn: &Connection) -> Arena {
        self.tournaments.retain(|t| !t.teams.is_empty());
        if self.cfg.data.merge_cities {
            let d = &self.cfg.data;
            merge_cities(&mut self.tournaments, d.merge_days, &links(conn), d.merge_venue_drop);
        }
        let rosters = base_rosters(conn);
        for t in self.tournaments.iter_mut() {
            mark_base(t, &rosters, &self.teams.ids, &self.players.ids);
            if t.n_q == 0 {
                t.n_q = t.n_q_declared;
            }
        }
        Arena {
            tournaments: self.tournaments,
            player_ids: self.players.ids,
            editor_ids: self.editors.ids,
            team_ids: self.teams.ids,
            towns: self.towns.ids,
        }
    }
}

/// Whether each member of each result was on the team's base roster that day.
fn mark_base(t: &mut Tournament, rosters: &Rosters, team_ids: &[i64], player_ids: &[i64]) {
    let day = t.day;
    for r in t.teams.iter_mut() {
        let team = team_ids[r.team as usize];
        let on = |&m: &u32| rosters.get(&(team, player_ids[m as usize])).is_some_and(|v| v.iter().any(|&(a, b)| day >= a && day <= b));
        r.base = r.members.iter().map(on).collect();
    }
}
