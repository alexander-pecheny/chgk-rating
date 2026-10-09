//! Does the rating catch cheats?
//!
//! Cheating is close to impossible at an in-person tournament, awkward at a
//! synchronous one and trivial when the synchronous circuit is played at home,
//! as it was through 2020 and 2021. So a cheat's strength depends on the format
//! and an honest player's does not, and the rating stores enough to measure it.
//!
//! Two statistics per result, both centred on the mean of results of the same
//! kind, so the systematic differences between formats divide out:
//!
//! * **surprise** — `ln(forecast place) − ln(actual place)`. Available for every
//!   result, including the ones flagged `inRating = 0`, where the forecast is
//!   made from the lineup's individual ratings alone. That is exactly the
//!   question a cheat fails: given these people, where should this team finish?
//! * **residual** — `s* − s_r`, how far above the sum of its parts the team
//!   actually played, on the θ scale. `s*` is recovered by inverting the stored
//!   `z`. Only rated results carry it, but it is in the model's own units.
//!
//! `--profile ID` then reads the masks and asks the independent question: when a
//! suspect's team beat the field, did it beat them on the questions everyone
//! else took, or on the hard ones?
//!
//! The statistics live in `chgk_rating::anomaly`, shared with the site's
//! `/anomaly/` table; this binary reads, groups and prints.

use chgk_rating::anomaly::{self as stat, Acc, MaskEdge};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;

const SIGMA_MODEL: f64 = 0.3669;
const COVID: (&str, &str) = ("2020-03-01", "2021-12-31");
/// The current top the report reads, unless `--top` says otherwise.
const TOP: usize = 200;
/// Section 1: this many in-person results or fewer is almost none.
const THIN_IN_PERSON: f64 = 4.0;
/// The peers a thin player's online mean is read against: players with this
/// many online results.
const PEERS_MIN: f64 = 10.0;
const PEERS_MAX: f64 = 30.0;
const PEER_QUANTILES: [f64; 4] = [0.5, 0.95, 0.99, 0.999];
/// Results of each kind a player needs for a format comparison.
const MIN_EACH: f64 = 10.0;
/// Section 4: in-person results a player needs to be called worse at the table.
const MIN_IN_PERSON_DROP: f64 = 15.0;
/// Rows the gap tables show, and the rest.
const SHORT_TABLE: usize = 15;
const LONG_TABLE: usize = 20;
/// Characters of a tournament's name a profile shows.
const NAME_WIDTH: usize = 34;
/// A profile splits a package into thirds only with this many questions and
/// teams.
const PROFILE_MIN_QUESTIONS: usize = 9;
const PROFILE_MIN_TEAMS: usize = 10;

/// Mask test: an easy-edge bin needs this many results to say what a team's
/// hard edge normally is.
const MIN_BIN: f64 = 30.0;
/// Mask test: the current top it reads, always.
const MASK_TOP: i64 = 200;
/// Mask test: covid online results, and the in-person results to compare them
/// with, a player needs.
const MIN_COVID: f64 = 12.0;
const MIN_IN_PERSON_COVID: f64 = 20.0;
/// Mask test: online results a player needs for the all-players table.
const MIN_ONLINE_ALL: f64 = 12.0;
const MASK_QUANTILES: [f64; 3] = [0.5, 0.99, 0.999];
/// Mask test: results with masks a team needs, all told and in each format of
/// the covid comparison.
const MIN_TEAM: f64 = 30.0;
const MIN_TEAM_EACH: f64 = 12.0;

/// Every player of a played game, joined so the roster has only known players.
const ROSTER_SQL: &str = "select g.player_id, g.tournament_id, g.team_id \
    from x.player_game g join x.player p on p.id = g.player_id \
    left join x.player_rank k on k.player_id = g.player_id";
const INFO_SQL: &str = "select p.id, p.name, p.games, k.rank, k.display \
    from x.player p left join x.player_rank k on k.player_id = p.id";
const EVENTS_SQL: &str = "select v.tournament_id, v.team_id, t.type, t.date, v.rated, \
    v.forecast_place, v.actual_place, v.x, v.z, v.sigma_before, v.s_r \
    from team_events v join x.tournament t on t.id = v.tournament_id";
const GAMES_SQL: &str = "select g.tournament_id, t.name as tournament, t.date, t.type, g.team_id, m.name as team, \
    v.forecast_place, v.actual_place, v.rated \
    from x.player_game g join x.tournament t on t.id = g.tournament_id \
    left join x.team m on m.id = g.team_id \
    left join team_events v on v.tournament_id = g.tournament_id and v.team_id = g.team_id \
    where g.player_id = ?1 order by t.date";
const SPAN_SQL: &str = "select min(t.date), max(t.date) from x.result_ r \
    join x.tournament t on t.id = r.tournament_id where r.team_id = ?1";

const INTRO: &str = "Списать на очном турнире почти невозможно, на синхроне трудно, дома за монитором — \
    тривиально. Поэтому у читера сила зависит от формата, а у честного игрока нет.\n";
const THIN_TEXT: &str = "Игроки текущего топа, у которых почти нет очных турниров, отсортированные по тому, \
    насколько их команды обыгрывают собственный прогноз на синхроне.\n";
const GAP_TEXT: &str = "У кого разница «обыгрываю прогноз онлайн, не обыгрываю очно» больше всего. \
    Нужно не меньше десяти турниров каждого вида.\n";
const COVID_TEXT: &str = "Синхроны этих двух лет играли из дома. Игроки, чьи команды тогда обыгрывали прогноз \
    сильнее всего, — и та же величина на их очных турнирах в другие годы.\n";
const DROP_TEXT: &str = "То же, но с требованием, чтобы очный результат был отрицательным: играл дома сильно \
    выше прогноза, за столом — ниже.\n";
const RESID_TEXT: &str = "Остаток `s* − s_r` — насколько команда сыграла выше суммы своих частей на шкале θ. \
    Считается только по зачтённым результатам, поэтому выборка меньше.\n";
const MASK_COVID_TEXT: &str = "С поправкой на собственную силу: у сильного игрока избыток положителен и очно, \
    поэтому здесь сортировка по разнице «дома минус за столом», и очных турниров \
    требуется не меньше двадцати.\n";

const GAP_HEAD: &str = "| Игрок | Рейтинг | Место | Очно n | Очно | Онлайн n | Онлайн | Разрыв | t |";
const RULE_9: &str = "|---|--:|--:|--:|--:|--:|--:|--:|--:|";
const RULE_8: &str = "|---|--:|--:|--:|--:|--:|--:|--:|";

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    InPerson,
    Sync,
    StrictSync,
}

impl Kind {
    fn parse(s: &str) -> Option<Kind> {
        match s {
            "in-person" => Some(Kind::InPerson),
            "sync" => Some(Kind::Sync),
            "strict-sync" => Some(Kind::StrictSync),
            _ => None,
        }
    }
    fn online(self) -> bool {
        self != Kind::InPerson
    }
    fn label(self) -> &'static str {
        match self {
            Kind::InPerson => "in person",
            Kind::Sync => "sync     ",
            Kind::StrictSync => "strict   ",
        }
    }
}

fn in_covid(date: &str) -> bool {
    date >= COVID.0 && date <= COVID.1
}

struct Event {
    kind: Kind,
    covid: bool,
    resid: Option<f64>,
    surprise: Option<f64>,
}

/// One player's numbers. The main report splits surprise and residual by
/// where they were earned; the mask test splits the hard-third surplus the
/// same way in `on`, `ip` and `covid`.
#[derive(Default)]
struct Player {
    name: String,
    rank: Option<i64>,
    display: f64,
    games: i64,
    ip: Acc,
    on: Acc,
    /// The online results of the covid years, also counted in `on`.
    covid: Acc,
    r_ip: Acc,
    r_on: Acc,
}

impl Player {
    fn push(&mut self, kind: Kind, covid: bool, v: f64) {
        if kind.online() {
            self.on.push(v);
            if covid {
                self.covid.push(v);
            }
        } else {
            self.ip.push(v);
        }
    }

    fn push_resid(&mut self, kind: Kind, v: f64) {
        if kind.online() {
            self.r_on.push(v);
        } else {
            self.r_ip.push(v);
        }
    }
}

type Players = HashMap<i64, Player>;

struct Args {
    ratings: String,
    web: String,
    buff: String,
    top: usize,
    masks: bool,
    who: Vec<i64>,
    profile: Vec<i64>,
}

fn args() -> Args {
    let mut a = std::env::args().skip(1);
    let mut v = Args {
        ratings: "out/ratings.db".into(),
        web: "out/web.db".into(),
        buff: "buff.db".into(),
        top: TOP,
        masks: false,
        who: Vec::new(),
        profile: Vec::new(),
    };
    while let Some(x) = a.next() {
        let mut next = || a.next().unwrap();
        match x.as_str() {
            "--db" => v.ratings = next(),
            "--web" => v.web = next(),
            "--buff" => v.buff = next(),
            "--top" => v.top = next().parse().unwrap(),
            "--masks" => v.masks = true,
            "--who" => {
                v.masks = true;
                v.who.push(next().parse().unwrap());
            }
            "--profile" => v.profile.push(next().parse().unwrap()),
            _ => panic!("unknown argument {x}"),
        }
    }
    v
}

fn open(path: &str) -> Connection {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|e| panic!("open {path}: {e}"))
}

fn main() {
    let a = args();
    let db = open(&a.ratings);
    db.execute("ATTACH DATABASE ?1 AS x", [&a.web]).expect("attach the serving index");
    if a.masks {
        return masks_mode(&db, &open(&a.buff), &a.who);
    }
    let (evs, (sd_s, sd_r)) = load_events(&db);
    eprintln!("{} results, surprise sd {sd_s:.3}, residual sd {sd_r:.3}", evs.len());
    if !a.profile.is_empty() {
        let b = open(&a.buff);
        for id in a.profile {
            profile_player(&db, &b, &evs, id);
        }
        return;
    }
    report(&db, &evs, a.top, (sd_s, sd_r));
}

fn report(db: &Connection, evs: &HashMap<(i64, i64), Event>, top: usize, (sd_s, sd_r): (f64, f64)) {
    let players = attribute(db, evs);
    eprintln!("{} players with attributable results", players.len());
    let top_n = top_players(&players, top);
    println!("# Ищем читеров в собственных числах рейтинга\n");
    println!("{INTRO}");
    thin_section(top, &top_n, sd_s);
    peers_line(&players);
    println!("\n## 2. Топ-{top}: разрыв между синхроном и очными\n");
    println!("{GAP_TEXT}");
    gap_table(&top_n, |p| &p.on, |p| &p.ip, sd_s);
    covid_section(&players, sd_s);
    drop_section(&players);
    println!("\n## 5. Для сверки: тот же разрыв в единицах модели\n");
    println!("{RESID_TEXT}");
    gap_table(&top_n, |p| &p.r_on, |p| &p.r_ip, sd_r);
}

/// The current top `top`, best first.
fn top_players(players: &Players, top: usize) -> Vec<&Player> {
    let mut v: Vec<&Player> = players.values().filter(|p| p.rank.map_or(false, |r| r as usize <= top)).collect();
    v.sort_by_key(|p| p.rank.unwrap());
    v
}

// ── table cells ──────────────────────────────────────────────────────────────

fn fmt(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.0}")
    } else {
        "—".into()
    }
}

fn n0(v: f64) -> String {
    format!("{v:.0}")
}

fn s3(v: f64) -> String {
    format!("{v:+.3}")
}

fn t1(v: f64) -> String {
    format!("{v:.1}")
}

fn rank(r: Option<i64>) -> String {
    r.map(|r| r.to_string()).unwrap_or_else(|| "—".into())
}

/// A mean, or a dash when there is none.
fn mean_or_dash(a: &Acc) -> String {
    if a.n > 0.0 {
        s3(a.mean())
    } else {
        "—".into()
    }
}

fn md_row(cells: &[String]) {
    println!("| {} |", cells.join(" | "));
}

/// A row that starts with the player's name, rating and place.
fn player_row(p: &Player, rest: &[String]) {
    let mut cells = vec![p.name.clone(), fmt(p.display), rank(p.rank)];
    cells.extend_from_slice(rest);
    md_row(&cells);
}

fn head(names: &str, rule: &str) {
    println!("{names}");
    println!("{rule}");
}

/// The values at the given shares of a sorted list.
fn quantiles(sorted: &[f64], at: &[f64]) -> Vec<f64> {
    at.iter().map(|f| sorted[((sorted.len() - 1) as f64 * f) as usize]).collect()
}

// ── the main report ──────────────────────────────────────────────────────────

fn thin_section(top: usize, top_n: &[&Player], sd: f64) {
    println!("## 1. Топ-{top}: рейтинг, не подтверждённый за столом\n");
    println!("{THIN_TEXT}");
    let mut thin: Vec<&Player> = top_n.iter().copied().filter(|p| p.ip.n <= THIN_IN_PERSON).collect();
    thin.sort_by(|a, b| b.on.t(sd).total_cmp(&a.on.t(sd)));
    head("| Игрок | Рейтинг | Место | Игр | Очно n | Онлайн n | Онлайн | t |", RULE_8);
    for p in thin.iter().take(LONG_TABLE) {
        player_row(p, &[p.games.to_string(), n0(p.ip.n), n0(p.on.n), s3(p.on.mean()), t1(p.on.t(sd))]);
    }
}

/// How rare a mean that large is, among players with a comparable number of
/// online results. Without this the table above is unreadable.
fn peers_line(players: &Players) {
    let mut peers: Vec<f64> =
        players.values().filter(|p| (PEERS_MIN..=PEERS_MAX).contains(&p.on.n)).map(|p| p.on.mean()).collect();
    peers.sort_by(f64::total_cmp);
    let q = quantiles(&peers, &PEER_QUANTILES);
    println!(
        "\nСреди {} игроков с 10–30 онлайн-результатами среднее превышение прогноза \
         распределено так: медиана {:+.3}, 95-й процентиль {:+.3}, 99-й {:+.3}, \
         99.9-й {:+.3}, максимум {:+.3}.",
        peers.len(),
        q[0],
        q[1],
        q[2],
        q[3],
        peers[peers.len() - 1]
    );
}

/// Players of the top with enough results of both kinds, by how sure the
/// online-minus-in-person gap is.
fn gap_table(top_n: &[&Player], on: fn(&Player) -> &Acc, ip: fn(&Player) -> &Acc, sd: f64) {
    let mut gap: Vec<(&Player, f64)> = top_n
        .iter()
        .filter(|p| ip(p).n >= MIN_EACH && on(p).n >= MIN_EACH)
        .map(|p| (*p, stat::split_t(on(p), ip(p), sd)))
        .collect();
    gap.sort_by(|a, b| b.1.total_cmp(&a.1));
    head(GAP_HEAD, RULE_9);
    for (p, t) in gap.iter().take(SHORT_TABLE) {
        let (on, ip) = (on(p), ip(p));
        let g = on.mean() - ip.mean();
        player_row(p, &[n0(ip.n), s3(ip.mean()), n0(on.n), s3(on.mean()), s3(g), t1(*t)]);
    }
}

fn covid_section(players: &Players, sd: f64) {
    println!("\n## 3. Ковидный онлайн: {} — {}\n", COVID.0, COVID.1);
    println!("{COVID_TEXT}");
    let mut covid: Vec<&Player> = players.values().filter(|p| p.covid.n >= MIN_EACH && p.ip.n >= MIN_EACH).collect();
    covid.sort_by(|a, b| b.covid.t(sd).total_cmp(&a.covid.t(sd)));
    head("| Игрок | Рейтинг | Место | Ковид n | Ковид | t | Очно n | Очно |", RULE_8);
    for p in covid.iter().take(LONG_TABLE) {
        let c = &p.covid;
        player_row(p, &[n0(c.n), s3(c.mean()), t1(c.t(sd)), n0(p.ip.n), s3(p.ip.mean())]);
    }
}

fn drop_section(players: &Players) {
    println!("\n## 4. Ковидный онлайн: кто именно тогда вырос\n");
    println!("{DROP_TEXT}");
    let rose = |p: &Player| p.covid.mean() - p.ip.mean();
    let mut both: Vec<&Player> = players
        .values()
        .filter(|p| p.covid.n >= MIN_EACH && p.ip.n >= MIN_IN_PERSON_DROP && p.ip.mean() < 0.0)
        .collect();
    both.sort_by(|a, b| rose(b).total_cmp(&rose(a)));
    head("| Игрок | Рейтинг | Место | Ковид n | Ковид | Очно n | Очно | Разрыв |", RULE_8);
    for p in both.iter().take(LONG_TABLE) {
        player_row(p, &[n0(p.covid.n), s3(p.covid.mean()), n0(p.ip.n), s3(p.ip.mean()), s3(rose(p))]);
    }
}

/// A result's raw statistics and the cell they are centred in.
struct Raw {
    key: (i64, i64),
    kind: Kind,
    covid: bool,
    rated: bool,
    surprise: Option<f64>,
    resid: Option<f64>,
}

/// `s* − s_r`, with `s*` recovered from the stored `z`.
fn residual(x: Option<f64>, z: Option<f64>, sb: Option<f64>, sr: Option<f64>) -> Option<f64> {
    let (x, z, sb, sr) = (x?, z?, sb?, sr?);
    Some(x + z * (sb * sb + SIGMA_MODEL * SIGMA_MODEL).sqrt() - sr)
}

fn raw_event(r: &rusqlite::Row) -> Option<Raw> {
    let kind = Kind::parse(&r.get::<_, String>("type").unwrap_or_default())?;
    let date: String = r.get("date").unwrap_or_default();
    let rated = r.get::<_, Option<i64>>("rated").unwrap().unwrap_or(1) != 0;
    let num = |c: &str| r.get::<_, Option<f64>>(c).ok().flatten();
    let surprise = num("forecast_place").zip(num("actual_place")).and_then(|(f, a)| stat::surprise(f, a));
    let resid = residual(num("x"), num("z"), num("sigma_before"), num("s_r"));
    let key = (r.get("tournament_id").unwrap(), r.get("team_id").unwrap());
    let (surprise, resid) = (surprise.filter(|v| v.is_finite()), resid.filter(|v| v.is_finite()));
    Some(Raw { key, kind, covid: in_covid(&date), rated, surprise, resid })
}

fn read_events(db: &Connection) -> Vec<Raw> {
    let mut st = db.prepare(EVENTS_SQL).unwrap();
    let mut rows = st.query([]).unwrap();
    let mut raw = Vec::new();
    while let Some(r) = rows.next().unwrap() {
        raw.extend(raw_event(r));
    }
    raw
}

/// Every result, with both statistics centred on the mean of its own cell —
/// format crossed with whether the result counted for the team's rating —
/// and each statistic's spread about those means.
fn load_events(db: &Connection) -> (HashMap<(i64, i64), Event>, (f64, f64)) {
    let raw = read_events(db);
    let s_items = || raw.iter().filter_map(|e| e.surprise.map(|v| ((e.kind, e.rated), v)));
    let r_items = || raw.iter().filter_map(|e| e.resid.map(|v| (e.kind, v)));
    let (cs, cr) = (stat::cell_means(s_items()), stat::cell_means(r_items()));
    let pop = (stat::centred_sd(s_items(), &cs), stat::centred_sd(r_items(), &cr));
    for (k, v) in &cs {
        let rated = if k.1 { "rated  " } else { "unrated" };
        eprintln!("  {} {}: n {:.0}, mean surprise {:+.3}", k.0.label(), rated, v.n, v.mean());
    }
    let event = |e: Raw| {
        let surprise = e.surprise.map(|v| v - cs[&(e.kind, e.rated)].mean());
        let resid = e.resid.map(|v| v - cr[&e.kind].mean());
        (e.key, Event { kind: e.kind, covid: e.covid, surprise, resid })
    };
    (raw.into_iter().map(event).collect(), pop)
}

fn attribute(db: &Connection, evs: &HashMap<(i64, i64), Event>) -> Players {
    let mut players = Players::new();
    stat::attribute(&stat::roster(db, ROSTER_SQL), evs, |pid, ev| {
        let e = players.entry(pid).or_default();
        if let Some(v) = ev.surprise {
            e.push(ev.kind, ev.covid, v);
        }
        if let Some(v) = ev.resid {
            e.push_resid(ev.kind, v);
        }
    });
    fill_info(db, &mut players);
    players
}

/// Name, games, place and rating of every player attributed anything.
fn fill_info(db: &Connection, players: &mut Players) {
    let mut st = db.prepare(INFO_SQL).unwrap();
    let mut rows = st.query([]).unwrap();
    while let Some(r) = rows.next().unwrap() {
        let Some(p) = players.get_mut(&r.get("id").unwrap()) else { continue };
        p.name = r.get::<_, Option<String>>("name").unwrap().unwrap_or_default();
        p.games = r.get::<_, Option<i64>>("games").unwrap().unwrap_or(0);
        p.rank = r.get("rank").unwrap();
        p.display = r.get::<_, Option<f64>>("display").unwrap().unwrap_or(f64::NAN);
    }
}

// ── one player's profile ─────────────────────────────────────────────────────

/// One game of a profile.
struct Game {
    tid: i64,
    name: String,
    date: String,
    kind: String,
    team: i64,
    team_name: String,
    forecast: Option<f64>,
    place: Option<f64>,
    rated: bool,
}

fn game(r: &rusqlite::Row) -> rusqlite::Result<Game> {
    let text = |c: &str| r.get::<_, Option<String>>(c).map(Option::unwrap_or_default);
    Ok(Game {
        tid: r.get("tournament_id")?,
        name: text("tournament")?,
        date: text("date")?,
        kind: text("type")?,
        team: r.get("team_id")?,
        team_name: text("team")?,
        forecast: r.get("forecast_place")?,
        place: r.get("actual_place")?,
        rated: r.get::<_, Option<i64>>("rated")?.unwrap_or(1) != 0,
    })
}

fn games(db: &Connection, id: i64) -> Vec<Game> {
    let mut st = db.prepare(GAMES_SQL).unwrap();
    st.query_map([id], game).unwrap().map(|r| r.unwrap()).collect()
}

fn or_dash<T>(v: Option<T>, f: impl Fn(T) -> String) -> String {
    v.map(f).unwrap_or_else(|| "—".into())
}

/// One player's results, and the independent mask test: on the questions the
/// field found hardest, how far above the field did this team score?
fn profile_player(db: &Connection, buff: &Connection, evs: &HashMap<(i64, i64), Event>, id: i64) {
    let name: String = db
        .query_row("select name from x.player where id = ?1", [id], |r| r.get(0))
        .unwrap_or_else(|_| format!("игрок {id}"));
    println!("\n## {name} (id {id})\n");
    head(
        "| Дата | Турнир | Тип | Зач. | Команда | Прогноз | Место | Взято | Лёгкие | Трудные |",
        "|---|---|---|---|---|--:|--:|--:|--:|--:|",
    );
    let (mut easy, mut hard) = (Acc::default(), Acc::default());
    for g in &games(db, id) {
        let edge = mask_edge(buff, g.tid, g.team);
        if let Some((e, h)) = edge {
            easy.push(e);
            hard.push(h);
        }
        profile_row(g, evs.get(&(g.tid, g.team)).and_then(|e| e.surprise), edge);
    }
    println!(
        "\nСредний отрыв от поля: на лёгкой трети пакета {:+.3}, на трудной {:+.3} ({:.0} турниров).",
        easy.mean(),
        hard.mean(),
        easy.n
    );
}

fn profile_row(g: &Game, surprise: Option<f64>, edge: Option<(f64, f64)>) {
    md_row(&[
        g.date.clone(),
        g.name.chars().take(NAME_WIDTH).collect(),
        g.kind.clone(),
        (if g.rated { "да" } else { "нет" }).into(),
        g.team_name.clone(),
        or_dash(g.forecast, |v| format!("{v:.0}")),
        or_dash(g.place, |v| format!("{v:.0}")),
        or_dash(surprise, |v| format!("{v:+.2}")),
        or_dash(edge, |x| format!("{:+.2}", x.0)),
        or_dash(edge, |x| format!("{:+.2}", x.1)),
    ]);
}

/// How far this team's take rate sits above the field's, on the easiest and the
/// hardest third of the package. A team that looks answers up gains where the
/// field cannot, so its edge is lopsided towards the hard third.
fn mask_edge(buff: &Connection, tid: i64, team: i64) -> Option<(f64, f64)> {
    let mut st = buff.prepare("select team_id, mask from tournament_results where id = ?1").ok()?;
    let rows: Vec<(i64, Vec<u8>)> = st
        .query_map([tid], |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default().into_bytes())))
        .ok()?
        .filter_map(|r| r.ok())
        .filter(|r| !r.1.is_empty())
        .collect();
    let mine = &rows.iter().find(|r| r.0 == team)?.1;
    let n_q = rows.iter().map(|r| r.1.len()).max()?;
    if n_q < PROFILE_MIN_QUESTIONS || rows.len() < PROFILE_MIN_TEAMS {
        return None;
    }
    let take = stat::take_rates(rows.iter().map(|r| r.1.as_slice()));
    stat::Package::new(take, &stat::PROFILE_THIRDS)?.edges(mine, &stat::PROFILE_THIRDS)
}

// ── the mask test ────────────────────────────────────────────────────────────
//
// A team's answers against the field's, on the third of the package the field
// found hardest and the third it found easiest. This uses no rating at all: the
// field's own take rate is the difficulty scale.
//
// A strong team beats the field everywhere, so the hard-third edge on its own
// is just strength. What is calibrated away is the hard-third edge a team with
// this easy-third edge normally has — the easy third being the part of the
// package a cheat has no reason to touch, because everyone already knows it.
// The remainder is the surplus: points taken where nobody in the hall knew the
// answer, by a team that was ordinary where everybody did.
//
// Conditioning on the easy third rather than on the overall edge matters. A
// team whose mask is misaligned with the question order looks flat across
// difficulty — far below the field on the easy third, level with it on the
// hard — and against an overall-edge baseline that reads as the strongest
// signal in the data. Against the easy third it does not.

/// Format and covid flag of every tournament.
type Meta = HashMap<i64, (Kind, bool)>;

fn tournament_kinds(db: &Connection) -> Meta {
    let mut meta = Meta::new();
    let mut st = db.prepare("select id, type, date from x.tournament").unwrap();
    let mut rows = st.query([]).unwrap();
    while let Some(r) = rows.next().unwrap() {
        let Some(k) = Kind::parse(&r.get::<_, String>(1).unwrap_or_default()) else { continue };
        let d: String = r.get(2).unwrap_or_default();
        meta.insert(r.get(0).unwrap(), (k, in_covid(&d)));
    }
    meta
}

/// The hard-third surplus of every result, and of every team.
#[derive(Default)]
struct Surplus {
    /// Its spread over all results.
    sd: f64,
    /// Per result: the surplus, the format and the covid flag.
    result: HashMap<(i64, i64), (f64, Kind, bool)>,
    team: HashMap<i64, Acc>,
    team_covid: HashMap<i64, Acc>,
    team_ip: HashMap<i64, Acc>,
}

impl Surplus {
    /// The hard-third edge less the one a team with this easy-third edge
    /// normally has; results in a thin bin have no surplus.
    fn new(res: &[MaskEdge], meta: &Meta) -> Surplus {
        let curve = stat::cell_means(res.iter().map(|r| (stat::edge_bin(r.easy), r.hard)));
        let expect = |t: f64| curve.get(&stat::edge_bin(t)).filter(|a| a.n >= MIN_BIN).map(|a| a.mean());
        let usable = || res.iter().filter(|r| expect(r.easy).is_some());
        let sd = stat::centred_sd(usable().map(|r| (stat::edge_bin(r.easy), r.hard)), &curve);
        eprintln!("surplus sd {sd:.4} over {} results", usable().count());
        let mut s = Surplus { sd, ..Default::default() };
        for r in res {
            let Some(e) = expect(r.easy) else { continue };
            s.add(r, r.hard - e, meta[&r.tid]);
        }
        s
    }

    fn add(&mut self, r: &MaskEdge, s: f64, (kind, covid): (Kind, bool)) {
        self.result.insert((r.tid, r.team), (s, kind, covid));
        self.team.entry(r.team).or_default().push(s);
        if covid && kind.online() {
            self.team_covid.entry(r.team).or_default().push(s);
        }
        if !kind.online() {
            self.team_ip.entry(r.team).or_default().push(s);
        }
    }
}

fn masks_mode(db: &Connection, buff: &Connection, who: &[i64]) {
    let t0 = std::time::Instant::now();
    let meta = tournament_kinds(db);
    let masks = stat::read_masks(buff, &meta);
    eprintln!("masks for {} tournaments [{:.1}s]", masks.len(), t0.elapsed().as_secs_f64());
    let res = stat::mask_edges(&masks);
    eprintln!("{} results with usable masks [{:.1}s]", res.len(), t0.elapsed().as_secs_f64());
    let s = Surplus::new(&res, &meta);
    let mut ps = Players::new();
    stat::attribute(&stat::roster(db, ROSTER_SQL), &s.result, |pid, &(v, kind, covid)| {
        ps.entry(pid).or_default().push(kind, covid, v);
    });
    fill_info(db, &mut ps);
    if !who.is_empty() {
        return who_table(&ps, who, s.sd);
    }
    mask_report(db, &ps, &s);
}

fn who_table(ps: &Players, who: &[i64], sd: f64) {
    head("| Игрок | Место | Онлайн n | Избыток | t | Очно n | Очно |", "|---|--:|--:|--:|--:|--:|--:|");
    for p in who.iter().filter_map(|id| ps.get(id)) {
        let on = &p.on;
        md_row(&[p.name.clone(), rank(p.rank), n0(on.n), s3(on.mean()), t1(on.t(sd)), n0(p.ip.n), mean_or_dash(&p.ip)]);
    }
}

fn mask_report(db: &Connection, ps: &Players, s: &Surplus) {
    println!("# Тест по маскам: где именно команда обыгрывает поле\n");
    println!(
        "Вопросы каждого пакета делятся на трети по доле взятия в поле. Для каждой команды \
         считается отрыв от поля на трудной трети и на лёгкой, и первый сравнивается с тем, \
         какой он обычно бывает при таком же отрыве на лёгкой. Лёгкую треть списывать незачем — \
         её и так берут все, — поэтому остаток и есть избыток на трудных вопросах. Рейтинг \
         в этом тесте не используется вообще; среднеквадратичный разброс остатка {:.3}.\n",
        s.sd
    );
    mask_top(ps, s.sd);
    mask_covid(ps, s.sd);
    mask_all(ps, s.sd);
    let names = team_names(db);
    team_table(db, s, &names);
    team_covid_table(s, &names);
}

/// A surplus row: online count, mean and t, then the in-person count and mean.
fn surplus_row(p: &Player, sd: f64) {
    player_row(p, &[n0(p.on.n), s3(p.on.mean()), t1(p.on.t(sd)), n0(p.ip.n), mean_or_dash(&p.ip)]);
}

fn by_online_t<'a>(ps: &'a Players, keep: impl Fn(&Player) -> bool, sd: f64) -> Vec<&'a Player> {
    let mut v: Vec<&Player> = ps.values().filter(|p| keep(p)).collect();
    v.sort_by(|a, b| b.on.t(sd).total_cmp(&a.on.t(sd)));
    v
}

fn mask_top(ps: &Players, sd: f64) {
    println!("## 1. Текущий топ-200: избыток на трудных вопросах онлайн\n");
    let v = by_online_t(ps, |p| p.rank.map_or(false, |r| r <= MASK_TOP) && p.on.n >= MIN_EACH, sd);
    head("| Игрок | Рейтинг | Место | Онлайн n | Избыток | t | Очно n | Избыток очно |", RULE_8);
    for p in v.iter().take(SHORT_TABLE) {
        surplus_row(p, sd);
    }
}

fn mask_covid(ps: &Players, sd: f64) {
    println!("\n## 2. Ковидный онлайн ({} — {}): игроки\n", COVID.0, COVID.1);
    println!("{MASK_COVID_TEXT}");
    let split_t = |p: &Player| stat::split_t(&p.covid, &p.ip, sd);
    let mut v: Vec<&Player> = ps.values().filter(|p| p.covid.n >= MIN_COVID && p.ip.n >= MIN_IN_PERSON_COVID).collect();
    covid_shift(&v);
    v.sort_by(|a, b| split_t(b).total_cmp(&split_t(a)));
    head("| Игрок | Рейтинг | Место | Ковид n | Ковид | Очно n | Очно | Разрыв | t |", RULE_9);
    for p in v.iter().take(LONG_TABLE) {
        let (c, ip) = (&p.covid, &p.ip);
        player_row(p, &[n0(c.n), s3(c.mean()), n0(ip.n), s3(ip.mean()), s3(c.mean() - ip.mean()), t1(split_t(p))]);
    }
}

/// Everyone's hard-third surplus was higher in those two years, the top three
/// players in the country included, so the population's own shift is the zero
/// the covid table is read against.
fn covid_shift(v: &[&Player]) {
    let mut a = Acc::default();
    for p in v {
        a.push(p.covid.mean() - p.ip.mean());
    }
    println!(
        "Средний сдвиг по всем {:.0} игрокам таблицы — {:+.3}: в ковид избыток вырос у всех, \
         включая первую тройку рейтинга. Интересны только те, кто заметно выше этого.\n",
        a.n,
        a.mean()
    );
}

fn mask_all(ps: &Players, sd: f64) {
    println!("\n## 2b. Самый большой избыток онлайн среди всех игроков\n");
    println!("Без ограничения на текущий топ; двенадцать и более онлайн-турниров.\n");
    let v = by_online_t(ps, |p| p.on.n >= MIN_ONLINE_ALL, sd);
    head("| Игрок | Рейтинг | Место | Онлайн n | Избыток | t | Очно n | Очно |", RULE_8);
    for p in v.iter().take(LONG_TABLE) {
        surplus_row(p, sd);
    }
    let mut all: Vec<f64> = v.iter().map(|p| p.on.mean()).collect();
    all.sort_by(f64::total_cmp);
    let q = quantiles(&all, &MASK_QUANTILES);
    println!(
        "\nПо всем {} игрокам с двенадцатью и более онлайн-турнирами избыток распределён так: \
         медиана {:+.3}, 99-й процентиль {:+.3}, 99.9-й {:+.3}, максимум {:+.3}.",
        all.len(),
        q[0],
        q[1],
        q[2],
        all[all.len() - 1]
    );
}

/// Name and town of every team.
fn team_names(db: &Connection) -> HashMap<i64, (String, String)> {
    let mut st = db.prepare("select id, name, town from x.team").unwrap();
    let text = |r: &rusqlite::Row, i: usize| r.get::<_, Option<String>>(i).map(Option::unwrap_or_default);
    st.query_map([], |r| Ok((r.get(0)?, (text(r, 1)?, text(r, 2)?))))
        .unwrap()
        .map(|x| x.unwrap())
        .collect()
}

/// The first and the last date a team played.
fn team_span(db: &Connection, id: i64) -> (String, String) {
    db.query_row(SPAN_SQL, [id], |r| Ok((r.get(0).unwrap_or_default(), r.get(1).unwrap_or_default())))
        .unwrap_or_default()
}

fn team_table(db: &Connection, s: &Surplus, names: &HashMap<i64, (String, String)>) {
    println!("\n## 4. Команды: наибольший избыток на трудных вопросах за всё время\n");
    println!("Не меньше тридцати результатов с масками.\n");
    let mut w: Vec<(&i64, &Acc)> = s.team.iter().filter(|(_, a)| a.n >= MIN_TEAM).collect();
    w.sort_by(|a, b| b.1.t(s.sd).total_cmp(&a.1.t(s.sd)));
    head("| Команда | Город | n | Избыток | t | Очно n | Очно | Первый | Последний |", "|---|---|--:|--:|--:|--:|--:|---|---|");
    for (id, a) in w.iter().take(LONG_TABLE) {
        let ip = s.team_ip.get(*id).copied().unwrap_or_default();
        let (n, t) = names.get(*id).cloned().unwrap_or_default();
        let span = team_span(db, **id);
        md_row(&[n, t, n0(a.n), s3(a.mean()), t1(a.t(s.sd)), n0(ip.n), mean_or_dash(&ip), span.0, span.1]);
    }
}

fn team_covid_table(s: &Surplus, names: &HashMap<i64, (String, String)>) {
    println!("\n## 3. Ковидный онлайн: команды\n");
    let ip_of = |id: &i64| s.team_ip.get(id).copied().unwrap_or_default();
    let mut v: Vec<(&i64, &Acc)> =
        s.team_covid.iter().filter(|(id, a)| a.n >= MIN_TEAM_EACH && ip_of(id).n >= MIN_TEAM_EACH).collect();
    let t = |id: &i64, x: &Acc| stat::split_t(x, &s.team_ip[id], s.sd);
    v.sort_by(|a, b| t(b.0, b.1).total_cmp(&t(a.0, a.1)));
    head("| Команда | Город | Ковид n | Ковид | Очно n | Очно | Разрыв | Всего n |", "|---|---|--:|--:|--:|--:|--:|--:|");
    for (id, a) in v.iter().take(LONG_TABLE) {
        let (all, ip) = (s.team.get(id).copied().unwrap_or_default(), ip_of(id));
        let (n, town) = names.get(id).cloned().unwrap_or_default();
        let gap = a.mean() - if ip.n > 0.0 { ip.mean() } else { 0.0 };
        md_row(&[n, town, n0(a.n), s3(a.mean()), n0(ip.n), mean_or_dash(&ip), s3(gap), n0(all.n)]);
    }
}
