use crate::util::parse_day;
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;

pub const MU_NEW: f64 = -1.0;

pub struct Tourn {
    pub id: i64,
    pub day: i64,
    pub date: String,
    pub ty: String,
    pub w: f64,
    pub name: String,
    pub editors: String,
    pub qbt: String,
}

pub struct Res {
    pub team: i64,
    pub mask: Vec<u8>, // 0 wrong, 1 right, 2 skip
    pub lineup: Vec<i64>,
    pub in_rating: bool,
    pub rg: f64,
}

pub fn weight_of(ty: &str) -> f64 {
    match ty {
        "Обычный" | "Региональный" => 1.0,
        "Строго синхронный" => 2.0 / 3.0,
        "Синхрон" => 0.5,
        _ => 0.0,
    }
}

pub fn open_ro(path: &str) -> Connection {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("open db")
}

pub fn load_tournaments(buff: &Connection, from: &str) -> Vec<Tourn> {
    let mut st = buff
        .prepare(
            "select id, date_start, tournament_type, name, coalesce(editors,''), coalesce(questions_by_tour,'')
             from tournaments
             where in_rating = 1 and date_start >= ?1 and tournament_type is not null
             order by date_start, id",
        )
        .unwrap();
    let rows = st
        .query_map([from], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .unwrap();
    let mut out = Vec::new();
    for row in rows {
        let (id, ds, ty, name, editors, qbt) = row.unwrap();
        let w = weight_of(&ty);
        if w == 0.0 {
            continue;
        }
        if let Some((day, date)) = parse_day(&ds) {
            out.push(Tourn { id, day, date, ty, w, name, editors, qbt });
        }
    }
    out.sort_by_key(|t| (t.day, t.id));
    out
}

fn parse_lineup(s: &str) -> Vec<i64> {
    let mut v = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            if let Ok(x) = cur.parse::<i64>() {
                v.push(x);
            }
            cur.clear();
        }
    }
    if !cur.is_empty() {
        if let Ok(x) = cur.parse::<i64>() {
            v.push(x);
        }
    }
    v
}

pub fn load_results(buff: &Connection, from: &str) -> HashMap<i64, Vec<Res>> {
    let mut st = buff
        .prepare(
            "select tr.id, tr.team_id, coalesce(tr.mask,''), coalesce(tr.team_members,''),
                    coalesce(json_extract(tr.rating,'$.inRating'), 0),
                    coalesce(json_extract(tr.rating,'$.rg'), -1)
             from tournament_results tr join tournaments t on t.id = tr.id
             where t.in_rating = 1 and t.date_start >= ?1",
        )
        .unwrap();
    let mut out: HashMap<i64, Vec<Res>> = HashMap::new();
    let rows = st
        .query_map([from], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, f64>(5)?,
            ))
        })
        .unwrap();
    for row in rows {
        let (tid, team, mask, members, inr, rg) = row.unwrap();
        let m: Vec<u8> = mask
            .bytes()
            .map(|c| match c {
                b'1' => 1u8,
                b'0' => 0u8,
                _ => 2u8,
            })
            .collect();
        out.entry(tid).or_default().push(Res {
            team,
            mask: m,
            lineup: parse_lineup(&members),
            in_rating: inr != 0,
            rg,
        });
    }
    out
}

/// Real B-rating releases: (day, team -> place), sorted by day.
pub fn load_releases(buff: &Connection) -> Vec<(i64, HashMap<i64, f64>)> {
    let mut st = buff
        .prepare("select e.date, r.team_id, r.place from ratings r join releases e on e.id = r.release_id")
        .unwrap();
    let mut by_day: HashMap<i64, HashMap<i64, f64>> = HashMap::new();
    let rows = st
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, f64>(2)?))
        })
        .unwrap();
    for row in rows {
        let (d, team, place) = row.unwrap();
        if let Some((day, _)) = parse_day(&d) {
            by_day.entry(day).or_default().insert(team, place);
        }
    }
    let mut v: Vec<(i64, HashMap<i64, f64>)> = by_day.into_iter().collect();
    v.sort_by_key(|x| x.0);
    v
}

pub struct QParams {
    pub b: Vec<f64>,
    pub a: Vec<f64>,
    pub m: Vec<f64>,
}

pub fn load_question_params(snap: &Connection) -> HashMap<i64, QParams> {
    let mut st = snap
        .prepare("select tournament_id, q, b, a, m from question_params order by tournament_id, q")
        .unwrap();
    let mut out: HashMap<i64, QParams> = HashMap::new();
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, f64>(4)?,
            ))
        })
        .unwrap();
    for row in rows {
        let (tid, q, b, a, m) = row.unwrap();
        let e = out.entry(tid).or_insert_with(|| QParams {
            b: Vec::new(),
            a: Vec::new(),
            m: Vec::new(),
        });
        let qi = q as usize;
        while e.b.len() <= qi {
            e.b.push(f64::NAN);
            e.a.push(f64::NAN);
            e.m.push(1.0);
        }
        e.b[qi] = b;
        e.a[qi] = a;
        e.m[qi] = m;
    }
    out
}

/// Release id -> day, from the snapshot.
pub fn load_snapshot_releases(snap: &Connection) -> Vec<(i64, i64)> {
    let mut st = snap.prepare("select id, date from releases order by date").unwrap();
    let rows = st
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .unwrap();
    let mut v = Vec::new();
    for row in rows {
        let (id, d) = row.unwrap();
        if let Some((day, _)) = parse_day(&d) {
            v.push((id, day));
        }
    }
    v.sort_by_key(|x| x.1);
    v
}

pub fn load_thetas(snap: &Connection, release_id: i64) -> HashMap<i64, f64> {
    let mut st = snap
        .prepare("select player_id, theta from player_ratings where release_id = ?1")
        .unwrap();
    let rows = st
        .query_map([release_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))
        .unwrap();
    let mut m = HashMap::new();
    for row in rows {
        let (p, t) = row.unwrap();
        m.insert(p, t);
    }
    m
}
