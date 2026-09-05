use rusqlite::{Connection, OpenFlags};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct Release {
    pub id: i64,
    pub date: String,
    pub shift: f64,
    pub shift_cum: f64,
    pub n_players: i64,
}

#[derive(Clone, Copy)]
pub struct TeamPoint {
    pub rel: u16,
    pub sigma: f32,
    pub display: f32,
}

#[derive(Clone, Copy)]
pub struct PlayerPoint {
    pub rel: u16,
    pub sigma: f32,
    pub display: f32,
}

pub struct Event {
    pub tid: i64,
    pub team_id: i64,
    pub c: f64,
    pub rated: bool,
    pub date: Option<String>,
    pub forecast_place: Option<f64>,
    pub actual_place: Option<f64>,
    pub mu_before: f64,
    pub mu_after: f64,
}

pub struct Metrics {
    pub date: String,
    pub ttype: String,
    pub weight: f64,
    pub n_teams: i64,
    pub n_q: i64,
    pub n_obs: i64,
    pub ll_in: Option<f64>,
    pub ll_cf: Option<f64>,
    pub sp_forecast: Option<f64>,
    pub sp_old: Option<f64>,
    /// Package difficulty on the shared 0–10 scale; absent on older schemas.
    pub truedl: Option<f64>,
}

pub struct Tournament {
    pub name: String,
}

pub struct Result_ {
    pub team_id: i64,
    pub score: Option<i64>,
}

pub struct Display {
    pub a_team: f64,
    pub b_team: f64,
    pub a_player: f64,
    pub b_player: f64,
    pub provisional_sigma: f64,
}

/// DESIGN.md §5: the published scale is an affine map of the internal one.
/// When the model publishes `display`, the map is read off two of its own rows,
/// so the same conversion carries every `mu_before` / `mu_after` and the event
/// chain lands on the published rating. Otherwise it is pinned so the 50th and
/// 1000th settled entries at the 2019 anchor release keep the values the old
/// system gave those places. Non-identity constants in `config.toml` win over
/// both: that is `calibrate` having already spoken.
pub fn fit_scale(db: &Connection, d: &mut Display) {
    let anchor = |date: &str| -> i64 {
        db.query_row(
            "select id from releases order by abs(julianday(date) - julianday(?1)) limit 1",
            [date],
            |r| r.get(0),
        )
        .expect("no releases to anchor the display scale on")
    };

    let published = |table: &str, value: &str, rel: i64| -> Option<(f64, f64)> {
        if !columns(db, table).contains("display") {
            return None;
        }
        let q = format!(
            "select {value}, display from {table} where release_id = ?1 and display is not null \
             order by {value} desc limit 1 offset ?2"
        );
        let row = |n: i64| -> Option<(f64, f64)> {
            db.query_row(&q, rusqlite::params![rel, n], |r| Ok((r.get(0)?, r.get(1)?))).ok()
        };
        let ((v_hi, p_hi), (v_lo, p_lo)) = (row(0)?, row(999).or_else(|| row(49))?);
        if (v_hi - v_lo).abs() < 1e-9 {
            return None;
        }
        let b = (p_hi - p_lo) / (v_hi - v_lo);
        Some((p_hi - b * v_hi, b))
    };

    // Two places on the anchor release pin the line. Too few settled entries to
    // reach the 1000th and the map stays the identity, which publishes the
    // internal numbers rather than wrong ones.
    let anchored = |q: &str, rel: i64, sigma: f64, t_hi: f64, t_lo: f64| -> Option<(f64, f64)> {
        let nth = |n: i64| -> Option<f64> {
            db.query_row(q, rusqlite::params![rel, sigma, n], |r| r.get(0)).ok()
        };
        let (v_hi, v_lo) = (nth(49)?, nth(999)?);
        if (v_hi - v_lo).abs() < 1e-9 {
            return None;
        }
        let b = (t_hi - t_lo) / (v_hi - v_lo);
        Some((t_hi - b * v_hi, b))
    };

    if d.a_team == 0.0 && d.b_team == 1.0 {
        let rel = anchor("2019-09-05");
        let q = "select mu from team_ratings where release_id = ?1 and listed = 1 \
                 and sigma <= ?2 order by mu desc limit 1 offset ?3";
        match published("team_ratings", "mu", rel)
            .or_else(|| anchored(q, rel, d.provisional_sigma, 7028.0, 2992.0))
        {
            Some((a, b)) => {
                d.a_team = a;
                d.b_team = b;
            }
            None => eprintln!("warning: cannot anchor the team scale, publishing internal values"),
        }
    }
    if d.a_player == 0.0 && d.b_player == 1.0 {
        let rel = anchor("2019-10-03");
        let q = "select theta from player_ratings where release_id = ?1 \
                 and sigma <= ?2 order by theta desc limit 1 offset ?3";
        match published("player_ratings", "theta", rel)
            .or_else(|| anchored(q, rel, d.provisional_sigma, 12865.0, 9778.0))
        {
            Some((a, b)) => {
                d.a_player = a;
                d.b_player = b;
            }
            None => eprintln!("warning: cannot anchor the player scale, publishing internal values"),
        }
    }
}

/// Column names a table actually has, so the generator survives the schema
/// growing `display`, `change`, `inRating` or `date` under it.
pub fn columns(db: &Connection, table: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(mut st) = db.prepare(&format!("pragma table_info({table})")) else {
        return out;
    };
    if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(1)) {
        out.extend(rows.flatten());
    }
    out
}

/// `name` when the table has it, the literal NULL when it does not, so one
/// projection string serves both schemas.
pub fn col<'a>(have: &HashSet<String>, name: &'a str) -> &'a str {
    if have.contains(name) {
        name
    } else {
        "NULL"
    }
}

/// The first of `names` the table has. Used where a flag may be spelled either
/// way (`inRating` / `in_rating`).
pub fn any_col(have: &HashSet<String>, names: &[&str]) -> Option<String> {
    names.iter().find(|n| have.contains(**n)).map(|n| n.to_string())
}

pub fn open(path: &Path) -> Connection {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

pub fn read_display(path: &Path) -> Display {
    let txt = std::fs::read_to_string(path).unwrap_or_default();
    let v: toml::Value = txt.parse().unwrap_or(toml::Value::Table(Default::default()));
    let g = |k: &str, d: f64| {
        v.get("display")
            .and_then(|t| t.get(k))
            .and_then(|x| x.as_float())
            .unwrap_or(d)
    };
    Display {
        a_team: g("a_team", 0.0),
        b_team: g("b_team", 1.0),
        a_player: g("a_player", 0.0),
        b_player: g("b_player", 1.0),
        provisional_sigma: g("provisional_sigma", 0.3),
    }
}

/// `change` is a delta of the internal value, so it needs the display factor —
/// but a future schema could publish it already scaled. Deltas on the internal
/// scale never reach 1; on a published scale of thousands they routinely do, so
/// the largest one in the table says which this is.
pub fn change_factor(db: &Connection, table: &str, release: i64, b: f64) -> f64 {
    if !columns(db, table).contains("change") || b <= 10.0 {
        return 1.0;
    }
    let biggest: Option<f64> = db
        .query_row(
            &format!("select max(abs(change)) from {table} where release_id = ?1"),
            [release],
            |r| r.get(0),
        )
        .unwrap_or(None);
    match biggest {
        Some(x) if x > 1.0 => 1.0,
        _ => b,
    }
}

pub fn releases(db: &Connection) -> Vec<Release> {
    let mut st = db
        .prepare("select id, date, shift, shift_cum, n_players from releases order by date, id")
        .unwrap();
    let rows = st
        .query_map([], |r| {
            Ok(Release {
                id: r.get(0)?,
                date: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                shift: r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                shift_cum: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                n_players: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        })
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

/// Histories for the given teams, keyed by team id, ordered by release.
pub fn team_history(
    db: &Connection,
    want: &HashSet<i64>,
    idx: &HashMap<i64, u16>,
    d: &Display,
) -> HashMap<i64, Vec<TeamPoint>> {
    let mut out: HashMap<i64, Vec<TeamPoint>> = HashMap::with_capacity(want.len());
    let have = columns(db, "team_ratings");
    let sql = format!(
        "select release_id, team_id, mu, sigma, {} from team_ratings order by release_id",
        col(&have, "display")
    );
    let mut st = db.prepare(&sql).unwrap();
    let mut rows = st.query([]).unwrap();
    while let Some(r) = rows.next().unwrap() {
        let team: i64 = r.get(1).unwrap();
        if !want.contains(&team) {
            continue;
        }
        let Some(&rel) = idx.get(&r.get::<_, i64>(0).unwrap()) else {
            continue;
        };
        let mu: f64 = r.get(2).unwrap();
        let sigma: f64 = r.get(3).unwrap();
        let display: f64 = r
            .get::<_, Option<f64>>(4)
            .unwrap()
            .unwrap_or(d.a_team + d.b_team * mu);
        out.entry(team).or_default().push(TeamPoint {
            rel,
            sigma: sigma as f32,
            display: display as f32,
        });
    }
    out
}

pub fn player_history(
    db: &Connection,
    want: &HashSet<i64>,
    idx: &HashMap<i64, u16>,
    d: &Display,
) -> HashMap<i64, Vec<PlayerPoint>> {
    let mut out: HashMap<i64, Vec<PlayerPoint>> = HashMap::with_capacity(want.len());
    let have = columns(db, "player_ratings");
    let sql = format!(
        "select release_id, player_id, theta, sigma, {} from player_ratings order by release_id",
        col(&have, "display")
    );
    let mut st = db.prepare(&sql).unwrap();
    let mut rows = st.query([]).unwrap();
    while let Some(r) = rows.next().unwrap() {
        let pid: i64 = r.get(1).unwrap();
        if !want.contains(&pid) {
            continue;
        }
        let Some(&rel) = idx.get(&r.get::<_, i64>(0).unwrap()) else {
            continue;
        };
        let theta: f64 = r.get(2).unwrap();
        let sigma: f64 = r.get(3).unwrap();
        let display: f64 = r
            .get::<_, Option<f64>>(4)
            .unwrap()
            .unwrap_or(d.a_player + d.b_player * theta);
        out.entry(pid).or_default().push(PlayerPoint {
            rel,
            sigma: sigma as f32,
            display: display as f32,
        });
    }
    out
}

pub fn events(db: &Connection) -> Vec<Event> {
    let have = columns(db, "team_events");
    // An unrated result still gets a forecast and still belongs on the page —
    // it is marked, not hidden — so this flag is loaded, never filtered on.
    let rated = any_col(&have, &["rated", "inRating", "in_rating"])
        .unwrap_or_else(|| "1".into());
    let sql = format!(
        "select tournament_id, team_id, c, {}, forecast_place, actual_place, \
         mu_before, mu_after, {rated} from team_events",
        col(&have, "date")
    );
    let mut st = db.prepare(&sql).unwrap();
    let rows = st
        .query_map([], |r| {
            Ok(Event {
                tid: r.get(0)?,
                team_id: r.get(1)?,
                c: r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                date: r.get(3)?,
                forecast_place: r.get(4)?,
                actual_place: r.get(5)?,
                mu_before: r.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                mu_after: r.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
                rated: r.get::<_, Option<i64>>(8)?.unwrap_or(1) != 0,
            })
        })
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

pub fn metrics(db: &Connection) -> HashMap<i64, Metrics> {
    let have = columns(db, "tournament_metrics");
    let rated = any_col(&have, &["rated", "inRating", "in_rating"])
        .map(|c| format!(" where {c} != 0"))
        .unwrap_or_default();
    let sql = format!(
        "select tournament_id, date, type, weight, n_teams, n_q, n_obs, logloss_insample, \
         logloss_crossfit, spearman_forecast, spearman_old_system, {} \
         from tournament_metrics{rated}",
        col(&have, "truedl")
    );
    let mut st = db.prepare(&sql).unwrap();
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Metrics {
                    date: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    ttype: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    weight: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                    n_teams: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    n_q: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    n_obs: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    ll_in: r.get(7)?,
                    ll_cf: r.get(8)?,
                    sp_forecast: r.get(9)?,
                    sp_old: r.get(10)?,
                    truedl: r.get(11)?,
                },
            ))
        })
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

pub fn question_b(db: &Connection) -> HashMap<i64, Vec<(i64, f64)>> {
    let mut out: HashMap<i64, Vec<(i64, f64)>> = HashMap::new();
    let mut st = db
        .prepare("select tournament_id, q, b from question_params order by tournament_id, q")
        .unwrap();
    let mut rows = st.query([]).unwrap();
    while let Some(r) = rows.next().unwrap() {
        let t: i64 = r.get(0).unwrap();
        out.entry(t)
            .or_default()
            .push((r.get(1).unwrap(), r.get::<_, Option<f64>>(2).unwrap().unwrap_or(0.0)));
    }
    out
}

pub fn player_names(buff: &Connection) -> HashMap<i64, String> {
    let mut st = buff.prepare("select id, name, patronymic, surname from players").unwrap();
    let mut rows = st.query([]).unwrap();
    let mut out = HashMap::new();
    while let Some(r) = rows.next().unwrap() {
        let id: i64 = r.get(0).unwrap();
        let name: String = r.get::<_, Option<String>>(1).unwrap().unwrap_or_default();
        let sur: String = r.get::<_, Option<String>>(3).unwrap().unwrap_or_default();
        let full = format!("{} {}", sur.trim(), name.trim());
        let full = full.trim().to_string();
        out.insert(id, if full.is_empty() { format!("Игрок {id}") } else { full });
    }
    out
}

pub fn tournaments(buff: &Connection, want: &HashMap<i64, Metrics>) -> HashMap<i64, Tournament> {
    let mut st = buff
        .prepare("select id, name from tournaments")
        .unwrap();
    let mut rows = st.query([]).unwrap();
    let mut out = HashMap::with_capacity(want.len());
    while let Some(r) = rows.next().unwrap() {
        let id: i64 = r.get(0).unwrap();
        if !want.contains_key(&id) {
            continue;
        }
        let name = r.get::<_, Option<String>>(1).unwrap().unwrap_or_default();
        out.insert(id, Tournament { name });
    }
    out
}

pub struct Rosters {
    /// tournament id -> results
    pub by_tournament: HashMap<i64, Vec<Result_>>,
    /// team id -> most recent (name, town)
    pub team_names: HashMap<i64, (String, String)>,
    /// player id -> (tournament id, team id), most recent last
    pub player_games: HashMap<i64, Vec<(i64, i64)>>,
}

/// One pass over `tournament_results` for the rated tournaments, in date order,
/// so the last write of a team name is the most recent one.
pub fn rosters(
    buff: &Connection,
    metrics: &HashMap<i64, Metrics>,
    players_wanted: &HashSet<i64>,
) -> Rosters {
    let mut order: Vec<(&String, i64)> = metrics.iter().map(|(id, m)| (&m.date, *id)).collect();
    order.sort();

    let mut by_tournament: HashMap<i64, Vec<Result_>> = HashMap::with_capacity(metrics.len());
    let mut team_names: HashMap<i64, (String, String)> = HashMap::new();
    let mut player_games: HashMap<i64, Vec<(i64, i64)>> = HashMap::new();

    let mut st = buff
        .prepare(
            "select team_id, team_current_name, team_current_town, questions_total, team_members \
             from tournament_results where id = ?1",
        )
        .unwrap();
    for (_, tid) in order {
        let mut rows = st.query([tid]).unwrap();
        let mut list = Vec::new();
        while let Some(r) = rows.next().unwrap() {
            let team_id: i64 = match r.get(0) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let name: String = r.get::<_, Option<String>>(1).unwrap().unwrap_or_default();
            let town: String = r.get::<_, Option<String>>(2).unwrap().unwrap_or_default();
            let score = r
                .get::<_, Option<String>>(3)
                .unwrap()
                .and_then(|s| s.trim().parse::<i64>().ok());
            if !name.is_empty() {
                team_names.insert(team_id, (name.clone(), town.clone()));
            }
            if let Some(members) = r.get::<_, Option<String>>(4).unwrap() {
                if members.len() > 2 {
                    if let Ok(ids) = serde_json::from_str::<Vec<i64>>(&members) {
                        for pid in ids {
                            if players_wanted.contains(&pid) {
                                player_games.entry(pid).or_default().push((tid, team_id));
                            }
                        }
                    }
                }
            }
            list.push(Result_ { team_id, score });
        }
        by_tournament.insert(tid, list);
    }
    Rosters { by_tournament, team_names, player_games }
}
