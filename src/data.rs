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
    pub members: Vec<u32>,
    pub mask: Vec<u8>,
}

pub struct Tournament {
    pub id: i64,
    pub day: i32,
    pub format: Format,
    pub weight: f64,
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

pub fn load(cfg: &Config, from: &str, to: &str) -> Result<Arena, rusqlite::Error> {
    let conn = Connection::open_with_flags(
        &cfg.data.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let rated = if cfg.data.include_unrated {
        ""
    } else {
        "t.in_rating = 1 AND"
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT t.id, substr(t.date_start,1,10), t.tournament_type, t.editors,
                r.team_id, r.position, r.questions_total, r.mask, r.team_members, r.rating,
                t.questions_by_tour
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
            let weight = format.weight(cfg);
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
            tournaments.push(Tournament {
                id: tid,
                day,
                format,
                weight,
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
        let t = tournaments.last_mut().unwrap();
        t.n_q = t.n_q.max(mask.len());
        t.teams.push(TeamResult {
            team,
            position: row.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
            score,
            pred_pos,
            rated,
            members,
            mask,
        });
    }
    tournaments.retain(|t| !t.teams.is_empty());
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
    })
}
