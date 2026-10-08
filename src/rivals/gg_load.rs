//! chgk.gg's inputs: its tournaments and lineups off the mirror (and
//! `extra.db`, the tournaments the mirror misses), the base rosters, the
//! bootstrap release, and optionally our player ratings to swap in.
use super::gg::{heredity, round_even, Bonus, Player, Res, Roster, Team, Trn, BONUS_DECAY};
use super::{open_read_only, parse_ids, Ids};
use crate::data::Format;
use crate::util::{csv_rows, format_date, parse_date};
use rusqlite::{Connection, Row};
use std::collections::{HashMap, HashSet};

/// Length of the date part of an ISO timestamp, and where its time fields sit.
const DATE_LEN: usize = 10;
const HOUR_AT: usize = 11;
const MINUTE_AT: usize = 14;
const OFFSET_FROM: usize = 16;
const FIELD_LEN: usize = 2;
const MINUTES_PER_HOUR: i32 = 60;
const MINUTES_PER_DAY: i32 = 1440;
/// Europe/Moscow is UTC+3, in minutes.
const MOSCOW_OFFSET: i32 = 180;
/// The mirror's position for a team that did not finish.
const NOT_PLACED: f64 = 9999.0;
/// Teams that leave the field from a shared place lift the rest by half a place each.
const HALF: f64 = 0.5;
const FLAG_KEY: &str = "\"flag\"";
/// The base-roster flag, as the mirror's JSON escapes it and as it does not.
const BASE_FLAGS: [&str; 2] = ["\\u0411", "Б"];
/// Tournament coefficients of A.4.6: strictly synchronous, and anything not in
/// person or regional.
const COEFF_STRICT_SYNC: f64 = 2.0 / 3.0;
const COEFF_OTHER: f64 = 0.5;
/// The days the heredity rules of A.4.7 changed.
const RULES_2021: &str = "2021-08-28";
const RULES_2022: &str = "2022-11-18";
/// How far before the bootstrap our player ratings are read from.
const OURS_LEAD_DAYS: i32 = 14;
const TOURNAMENTS: &str = "SELECT t.id, t.date_start, t.date_end, t.tournament_type, t.maii_rating,
            r.team_id, r.position, r.team_members, r.team_members_full,
            r.team_current_name, r.rating
     FROM tournaments t JOIN tournament_results r ON r.id = t.id
     WHERE substr(t.date_end,1,10) >= '2021-09-01'
     ORDER BY t.id";
const OURS: &str = "SELECT r.date, p.player_id, p.theta FROM player_ratings p JOIN releases r ON r.id = p.release_id \
     WHERE r.date >= ?1 ORDER BY r.date";

/// Date part of an ISO timestamp in Europe/Moscow, the timezone the release window
/// is evaluated in.
fn moscow_day(s: &str) -> Option<i32> {
    let day = parse_date(s.get(..DATE_LEN)?)?;
    if s.len() < OFFSET_FROM || s.as_bytes()[DATE_LEN] != b'T' {
        return Some(day);
    }
    let field = |at: usize| s[at..at + FIELD_LEN].parse::<i32>().ok();
    let (hh, mm) = (field(HOUR_AT)?, field(MINUTE_AT)?);
    let mut off = 0;
    if let Some(i) = s[OFFSET_FROM..].find(['+', '-']).map(|i| i + OFFSET_FROM) {
        let sign = if s.as_bytes()[i] == b'-' { -1 } else { 1 };
        off = sign * (field(i + 1)? * MINUTES_PER_HOUR + field(i + 1 + FIELD_LEN + 1)?);
    }
    Some(day + (hh * MINUTES_PER_HOUR + mm - off + MOSCOW_OFFSET).div_euclid(MINUTES_PER_DAY))
}

/// Roster flags in `team_members_full`, in the order of `team_members`.
fn flags_of(full: &str) -> Vec<bool> {
    let mut out = Vec::new();
    let mut rest = full;
    while let Some(i) = rest.find(FLAG_KEY) {
        rest = rest[i + FLAG_KEY.len()..].trim_start().trim_start_matches(':').trim_start();
        match rest.strip_prefix('"') {
            Some(r) => {
                let end = r.find('"').unwrap_or(0);
                out.push(BASE_FLAGS.contains(&&r[..end]));
                rest = &r[end..];
            }
            None => out.push(false),
        }
    }
    out
}

fn json_number(s: &str, key: &str) -> Option<f64> {
    let i = s.find(key)? + key.len();
    let rest = s[i..].trim_start().strip_prefix(':')?.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn coeff_of(kind: &str) -> f64 {
    match kind {
        "Обычный" | "Региональный" => 1.0,
        "Строго синхронный" => COEFF_STRICT_SYNC,
        _ => COEFF_OTHER,
    }
}

struct Meta {
    id: i64,
    start: i32,
    end: i32,
    coeff: f64,
    maii: bool,
    kind: String,
}

struct RawRes {
    team_id: i64,
    position: f64,
    members: Vec<i64>,
    flags: Vec<bool>,
    name_kept: bool,
    pred_pos: Option<f64>,
}

/// Which team each player counts for: a base seat beats a legionnaire's, then
/// the lower team id.
fn owners(raw: &[RawRes]) -> HashMap<i64, (bool, i64)> {
    let mut owner: HashMap<i64, (bool, i64)> = HashMap::new();
    for r in raw {
        for (k, p) in r.members.iter().enumerate() {
            let base = r.flags[k];
            let e = owner.entry(*p).or_insert((base, r.team_id));
            if (base, -r.team_id) > (e.0, -e.1) {
                *e = (base, r.team_id);
            }
        }
    }
    owner
}

/// The ids interned while a tournament is finished, and the heredity rule days.
struct Interning<'a> {
    teams: &'a mut Ids,
    players: &'a mut Ids,
    rules: (i32, i32),
}

impl Interning<'_> {
    /// A player rostered on two teams here counts for one; a team whose roster is
    /// unknown leaves the field and everyone below it moves up.
    fn finish(&mut self, m: Meta, raw: &[RawRes]) -> Option<Trn> {
        let n = raw.len();
        if n == 0 || raw.iter().any(|r| r.position > n as f64) {
            return None;
        }
        let owner = owners(raw);
        let mine = |r: &RawRes| -> Vec<usize> {
            (0..r.members.len()).filter(|&k| owner[&r.members[k]].1 == r.team_id).collect()
        };
        let keep: Vec<bool> = raw.iter().map(|r| !mine(r).is_empty()).collect();
        if keep.iter().all(|k| !k) {
            return None;
        }
        let gone: Vec<f64> = (0..n).filter(|&i| !keep[i]).map(|i| raw[i].position).collect();
        let teams = (0..n).filter(|&i| keep[i]).map(|i| self.result(&m, &raw[i], &mine(&raw[i]), &gone)).collect();
        Some(Trn { id: m.id, coeff: m.coeff, maii: m.maii, end: m.end, kind: m.kind, teams })
    }

    fn result(&mut self, m: &Meta, r: &RawRes, mine: &[usize], gone: &[f64]) -> Res {
        let above = gone.iter().filter(|p| **p < r.position).count() as f64;
        let same = gone.iter().filter(|p| **p == r.position).count() as f64;
        let (mut members, mut base, mut n_legs) = (Vec::new(), Vec::new(), 0);
        for &k in mine {
            let idx = self.players.get(r.members[k]);
            members.push(idx);
            if r.flags[k] {
                base.push(idx);
            } else {
                n_legs += 1;
            }
        }
        Res {
            team: self.teams.get(r.team_id),
            position: r.position - above - same * HALF,
            heredity: heredity(m.start, self.rules, base.len() as u32, n_legs, r.name_kept),
            members,
            base,
            n_legs,
            pred_pos: r.pred_pos,
        }
    }
}

fn names(conn: &Connection) -> rusqlite::Result<HashMap<i64, String>> {
    let mut names = HashMap::new();
    let mut st = conn.prepare("SELECT id, name FROM teams")?;
    let mut rs = st.query([])?;
    while let Some(row) = rs.next()? {
        names.insert(row.get("id")?, row.get("name")?);
    }
    Ok(names)
}

fn meta_of(row: &Row, id: i64) -> rusqlite::Result<Option<Meta>> {
    let ds: String = row.get("date_start")?;
    let de: String = row.get("date_end")?;
    let kind: String = row.get::<_, Option<String>>("tournament_type")?.unwrap_or_default();
    let (Some(start), Some(end)) = (moscow_day(&ds), moscow_day(&de)) else {
        return Ok(None);
    };
    Ok(Some(Meta {
        id,
        start,
        end,
        coeff: coeff_of(&kind),
        maii: row.get::<_, Option<i64>>("maii_rating")? == Some(1),
        kind: Format::parse(&kind).map_or("other", |f| f.name()).to_string(),
    }))
}

fn raw_of(row: &Row, names: &HashMap<i64, String>) -> rusqlite::Result<Option<RawRes>> {
    let position = match row.get::<_, Option<f64>>("position")? {
        Some(p) if p != 0.0 && p != NOT_PLACED => p,
        _ => return Ok(None),
    };
    let members = parse_ids(&row.get::<_, Option<String>>("team_members")?.unwrap_or_default());
    let mut flags = flags_of(&row.get::<_, Option<String>>("team_members_full")?.unwrap_or_default());
    if flags.len() != members.len() {
        flags = vec![false; members.len()];
    }
    let team_id: i64 = row.get("team_id")?;
    let current: Option<String> = row.get("team_current_name")?;
    let rating = row.get::<_, Option<String>>("rating")?;
    Ok(Some(RawRes {
        team_id,
        position,
        members,
        flags,
        name_kept: names.get(&team_id) == current.as_ref(),
        pred_pos: rating.as_deref().and_then(|s| json_number(s, "\"predictedPosition\"")),
    }))
}

fn tournaments(conn: &Connection, teams: &mut Ids, players: &mut Ids) -> rusqlite::Result<Vec<Trn>> {
    let rules = (parse_date(RULES_2021).unwrap(), parse_date(RULES_2022).unwrap());
    let mut it = Interning { teams, players, rules };
    let names = names(conn)?;
    let mut st = conn.prepare(TOURNAMENTS)?;
    let mut rows = st.query([])?;
    let (mut out, mut raw, mut cur): (Vec<Trn>, Vec<RawRes>, Option<Meta>) = (vec![], vec![], None);
    let mut last_id = None;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get("id")?;
        if last_id != Some(id) {
            out.extend(cur.take().and_then(|m| it.finish(m, &raw)));
            raw.clear();
            cur = meta_of(row, id)?;
            last_id = Some(id);
        }
        if cur.is_some() {
            raw.extend(raw_of(row, &names)?);
        }
    }
    out.extend(cur.take().and_then(|m| it.finish(m, &raw)));
    Ok(out)
}

/// The mirror's tournaments, and those of `extra.db` it lacks: the mirror
/// catches up with some of them, and its copy wins.
pub(super) fn all_tournaments(conn: &Connection, ext: &str, teams: &mut Ids, players: &mut Ids) -> rusqlite::Result<Vec<Trn>> {
    let mut out = tournaments(conn, teams, players)?;
    let extra_path = format!("{ext}/extra.db");
    if std::path::Path::new(&extra_path).exists() {
        let have: HashSet<i64> = out.iter().map(|t| t.id).collect();
        let more: Vec<Trn> = (tournaments(&open_read_only(&extra_path)?, teams, players)?.into_iter())
            .filter(|t| !have.contains(&t.id))
            .collect();
        eprintln!("{} tournaments from extra.db", more.len());
        out.extend(more);
    }
    Ok(out)
}

/// The seasons as (id, first day, last day).
pub(super) fn seasons(conn: &Connection) -> rusqlite::Result<Vec<(i32, i32, i32)>> {
    let mut out = Vec::new();
    let mut st = conn.prepare("SELECT id, date_start, date_end FROM seasons")?;
    let mut rs = st.query([])?;
    while let Some(row) = rs.next()? {
        let a: String = row.get("date_start")?;
        let b: String = row.get("date_end")?;
        if let (Some(x), Some(y)) = (parse_date(&a[..DATE_LEN]), parse_date(&b[..DATE_LEN])) {
            out.push((row.get("id")?, x, y));
        }
    }
    Ok(out)
}

/// `base_rosters.csv`: team, player, season, from, to. In order of start, then team id.
pub(super) fn rosters(ext: &str, teams: &mut Ids, players: &mut Ids) -> Result<Vec<Roster>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    for r in csv_rows(&format!("{ext}/base_rosters.csv"))?.into_iter().skip(1) {
        out.push(Roster {
            team: teams.get(r[0].parse()?),
            player: players.get(r[1].parse()?),
            season: r[2].parse()?,
            start: parse_date(&r[3]),
            end: parse_date(&r[4]),
        });
    }
    out.sort_by_key(|r| (r.start.unwrap_or(i32::MIN), teams.list[r.team as usize]));
    Ok(out)
}

/// `bootstrap_players.csv`: each player's bonuses as of the bootstrap release.
pub(super) fn bootstrap_players(ext: &str, ids: &mut Ids) -> Result<Vec<Player>, Box<dyn std::error::Error>> {
    let mut players: Vec<Player> = Vec::new();
    for r in csv_rows(&format!("{ext}/bootstrap_players.csv"))?.into_iter().skip(1) {
        let p = ids.get(r[0].parse()?) as usize;
        if players.len() <= p {
            players.resize_with(p + 1, Player::default);
        }
        let initial: f64 = r[2].parse()?;
        let weeks: i32 = r[3].parse()?;
        let raw = initial * BONUS_DECAY.powi(weeks);
        players[p].bonuses.push(Bonus { tournament: r[1].parse()?, initial, weeks, cur: round_even(raw), raw });
    }
    for p in players.iter_mut() {
        p.rating = p.bonuses.iter().map(|b| b.cur).sum();
    }
    Ok(players)
}

/// Our releases from `boot - 14` on, as (release day, [(player index, theta)]), ascending.
pub(super) fn ours(path: &str, boot: i32, ids: &mut Ids) -> rusqlite::Result<Vec<(i32, Vec<(u32, f64)>)>> {
    let mut ours: Vec<(i32, Vec<(u32, f64)>)> = Vec::new();
    let oc = open_read_only(path)?;
    let mut st = oc.prepare(OURS)?;
    let mut rs = st.query([format_date(boot - OURS_LEAD_DAYS)])?;
    while let Some(row) = rs.next()? {
        let d = parse_date(&row.get::<_, String>("date")?[..DATE_LEN]).unwrap();
        if ours.last().is_none_or(|l| l.0 != d) {
            ours.push((d, Vec::new()));
        }
        let p = ids.get(row.get("player_id")?);
        ours.last_mut().unwrap().1.push((p, row.get("theta")?));
    }
    eprintln!("{} of our releases from {path}", ours.len());
    Ok(ours)
}

/// The official team table of the bootstrap release, by team index.
pub(super) fn bootstrap_teams(conn: &Connection, release: i64, ids: &mut Ids) -> rusqlite::Result<Vec<Option<Team>>> {
    let mut teams: Vec<Option<Team>> = Vec::new();
    let mut st = conn.prepare("SELECT team_id, rating, trb FROM ratings WHERE release_id = ?1")?;
    let mut rs = st.query([release])?;
    while let Some(row) = rs.next()? {
        let t = ids.get(row.get("team_id")?) as usize;
        if teams.len() <= t {
            teams.resize(t + 1, None);
        }
        teams[t] = Some(Team { rating: row.get::<_, i64>("rating")? as f64, trb: row.get::<_, i64>("trb")? as f64 });
    }
    Ok(teams)
}
