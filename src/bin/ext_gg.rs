//! rating.chgk.gg, the current official rating of sports ChGK (the "b" model),
//! reimplemented from `chgk-gg/rating-b`. A.x tags name the clause of Приложение А
//! (`data/external/chgk_gg/prilozhenie_A.txt`) that the code below transcribes.
//! Bootstrapped from the official release of 2021-09-09, stepped weekly to the
//! present, written to the ratings.db schema of DESIGN 6.
use chgk_rating::data::Format;
use chgk_rating::output::Writer;
use chgk_rating::team_model::TeamEvent;
use chgk_rating::util::{csv_rows, format_date, parse_date, spearman};
use rusqlite::Connection;
use std::collections::HashMap;

const J: f64 = 0.99;
const N_BEST: usize = 7;
const MAX_BONUS: f64 = 2300.0;
const N_BP: usize = 15;
const NEW_TEAM_COEFF: f64 = 0.8;
const TOP_TEAMS_Q: usize = 100;
const MIN_BASE_FOR_Q: usize = 6;
const D2_MUL: f64 = 300.0;
const D2_DEN: f64 = 350.0;
const D1_NEG: f64 = 0.5;
const MIN_TR_RATIO: f64 = 0.5;
const MIN_LEGS: u32 = 3;
const TR_PLAYERS: usize = 6;
const BOOTSTRAP: &str = "2021-09-09";

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

/// Postgres's numeric -> integer cast: half away from zero.
fn round_half_up(x: f64) -> f64 {
    if x < 0.0 {
        -(-x + 0.5).floor()
    } else {
        (x + 0.5).floor()
    }
}

/// numpy's and Python's round: half to even.
fn round_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// A.4.9.1
fn tech_rating(ratings: &mut [f64], q: f64) -> f64 {
    ratings.sort_by(|a, b| b.total_cmp(a));
    let mut s = 0.0;
    for (k, v) in ratings.iter().take(TR_PLAYERS).enumerate() {
        s += v * (TR_PLAYERS - k) as f64 / TR_PLAYERS as f64;
    }
    round_even(s) * q
}

/// Places from scores, ties sharing the mean of the range they span.
fn calc_places(points: &[f64]) -> Vec<f64> {
    let mut o: Vec<usize> = (0..points.len()).collect();
    o.sort_by(|&a, &b| points[b].total_cmp(&points[a]));
    let mut out = vec![0.0; points.len()];
    let mut i = 0;
    while i < o.len() {
        let mut j = i;
        while j + 1 < o.len() && points[o[j + 1]] == points[o[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for &k in &o[i..=j] {
            out[k] = avg;
        }
        i = j + 1;
    }
    out
}

/// A.4.4.3
fn bonus_predictions(rg_desc: &[f64], c: f64) -> Vec<f64> {
    let n = rg_desc.len();
    let mut out = vec![0.0; n];
    for (i, o) in out.iter_mut().enumerate() {
        let mut s = 0.0;
        for k in 0..N_BP.min(n - i) {
            s += rg_desc[i + k] * 2f64.powi(-(k as i32));
        }
        *o = round_even(s * c);
    }
    for i in 1..n {
        if rg_desc[i] == rg_desc[i - 1] {
            out[i] = out[i - 1];
        }
    }
    out
}

/// A.4.5
fn score_real(pred: &[f64], positions: &[f64]) -> Vec<f64> {
    let mut shared: HashMap<u64, usize> = HashMap::new();
    for p in positions {
        *shared.entry(p.to_bits()).or_insert(0) += 1;
    }
    positions
        .iter()
        .map(|p| {
            let pos = p - 1.0;
            let n = shared[&p.to_bits()] as f64;
            let lo = ((pos - (n - 1.0) / 2.0) as i64).max(0) as usize;
            let hi = ((pos + (n - 1.0) / 2.0) as i64).max(0) as usize + 1;
            let s = &pred[lo.min(pred.len())..hi.min(pred.len())];
            if s.is_empty() {
                0.0
            } else {
                round_even(s.iter().sum::<f64>() / s.len() as f64)
            }
        })
        .collect()
}

/// A.4.7, in the three forms it has had.
fn heredity(start: i32, rules_2021: i32, rules_2022: i32, n_base: u32, n_legs: u32, name_kept: bool) -> bool {
    if start < rules_2021 {
        n_base >= 4 || (n_base == 3 && name_kept)
    } else if start < rules_2022 {
        n_base >= 4
    } else {
        n_base >= 3 && n_legs < n_base && n_legs <= MIN_LEGS
    }
}

fn coeff_of(kind: &str) -> f64 {
    match kind {
        "Обычный" | "Региональный" => 1.0,
        "Строго синхронный" => 2.0 / 3.0,
        _ => 0.5,
    }
}

/// Date part of an ISO timestamp in Europe/Moscow, the timezone the release window
/// is evaluated in.
fn moscow_day(s: &str) -> Option<i32> {
    let day = parse_date(s.get(..10)?)?;
    if s.len() < 16 || s.as_bytes()[10] != b'T' {
        return Some(day);
    }
    let hh: i32 = s[11..13].parse().ok()?;
    let mm: i32 = s[14..16].parse().ok()?;
    let mut off = 0;
    if let Some(i) = s[16..].find(['+', '-']).map(|i| i + 16) {
        let sign = if s.as_bytes()[i] == b'-' { -1 } else { 1 };
        off = sign * (s[i + 1..i + 3].parse::<i32>().ok()? * 60 + s[i + 4..i + 6].parse::<i32>().ok()?);
    }
    Some(day + (hh * 60 + mm - off + 180).div_euclid(1440))
}

/// Roster flags in `team_members_full`, in the order of `team_members`.
fn flags_of(full: &str) -> Vec<bool> {
    let mut out = Vec::new();
    let mut rest = full;
    while let Some(i) = rest.find("\"flag\"") {
        rest = rest[i + 6..].trim_start().trim_start_matches(':').trim_start();
        match rest.strip_prefix('"') {
            Some(r) => {
                let end = r.find('"').unwrap_or(0);
                out.push(&r[..end] == "\\u0411" || &r[..end] == "Б");
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

struct Res {
    team: u32,
    position: f64,
    members: Vec<u32>,
    base: Vec<u32>,
    n_legs: u32,
    heredity: bool,
    pred_pos: Option<f64>,
}

struct Trn {
    id: i64,
    coeff: f64,
    maii: bool,
    end: i32,
    kind: String,
    teams: Vec<Res>,
}

struct Scored {
    rt: Vec<f64>,
    r: Vec<f64>,
    rb: Vec<f64>,
    rg: Vec<f64>,
    mp: Vec<f64>,
    bp: Vec<f64>,
    b: Vec<f64>,
    d1: Vec<f64>,
    d2: Vec<f64>,
    bonus: Vec<f64>,
}

struct Ids {
    map: HashMap<i64, u32>,
    list: Vec<i64>,
}

impl Ids {
    fn new() -> Ids {
        Ids {
            map: HashMap::new(),
            list: Vec::new(),
        }
    }
    fn get(&mut self, id: i64) -> u32 {
        let n = self.list.len() as u32;
        *self.map.entry(id).or_insert_with(|| {
            self.list.push(id);
            n
        })
    }
}

fn parse_ids(s: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let mut cur: i64 = 0;
    let mut have = false;
    for c in s.bytes() {
        if c.is_ascii_digit() {
            cur = cur * 10 + (c - b'0') as i64;
            have = true;
        } else if have {
            out.push(cur);
            cur = 0;
            have = false;
        }
    }
    if have {
        out.push(cur);
    }
    out
}

struct Bonus {
    tournament: i64,
    initial: f64,
    weeks: i32,
    cur: f64,
    raw: f64,
}

#[derive(Default)]
struct Player {
    rating: f64,
    bonuses: Vec<Bonus>,
}

#[derive(Clone, Copy)]
struct Team {
    rating: f64,
    trb: f64,
}

struct Roster {
    team: u32,
    player: u32,
    season: i32,
    start: Option<i32>,
    end: Option<i32>,
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

/// A player rostered on two teams here counts for one; a team whose roster is
/// unknown leaves the field and everyone below it moves up.
fn finish(m: Meta, raw: &[RawRes], teams: &mut Ids, players: &mut Ids, rules: (i32, i32)) -> Option<Trn> {
    let n = raw.len();
    if n == 0 || raw.iter().any(|r| r.position > n as f64) {
        return None;
    }
    let mut owner: HashMap<i64, (bool, i64)> = HashMap::new();
    for r in raw.iter() {
        for (k, p) in r.members.iter().enumerate() {
            let base = r.flags[k];
            let e = owner.entry(*p).or_insert((base, r.team_id));
            if (base, -r.team_id) > (e.0, -e.1) {
                *e = (base, r.team_id);
            }
        }
    }
    let mine = |r: &RawRes| -> Vec<usize> {
        (0..r.members.len())
            .filter(|&k| owner[&r.members[k]].1 == r.team_id)
            .collect()
    };
    let keep: Vec<bool> = (0..n).map(|i| !mine(&raw[i]).is_empty()).collect();
    if keep.iter().all(|k| !k) {
        return None;
    }
    let gone: Vec<f64> = (0..n).filter(|&i| !keep[i]).map(|i| raw[i].position).collect();
    let mut out = Vec::new();
    for r in (0..n).filter(|&i| keep[i]).map(|i| &raw[i]) {
        let above = gone.iter().filter(|p| **p < r.position).count() as f64;
        let same = gone.iter().filter(|p| **p == r.position).count() as f64;
        let (mut members, mut base, mut n_legs) = (Vec::new(), Vec::new(), 0);
        for k in mine(r) {
            let idx = players.get(r.members[k]);
            members.push(idx);
            if r.flags[k] {
                base.push(idx);
            } else {
                n_legs += 1;
            }
        }
        out.push(Res {
            team: teams.get(r.team_id),
            position: r.position - above - same / 2.0,
            heredity: heredity(m.start, rules.0, rules.1, base.len() as u32, n_legs, r.name_kept),
            members,
            base,
            n_legs,
            pred_pos: r.pred_pos,
        });
    }
    Some(Trn {
        id: m.id,
        coeff: m.coeff,
        maii: m.maii,
        end: m.end,
        kind: m.kind,
        teams: out,
    })
}

fn load_tournaments(conn: &Connection, teams: &mut Ids, players: &mut Ids) -> rusqlite::Result<Vec<Trn>> {
    let rules = (parse_date("2021-08-28").unwrap(), parse_date("2022-11-18").unwrap());
    let mut names: HashMap<i64, String> = HashMap::new();
    let mut st = conn.prepare("SELECT id, name FROM teams")?;
    let mut rs = st.query([])?;
    while let Some(row) = rs.next()? {
        names.insert(row.get(0)?, row.get(1)?);
    }

    let mut st = conn.prepare(
        "SELECT t.id, t.date_start, t.date_end, t.tournament_type, t.maii_rating,
                r.team_id, r.position, r.team_members, r.team_members_full,
                r.team_current_name, r.rating
         FROM tournaments t JOIN tournament_results r ON r.id = t.id
         WHERE substr(t.date_end,1,10) >= '2021-09-01'
         ORDER BY t.id",
    )?;
    let mut rows = st.query([])?;
    let mut out: Vec<Trn> = Vec::new();
    let mut raw: Vec<RawRes> = Vec::new();
    let mut cur: Option<Meta> = None;

    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        if cur.as_ref().map(|c| c.id) != Some(id) {
            if let Some(m) = cur.take() {
                out.extend(finish(m, &raw, teams, players, rules));
            }
            raw.clear();
            let ds: String = row.get(1)?;
            let de: String = row.get(2)?;
            let kind: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
            cur = match (moscow_day(&ds), moscow_day(&de)) {
                (Some(start), Some(end)) => Some(Meta {
                    id,
                    start,
                    end,
                    coeff: coeff_of(&kind),
                    maii: row.get::<_, Option<i64>>(4)? == Some(1),
                    kind: Format::parse(&kind).map_or("other", |f| f.name()).to_string(),
                }),
                _ => None,
            };
        }
        if cur.is_none() {
            continue;
        }
        let position = match row.get::<_, Option<f64>>(6)? {
            Some(p) if p != 0.0 && p != 9999.0 => p,
            _ => continue,
        };
        let members = parse_ids(&row.get::<_, Option<String>>(7)?.unwrap_or_default());
        let mut flags = flags_of(&row.get::<_, Option<String>>(8)?.unwrap_or_default());
        if flags.len() != members.len() {
            flags = vec![false; members.len()];
        }
        let team_id: i64 = row.get(5)?;
        let current: Option<String> = row.get(9)?;
        raw.push(RawRes {
            team_id,
            position,
            members,
            flags,
            name_kept: names.get(&team_id) == current.as_ref(),
            pred_pos: row
                .get::<_, Option<String>>(10)?
                .as_deref()
                .and_then(|s| json_number(s, "\"predictedPosition\"")),
        });
    }
    if let Some(m) = cur.take() {
        out.extend(finish(m, &raw, teams, players, rules));
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = arg("--db").unwrap_or_else(|| "buff.db".into());
    let out = arg("--out").unwrap_or_else(|| "out/ext/gg.db".into());
    let to = arg("--to").unwrap_or_else(|| "2026-09-10".into());
    let rows_every: i64 = arg("--rows-every").and_then(|s| s.parse().ok()).unwrap_or(1);
    let ext = arg("--ext").unwrap_or_else(|| "data/external/chgk_gg".into());
    let started = std::time::Instant::now();

    let conn = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut team_ids = Ids::new();
    let mut player_ids = Ids::new();
    let mut tournaments = load_tournaments(&conn, &mut team_ids, &mut player_ids)?;
    let extra_path = format!("{ext}/extra.db");
    if std::path::Path::new(&extra_path).exists() {
        let extra = Connection::open_with_flags(&extra_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let more = load_tournaments(&extra, &mut team_ids, &mut player_ids)?;
        eprintln!("{} tournaments from extra.db", more.len());
        tournaments.extend(more);
    }

    let mut seasons: Vec<(i32, i32, i32)> = Vec::new();
    let mut st = conn.prepare("SELECT id, date_start, date_end FROM seasons")?;
    let mut rs = st.query([])?;
    while let Some(row) = rs.next()? {
        let a: String = row.get(1)?;
        let b: String = row.get(2)?;
        if let (Some(x), Some(y)) = (parse_date(&a[..10]), parse_date(&b[..10])) {
            seasons.push((row.get(0)?, x, y));
        }
    }
    let season_at = |d: i32| seasons.iter().find(|s| s.1 <= d && d <= s.2).map(|s| s.0).unwrap_or(0);
    let season_start = |id: i32| seasons.iter().find(|s| s.0 == id).map(|s| s.1).unwrap_or(0);

    let mut rosters: Vec<Roster> = Vec::new();
    for r in csv_rows(&format!("{ext}/base_rosters.csv"))?.into_iter().skip(1) {
        rosters.push(Roster {
            team: team_ids.get(r[0].parse()?),
            player: player_ids.get(r[1].parse()?),
            season: r[2].parse()?,
            start: parse_date(&r[3]),
            end: parse_date(&r[4]),
        });
    }
    rosters.sort_by_key(|r| (r.start.unwrap_or(i32::MIN), team_ids.list[r.team as usize]));
    let mut by_season: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, r) in rosters.iter().enumerate() {
        by_season.entry(r.season).or_default().push(i);
    }
    let season_rows = |s: i32| by_season.get(&s).map(|v| v.as_slice()).unwrap_or(&[]);

    let mut players: Vec<Player> = Vec::new();
    for r in csv_rows(&format!("{ext}/bootstrap_players.csv"))?.into_iter().skip(1) {
        let p = player_ids.get(r[0].parse()?) as usize;
        while players.len() <= p {
            players.push(Player::default());
        }
        let initial: f64 = r[2].parse()?;
        let weeks: i32 = r[3].parse()?;
        let raw = initial * J.powi(weeks);
        players[p].bonuses.push(Bonus {
            tournament: r[1].parse()?,
            initial,
            weeks,
            cur: round_even(raw),
            raw,
        });
    }
    for p in players.iter_mut() {
        p.rating = p.bonuses.iter().map(|b| b.cur).sum();
    }

    let boot_day = parse_date(BOOTSTRAP).unwrap();
    let boot_release: i64 =
        conn.query_row("SELECT id FROM releases WHERE date = ?1", [BOOTSTRAP], |r| r.get(0))?;
    let mut teams: Vec<Option<Team>> = Vec::new();
    let mut st = conn.prepare("SELECT team_id, rating, trb FROM ratings WHERE release_id = ?1")?;
    let mut rs = st.query([boot_release])?;
    while let Some(row) = rs.next()? {
        let t = team_ids.get(row.get(0)?) as usize;
        while teams.len() <= t {
            teams.push(None);
        }
        teams[t] = Some(Team {
            rating: row.get::<_, i64>(1)? as f64,
            trb: row.get::<_, i64>(2)? as f64,
        });
    }
    while teams.len() < team_ids.list.len() {
        teams.push(None);
    }
    while players.len() < player_ids.list.len() {
        players.push(Player::default());
    }
    eprintln!(
        "{} tournaments, {} roster rows; bootstrap {BOOTSTRAP}: {} teams, {} players",
        tournaments.len(),
        rosters.len(),
        teams.iter().filter(|t| t.is_some()).count(),
        players.iter().filter(|p| p.rating > 0.0).count()
    );

    let mut by_end: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, t) in tournaments.iter().enumerate() {
        by_end.entry(t.end).or_default().push(i);
    }
    let w = Writer::create(&out)?;
    let to_day = parse_date(&to).unwrap();
    let mut official_id = conn.prepare("SELECT id FROM releases WHERE date = ?1")?;
    let mut fallback = boot_release;
    let mut day = boot_day + 7;

    while day <= to_day {
        let prev_day = day - 7;
        fallback += 1;
        let release_id: i64 = official_id
            .query_row([format_date(day)], |r| r.get(0))
            .unwrap_or(fallback);
        let season = season_at(day);

        // A.4.4.3's C, off the release table as published, before this week's changes.
        let c = {
            let mut all: Vec<f64> = teams.iter().flatten().map(|t| t.rating).collect();
            all.sort_by(|a, b| b.total_cmp(a));
            MAX_BONUS
                / all.iter()
                    .take(N_BP)
                    .enumerate()
                    .map(|(k, v)| v * 2f64.powi(-(k as i32)))
                    .sum::<f64>()
        };

        // A.2.3
        let mut base_team: Vec<Option<u32>> = vec![None; players.len()];
        for r in season_rows(season).iter().map(|&i| &rosters[i]) {
            if r.start.is_some_and(|s| s <= day) && r.end.is_none_or(|e| e > day) {
                base_team[r.player as usize] = Some(r.team);
            }
        }
        let mut by_base: HashMap<u32, Vec<f64>> = HashMap::new();
        for (p, bt) in base_team.iter().enumerate() {
            if let (Some(t), true) = (bt, players[p].rating > 0.0) {
                by_base.entry(*t).or_default().push(players[p].rating);
            }
        }

        // A.4.9.2
        let mut order: Vec<usize> = (0..teams.len()).filter(|&i| teams[i].is_some()).collect();
        order.sort_by(|&a, &b| {
            teams[b]
                .unwrap()
                .rating
                .total_cmp(&teams[a].unwrap().rating)
                .then(team_ids.list[a].cmp(&team_ids.list[b]))
        });
        let (mut q_sum, mut q_n) = (0.0, 0.0);
        for &t in order.iter().take(TOP_TEAMS_Q) {
            match by_base.get(&(t as u32)) {
                Some(rs) if rs.len() >= MIN_BASE_FOR_Q => {
                    let raw = tech_rating(&mut rs.clone(), 1.0);
                    if raw != 0.0 {
                        q_sum += teams[t].unwrap().rating / raw;
                        q_n += 1.0;
                    }
                }
                _ => {}
            }
        }
        let q = q_sum / q_n;

        // A.4.9
        for t in teams.iter_mut().flatten() {
            t.trb = 0.0;
        }
        for (t, rs) in by_base.iter() {
            if let Some(Some(team)) = teams.get_mut(*t as usize) {
                team.trb = tech_rating(&mut rs.clone(), q);
            }
        }

        // A.4.2.3
        let sstart = season_start(season);
        for r in season_rows(season).iter().map(|&i| &rosters[i]) {
            let eff = r.start.unwrap_or(sstart).max(sstart);
            if eff > prev_day && eff <= day {
                if let Some(Some(team)) = teams.get_mut(r.team as usize) {
                    team.rating = team.rating.max(team.trb * NEW_TEAM_COEFF);
                }
            }
        }

        let mut idxs: Vec<usize> = Vec::new();
        for d in prev_day + 1..=day {
            if let Some(v) = by_end.get(&d) {
                idxs.extend(v);
            }
        }
        idxs.sort_by_key(|&i| tournaments[i].id);

        // Every tournament of the week is scored against the state as it stood at the
        // release; only MAII tournaments then move it.
        let mut scored: Vec<Scored> = Vec::new();
        for &ti in idxs.iter() {
            let t = &tournaments[ti];
            for res in t.teams.iter() {
                if teams[res.team as usize].is_none() {
                    let mut rs: Vec<f64> = res.base.iter().map(|&p| players[p as usize].rating).collect();
                    let trb = tech_rating(&mut rs, q);
                    teams[res.team as usize] = Some(Team {
                        rating: trb * NEW_TEAM_COEFF,
                        trb,
                    });
                }
            }
            let n = t.teams.len();
            let (mut rt, mut r, mut rb, mut rg) = (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
            for (k, res) in t.teams.iter().enumerate() {
                let team = teams[res.team as usize].unwrap();
                let mut rs: Vec<f64> = res.members.iter().map(|&p| players[p as usize].rating).collect();
                rt[k] = tech_rating(&mut rs, q);
                r[k] = if res.heredity { team.rating } else { 0.0 };
                rb[k] = if res.heredity { team.trb } else { 0.0 };
                let g = if rb[k] != 0.0 { r[k] * rt[k] / rb[k] } else { rt[k] };
                rg[k] = if rt[k] < rb[k] {
                    g.max(MIN_TR_RATIO * r[k])
                } else {
                    g.min(r[k].max(rt[k]))
                };
            }
            let mp = calc_places(&rg);
            let mut ord: Vec<usize> = (0..n).collect();
            ord.sort_by(|&a, &b| rg[b].total_cmp(&rg[a]));
            let rg_desc: Vec<f64> = ord.iter().map(|&i| rg[i]).collect();
            let pos_desc: Vec<f64> = ord.iter().map(|&i| t.teams[i].position).collect();
            let bp_desc = bonus_predictions(&rg_desc, c);
            let b_desc = score_real(&bp_desc, &pos_desc);
            let (mut bp, mut b) = (vec![0.0; n], vec![0.0; n]);
            for (j, &i) in ord.iter().enumerate() {
                bp[i] = bp_desc[j];
                b[i] = b_desc[j];
            }
            let (mut d1, mut d2, mut bonus) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
            for k in 0..n {
                d1[k] = b[k] - bp[k];
                if d1[k] < 0.0 {
                    d1[k] *= D1_NEG;
                }
                d2[k] = D2_MUL * ((b[k] - MAX_BONUS) / D2_DEN).exp();
                bonus[k] = t.coeff * (d1[k] + d2[k]);
                if t.teams[k].heredity && t.teams[k].n_legs >= MIN_LEGS {
                    bonus[k] *= 2.0 / t.teams[k].n_legs as f64;
                }
            }
            scored.push(Scored { rt, r, rb, rg, mp, bp, b, d1, d2, bonus });
        }

        // The step: everything above read the old state, everything below writes it.
        let prev_team: Vec<Option<f64>> = teams.iter().map(|t| t.map(|t| t.rating)).collect();
        let prev_player: Vec<f64> = players.iter().map(|p| p.rating).collect();
        for p in players.iter_mut() {
            for b in p.bonuses.iter_mut() {
                b.weeks += 1;
                b.raw = b.initial * J.powi(b.weeks);
                b.cur = round_even(b.raw);
            }
        }
        for (k, &ti) in idxs.iter().enumerate() {
            let t = &tournaments[ti];
            if !t.maii {
                continue;
            }
            for (j, res) in t.teams.iter().enumerate() {
                if res.heredity {
                    if let Some(team) = teams[res.team as usize].as_mut() {
                        team.rating += scored[k].bonus[j];
                    }
                }
                for &p in res.members.iter() {
                    players[p as usize].bonuses.push(Bonus {
                        tournament: t.id,
                        initial: scored[k].b[j],
                        weeks: 0,
                        cur: scored[k].b[j],
                        raw: scored[k].b[j],
                    });
                }
            }
        }
        for t in teams.iter_mut().flatten() {
            t.rating = t.rating.max(0.0);
        }
        for p in players.iter_mut() {
            p.bonuses
                .sort_by(|a, b| b.raw.total_cmp(&a.raw).then(a.tournament.cmp(&b.tournament)));
            p.bonuses.truncate(N_BEST);
            p.rating = p.bonuses.iter().map(|b| b.cur).sum();
            if p.rating == 0.0 {
                p.bonuses.clear();
            }
        }

        let live: Vec<usize> = (0..teams.len()).filter(|&i| teams[i].is_some()).collect();

        // A.3.2. The release table is also the state the next release starts from, so
        // a team whose base roster lapses drops out and returns at a start rating.
        let mut listed: Vec<bool> = vec![false; teams.len()];
        let prev_season = season_at(day - 180);
        let include_prev = sstart + 90 >= day;
        for r in season_rows(season).iter().map(|&i| &rosters[i]) {
            listed[r.team as usize] = true;
        }
        if include_prev {
            for r in season_rows(prev_season).iter().map(|&i| &rosters[i]) {
                listed[r.team as usize] = true;
            }
        }

        if (rows_every > 0 && release_id % rows_every == 0) || day + 7 > to_day {
            let mut trows = Vec::new();
            for &i in live.iter().filter(|&&i| listed[i]) {
                let t = teams[i].unwrap();
                trows.push((
                    team_ids.list[i],
                    round_half_up(t.rating),
                    0.0,
                    round_half_up(t.trb),
                    0.0,
                    match prev_team[i] {
                        Some(p) if p != 0.0 => round_half_up(t.rating - p),
                        _ => 0.0,
                    },
                ));
            }
            let mut prows = Vec::new();
            for (p, pl) in players.iter().enumerate() {
                if pl.rating > 0.0 {
                    prows.push((
                        player_ids.list[p],
                        pl.rating,
                        0.0,
                        pl.bonuses.len() as u32,
                        if prev_player[p] != 0.0 { pl.rating - prev_player[p] } else { 0.0 },
                    ));
                }
            }
            w.release(release_id, &format_date(day), 0.0, q, prows.len())?;
            w.player_ratings(release_id, &prows, |t| t)?;
            w.team_ratings(release_id, &trows, |m| m)?;
        }

        for (k, &ti) in idxs.iter().enumerate() {
            let t = &tournaments[ti];
            let s = &scored[k];
            let evs: Vec<TeamEvent> = t
                .teams
                .iter()
                .enumerate()
                .map(|(j, res)| TeamEvent {
                    team: res.team,
                    c: t.coeff,
                    delta_l: s.d1[j],
                    x: s.rt[j],
                    forecast: s.rg[j],
                    forecast_place: s.mp[j],
                    expected_place: s.mp[j],
                    actual_place: res.position,
                    pred_place: res.pred_pos,
                    s_r: s.b[j],
                    s_pair: s.d2[j],
                    i_pair: s.bp[j],
                    mu_before: s.r[j],
                    mu_after: s.r[j] + if res.heredity && t.maii { s.bonus[j] } else { 0.0 },
                    sigma_before: s.rb[j],
                    sigma_after: 0.0,
                    evidence: s.bonus[j],
                    gap_days: 0,
                    var_diff: 0.0,
                    lost_prev: 0.0,
                    rated: res.heredity,
                    z: None,
                })
                .collect();
            w.team_events(t.id, &format_date(t.end), &team_ids.list, &evs)?;
            let actual: Vec<f64> = t.teams.iter().map(|r| r.position).collect();
            let (theirs, act2): (Vec<f64>, Vec<f64>) = t
                .teams
                .iter()
                .filter_map(|r| r.pred_pos.map(|p| (p, r.position)))
                .unzip();
            w.tournament_metrics(
                t.id,
                &format_date(t.end),
                &t.kind,
                t.coeff,
                t.teams.len(),
                0,
                0,
                None,
                None,
                spearman(&s.mp, &actual),
                spearman(&theirs, &act2),
                None,
            )?;
        }

        for (i, t) in teams.iter_mut().enumerate() {
            match t {
                Some(_) if !listed[i] => *t = None,
                Some(t) => t.rating = round_half_up(t.rating),
                None => {}
            }
        }
        if release_id % 26 == 0 {
            w.checkpoint()?;
            eprintln!(
                "{} q={:.5} teams={} players={}",
                format_date(day),
                q,
                live.len(),
                players.iter().filter(|p| p.rating > 0.0).count()
            );
        }
        day += 7;
    }
    w.finish()?;
    eprintln!("done in {:.1}s", started.elapsed().as_secs_f64());
    Ok(())
}
