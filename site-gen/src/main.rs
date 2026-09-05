mod data;
mod html;
mod method;

use data::*;
use html::*;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

const PAGE: usize = 500;
const CHART_POINTS: usize = 160;

struct Args {
    ratings: PathBuf,
    buff: PathBuf,
    config: PathBuf,
    out: PathBuf,
    report: PathBuf,
}

fn args() -> Args {
    let mut a = std::env::args().skip(1);
    let mut ratings = None;
    let mut buff = PathBuf::from("buff.db");
    let mut config = PathBuf::from("config.toml");
    let mut out = PathBuf::from("site");
    let mut report = PathBuf::from("report/report.pdf");
    while let Some(x) = a.next() {
        match x.as_str() {
            "--buff" => buff = a.next().expect("--buff needs a path").into(),
            "--config" => config = a.next().expect("--config needs a path").into(),
            "--out" => out = a.next().expect("--out needs a path").into(),
            "--report" => report = a.next().expect("--report needs a path").into(),
            _ => ratings = Some(PathBuf::from(x)),
        }
    }
    Args {
        ratings: ratings.expect("usage: site-gen <ratings.db> [--buff b] [--config c] [--out d] [--report p]"),
        buff,
        config,
        out,
        report,
    }
}

struct TeamRow {
    id: i64,
    name: String,
    town: String,
    display: f64,
    sigma: f64,
    evidence: f64,
    provisional: bool,
    change: Option<f64>,
}

struct PlayerRow {
    id: i64,
    name: String,
    display: f64,
    sigma: f64,
    games: i64,
    provisional: bool,
    change: Option<f64>,
}

fn main() {
    let t0 = std::time::Instant::now();
    let a = args();
    let mut d = read_display(&a.config);
    let db = open(&a.ratings);
    let buff = open(&a.buff);
    fit_scale(&db, &mut d);
    let d = d;
    eprintln!(
        "display scale: A_T = {:.2}, B_T = {:.2}, A_P = {:.2}, B_P = {:.2}",
        d.a_team, d.b_team, d.a_player, d.b_player
    );

    let rels = releases(&db);
    assert!(!rels.is_empty(), "no releases in {}", a.ratings.display());
    let rel_idx: HashMap<i64, u16> = rels.iter().enumerate().map(|(i, r)| (r.id, i as u16)).collect();
    let last = rels.last().unwrap();
    let cur = (rels.len() - 1) as u16;
    let prev = if rels.len() >= 2 { Some(rels.len() as u16 - 2) } else { None };

    let listed_teams: HashSet<i64> = {
        let mut st = db
            .prepare("select team_id from team_ratings where release_id = ?1 and listed = 1")
            .unwrap();
        let v = st.query_map([last.id], |r| r.get(0)).unwrap();
        v.map(|x| x.unwrap()).collect()
    };
    let team_cols = columns(&db, "team_ratings");
    let player_cols = columns(&db, "player_ratings");
    // A `change` the model wrote is the movement the team earned — 0 for the
    // idle by contract — so it wins over anything computed here. It arrives on
    // the internal scale and needs the display factor.
    let team_change_f = change_factor(&db, "team_ratings", last.id, d.b_team);
    let player_change_f = change_factor(&db, "player_ratings", last.id, d.b_player);

    let team_state: HashMap<i64, (f64, Option<f64>)> = {
        let sql = format!(
            "select team_id, evidence, {} from team_ratings where release_id = ?1",
            col(&team_cols, "change")
        );
        let mut st = db.prepare(&sql).unwrap();
        let v = st
            .query_map([last.id], |r| {
                Ok((r.get(0)?, (r.get::<_, Option<f64>>(1)?.unwrap_or(0.0), r.get(2)?)))
            })
            .unwrap();
        v.map(|x| x.unwrap()).collect()
    };
    let listed_players: HashMap<i64, (i64, Option<f64>)> = {
        let sql = format!(
            "select player_id, games_365, {} from player_ratings where release_id = ?1",
            col(&player_cols, "change")
        );
        let mut st = db.prepare(&sql).unwrap();
        let v = st
            .query_map([last.id], |r| {
                Ok((r.get(0)?, (r.get::<_, Option<i64>>(1)?.unwrap_or(0), r.get(2)?)))
            })
            .unwrap();
        v.map(|x| x.unwrap()).collect()
    };
    eprintln!(
        "release {} ({}): {} teams, {} players [{:.1}s]",
        last.id,
        last.date,
        listed_teams.len(),
        listed_players.len(),
        t0.elapsed().as_secs_f64()
    );

    // The gauge subtracts `shift` from every rating each release, so a team that
    // did not play still moves by −Δshift_cum. Adding Δshift_cum back leaves the
    // movement the team actually earned, and exactly zero for the idle.
    let gauge = match prev {
        Some(p) => last.shift_cum - rels[p as usize].shift_cum,
        None => 0.0,
    };

    let team_hist = team_history(&db, &listed_teams, &rel_idx, &d);
    let player_set: HashSet<i64> = listed_players.keys().copied().collect();
    let player_hist = player_history(&db, &player_set, &rel_idx, &d);
    eprintln!("histories loaded [{:.1}s]", t0.elapsed().as_secs_f64());

    let evs = events(&db);
    let mut ev_by_team: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut ev_by_tour: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, ev) in evs.iter().enumerate() {
        ev_by_team.entry(ev.team_id).or_default().push(i);
        ev_by_tour.entry(ev.tid).or_default().push(i);
    }
    let mets = metrics(&db);
    let qb = question_b(&db);
    let tours = tournaments(&buff, &mets);
    let names = player_names(&buff);
    let ros = rosters(&buff, &mets, &player_set);
    let teams_per_release: HashMap<i64, i64> = {
        let mut st = db
            .prepare("select release_id, count(*) from team_ratings where listed = 1 group by release_id")
            .unwrap();
        let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        v.map(|x| x.unwrap()).collect()
    };
    eprintln!("data loaded [{:.1}s]", t0.elapsed().as_secs_f64());

    // ── the two ranking tables ────────────────────────────────────────────────
    let mut trows: Vec<TeamRow> = listed_teams
        .iter()
        .filter_map(|id| {
            let h = team_hist.get(id)?;
            let now = h.iter().rev().find(|p| p.rel == cur)?;
            let was = prev.and_then(|p| h.iter().rev().find(|q| q.rel == p));
            let (name, town) = ros
                .team_names
                .get(id)
                .cloned()
                .unwrap_or_else(|| (format!("Команда {id}"), String::new()));
            let (evidence, from_db) = team_state.get(id).cloned().unwrap_or((0.0, None));
            let change = match from_db {
                Some(ch) => Some(team_change_f * ch),
                None => was.map(|w| (now.display - w.display) as f64 + d.b_team * gauge),
            };
            Some(TeamRow {
                id: *id,
                name,
                town,
                display: now.display as f64,
                sigma: d.b_team * now.sigma as f64,
                evidence,
                provisional: now.sigma as f64 > d.provisional_sigma,
                change,
            })
        })
        .collect();
    trows.sort_by(|x, y| {
        x.provisional
            .cmp(&y.provisional)
            .then(y.display.partial_cmp(&x.display).unwrap())
    });

    let mut prows: Vec<PlayerRow> = listed_players
        .iter()
        .filter_map(|(id, (games, from_db))| {
            let h = player_hist.get(id)?;
            let now = h.iter().rev().find(|p| p.rel == cur)?;
            let was = prev.and_then(|p| h.iter().rev().find(|q| q.rel == p));
            let change = match from_db {
                Some(ch) => Some(player_change_f * ch),
                None => was.map(|w| (now.display - w.display) as f64 + d.b_player * gauge),
            };
            Some(PlayerRow {
                id: *id,
                name: names.get(id).cloned().unwrap_or_else(|| format!("Игрок {id}")),
                display: now.display as f64,
                sigma: d.b_player * now.sigma as f64,
                games: *games,
                provisional: now.sigma as f64 > d.provisional_sigma,
                change,
            })
        })
        .collect();
    prows.sort_by(|x, y| {
        x.provisional
            .cmp(&y.provisional)
            .then(y.display.partial_cmp(&x.display).unwrap())
    });

    // ── write ─────────────────────────────────────────────────────────────────
    let c = a.out.join("content");
    if c.exists() {
        fs::remove_dir_all(&c).unwrap();
    }
    for p in ["teams", "players", "players/page", "tournaments", "tournaments/page", "page"] {
        fs::create_dir_all(c.join(p)).unwrap();
    }
    let stat = a.out.join("static");
    fs::create_dir_all(&stat).unwrap();
    if a.report.exists() {
        fs::copy(&a.report, stat.join("report.pdf")).unwrap();
    } else {
        eprintln!("warning: no report at {}", a.report.display());
    }

    for (dir, title) in [
        ("teams", "Команды"),
        ("page", "Команды"),
        ("players/page", "Игроки"),
        ("tournaments/page", "Турниры"),
    ] {
        let md = format!("+++\ntitle = \"{title}\"\nrender = false\n+++\n");
        fs::write(c.join(dir).join("_index.md"), md).unwrap();
    }

    let team_pages = (trows.len() + PAGE - 1) / PAGE;
    let team_url = |i: usize| if i == 1 { "/".to_string() } else { format!("/page/{i}/") };
    for i in 1..=team_pages.max(1) {
        let body = team_table(&trows, i, team_pages, &team_url, last);
        let title = if i == 1 { "Команды".to_string() } else { format!("Команды — страница {i}") };
        let path = if i == 1 { c.join("_index.md") } else { c.join(format!("page/{i}.md")) };
        fs::write(path, front_matter(&title, &body)).unwrap();
    }

    let player_pages = (prows.len() + PAGE - 1) / PAGE;
    let player_url = |i: usize| if i == 1 { "/players/".to_string() } else { format!("/players/page/{i}/") };
    for i in 1..=player_pages.max(1) {
        let body = player_table(&prows, i, player_pages, &player_url, last);
        let title = if i == 1 { "Игроки".to_string() } else { format!("Игроки — страница {i}") };
        let path = if i == 1 { c.join("players/_index.md") } else { c.join(format!("players/page/{i}.md")) };
        fs::write(path, front_matter(&title, &body)).unwrap();
    }

    let mut tour_ids: Vec<i64> = mets.keys().copied().collect();
    tour_ids.sort_by(|x, y| mets[y].date.cmp(&mets[x].date).then(y.cmp(x)));
    let tour_pages = (tour_ids.len() + PAGE - 1) / PAGE;
    let tour_url = |i: usize| if i == 1 { "/tournaments/".to_string() } else { format!("/tournaments/page/{i}/") };
    for i in 1..=tour_pages.max(1) {
        let body = tournament_table(&tour_ids, &mets, &tours, i, tour_pages, &tour_url);
        let title = if i == 1 { "Турниры".to_string() } else { format!("Турниры — страница {i}") };
        let path = if i == 1 { c.join("tournaments/_index.md") } else { c.join(format!("tournaments/page/{i}.md")) };
        fs::write(path, front_matter(&title, &body)).unwrap();
    }

    fs::write(
        c.join("releases.md"),
        front_matter("Релизы", &releases_page(&rels, &teams_per_release)),
    )
    .unwrap();
    fs::write(c.join("method.md"), front_matter("Методика", method::BODY)).unwrap();

    let ticks = |from: u16, to: u16| -> Vec<(f64, String)> {
        let mut v = Vec::new();
        for k in 0..4 {
            let i = from as f64 + (to as f64 - from as f64) * k as f64 / 3.0;
            let r = &rels[i.round() as usize];
            v.push((i, r.date[..7].to_string()));
        }
        v.dedup_by(|x, y| x.1 == y.1);
        v
    };

    trows.par_iter().for_each(|t| {
        let body = team_page(t, &team_hist[&t.id], &evs, &ev_by_team, &mets, &tours, &d, &ticks);
        fs::write(c.join(format!("teams/{}.md", t.id)), front_matter(&t.name, &body)).unwrap();
    });
    prows.par_iter().for_each(|p| {
        let body = player_page(p, &player_hist[&p.id], &ros, &mets, &tours, &ticks);
        fs::write(c.join(format!("players/{}.md", p.id)), front_matter(&p.name, &body)).unwrap();
    });
    tour_ids.par_iter().for_each(|tid| {
        let body = tournament_page(*tid, &mets[tid], tours.get(tid), &evs, &ev_by_tour, &ros, &qb, &d);
        let title = tours.get(tid).map(|t| t.name.clone()).unwrap_or_else(|| format!("Турнир {tid}"));
        fs::write(c.join(format!("tournaments/{tid}.md")), front_matter(&title, &body)).unwrap();
    });

    let total = 2 + team_pages + player_pages + tour_pages + trows.len() + prows.len() + tour_ids.len();
    eprintln!(
        "wrote {total} pages ({} teams, {} players, {} tournaments) [{:.1}s]",
        trows.len(),
        prows.len(),
        tour_ids.len(),
        t0.elapsed().as_secs_f64()
    );
}

/// `team_events.date` when the schema has one, the tournament's date otherwise.
fn ev_date(ev: &Event, mets: &HashMap<i64, Metrics>) -> String {
    ev.date
        .clone()
        .or_else(|| mets.get(&ev.tid).map(|m| m.date.clone()))
        .unwrap_or_default()
}

/// A result the model saw but did not let move the rating.
fn unrated_tag(ev: &Event) -> &'static str {
    if ev.rated {
        ""
    } else {
        "<span class=\"unrated\" title=\"Результат не учтён в рейтинге\">вне рейтинга</span>"
    }
}

fn prov_badge() -> &'static str {
    "<span class=\"prov\" title=\"Рейтинг предварительный: неопределённость велика\">предв.</span>"
}

fn change_cell(v: Option<f64>) -> String {
    match v.map(f64::round) {
        None => "<td class=\"num muted\">—</td>".into(),
        Some(x) if x > 0.0 => format!("<td class=\"num delta-up\">{}</td>", signed(x)),
        Some(x) if x < 0.0 => format!("<td class=\"num delta-down\">{}</td>", signed(x)),
        Some(_) => "<td class=\"num muted\">0</td>".into(),
    }
}

fn delta_class(v: f64) -> &'static str {
    match v.round() {
        x if x > 0.0 => "delta-up",
        x if x < 0.0 => "delta-down",
        _ => "muted",
    }
}

/// The listing header: a title plus the standing explanation under it.
fn head(title: &str, lede: &str) -> String {
    format!(
        "<header class=\"page-head\"><h1>{}</h1><p class=\"lede\">{lede}</p></header>",
        e(title)
    )
}

/// The provisional switch. Unchecked by default, so a reader with no JS sees
/// only settled ratings; the choice is remembered per browser.
///
/// Provisional entries sort after every settled one, so on the early pages the
/// switch has nothing to reveal. Saying where the block starts keeps it from
/// reading as a dead control.
fn prov_toggle(hidden: usize, first_page: Option<usize>, page: usize) -> String {
    let note = match (hidden, first_page) {
        (0, Some(p)) if p > page => format!("предварительные — со страницы {p}"),
        (0, _) => String::new(),
        (n, _) => format!("скрыто на этой странице: {n}"),
    };
    format!(
        "<label class=\"checkbox prov-toggle\"><input type=\"checkbox\"> Показывать предварительные\
         <span class=\"prov-count\">{note}</span></label>"
    )
}

fn tour_type(t: &str) -> &str {
    match t {
        "in-person" => "очный",
        "strict-sync" => "строгий синхрон",
        "sync" => "синхрон",
        "async" => "асинхрон",
        other => other,
    }
}

fn team_table(
    rows: &[TeamRow],
    page: usize,
    pages: usize,
    url: &dyn Fn(usize) -> String,
    rel: &Release,
) -> String {
    let firm = rows.iter().filter(|r| !r.provisional).count();
    let slice = &rows[(page - 1) * PAGE..rows.len().min(page * PAGE)];
    let mut s = String::with_capacity(slice.len() * 320 + 1024);
    let _ = write!(
        s,
        "{}",
        head(
            "Команды",
            &format!(
                "Релиз от {}. В таблице {} команд, из них {} с установившимся рейтингом. \
                 Команда попадает в список, если сыграла рейтинговый турнир за последние 365 дней.",
                e(&rel.date),
                rows.len(),
                firm
            )
        )
    );
    s.push_str("<section class=\"section\">");
    s.push_str(&prov_toggle(
        slice.iter().filter(|r| r.provisional).count(),
        (firm < rows.len()).then(|| firm / PAGE + 1),
        page,
    ));
    s.push_str(&pager(page, pages, url));
    s.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">Место</th><th>Команда</th><th>Город</th>\
         <th class=\"num\">Рейтинг</th>\
         <th class=\"num\" title=\"Неопределённость оценки\">±</th>\
         <th class=\"num\">Изменение</th></tr></thead><tbody>",
    );
    let mut seen_prov = (page - 1) * PAGE >= firm;
    for (i, r) in slice.iter().enumerate() {
        let n = (page - 1) * PAGE + i + 1;
        if r.provisional && !seen_prov {
            seen_prov = true;
            s.push_str(
                "<tr class=\"row-split prov-row\"><td colspan=\"6\">Предварительные рейтинги — \
                 команды, чья неопределённость ещё велика</td></tr>",
            );
        }
        let _ = write!(
            s,
            "<tr{}><td class=\"num\">{}</td><td><a href=\"/teams/{}/\">{}</a>{}</td>\
             <td>{}</td><td class=\"num strong\">{}</td><td class=\"num muted\">{}</td>{}</tr>",
            if r.provisional { " class=\"prov-row\"" } else { "" },
            if r.provisional { "—".into() } else { n.to_string() },
            r.id,
            e(&r.name),
            if r.provisional { prov_badge() } else { "" },
            e(&r.town),
            disp(r.display),
            disp(r.sigma),
            change_cell(r.change)
        );
    }
    s.push_str("</tbody></table></div>");
    s.push_str(&pager(page, pages, url));
    s.push_str("</section>");
    s
}

fn player_table(
    rows: &[PlayerRow],
    page: usize,
    pages: usize,
    url: &dyn Fn(usize) -> String,
    rel: &Release,
) -> String {
    let firm = rows.iter().filter(|r| !r.provisional).count();
    let slice = &rows[(page - 1) * PAGE..rows.len().min(page * PAGE)];
    let mut s = String::with_capacity(slice.len() * 300 + 1024);
    let _ = write!(
        s,
        "{}",
        head(
            "Игроки",
            &format!(
                "Релиз от {}. В таблице {} игроков, из них {} с установившимся рейтингом. \
                 Индивидуальный рейтинг — оценка из модели вопросов, без лестничных правил.",
                e(&rel.date),
                rows.len(),
                firm
            )
        )
    );
    s.push_str("<section class=\"section\">");
    s.push_str(&prov_toggle(
        slice.iter().filter(|r| r.provisional).count(),
        (firm < rows.len()).then(|| firm / PAGE + 1),
        page,
    ));
    s.push_str(&pager(page, pages, url));
    s.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">Место</th><th>Игрок</th><th class=\"num\">Рейтинг</th>\
         <th class=\"num\" title=\"Неопределённость оценки\">±</th><th class=\"num\">Игр за год</th>\
         <th class=\"num\">Изменение</th></tr></thead><tbody>",
    );
    let mut seen_prov = (page - 1) * PAGE >= firm;
    for (i, r) in slice.iter().enumerate() {
        let n = (page - 1) * PAGE + i + 1;
        if r.provisional && !seen_prov {
            seen_prov = true;
            s.push_str(
                "<tr class=\"row-split prov-row\"><td colspan=\"6\">Предварительные рейтинги — \
                 игроки, чья неопределённость ещё велика</td></tr>",
            );
        }
        let _ = write!(
            s,
            "<tr{}><td class=\"num\">{}</td><td><a href=\"/players/{}/\">{}</a>{}</td>\
             <td class=\"num strong\">{}</td><td class=\"num muted\">{}</td>\
             <td class=\"num\">{}</td>{}</tr>",
            if r.provisional { " class=\"prov-row\"" } else { "" },
            if r.provisional { "—".into() } else { n.to_string() },
            r.id,
            e(&r.name),
            if r.provisional { prov_badge() } else { "" },
            disp(r.display),
            disp(r.sigma),
            r.games,
            change_cell(r.change)
        );
    }
    s.push_str("</tbody></table></div>");
    s.push_str(&pager(page, pages, url));
    s.push_str("</section>");
    s
}

fn tournament_table(
    ids: &[i64],
    mets: &HashMap<i64, Metrics>,
    tours: &HashMap<i64, Tournament>,
    page: usize,
    pages: usize,
    url: &dyn Fn(usize) -> String,
) -> String {
    let slice = &ids[(page - 1) * PAGE..ids.len().min(page * PAGE)];
    let mut s = String::with_capacity(slice.len() * 260 + 512);
    let _ = write!(
        s,
        "{}",
        head(
            "Турниры",
            &format!("Рейтинговые турниры с 2013 года — {} штук, от новых к старым.", ids.len())
        )
    );
    s.push_str("<section class=\"section\">");
    s.push_str(&pager(page, pages, url));
    s.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th>Дата</th><th>Турнир</th><th>Тип</th><th class=\"num\">Вес</th>\
         <th class=\"num\">Команд</th><th class=\"num\">Вопросов</th>\
         <th class=\"num\" title=\"Сложность пакета\">Сложность</th>\
         <th class=\"num\">Log-loss</th></tr></thead><tbody>",
    );
    for id in slice {
        let m = &mets[id];
        let name = tours.get(id).map(|t| t.name.as_str()).unwrap_or("");
        let _ = write!(
            s,
            "<tr><td class=\"nowrap\">{}</td><td><a href=\"/tournaments/{id}/\">{}</a></td>\
             <td>{}</td><td class=\"num\">{:.2}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            e(&m.date),
            e(if name.is_empty() { "без названия" } else { name }),
            e(tour_type(&m.ttype)),
            m.weight,
            m.n_teams,
            m.n_q,
            blank(m.truedl, 1),
            opt(m.ll_cf, 3)
        );
    }
    s.push_str("</tbody></table></div>");
    s.push_str(&pager(page, pages, url));
    s.push_str("</section>");
    s
}

fn stat(label: &str, value: &str) -> String {
    format!("<div class=\"stat\"><dt>{}</dt><dd>{}</dd></div>", e(label), value)
}

fn team_page(
    t: &TeamRow,
    hist: &[TeamPoint],
    evs: &[Event],
    by_team: &HashMap<i64, Vec<usize>>,
    mets: &HashMap<i64, Metrics>,
    tours: &HashMap<i64, Tournament>,
    d: &Display,
    ticks: &dyn Fn(u16, u16) -> Vec<(f64, String)>,
) -> String {
    let mut s = String::with_capacity(8192);
    let _ = write!(
        s,
        "<header class=\"page-head\"><h1>{}{}</h1><p class=\"muted\">{}</p></header>",
        e(&t.name),
        if t.provisional { prov_badge() } else { "" },
        e(if t.town.is_empty() { "город не указан" } else { &t.town })
    );
    s.push_str("<dl class=\"stats\">");
    s.push_str(&stat("Рейтинг", &e(&disp(t.display))));
    s.push_str(&stat("Неопределённость", &format!("±{}", e(&disp(t.sigma)))));
    s.push_str(&stat("Наигранность", &format!("{:.1}", t.evidence)));
    s.push_str(&stat(
        "Изменение за неделю",
        &t.change.map(|c| signed(c)).unwrap_or_else(|| "—".into()),
    ));
    s.push_str("</dl>");

    let pts = thin(hist, CHART_POINTS);
    let series: Vec<(f64, f64, f64)> = pts
        .iter()
        .map(|p| (p.rel as f64, p.display as f64, (d.b_team * p.sigma as f64).abs()))
        .collect();
    if let (Some(f), Some(l)) = (pts.first(), pts.last()) {
        s.push_str("<section class=\"section\"><h2 class=\"subhead\">История рейтинга</h2>");
        s.push_str(&chart(&series, &ticks(f.rel, l.rel), "История рейтинга команды"));
        s.push_str("</section>");
    }

    let mut idx = by_team.get(&t.id).cloned().unwrap_or_default();
    idx.sort_by(|x, y| {
        let (a, b) = (&evs[*x], &evs[*y]);
        ev_date(b, mets).cmp(&ev_date(a, mets)).then(b.tid.cmp(&a.tid))
    });
    let _ = write!(
        s,
        "<section class=\"section\"><h2 class=\"subhead\">Сыгранные турниры ({})</h2>",
        idx.len()
    );
    if idx.is_empty() {
        s.push_str("<p class=\"empty\">Рейтинговых турниров нет.</p></section>");
        return s;
    }
    s.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th>Дата</th><th>Турнир</th><th class=\"num\">Прогноз</th><th class=\"num\">Место</th>\
         <th class=\"num\">Было</th><th class=\"num\">Стало</th><th class=\"num\">Δ</th>\
         <th class=\"num\" title=\"Доля наигранного состава, вышедшая на этот турнир\">c</th>\
         </tr></thead><tbody>",
    );
    for i in idx {
        let ev = &evs[i];
        let date = ev_date(ev, mets);
        let name = tours.get(&ev.tid).map(|t| t.name.clone()).unwrap_or_default();
        let before = d.a_team + d.b_team * ev.mu_before;
        let after = d.a_team + d.b_team * ev.mu_after;
        let delta = after - before;
        let beat = match (ev.forecast_place, ev.actual_place) {
            (Some(f), Some(act)) if act < f - 0.5 => " place-better",
            (Some(f), Some(act)) if act > f + 0.5 => " place-worse",
            _ => "",
        };
        let _ = write!(
            s,
            "<tr><td class=\"nowrap\">{}</td><td><a href=\"/tournaments/{}/\">{}</a>{}</td>\
             <td class=\"num muted\">{}</td><td class=\"num{beat}\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num {}\">{}</td><td class=\"num muted\">{:.2}</td></tr>",
            e(&date),
            ev.tid,
            e(if name.is_empty() { "без названия" } else { &name }),
            unrated_tag(ev),
            place(ev.forecast_place),
            place(ev.actual_place),
            disp(before),
            disp(after),
            delta_class(delta),
            signed(delta),
            ev.c
        );
    }
    s.push_str("</tbody></table></div></section>");
    s
}

fn player_page(
    p: &PlayerRow,
    hist: &[PlayerPoint],
    ros: &Rosters,
    mets: &HashMap<i64, Metrics>,
    tours: &HashMap<i64, Tournament>,
    ticks: &dyn Fn(u16, u16) -> Vec<(f64, String)>,
) -> String {
    let mut s = String::with_capacity(8192);
    let _ = write!(
        s,
        "<header class=\"page-head\"><h1>{}{}</h1></header>",
        e(&p.name),
        if p.provisional { prov_badge() } else { "" }
    );
    s.push_str("<dl class=\"stats\">");
    s.push_str(&stat("Рейтинг", &e(&disp(p.display))));
    s.push_str(&stat("Неопределённость", &format!("±{}", e(&disp(p.sigma)))));
    s.push_str(&stat("Игр за 365 дней", &p.games.to_string()));
    s.push_str(&stat(
        "Изменение за неделю",
        &p.change.map(|c| signed(c)).unwrap_or_else(|| "—".into()),
    ));
    s.push_str("</dl>");

    let pts = thin(hist, CHART_POINTS);
    let series: Vec<(f64, f64, f64)> = pts
        .iter()
        .map(|q| (q.rel as f64, q.display as f64, q.sigma as f64))
        .collect();
    if let (Some(f), Some(l)) = (pts.first(), pts.last()) {
        s.push_str("<section class=\"section\"><h2 class=\"subhead\">История рейтинга</h2>");
        s.push_str(&chart(&series, &ticks(f.rel, l.rel), "История рейтинга игрока"));
        s.push_str("</section>");
    }

    let games = ros.player_games.get(&p.id).cloned().unwrap_or_default();
    let _ = write!(
        s,
        "<section class=\"section\"><h2 class=\"subhead\">Сыгранные турниры ({})</h2>",
        games.len()
    );
    if games.is_empty() {
        s.push_str("<p class=\"empty\">Рейтинговых турниров нет.</p></section>");
        return s;
    }
    s.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th>Дата</th><th>Турнир</th><th>Команда</th></tr></thead><tbody>",
    );
    for (tid, team_id) in games.iter().rev() {
        let date = mets.get(tid).map(|m| m.date.clone()).unwrap_or_default();
        let name = tours.get(tid).map(|t| t.name.clone()).unwrap_or_default();
        let team = ros
            .team_names
            .get(team_id)
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| format!("Команда {team_id}"));
        let _ = write!(
            s,
            "<tr><td class=\"nowrap\">{}</td><td><a href=\"/tournaments/{tid}/\">{}</a></td>\
             <td><a href=\"/teams/{team_id}/\">{}</a></td></tr>",
            e(&date),
            e(if name.is_empty() { "без названия" } else { &name }),
            e(&team)
        );
    }
    s.push_str("</tbody></table></div></section>");
    s
}

fn tournament_page(
    tid: i64,
    m: &Metrics,
    t: Option<&Tournament>,
    evs: &[Event],
    by_tour: &HashMap<i64, Vec<usize>>,
    ros: &Rosters,
    qb: &HashMap<i64, Vec<(i64, f64)>>,
    d: &Display,
) -> String {
    let mut s = String::with_capacity(16384);
    let name = t.map(|x| x.name.as_str()).unwrap_or("");
    let _ = write!(
        s,
        "<header class=\"page-head\"><h1>{}</h1><p class=\"muted\">{} · {} · вес {:.2}</p></header>",
        e(if name.is_empty() { "Турнир без названия" } else { name }),
        e(&m.date),
        e(tour_type(&m.ttype)),
        m.weight
    );
    s.push_str("<dl class=\"stats\">");
    s.push_str(&stat("Команд", &m.n_teams.to_string()));
    s.push_str(&stat(
        "Вопросов",
        &if m.n_q > 0 { m.n_q.to_string() } else { "—".into() },
    ));
    if let Some(dl) = m.truedl {
        s.push_str(&stat("Сложность", &format!("{dl:.1}")));
    }
    if m.ll_cf.is_some() || m.ll_in.is_some() {
        s.push_str(&stat("Log-loss (кросс-фит)", &e(&opt(m.ll_cf, 3))));
        s.push_str(&stat("Log-loss (по выборке)", &e(&opt(m.ll_in, 3))));
    }
    s.push_str(&stat("ρ прогноза", &e(&opt(m.sp_forecast, 3))));
    s.push_str(&stat("ρ старой системы", &e(&opt(m.sp_old, 3))));
    s.push_str("</dl>");
    s.push_str(
        "<p class=\"hint\">ρ — ранговая корреляция Спирмена между предсказанными и занятыми \
         местами; чем ближе к единице, тем точнее прогноз.",
    );
    if m.n_obs > 0 {
        let _ = write!(s, " Log-loss измерен на {} ответах.", m.n_obs);
    } else {
        s.push_str(" Ответы по вопросам этого турнира не разобраны, поэтому log-loss не считался.");
    }
    s.push_str("</p>");

    if let Some(qs) = qb.get(&tid) {
        // A pack whose difficulties never moved off the prior is every bar the
        // same height: a strip that says nothing. Say so instead of drawing it.
        let lo = qs.iter().map(|q| q.1).fold(f64::MAX, f64::min);
        let hi = qs.iter().map(|q| q.1).fold(f64::MIN, f64::max);
        if !qs.is_empty() && hi - lo > 1e-3 {
            let span = hi - lo;
            s.push_str("<section class=\"section\"><h2 class=\"subhead\">Трудность вопросов</h2>");
            s.push_str("<div class=\"qbars\">");
            for (q, b) in qs {
                let h = 12.0 + 88.0 * (b - lo) / span;
                let _ = write!(
                    s,
                    "<i style=\"height:{h:.0}%\" title=\"Вопрос {q}: b = {b:.2}\"></i>"
                );
            }
            s.push_str("</div>");
            let _ = write!(
                s,
                "<p class=\"hint\">Столбик — трудность вопроса b: выше значит сложнее. \
                 От {} до {} по этому пакету.</p>",
                e(&format!("{lo:.2}")),
                e(&format!("{hi:.2}"))
            );
            s.push_str("</section>");
        }
    }

    let mut idx = by_tour.get(&tid).cloned().unwrap_or_default();
    idx.sort_by(|x, y| {
        let (a, b) = (&evs[*x], &evs[*y]);
        a.actual_place
            .unwrap_or(f64::MAX)
            .partial_cmp(&b.actual_place.unwrap_or(f64::MAX))
            .unwrap()
    });
    let scores: HashMap<i64, Option<i64>> = ros
        .by_tournament
        .get(&tid)
        .map(|v| v.iter().map(|r| (r.team_id, r.score)).collect())
        .unwrap_or_default();

    s.push_str("<section class=\"section\"><h2 class=\"subhead\">Итоги</h2>");
    let unrated = idx.iter().filter(|i| !evs[**i].rated).count();
    if unrated > 0 {
        let _ = write!(
            s,
            "<p class=\"hint\">Не зачтено результатов: {unrated} из {}. Такая команда получает \
             прогноз, но её рейтинг турнир не двигает — за старым номером вышел состав, \
             ничего этой команде не наигравший.</p>",
            idx.len()
        );
    }
    if idx.is_empty() {
        s.push_str("<p class=\"empty\">Данных по командам нет.</p></section>");
        return s;
    }
    s.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">Место</th><th class=\"num\">Прогноз</th><th>Команда</th>\
         <th class=\"num\">Взято</th><th class=\"num\">Было</th><th class=\"num\">Стало</th>\
         <th class=\"num\">Δ</th></tr></thead><tbody>",
    );
    for i in idx {
        let ev = &evs[i];
        let team = ros
            .team_names
            .get(&ev.team_id)
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| format!("Команда {}", ev.team_id));
        let before = d.a_team + d.b_team * ev.mu_before;
        let after = d.a_team + d.b_team * ev.mu_after;
        let delta = after - before;
        let beat = match (ev.forecast_place, ev.actual_place) {
            (Some(f), Some(act)) if act < f - 0.5 => " place-better",
            (Some(f), Some(act)) if act > f + 0.5 => " place-worse",
            _ => "",
        };
        let _ = write!(
            s,
            "<tr><td class=\"num{beat}\">{}</td><td class=\"num muted\">{}</td>\
             <td><a href=\"/teams/{}/\">{}</a>{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num {}\">{}</td></tr>",
            place(ev.actual_place),
            place(ev.forecast_place),
            ev.team_id,
            e(&team),
            unrated_tag(ev),
            scores.get(&ev.team_id).copied().flatten().map(|x| x.to_string()).unwrap_or("—".into()),
            disp(before),
            disp(after),
            delta_class(delta),
            signed(delta)
        );
    }
    s.push_str("</tbody></table></div></section>");
    s
}

fn releases_page(rels: &[Release], teams: &HashMap<i64, i64>) -> String {
    let mut s = String::with_capacity(rels.len() * 200 + 512);
    let _ = write!(
        s,
        "{}",
        head(
            "Релизы",
            &format!(
                "Релиз — недельный снимок состояния, по четвергам. Опубликованное число никогда \
                 не переписывается. Сдвиг — калибровка шкалы к нулевому среднему на этом релизе, \
                 накопленный сдвиг показывает, насколько шкала ушла с начала счёта. \
                 Всего релизов: {}.",
                rels.len()
            )
        )
    );
    s.push_str(
        "<section class=\"section\"><div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">№</th><th>Дата</th><th class=\"num\">Игроков</th>\
         <th class=\"num\">Команд</th><th class=\"num\">Сдвиг</th>\
         <th class=\"num\">Накопленный сдвиг</th></tr></thead><tbody>",
    );
    for r in rels.iter().rev() {
        let _ = write!(
            s,
            "<tr><td class=\"num muted\">{}</td><td class=\"nowrap\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num muted\">{:+.4}</td><td class=\"num muted\">{:+.4}</td></tr>",
            r.id,
            e(&r.date),
            r.n_players,
            teams.get(&r.id).copied().unwrap_or(0),
            r.shift,
            r.shift_cum
        );
    }
    s.push_str("</tbody></table></div></section>");
    s
}
