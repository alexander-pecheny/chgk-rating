//! One pass over `buff.tournament_results`, which has no index, so everything
//! that needs it is collected in the same scan: each player's games, each
//! team's results, and the names a team has played under.

use crate::data::{log, text};
use crate::index::{fold, Ctx};
use rusqlite::{params, Connection, Row, Statement};
use std::collections::HashMap;

/// The name and town of a team's latest result, and how many it has.
pub struct TeamLast {
    pub name: String,
    pub town: String,
    pub date: String,
    pub games: i64,
}

#[derive(Default)]
pub struct Results {
    pub team_last: HashMap<i64, TeamLast>,
    /// Each player's last game date and number of games.
    pub player_last: HashMap<i64, (String, i64)>,
    /// Every name a team ever played under, kept as (folded, shown) so the
    /// same name in another case is not filed twice. Which of them is the
    /// team's own is decided later, once; the rest become search aliases.
    alias: HashMap<i64, Vec<(String, String)>>,
}

/// One result row, as far as the index cares.
struct Played {
    tid: i64,
    team: i64,
    name: String,
    town: String,
    score: Option<i64>,
    position: Option<f64>,
    members: Option<String>,
}

const RESULTS_SQL: &str = "select id, team_id, team_current_name as name, team_current_town as town, \
    questions_total, position, team_members from tournament_results";

/// Reads every result of a rated tournament into `player_game` and `result_`.
pub fn scan(c: &Ctx, tour_date: &HashMap<i64, String>) -> Results {
    let mut res = Results::default();
    let mut pg = c.w.prepare("insert into player_game values (?1,?2,?3,?4,?5)").unwrap();
    let mut rs = c.w.prepare("insert into result_ values (?1,?2,?3,?4)").unwrap();
    crate::data::each(c.b, RESULTS_SQL, |x| {
        let Some(p) = played(x) else { return };
        let Some(date) = tour_date.get(&p.tid) else { return };
        res.team(&p, date);
        rs.execute(params![p.tid, p.team, p.score, p.name]).unwrap();
        res.players(&p, date, &mut pg);
    });
    let what = format!("{} players with games, {} teams", res.player_last.len(), res.team_last.len());
    log(c.t0, &what);
    res
}

fn played(x: &Row) -> Option<Played> {
    Some(Played {
        tid: x.get("id").ok()?,
        team: x.get("team_id").ok()?,
        name: text(x, "name"),
        town: text(x, "town"),
        score: x.get::<_, Option<String>>("questions_total").unwrap().and_then(|s| s.trim().parse().ok()),
        position: x.get("position").unwrap_or(None),
        members: x.get("team_members").unwrap(),
    })
}

impl Results {
    /// Keeps the team's latest name and town and every name it played under.
    /// A result with no name says nothing about either.
    fn team(&mut self, p: &Played, date: &String) {
        if p.name.is_empty() {
            return;
        }
        let e = self.team_last.entry(p.team).or_insert_with(|| TeamLast {
            name: p.name.clone(), town: p.town.clone(), date: date.clone(), games: 0,
        });
        if *date >= e.date {
            (e.name, e.town, e.date) = (p.name.clone(), p.town.clone(), date.clone());
        }
        e.games += 1;
        // A one-off name is still a name people search for.
        let f = fold(&p.name);
        let v = self.alias.entry(p.team).or_default();
        if !v.iter().any(|x| x.0 == f) {
            v.push((f, p.name.clone()));
        }
    }

    /// Files the result under each player in its lineup.
    fn players(&mut self, p: &Played, date: &String, pg: &mut Statement) {
        let Some(ids) = p.members.as_deref().filter(|m| m.len() > 2) else { return };
        let Ok(ids) = serde_json::from_str::<Vec<i64>>(ids) else { return };
        for pid in ids {
            pg.execute(params![pid, p.tid, p.team, p.position, p.score]).unwrap();
            let e = self.player_last.entry(pid).or_insert((String::new(), 0));
            if *date >= e.0 {
                e.0 = date.clone();
            }
            e.1 += 1;
        }
    }

    /// Every other name the team has played under, so a search for the
    /// one-off still lands on the team.
    pub fn aliases(&self, id: i64, name: &str) -> String {
        let f = fold(name);
        let Some(v) = self.alias.get(&id) else { return String::new() };
        v.iter().filter(|x| x.0 != f).map(|x| x.1.as_str()).collect::<Vec<_>>().join(" ")
    }
}

/// `id -> (name, town)` from a table of names, skipping rows with no name. A
/// file that is not there, or has no such table, is not an error: it is one
/// source of names missing, and the caller has others.
pub fn read_names(db: &Connection, sql: &str) -> HashMap<i64, (String, String)> {
    let mut out = HashMap::new();
    let Ok(mut st) = db.prepare(sql) else { return out };
    let Ok(mut rows) = st.query([]) else { return out };
    while let Ok(Some(x)) = rows.next() {
        let Ok(id) = x.get::<_, i64>("id") else { continue };
        let name: String = x.get::<_, Option<String>>("name").unwrap_or_default().unwrap_or_default();
        let town: String = x.get::<_, Option<String>>("town").unwrap_or_default().unwrap_or_default();
        if !name.trim().is_empty() {
            out.insert(id, (name, town));
        }
    }
    out
}
