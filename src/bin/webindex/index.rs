//! The serving index: everything a page needs that `out/ratings.db` and
//! `buff.db` cannot answer with an index they already have.
//!
//! Two things are missing there. Names and rosters live in `buff.db` behind a
//! JSON blob and no index at all, so finding one player's games means scanning
//! 940 000 results. And the two ranking tables need the last release ordered by
//! rating, which `player_ratings` has no index for and would not want one for:
//! 18.7 million rows to serve 26 thousand. Both are computed once here.

use crate::data::{col, columns, each, f64_or, i64_or_zero, log, opt_f64, table_exists, text, Display};
use crate::history::{Current, Listed};
use crate::results::{Results, TeamLast};
use crate::schema::{INDEXES, KIND_PLAYER, KIND_TEAM, SCHEMA, VERSION};
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

/// How names compare when the question is "is this the same name": ignoring the
/// padding people type and the case they type it in. SQLite's own `lower()` is
/// ASCII-only, so this has to happen in Rust.
pub fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

/// What `out/ratings.db` offers the server beyond the tables it must have.
pub struct Reads {
    pub achieve: bool,
    pub team_steps: bool,
}

/// The three databases and the clock every step reports against.
pub struct Ctx<'a> {
    pub r: &'a Connection,
    pub b: &'a Connection,
    pub w: &'a Connection,
    pub d: &'a Display,
    pub t0: Instant,
}

/// The last release: its id and date.
type Release = (i64, String);

pub fn build(ratings: &Path, buff: &Path, ext: &Path, out: &Path, d: &Display, reads: &Reads) {
    let (r, b) = (crate::data::open(ratings), crate::data::open(buff));
    let w = create(out);
    let c = Ctx { r: &r, b: &b, w: &w, d, t0: Instant::now() };
    let last = releases(&c);
    assert!(last.0 > 0, "no releases in {}", ratings.display());
    let tours = tournaments(&c);
    venues(&c);
    field_scores(&c);
    crate::forecasts::build(&c, ext, &d.clean_from);
    let res = crate::results::scan(&c, &tours.date);
    base_rosters(&c);
    let players = crate::history::scan(&c, Listed::players(d));
    let teams = crate::history::scan(&c, Listed::teams(d));
    player_names(&c, &res, &players.now);
    team_names(&c, &res, &teams.now);
    let n = ranks(&c, last.0);
    meta(&c, &last, n, reads);
    let rows = crate::anomaly::build(&r, &b, &w, &tours.online);
    crate::anomaly::write(&w, &rows);
    log(c.t0, "anomaly scores");
    crate::history::write(&c, KIND_PLAYER, &players);
    crate::history::write(&c, KIND_TEAM, &teams);
    finish(&c, &last, n);
}

fn create(out: &Path) -> Connection {
    let _ = std::fs::remove_file(out);
    let w = Connection::open(out).unwrap_or_else(|e| panic!("create {}: {e}", out.display()));
    w.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;").unwrap();
    w.execute_batch(SCHEMA).unwrap();
    w.execute_batch("BEGIN").unwrap();
    w
}

fn finish(c: &Ctx, last: &Release, (players, teams): (usize, usize)) {
    c.w.execute_batch("COMMIT").unwrap();
    c.w.execute_batch(INDEXES).unwrap();
    log(c.t0, &format!("release {} ({}), {teams} teams, {players} players ranked", last.0, last.1));
}

/// Every release, with the number of teams listed in it; returns the last.
fn releases(c: &Ctx) -> Release {
    let teams = count_by(c.r, "select release_id, count(*) from team_ratings where listed = 1 group by release_id");
    let mut ins = c.w.prepare("insert into release_ values (?1,?2,?3,?4,?5,?6)").unwrap();
    let mut last = (0, String::new());
    each(c.r, "select id, date, shift, shift_cum, n_players from releases order by date, id", |x| {
        let id: i64 = x.get("id").unwrap();
        let date = text(x, "date");
        let n_teams = teams.get(&id).copied().unwrap_or(0);
        ins.execute(params![id, date, f64_or(x, "shift", 0.0), f64_or(x, "shift_cum", 0.0),
            i64_or_zero(x, "n_players"), n_teams])
        .unwrap();
        last = (id, date);
    });
    last
}

/// `key -> count` from a query of two columns.
fn count_by(db: &Connection, sql: &str) -> HashMap<i64, i64> {
    let mut st = db.prepare(sql).unwrap();
    let v = st.query_map([], |x| Ok((x.get(0)?, x.get(1)?))).unwrap();
    v.map(|x| x.unwrap()).collect()
}

/// What the later steps need to know about each rated tournament.
pub struct Tours {
    pub date: HashMap<i64, String>,
    /// Whether it could be played from home, which is what the anomaly score
    /// of `/anomaly/` is a comparison across.
    pub online: HashMap<i64, bool>,
}

const TOURNAMENT_SQL: &str = "select tournament_id as id, date, type, weight, n_teams, n_q, n_obs, \
    logloss_insample, logloss_crossfit, spearman_forecast, spearman_old_system, truedl from tournament_metrics";

fn tournaments(c: &Ctx) -> Tours {
    let names = tournament_names(c.b);
    let mut t = Tours { date: HashMap::new(), online: HashMap::new() };
    let mut ins = c.w.prepare("insert into tournament values (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)").unwrap();
    let mut fts = c.w.prepare("insert into tournament_fts(rowid, name) values (?1,?2)").unwrap();
    each(c.r, TOURNAMENT_SQL, |x| {
        let id: i64 = x.get("id").unwrap();
        let (date, kind) = (text(x, "date"), text(x, "type"));
        let name = names.get(&id).cloned().unwrap_or_default();
        fts.execute(params![id, without_yo(&name)]).unwrap();
        ins.execute(params![id, name, date, kind,
            f64_or(x, "weight", 0.0), i64_or_zero(x, "n_teams"), i64_or_zero(x, "n_q"), i64_or_zero(x, "n_obs"),
            opt_f64(x, "logloss_insample"), opt_f64(x, "logloss_crossfit"), opt_f64(x, "spearman_forecast"),
            opt_f64(x, "spearman_old_system"), opt_f64(x, "truedl")])
        .unwrap();
        t.online.insert(id, kind != "in-person");
        t.date.insert(id, date);
    });
    log(c.t0, &format!("{} tournaments", t.date.len()));
    t
}

fn tournament_names(b: &Connection) -> HashMap<i64, String> {
    let mut out = HashMap::new();
    each(b, "select id, name from tournaments", |x| {
        out.insert(x.get("id").unwrap(), text(x, "name"));
    });
    out
}

/// One package played in several cities is one field in the achievement
/// rating, which records each result's venue and the field it went into. Only
/// fields of two venues or more are kept: they are the pages that show every
/// venue at once.
fn venues(c: &Ctx) {
    if !table_exists(c.r, "achieve_values") {
        return;
    }
    let mut ins = c.w.prepare("insert or ignore into venue values (?1, ?2)").unwrap();
    each(c.r, "select distinct tournament_id, field_id from achieve_values where n_venues > 1", |x| {
        let (t, f): (i64, i64) = (x.get("tournament_id").unwrap(), x.get("field_id").unwrap());
        ins.execute(params![t, f]).unwrap();
    });
}

/// A merged field's own count of each team's questions. A question one venue
/// lost as a whole, to an appeal or a misprint, is struck from every venue, so
/// the teams are ranked on the questions they all played; a single venue's
/// page keeps its own count.
fn field_scores(c: &Ctx) {
    let mut fields: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    each(c.w, "select tournament_id, field_id from venue order by tournament_id", |x| {
        fields.entry(x.get("field_id").unwrap()).or_default().push(x.get("tournament_id").unwrap());
    });
    let mut ins = c.w.prepare("insert or replace into field_score values (?1, ?2, ?3)").unwrap();
    for tids in fields.values() {
        for (tid, team, score) in recount(c, tids) {
            ins.execute(params![tid, team, score]).unwrap();
        }
    }
}

/// `(venue, team, questions taken)` over the questions every venue played.
fn recount(c: &Ctx, tids: &[i64]) -> Vec<(i64, i64, i64)> {
    let list = tids.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",");
    let sql = format!("select id, team_id, mask from tournament_results where id in ({list}) and mask is not null and mask != ''");
    let mut rows: Vec<(i64, i64, Vec<u8>)> = Vec::new();
    each(c.b, &sql, |x| rows.push((x.get("id").unwrap(), x.get("team_id").unwrap(), text(x, "mask").into_bytes())));
    let nq = rows.iter().map(|r| r.2.len()).max().unwrap_or(0);
    let answered = |m: &[u8], q: usize| matches!(m.get(q), Some(b'0' | b'1'));
    let lost = |q: usize| tids.iter().any(|t| {
        let at: Vec<&Vec<u8>> = rows.iter().filter(|r| r.0 == *t).map(|r| &r.2).collect();
        !at.is_empty() && at.iter().all(|m| !answered(m, q))
    });
    let kept: Vec<usize> = (0..nq).filter(|&q| !lost(q)).collect();
    rows.iter().map(|(t, team, m)| (*t, *team, kept.iter().filter(|&&q| m.get(q) == Some(&b'1')).count() as i64)).collect()
}

const ROSTER_SQL: &str = "select s.team_id, s.player_id, p.surname, p.name, \
    substr(s.date_added,1,10) as added from team_seasons s left join players p on p.id = s.player_id \
    where s.season_id = (select max(season_id) from team_seasons l where l.team_id = s.team_id) \
    and (s.date_removed is null or substr(s.date_removed,1,10) > date('now'))";

/// Each team's roster in the latest season it declared one for, less the
/// players removed by today. The mirror's `team_seasons` holds only the two
/// current seasons, and a new season's rosters are entered through its first
/// weeks, so the previous season stands until the team files the next.
fn base_rosters(c: &Ctx) {
    let mut ins = c.w.prepare("insert into base_roster values (?1,?2,?3,?4)").unwrap();
    let mut n = 0;
    each(c.b, ROSTER_SQL, |x| {
        let id: i64 = x.get("player_id").unwrap();
        let full = crate::data::full_name(id, &text(x, "name"), &text(x, "surname"));
        let team: i64 = x.get("team_id").unwrap();
        ins.execute(params![team, id, full, x.get::<_, Option<String>>("added").unwrap()]).unwrap();
        n += 1;
    });
    log(c.t0, &format!("{n} base-roster places"));
}

/// Text as the search index holds it: ё is е, as most people type it.
fn without_yo(s: &str) -> String {
    s.replace('ё', "е").replace('Ё', "Е")
}

/// Every player with a game, under the name `buff.players` gives them.
fn player_names(c: &Ctx, res: &Results, now: &HashMap<i64, Current>) {
    let mut ins = c.w.prepare("insert into player values (?1,?2,?3,?4,?5,?6,?7,?8)").unwrap();
    let mut fts = c.w.prepare("insert into player_fts(rowid, name) values (?1,?2)").unwrap();
    each(c.b, "select id, name, surname from players", |x| {
        let id: i64 = x.get("id").unwrap();
        let Some((last, games)) = res.player_last.get(&id) else { return };
        let full = crate::data::full_name(id, &text(x, "name"), &text(x, "surname"));
        let n = now.get(&id);
        ins.execute(params![id, full, games, last, n.map(|n| n.display), n.map(|n| n.sigma),
            n.map(|n| n.games_365), n.map(|n| n.provisional)])
        .unwrap();
        fts.execute(params![id, without_yo(&full)]).unwrap();
    });
}

/// What a team is called. A result carries the name a team entered that one
/// tournament under, which is sometimes a joke and sometimes a sponsor, so the
/// last result is no answer. `buff.teams` is: the mirror fills it nightly from
/// rating.chgk.info's own team listing, falling back to the latest result for
/// a team the listing lacks. The last result stays behind it for a team the
/// mirror has not seen.
fn team_names(c: &Ctx, res: &Results, now: &HashMap<i64, Current>) {
    let base = crate::results::read_names(c.b, "select id, name, town from teams");
    log(c.t0, &format!("{} names from buff.teams", base.len()));
    let mut ins = c.w.prepare("insert into team values (?1,?2,?3,?4,?5,?6,?7,?8)").unwrap();
    let mut fts = c.w.prepare("insert into team_fts(rowid, name, town, aliases) values (?1,?2,?3,?4)").unwrap();
    // How many teams each source of names ends up naming, and how many are now
    // called something other than what their last result called them.
    let (mut from_buff, mut renamed) = (0, 0);
    for (id, last) in &res.team_last {
        let (name, town) = team_name(base.get(id), last);
        from_buff += base.contains_key(id) as usize;
        renamed += (*name != last.name) as usize;
        let n = now.get(id);
        ins.execute(params![id, name, town, last.games, last.date, n.map(|n| n.display),
            n.map(|n| n.sigma), n.map(|n| n.provisional)])
        .unwrap();
        fts.execute(params![id, without_yo(name), without_yo(town), without_yo(&res.aliases(*id, name))]).unwrap();
    }
    let from_last = res.team_last.len() - from_buff;
    eprintln!("index: team names — {from_buff} from buff.teams, \
               {from_last} from the last result; {renamed} differ from the last result");
}

/// The name and town from `buff.teams`, or from the last result when the
/// mirror lacks the team. An empty town in the source is no reason to lose the
/// town the team last entered under.
fn team_name<'a>(base: Option<&'a (String, String)>, last: &'a TeamLast) -> (&'a String, &'a String) {
    let (name, town) = base.map_or((&last.name, &last.town), |(n, t)| (n, t));
    (name, if town.trim().is_empty() { &last.town } else { town })
}

/// One row of a ranking table, before it has a rank.
struct Ranked {
    id: i64,
    display: f64,
    sigma: f64,
    extra: f64,
    change: f64,
    provisional: bool,
}

/// What a ranking table reads: the entity's table, its id and value columns,
/// and the figure it shows beside the rating.
struct RankSpec<'a> {
    table: &'a str,
    id: &'a str,
    value: &'a str,
    extra: &'a str,
    a: f64,
    b: f64,
    /// Rank by the rating less this many sigmas; 0 settled first, by rating.
    sigmas: f64,
}

/// The two ranking tables of the last release; returns how many players and
/// teams they list.
fn ranks(c: &Ctx, release: i64) -> (usize, usize) {
    let d = c.d;
    let p = RankSpec { table: "player_ratings", id: "player_id", value: "theta", extra: "games_365", a: d.a_player, b: d.b_player, sigmas: d.rank_sigmas };
    let t = RankSpec { table: "team_ratings", id: "team_id", value: "mu", extra: "evidence", a: d.a_team, b: d.b_team, sigmas: 0.0 };
    let (prows, trows) = (ranked(c, &p, release), ranked(c, &t, release));
    let mut ins = c.w.prepare("insert into player_rank values (?1,?2,?3,?4,?5,?6,?7)").unwrap();
    for (i, x) in prows.iter().enumerate() {
        ins.execute(params![i as i64 + 1, x.id, x.display, x.sigma, x.extra as i64, x.change, x.provisional as i64]).unwrap();
    }
    let ql = qls(c, release, &trows);
    let mut ins = c.w.prepare("insert into team_rank values (?1,?2,?3,?4,?5,?6,?7,?8,?9)").unwrap();
    for (i, x) in trows.iter().enumerate() {
        let (q, k) = ql.get(&x.id).map_or((None, None), |&(q, k)| (Some(q), Some(k)));
        ins.execute(params![i as i64 + 1, x.id, x.display, x.sigma, x.extra, x.change, x.provisional as i64, q, k]).unwrap();
    }
    (prows.len(), trows.len())
}

/// Each ranked team's Q(L) at the release, the strength of its base roster, and
/// its place among the ranked teams by it; none for a fit without `team_ql`.
fn qls(c: &Ctx, release: i64, teams: &[Ranked]) -> HashMap<i64, (f64, i64)> {
    if !table_exists(c.r, "team_ql") {
        return HashMap::new();
    }
    let ranked: HashSet<i64> = teams.iter().map(|t| t.id).collect();
    let mut rows: Vec<(i64, f64)> = Vec::new();
    each(c.r, &format!("select team_id, ql from team_ql where release_id = {release}"), |x| {
        let t: i64 = x.get("team_id").unwrap();
        if ranked.contains(&t) {
            rows.push((t, x.get("ql").unwrap()));
        }
    });
    rows.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    rows.into_iter().enumerate().map(|(i, (t, q))| (t, (q, i as i64 + 1))).collect()
}

/// The release's rows of one ratings table in the published order: settled
/// ratings first, then by rating.
fn ranked(c: &Ctx, s: &RankSpec, release: i64) -> Vec<Ranked> {
    let have = columns(c.r, s.table);
    // `delta` is already the move of the published number; an older table has
    // only `change`, on the internal scale.
    let (change, factor) =
        if have.contains("delta") { ("delta", 1.0) } else { (col(&have, "change"), change_factor(c.r, s.table, release, s.b)) };
    let sql = format!(
        "select {} as id, {} as raw, sigma, {} as extra, {change} as change, {} as display from {} where release_id = {release}",
        s.id, s.value, s.extra, col(&have, "display"), s.table
    );
    let mut out = Vec::new();
    each(c.r, &sql, |x| {
        let sigma = f64_or(x, "sigma", 0.0);
        out.push(Ranked {
            id: x.get("id").unwrap(),
            display: f64_or(x, "display", s.a + s.b * f64_or(x, "raw", 0.0)),
            sigma: s.b.abs() * sigma,
            extra: f64_or(x, "extra", 0.0),
            change: factor * f64_or(x, "change", 0.0),
            provisional: sigma > c.d.provisional_sigma,
        });
    });
    if s.sigmas > 0.0 {
        let low = |x: &Ranked| x.display - s.sigmas * x.sigma;
        out.sort_by(|x, y| low(y).total_cmp(&low(x)).then(x.id.cmp(&y.id)));
    } else {
        out.sort_by(|x, y| x.provisional.cmp(&y.provisional).then(y.display.total_cmp(&x.display)).then(x.id.cmp(&y.id)));
    }
    out
}

/// `change` is a delta of the internal value, so it needs the display factor —
/// but a future schema could publish it already scaled. Deltas on the internal
/// scale never reach 1; on a published scale of thousands they routinely do, so
/// the largest one in the table says which this is.
fn change_factor(db: &Connection, table: &str, release: i64, b: f64) -> f64 {
    const SCALED: f64 = 10.0;
    if !columns(db, table).contains("change") || b <= SCALED {
        return 1.0;
    }
    let biggest: Option<f64> = db
        .query_row(&format!("select max(abs(change)) from {table} where release_id = ?1"), [release], |r| r.get(0))
        .unwrap_or(None);
    match biggest {
        Some(x) if x > 1.0 => 1.0,
        _ => b,
    }
}

/// The meta table: the release, the display maps and what the server may read.
fn meta(c: &Ctx, last: &Release, (players, teams): (usize, usize), reads: &Reads) {
    let d = c.d;
    let flag = |b: bool| (b as u8).to_string();
    // A fit from before the rules had a version leaves it empty.
    let model: String =
        c.r.query_row("select value from meta where key = 'model_version'", [], |r| r.get(0)).unwrap_or_default();
    for (k, v) in [
        ("model_version", model),
        ("index_version", VERSION.to_string()),
        ("release_id", last.0.to_string()),
        ("release_date", last.1.clone()),
        ("n_player_rank", players.to_string()),
        ("n_team_rank", teams.to_string()),
        ("a_team", d.a_team.to_string()),
        ("b_team", d.b_team.to_string()),
        ("a_player", d.a_player.to_string()),
        ("b_player", d.b_player.to_string()),
        ("player_phi", d.player_phi.to_string()),
        ("provisional_sigma", d.provisional_sigma.to_string()),
        ("rank_sigmas", d.rank_sigmas.to_string()),
        ("clean_from", d.clean_from.clone()),
        ("s2", flag(d.s2)),
        ("achieve", flag(reads.achieve)),
        ("team_steps", flag(reads.team_steps)),
    ] {
        c.w.execute("insert into meta values (?1,?2)", params![k, v]).unwrap();
    }
}
