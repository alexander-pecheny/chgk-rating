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
/// `tools/fetch_towns.py`. An absent file leaves every tournament townless,
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

fn parse_ids(s: &str, index: &mut HashMap<i64, u32>, ids: &mut Vec<i64>, out: &mut Vec<u32>) {
    let mut cur: i64 = 0;
    let mut have = false;
    for c in s.bytes() {
        if c.is_ascii_digit() {
            cur = cur * 10 + (c - b'0') as i64;
            have = true;
        } else if have {
            let n = ids.len() as u32;
            out.push(*index.entry(cur).or_insert_with(|| {
                ids.push(cur);
                n
            }));
            cur = 0;
            have = false;
        }
    }
    if have {
        let n = ids.len() as u32;
        out.push(*index.entry(cur).or_insert_with(|| {
            ids.push(cur);
            n
        }));
    }
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
/// editors and question count played one package in several cities, so they
/// are one field. Each group is folded into its lowest id, provided every
/// result carries a score, and re-ranked by it, ties sharing the mean place.
fn merge_cities(ts: &mut Vec<Tournament>, days: i32) {
    let mut key_of: HashMap<(Vec<u32>, usize), Vec<(i32, usize)>> = HashMap::new();
    let mut into: Vec<Option<usize>> = vec![None; ts.len()];
    for (k, t) in ts.iter().enumerate() {
        if t.format != Format::InPerson || t.train_only || t.editors.is_empty() || t.n_q == 0 {
            continue;
        }
        if t.teams.iter().any(|r| r.score.is_none()) {
            continue;
        }
        let mut ed = t.editors.clone();
        ed.sort_unstable();
        let firsts = key_of.entry((ed, t.n_q)).or_default();
        match firsts.iter().find(|&&(d, _)| (t.day - d).abs() <= days) {
            Some(&(_, first)) => into[k] = Some(first),
            None => firsts.push((t.day, k)),
        }
    }
    let mut merged = vec![false; ts.len()];
    for k in 0..ts.len() {
        if let Some(first) = into[k] {
            let teams = std::mem::take(&mut ts[k].teams);
            ts[first].teams.extend(teams);
            if ts[k].id < ts[first].id {
                ts[first].id = ts[k].id;
            }
            merged[first] = true;
        }
    }
    let (mut groups, mut folded) = (0, 0);
    for (k, t) in ts.iter_mut().enumerate() {
        if into[k].is_some() {
            folded += 1;
        }
        if !merged[k] {
            continue;
        }
        groups += 1;
        // A question dropped at any venue is dropped at all of them, and the
        // scores are recounted, so every venue plays for the same questions.
        let nq = t.teams.iter().map(|r| r.mask.len()).max().unwrap_or(0);
        let mut drop = vec![false; nq];
        for r in t.teams.iter().filter(|r| !r.mask.is_empty()) {
            for (q, d) in drop.iter_mut().enumerate() {
                if r.mask.get(q).map_or(true, |&y| y == SKIP) {
                    *d = true;
                }
            }
        }
        for r in t.teams.iter_mut().filter(|r| !r.mask.is_empty()) {
            r.mask.resize(nq, SKIP);
            for (q, &d) in drop.iter().enumerate() {
                if d {
                    r.mask[q] = SKIP;
                }
            }
            r.score = Some(r.mask.iter().filter(|&&y| y == CORRECT).count() as f64);
        }
        let s: Vec<f64> = t.teams.iter().map(|r| r.score.unwrap_or(0.0)).collect();
        for (i, r) in t.teams.iter_mut().enumerate() {
            let above = s.iter().filter(|&&x| x > s[i]).count() as f64;
            let level = s.iter().filter(|&&x| x == s[i]).count() as f64;
            r.position = above + (level + 1.0) / 2.0;
        }
    }
    ts.retain(|t| !t.teams.is_empty());
    eprintln!("merge_cities: {folded} fields folded into {groups} tournaments");
}

/// The roster history `tools/prep_gg_inputs.py` writes from chgk.gg's dump, and
/// the season calendar that closes an open entry.
pub const BASE_ROSTERS: &str = "data/external/chgk_gg/base_rosters.csv";
pub const SEASONS: &str = "data/external/chgk_gg/seasons.csv";

/// For each (team, player) the day ranges they were on its base roster: the
/// chgk.gg history, joined with `buff.db`'s `team_seasons`, which holds the two
/// current seasons and so keeps the daily update current. An entry with no end
/// runs to the end of its season.
fn base_rosters(conn: &Connection) -> HashMap<(i64, i64), Vec<(i32, i32)>> {
    let mut season_end: HashMap<i64, i32> = HashMap::new();
    for l in std::fs::read_to_string(SEASONS).unwrap_or_default().lines().skip(1) {
        let f: Vec<&str> = l.trim().split(',').collect();
        if let (Some(Ok(id)), Some(e)) = (f.first().map(|x| x.parse()), f.get(2).and_then(|x| parse_date(x))) {
            season_end.insert(id, e);
        }
    }
    let mut m: HashMap<(i64, i64), Vec<(i32, i32)>> = HashMap::new();
    let mut add = |t: i64, pl: i64, s: i64, a: Option<i32>, b: Option<i32>| {
        if let Some(a) = a {
            let b = b.or(season_end.get(&s).copied()).unwrap_or(i32::MAX);
            m.entry((t, pl)).or_default().push((a, b));
        }
    };
    let csv = std::fs::read_to_string(BASE_ROSTERS).unwrap_or_default();
    if csv.is_empty() {
        eprintln!("warning: no {BASE_ROSTERS}; base rosters from buff.db alone");
    }
    for l in csv.lines().skip(1) {
        let f: Vec<&str> = l.trim().split(',').collect();
        if let (Some(Ok(t)), Some(Ok(pl)), Some(Ok(s))) = (
            f.first().map(|x| x.parse()),
            f.get(1).map(|x| x.parse()),
            f.get(2).map(|x| x.parse()),
        ) {
            add(t, pl, s, f.get(3).and_then(|x| parse_date(x)), f.get(4).and_then(|x| parse_date(x)));
        }
    }
    let rows = conn
        .prepare(
            "SELECT team_id, player_id, season_id, substr(date_added,1,10), substr(date_removed,1,10)
             FROM team_seasons",
        )
        .and_then(|mut st| {
            st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();
    for (t, pl, s, a, b) in rows {
        add(t, pl, s, a.as_deref().and_then(parse_date), b.as_deref().and_then(parse_date));
    }
    m
}

pub fn load(cfg: &Config, from: &str, to: &str) -> Result<Arena, rusqlite::Error> {
    let conn = Connection::open_with_flags(
        &cfg.data.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let rated = if cfg.data.include_unrated || cfg.data.train_unrated {
        ""
    } else if cfg.data.unrated_in_person {
        "(t.in_rating = 1 OR t.tournament_type IN ('Обычный', 'Региональный')) AND"
    } else {
        "t.in_rating = 1 AND"
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT t.id, substr(t.date_start,1,10), t.tournament_type, t.editors,
                r.team_id, r.position, r.questions_total, r.mask, r.team_members, r.rating,
                t.questions_by_tour, t.in_rating, t.town_id, r.team_current_town
         FROM tournaments t JOIN tournament_results r ON r.id = t.id
         WHERE {rated} substr(t.date_start,1,10) >= ?1 AND substr(t.date_start,1,10) < ?2
         ORDER BY substr(t.date_start,1,10), t.id"
    ))?;
    let mut rows = stmt.query([from, to])?;

    let mut pidx: HashMap<i64, u32> = HashMap::new();
    let mut player_ids: Vec<i64> = Vec::new();
    let mut eidx: HashMap<i64, u32> = HashMap::new();
    let mut editor_ids: Vec<i64> = Vec::new();
    let mut tidx: HashMap<i64, u32> = HashMap::new();
    let mut team_ids: Vec<i64> = Vec::new();
    let mut tournaments: Vec<Tournament> = Vec::new();
    let mut skip_id: i64 = i64::MIN;
    // One index per town name, shared by the tournament's own town and by the
    // towns the teams are registered in.
    let names = town_names(TOWNS);
    let mut wnames: Vec<String> = Vec::new();
    let mut widx: HashMap<String, u32> = HashMap::new();
    let mut town_of = |name: Option<String>| -> u32 {
        let Some(name) = name.filter(|s| !s.is_empty()) else {
            return u32::MAX;
        };
        let n = wnames.len() as u32;
        *widx.entry(name.clone()).or_insert_with(|| {
            wnames.push(name);
            n
        })
    };

    while let Some(row) = rows.next()? {
        let tid: i64 = row.get(0)?;
        if tid == skip_id {
            continue;
        }
        if tournaments.last().map(|t| t.id) != Some(tid) {
            let date: String = row.get(1)?;
            let kind: Option<String> = row.get(2)?;
            let format = kind.as_deref().and_then(Format::parse);
            let (day, format) = match (parse_date(&date), format) {
                (Some(d), Some(f)) => (d, f),
                _ => {
                    skip_id = tid;
                    continue;
                }
            };
            // `train_unrated` keeps the unrated tournaments for the player side
            // only, at a weight of their own; `include_unrated` treats them as
            // ordinary tournaments, which is the switch round 2 measured.
            let in_rating: i64 = row.get::<_, Option<i64>>(11)?.unwrap_or(0);
            let train_only = cfg.data.train_unrated && !cfg.data.include_unrated && in_rating == 0;
            let mut weight = format.weight(cfg);
            if train_only {
                weight *= cfg.data.w_unrated;
            }
            if weight <= 0.0 {
                skip_id = tid;
                continue;
            }
            let mut editors = Vec::new();
            if let Some(e) = row.get::<_, Option<String>>(3)? {
                parse_ids(&e, &mut eidx, &mut editor_ids, &mut editors);
            }
            // Tournaments whose results carry no mask still need a question count
            // for the pairwise scale; the tournament row declares it.
            let declared: usize = row
                .get::<_, Option<String>>(10)?
                .map(|s| {
                    s.split(',')
                        .filter_map(|x| x.trim().parse::<usize>().ok())
                        .sum()
                })
                .unwrap_or(0);
            let town = town_of(
                row.get::<_, Option<i64>>(12)?
                    .and_then(|i| names.get(&i).cloned()),
            );
            tournaments.push(Tournament {
                id: tid,
                day,
                format,
                weight,
                train_only,
                town,
                editors,
                teams: Vec::new(),
                n_q: 0,
                n_q_declared: declared,
            });
        }
        let mut members = Vec::new();
        if let Some(m) = row.get::<_, Option<String>>(8)? {
            parse_ids(&m, &mut pidx, &mut player_ids, &mut members);
        }
        let mut mask = Vec::new();
        if let Some(m) = row.get::<_, Option<String>>(7)? {
            parse_mask(&m, &mut mask);
        }
        let score: Option<f64> = row
            .get::<_, Option<String>>(6)?
            .and_then(|s| s.trim().parse().ok());
        let rating_json: Option<String> = row.get(9)?;
        let pred_pos = rating_json
            .as_deref()
            .and_then(|s| json_number(s, "\"predictedPosition\""));
        let rated = rating_json
            .as_deref()
            .is_some_and(|s| json_true(s, "\"inRating\""));
        let team_id: i64 = row.get(4)?;
        let n = team_ids.len() as u32;
        let team = *tidx.entry(team_id).or_insert_with(|| {
            team_ids.push(team_id);
            n
        });
        let town = town_of(row.get::<_, Option<String>>(13)?);
        let t = tournaments.last_mut().unwrap();
        t.n_q = t.n_q.max(mask.len());
        let position = row.get::<_, Option<f64>>(5)?.unwrap_or(0.0);
        let venue = t.id;
        t.teams.push(TeamResult {
            team,
            position,
            venue,
            venue_position: position,
            score,
            pred_pos,
            rated,
            town,
            base: Vec::new(),
            members,
            mask,
        });
    }
    tournaments.retain(|t| !t.teams.is_empty());
    if cfg.data.merge_cities {
        merge_cities(&mut tournaments, cfg.data.merge_days);
    }
    let rosters = base_rosters(&conn);
    for t in tournaments.iter_mut() {
        let day = t.day;
        for r in t.teams.iter_mut() {
            let team = team_ids[r.team as usize];
            r.base = r
                .members
                .iter()
                .map(|&m| {
                    rosters
                        .get(&(team, player_ids[m as usize]))
                        .is_some_and(|v| v.iter().any(|&(a, b)| day >= a && day <= b))
                })
                .collect();
        }
    }
    for t in tournaments.iter_mut() {
        if t.n_q == 0 {
            t.n_q = t.n_q_declared;
        }
    }
    Ok(Arena {
        tournaments,
        player_ids,
        editor_ids,
        team_ids,
        towns: wnames,
    })
}
