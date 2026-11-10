//! Tournaments not yet played whose lineups are already listed, as `fit`
//! forecast them: each goes into `tournament` beside the played ones, so the
//! list, the search and its page find it, and its forecast into the tables of
//! its own. None of it reaches `player_game` or `result_`'s scores, so no
//! player or team page counts a game that has not happened.

use crate::data::{each, f64_or, i64_or_zero, log, opt_f64, table_exists, text};
use crate::index::Ctx;
use rusqlite::params;
use std::collections::HashMap;

pub fn build(c: &Ctx) {
    if !table_exists(c.r, "upcoming") {
        return;
    }
    let ids = ids(c, "select tournament_id from upcoming");
    let mut names: HashMap<i64, String> = HashMap::new();
    each(c.b, &format!("select id, name from tournaments where id in ({ids})"), |x| {
        names.insert(x.get(0).unwrap(), text(x, 1));
    });
    let mut tour = c.w.prepare("insert into tournament (id, name, date, type, weight, n_teams, n_q) values (?1,?2,?3,?4,?5,?6,?7)").unwrap();
    let mut fts = c.w.prepare("insert into tournament_fts(rowid, name) values (?1,?2)").unwrap();
    let mut up = c.w.prepare("insert into upcoming values (?1,?2,?3,?4,?5,?6)").unwrap();
    let mut n = 0;
    each(c.r, "select * from upcoming", |x| {
        let id: i64 = x.get("tournament_id").unwrap();
        let name = names.get(&id).cloned().unwrap_or_default();
        tour.execute(params![id, name, text(x, "date"), text(x, "type"), f64_or(x, "weight", 1.0),
            i64_or_zero(x, "n_teams"), i64_or_zero(x, "n_q")])
        .unwrap();
        fts.execute(params![id, crate::index::without_yo(&name)]).unwrap();
        up.execute(params![id, text(x, "as_of"), x.get::<_, Option<i64>>("ref_id").unwrap(),
            x.get::<_, Option<String>>("ref_rule").unwrap(), opt_f64(x, "ref_truedl"), opt_f64(x, "announced")])
        .unwrap();
        n += 1;
    });
    teams(c, &ids);
    players(c);
    log(c.t0, &format!("{n} upcoming tournaments"));
}

/// Each listed team with its forecast and the name it entered under.
fn teams(c: &Ctx, ids: &str) {
    let mut entered: HashMap<(i64, i64), String> = HashMap::new();
    each(c.b, &format!("select id, team_id, team_current_name from tournament_results where id in ({ids})"), |x| {
        entered.insert((x.get(0).unwrap(), x.get(1).unwrap()), text(x, 2));
    });
    let mut ins = c.w.prepare("insert into upcoming_team values (?1,?2,?3,?4,?5,?6,?7)").unwrap();
    each(c.r, "select * from upcoming_events", |x| {
        let (tid, team): (i64, i64) = (x.get("tournament_id").unwrap(), x.get("team_id").unwrap());
        let name = entered.get(&(tid, team)).cloned().unwrap_or_default();
        ins.execute(params![tid, team, name, opt_f64(x, "forecast"), opt_f64(x, "forecast_place"),
            opt_f64(x, "e_score"), opt_f64(x, "s_r")])
        .unwrap();
    });
}

/// Each listed player with the state the forecast read, under the name
/// `buff.players` gives them: many have never played, so `player` lacks them.
fn players(c: &Ctx) {
    let ids = ids(c, "select distinct player_id from upcoming_players");
    let mut name: HashMap<i64, String> = HashMap::new();
    each(c.b, &format!("select id, name, surname from players where id in ({ids})"), |x| {
        let id: i64 = x.get("id").unwrap();
        name.insert(id, crate::data::full_name(id, &text(x, "name"), &text(x, "surname")));
    });
    let mut ins = c.w.prepare("insert into upcoming_player values (?1,?2,?3,?4,?5,?6,?7)").unwrap();
    each(c.r, "select * from upcoming_players", |x| {
        let id: i64 = x.get("player_id").unwrap();
        ins.execute(params![x.get::<_, i64>("tournament_id").unwrap(), x.get::<_, i64>("team_id").unwrap(), id,
            name.get(&id).cloned().unwrap_or_default(), opt_f64(x, "theta"), opt_f64(x, "phi"), opt_f64(x, "sigma")])
        .unwrap();
    });
}

/// The ids a one-column query of `out/ratings.db` returns, as a literal list
/// for the SQL against `buff.db`.
fn ids(c: &Ctx, sql: &str) -> String {
    let mut v = Vec::new();
    each(c.r, sql, |x| v.push(x.get::<_, i64>(0).unwrap().to_string()));
    v.join(",")
}
