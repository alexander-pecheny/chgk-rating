use crate::data::Display;
use crate::html::*;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::fmt::Write as _;

pub const PAGE: usize = 100;
const CHART_POINTS: usize = 200;

pub struct Rel {
    pub id: i64,
    pub date: String,
}

/// Everything read once at start-up and shared by every worker.
pub struct Site {
    pub d: Display,
    pub rels: Vec<Rel>,
    pub rel_idx: HashMap<i64, usize>,
    pub release_date: String,
    pub n_teams: usize,
    pub n_players: usize,
    pub n_anomaly: usize,
    /// Stamped onto the stylesheet and script URLs. Assets are served with an
    /// hour of cache, so without it a deploy leaves readers on the old CSS.
    pub assets_version: String,
}

impl Site {
    pub fn load(c: &Connection) -> Site {
        let d = Display {
            a_team: meta(c, "a_team"),
            b_team: meta(c, "b_team"),
            a_player: meta(c, "a_player"),
            b_player: meta(c, "b_player"),
            provisional_sigma: 0.3,
            s2: meta(c, "s2") != 0.0,
        };
        let mut st = c.prepare("select id, date from x.release_ order by date, id").unwrap();
        let rels: Vec<Rel> = st
            .query_map([], |r| Ok(Rel { id: r.get(0)?, date: r.get(1)? }))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let rel_idx = rels.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        let s: String = c
            .query_row("select value from x.meta where key = 'release_date'", [], |r| r.get(0))
            .unwrap_or_default();
        Site {
            d,
            rels,
            rel_idx,
            release_date: s,
            n_teams: meta(c, "n_team_rank") as usize,
            n_players: meta(c, "n_player_rank") as usize,
            n_anomaly: c
                .query_row("select count(*) from x.anomaly", [], |r| r.get::<_, i64>(0))
                .unwrap_or(0) as usize,
            assets_version: String::new(),
        }
    }

    /// Four dated ticks across the span a chart covers.
    fn ticks(&self, from: usize, to: usize) -> Vec<(f64, String)> {
        let mut v = Vec::new();
        for k in 0..4 {
            let i = from as f64 + (to as f64 - from as f64) * k as f64 / 3.0;
            v.push((i, self.rels[i.round() as usize].date[..7].to_string()));
        }
        v.dedup_by(|a, b| a.1 == b.1);
        v
    }
}

fn meta(c: &Connection, key: &str) -> f64 {
    c.query_row("select value from x.meta where key = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .ok()
    .and_then(|s| s.parse().ok())
    .unwrap_or(0.0)
}

// ── chrome ───────────────────────────────────────────────────────────────────

pub fn layout(s: &Site, title: &str, body: &str) -> String {
    let v = &s.assets_version;
    format!(
        r##"<!DOCTYPE html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{}</title>
<link rel="preload" href="/fonts/noto-sans-var.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="/core.css?v={v}">
<link rel="stylesheet" href="/site.css?v={v}">
<script>if(matchMedia("(prefers-color-scheme: dark)").matches)document.documentElement.dataset.theme="dark";</script>
</head>
<body>
<header class="host-top">
  <div class="host-brand"><a class="site-brand" href="/">Рейтинг ЧГК</a></div>
  <nav class="site-nav">
    <a href="/">Команды</a>
    <a href="/players/">Игроки</a>
    <a href="/tournaments/">Турниры</a>
    <a href="/releases/">Релизы</a>
    <a href="/method/">Методика</a>
  </nav>
  <form class="site-search" action="/search/" method="get" role="search">
    {icon}
    <input type="search" name="q" placeholder="Игрок или команда" aria-label="Поиск" autocomplete="off">
  </form>
</header>
<main class="wrap">
{body}
</main>
<footer class="site-foot">
  <span>Экспериментальный рейтинг спортивного ЧГК. Данные — зеркало rating.chgk.info.</span>
  <a href="/report.pdf">Отчёт (PDF)</a>
</footer>
<script src="/site.js?v={v}"></script>
</body>
</html>
"##,
        e(title),
        icon = SEARCH_ICON,
    )
}

/// lucide `search`, as `dopeuikit` renders one: inline, currentColor, .ico.
const SEARCH_ICON: &str = "<svg class=\"ico\" viewBox=\"0 0 24 24\" fill=\"none\" \
     stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" \
     stroke-linejoin=\"round\" aria-hidden=\"true\">\
     <path d=\"m21 21-4.34-4.34\"/><circle cx=\"11\" cy=\"11\" r=\"8\"/></svg>";

fn head(title: &str, lede: &str) -> String {
    format!(
        "<header class=\"page-head\"><h1>{}</h1><p class=\"lede\">{lede}</p></header>",
        e(title)
    )
}

fn stat(label: &str, value: &str) -> String {
    format!("<div class=\"stat\"><dt>{}</dt><dd>{}</dd></div>", e(label), value)
}

fn prov_badge() -> &'static str {
    "<span class=\"prov\" title=\"Рейтинг предварительный: неопределённость велика\">предв.</span>"
}

fn unrated_tag(rated: bool) -> &'static str {
    if rated {
        ""
    } else {
        "<span class=\"unrated\" title=\"Результат не учтён в рейтинге\">вне рейтинга</span>"
    }
}

fn change_cell(v: Option<f64>) -> String {
    match v.map(f64::round) {
        None => "<td class=\"num muted\">—</td>".into(),
        Some(x) if x > 0.0 => format!("<td class=\"num delta-up\">{}</td>", signed(x)),
        Some(x) if x < 0.0 => format!("<td class=\"num delta-down\">{}</td>", signed(x)),
        Some(_) => "<td class=\"num muted\">0</td>".into(),
    }
}

/// Three cells: rating before, after and the step, or dashes when there is no rating.
fn step_cells(a: f64, b: f64, before: Option<f64>, after: Option<f64>) -> String {
    match (before, after) {
        (Some(x), Some(y)) => {
            let (x, y) = (a + b * x, a + b * y);
            format!(
                "<td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num {}\">{}</td>",
                disp(x),
                disp(y),
                delta_class(y - x),
                signed(y - x)
            )
        }
        _ => "<td class=\"num\">—</td><td class=\"num\">—</td><td class=\"num\">—</td>".into(),
    }
}

fn delta_class(v: f64) -> &'static str {
    match v.round() {
        x if x > 0.0 => "delta-up",
        x if x < 0.0 => "delta-down",
        _ => "muted",
    }
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

/// The provisional switch. Unchecked by default, so a reader with no JS sees
/// only settled ratings; the choice is remembered per browser.
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

/// A signed percentage-point figure, to one decimal.
fn signed1(v: f64) -> String {
    if v < 0.0 {
        format!("−{:.1}", -v)
    } else {
        format!("+{v:.1}")
    }
}

/// A count inside running text reads better in groups of three.
fn spaced(n: usize) -> String {
    let d = n.to_string();
    let mut out = String::with_capacity(d.len() + d.len() / 3);
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            out.push('\u{2009}');
        }
        out.push(c);
    }
    out
}

fn pages_of(n: usize) -> usize {
    ((n + PAGE - 1) / PAGE).max(1)
}

// ── the two ranking tables ───────────────────────────────────────────────────

pub fn team_table(s: &Site, c: &Connection, page: usize) -> String {
    let pages = pages_of(s.n_teams);
    let page = page.clamp(1, pages);
    let firm: usize = c
        .query_row("select count(*) from x.team_rank where provisional = 0", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_or(0) as usize;

    let mut st = c
        .prepare(
            "select k.rank, k.team_id, t.name, t.town, k.display, k.sigma, k.evidence, k.change, \
             k.provisional from x.team_rank k left join x.team t on t.id = k.team_id \
             where k.rank > ?1 and k.rank <= ?2 order by k.rank",
        )
        .unwrap();
    let lo = ((page - 1) * PAGE) as i64;
    let rows: Vec<(i64, i64, String, String, f64, f64, f64, Option<f64>, bool)> = st
        .query_map(params![lo, lo + PAGE as i64], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                r.get(4)?,
                r.get(5)?,
                r.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                r.get(7)?,
                r.get::<_, i64>(8)? != 0,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    let mut out = String::with_capacity(rows.len() * 320 + 1024);
    out.push_str(&head(
        "Команды",
        &format!(
            "Релиз от {}. В таблице {} команд. Рейтинг команды — сумма семи её лучших результатов, \
             делённая на семь: чем сильнее сыграно, чем выше место и сильнее поле, тем дороже \
             результат, и со временем он дешевеет. В таблице команды с заявленной базой и хотя бы \
             одним зачтённым результатом за последний год. <a href=\"/method/\">Как это считается</a>.",
            e(&s.release_date),
            s.n_teams,
        ),
    ));
    out.push_str("<section class=\"section\">");
    // The switch only matters when some team is provisional.
    if firm < s.n_teams {
        out.push_str(&prov_toggle(
            rows.iter().filter(|r| r.8).count(),
            Some(firm / PAGE + 1),
            page,
        ));
    }
    let url = |i: usize| if i == 1 { "/".to_string() } else { format!("/page/{i}/") };
    out.push_str(&pager(page, pages, &url));
    let _ = write!(
        out,
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">Место</th><th>Команда</th><th class=\"town\">Город</th>\
         <th class=\"num\">Рейтинг</th>{}\
         <th class=\"num\">Изменение</th></tr></thead><tbody>",
        if s.d.s2 {
            "<th class=\"num\" title=\"Сколько результатов из семи входит в рейтинг\">Результатов</th>"
        } else {
            "<th class=\"num\" title=\"Неопределённость оценки\">±</th>"
        }
    );
    let mut split = lo as usize >= firm;
    for r in &rows {
        if r.8 && !split {
            split = true;
            out.push_str(
                "<tr class=\"row-split prov-row\"><td colspan=\"6\">Предварительные рейтинги — \
                 команды, чья неопределённость ещё велика</td></tr>",
            );
        }
        let name = if r.2.is_empty() { format!("Команда {}", r.1) } else { r.2.clone() };
        let _ = write!(
            out,
            "<tr{}><td class=\"num\">{}</td><td><a href=\"/teams/{}/\">{}</a>{}\
             <span class=\"town-sub\">{}</span></td>\
             <td class=\"town\">{}</td><td class=\"num strong\">{}</td><td class=\"num muted\">{}</td>{}</tr>",
            if r.8 { " class=\"prov-row\"" } else { "" },
            if r.8 { "—".into() } else { r.0.to_string() },
            r.1,
            e(&name),
            if r.8 { prov_badge() } else { "" },
            e(&r.3),
            e(&r.3),
            disp(r.4),
            if s.d.s2 { format!("{:.0}", r.6) } else { disp(r.5) },
            change_cell(r.7)
        );
    }
    out.push_str("</tbody></table></div>");
    out.push_str(&pager(page, pages, &url));
    out.push_str("</section>");
    out
}

pub fn player_table(s: &Site, c: &Connection, page: usize) -> String {
    let pages = pages_of(s.n_players);
    let page = page.clamp(1, pages);
    let firm: usize = c
        .query_row("select count(*) from x.player_rank where provisional = 0", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap_or(0) as usize;

    let mut st = c
        .prepare(
            "select k.rank, k.player_id, p.name, k.display, k.sigma, k.games, k.change, \
             k.provisional from x.player_rank k left join x.player p on p.id = k.player_id \
             where k.rank > ?1 and k.rank <= ?2 order by k.rank",
        )
        .unwrap();
    let lo = ((page - 1) * PAGE) as i64;
    let rows: Vec<(i64, i64, String, f64, f64, i64, Option<f64>, bool)> = st
        .query_map(params![lo, lo + PAGE as i64], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get(3)?,
                r.get(4)?,
                r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                r.get(6)?,
                r.get::<_, i64>(7)? != 0,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    let mut out = String::with_capacity(rows.len() * 300 + 1024);
    out.push_str(&head(
        "Игроки",
        &format!(
            "Релиз от {}. В таблице {} игроков, из них {} с установившимся рейтингом. \
             Индивидуальный рейтинг — оценка из модели вопросов, без лестничных правил.",
            e(&s.release_date),
            s.n_players,
            firm
        ),
    ));
    out.push_str("<section class=\"section\">");
    out.push_str(&prov_toggle(
        rows.iter().filter(|r| r.7).count(),
        (firm < s.n_players).then(|| firm / PAGE + 1),
        page,
    ));
    let url = |i: usize| if i == 1 { "/players/".to_string() } else { format!("/players/page/{i}/") };
    out.push_str(&pager(page, pages, &url));
    out.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">Место</th><th>Игрок</th><th class=\"num\">Рейтинг</th>\
         <th class=\"num\" title=\"Неопределённость оценки\">±</th><th class=\"num\">Игр за год</th>\
         <th class=\"num\">Изменение</th></tr></thead><tbody>",
    );
    let mut split = lo as usize >= firm;
    for r in &rows {
        if r.7 && !split {
            split = true;
            out.push_str(
                "<tr class=\"row-split prov-row\"><td colspan=\"6\">Предварительные рейтинги — \
                 игроки, чья неопределённость ещё велика</td></tr>",
            );
        }
        let name = if r.2.is_empty() { format!("Игрок {}", r.1) } else { r.2.clone() };
        let _ = write!(
            out,
            "<tr{}><td class=\"num\">{}</td><td><a href=\"/players/{}/\">{}</a>{}</td>\
             <td class=\"num strong\">{}</td><td class=\"num muted\">{}</td>\
             <td class=\"num\">{}</td>{}</tr>",
            if r.7 { " class=\"prov-row\"" } else { "" },
            if r.7 { "—".into() } else { r.0.to_string() },
            r.1,
            e(&name),
            if r.7 { prov_badge() } else { "" },
            disp(r.3),
            disp(r.4),
            r.5,
            change_cell(r.6)
        );
    }
    out.push_str("</tbody></table></div>");
    out.push_str(&pager(page, pages, &url));
    out.push_str("</section>");
    out
}

pub fn tournament_table(c: &Connection, page: usize) -> String {
    let n: usize = c
        .query_row("select count(*) from x.tournament", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0) as usize;
    let pages = pages_of(n);
    let page = page.clamp(1, pages);
    let mut st = c
        .prepare(
            "select id, name, date, type, weight, n_teams, n_q, truedl, ll_cf from x.tournament \
             order by date desc, id desc limit ?1 offset ?2",
        )
        .unwrap();
    let rows: Vec<(i64, String, String, String, f64, i64, i64, Option<f64>, Option<f64>)> = st
        .query_map(params![PAGE as i64, ((page - 1) * PAGE) as i64], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                r.get(7)?,
                r.get(8)?,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    let mut out = String::with_capacity(rows.len() * 260 + 512);
    out.push_str(&head(
        "Турниры",
        &format!("Рейтинговые турниры с 2013 года — {n} штук, от новых к старым."),
    ));
    out.push_str("<section class=\"section\">");
    let url = |i: usize| {
        if i == 1 { "/tournaments/".to_string() } else { format!("/tournaments/page/{i}/") }
    };
    out.push_str(&pager(page, pages, &url));
    out.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th>Дата</th><th>Турнир</th><th>Тип</th><th class=\"num\">Вес</th>\
         <th class=\"num\">Команд</th><th class=\"num\">Вопросов</th>\
         <th class=\"num\" title=\"Сложность пакета\">Сложность</th>\
         <th class=\"num\">Log-loss</th></tr></thead><tbody>",
    );
    for r in &rows {
        let _ = write!(
            out,
            "<tr><td class=\"nowrap\">{}</td><td><a href=\"/tournaments/{}/\">{}</a></td>\
             <td>{}</td><td class=\"num\">{:.2}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            e(&r.2),
            r.0,
            e(if r.1.is_empty() { "без названия" } else { &r.1 }),
            e(tour_type(&r.3)),
            r.4,
            r.5,
            r.6,
            blank(r.7, 1),
            opt(r.8, 3)
        );
    }
    out.push_str("</tbody></table></div>");
    out.push_str(&pager(page, pages, &url));
    out.push_str("</section>");
    out
}

// ── entity pages ─────────────────────────────────────────────────────────────

/// The rating history of one entity: the release it belongs to, the published
/// value and its uncertainty, in release order.
fn history(s: &Site, c: &Connection, sql: &str, id: i64, a: f64, b: f64) -> Vec<(usize, f64, f64)> {
    let mut st = c.prepare(sql).unwrap();
    let mut v: Vec<(usize, f64, f64)> = st
        .query_map([id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
            ))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .filter_map(|(rel, value, sigma)| {
            Some((*s.rel_idx.get(&rel)?, a + b * value, b.abs() * sigma))
        })
        .collect();
    v.sort_by_key(|p| p.0);
    v
}

/// Where the entity stood in the published table at each of those releases,
/// stored as one blob per entity in the same order.
fn places(c: &Connection, kind: i64, id: i64) -> Vec<u32> {
    let blob: Option<Vec<u8>> = c
        .query_row(
            "select places from x.place_hist where kind = ?1 and id = ?2",
            params![kind, id],
            |r| r.get(0),
        )
        .ok();
    blob.map(|b| b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
        .unwrap_or_default()
}

/// The rating history and, beside it, the same weeks as a place in the table.
fn chart_section(
    s: &Site,
    c: &Connection,
    kind: i64,
    id: i64,
    sql: &str,
    a: f64,
    b: f64,
    band: bool,
) -> String {
    let hist = history(s, c, sql, id, a, b);
    if hist.len() < 2 {
        return String::new();
    }
    let pl = places(c, kind, id);
    let date = |i: usize| s.rels[i].date.clone();

    let rating: Vec<Pt> = hist
        .iter()
        .map(|&(rel, mid, sigma)| Pt {
            x: rel as f64,
            mid,
            sigma,
            date: date(rel),
            text: if band {
                format!("{} ± {}", disp(mid), disp(sigma))
            } else {
                disp(mid)
            },
        })
        .collect();
    // Lengths differ only if the index was built against another ratings file;
    // taking the shorter of the two keeps the weeks lined up either way.
    let place_pts: Vec<Pt> = hist
        .iter()
        .zip(pl.iter())
        .map(|(&(rel, _, _), &n)| Pt {
            x: rel as f64,
            mid: n as f64,
            sigma: 0.0,
            date: date(rel),
            text: format!("№{n}"),
        })
        .collect();

    let (from, to) = (rating[0].x as usize, rating[rating.len() - 1].x as usize);
    let series = vec![
        Series {
            name: "Рейтинг".into(),
            pts: thin(&rating, CHART_POINTS),
            invert: false,
            band,
        },
        Series {
            name: "Место".into(),
            pts: thin(&place_pts, CHART_POINTS),
            invert: true,
            band: false,
        },
    ];
    format!(
        "<section class=\"section\"><h2 class=\"subhead\">История рейтинга</h2>{}</section>",
        chart(&series, &s.ticks(from, to), "История рейтинга")
    )
}

pub fn player_page(s: &Site, c: &Connection, id: i64) -> Option<String> {
    let (name, games, last): (String, i64, String) = c
        .query_row("select name, games, last_date from x.player where id = ?1", [id], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ))
        })
        .ok()?;
    let name = if name.is_empty() { format!("Игрок {id}") } else { name };

    // The last release this player was listed in, which for anyone still active
    // is the current one and for everyone else is where their history stops.
    let now: Option<(f64, f64, i64)> = c
        .query_row(
            "select display, sigma, games_365 from player_ratings where player_id = ?1 \
             order by release_id desc limit 1",
            [id],
            |r| {
                Ok((
                    r.get::<_, Option<f64>>(0)?.unwrap_or(0.0),
                    r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                    r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                ))
            },
        )
        .ok();
    let rank: Option<(i64, Option<f64>)> = c
        .query_row("select rank, change from x.player_rank where player_id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .ok();
    let provisional = now.map(|n| n.1 > s.d.provisional_sigma).unwrap_or(true);
    let listed = rank.is_some();

    let mut out = String::with_capacity(8192);
    let _ = write!(
        out,
        "<header class=\"page-head\"><h1>{}{}</h1><p class=\"muted\">{}</p></header>",
        e(&name),
        if listed && provisional { prov_badge() } else { "" },
        if listed {
            format!("Игрок №{}", rank.unwrap().0)
        } else {
            format!("Не в текущем релизе — последняя игра {}", e(&last))
        }
    );
    out.push_str("<dl class=\"stats\">");
    match now {
        Some((display, sigma, g365)) => {
            out.push_str(&stat("Рейтинг", &e(&disp(display))));
            out.push_str(&stat("Неопределённость", &format!("±{}", e(&disp(s.d.b_player * sigma)))));
            out.push_str(&stat("Игр за 365 дней", &g365.to_string()));
        }
        None => out.push_str(&stat("Рейтинг", "—")),
    }
    out.push_str(&stat("Всего игр", &games.to_string()));
    out.push_str(&stat(
        "Изменение за неделю",
        &rank.and_then(|r| r.1).map(signed).unwrap_or_else(|| "—".into()),
    ));
    out.push_str("</dl>");

    out.push_str(&chart_section(
        s,
        c,
        0,
        id,
        "select release_id, theta, sigma from player_ratings where player_id = ?1",
        s.d.a_player,
        s.d.b_player,
        true,
    ));

    // Before and after at an in-person table, theta + phi, like the release
    // number and the tournament roster.
    let mut st = c
        .prepare(
            "select g.tournament_id, t.name, t.date, t.type, g.team_id, m.name, g.position, \
             g.score, t.n_q, v.theta_before + coalesce(v.phi_before, 0), \
             v.theta_after + coalesce(v.phi_before, 0), te.rated \
             from x.player_game g join x.tournament t on t.id = g.tournament_id \
             left join x.team m on m.id = g.team_id \
             left join player_events v on v.player_id = g.player_id and v.tournament_id = g.tournament_id \
             left join team_events te on te.tournament_id = g.tournament_id and te.team_id = g.team_id \
             where g.player_id = ?1 order by t.date desc, t.id desc",
        )
        .unwrap();
    type Game = (i64, String, String, String, i64, String, Option<f64>, Option<i64>, i64, Option<f64>, Option<f64>, bool);
    let games: Vec<Game> = st
        .query_map([id], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                r.get(4)?,
                r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                r.get(6)?,
                r.get(7)?,
                r.get::<_, Option<i64>>(8)?.unwrap_or(0),
                r.get(9)?,
                r.get(10)?,
                r.get::<_, Option<i64>>(11)?.unwrap_or(1) != 0,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    let _ = write!(
        out,
        "<section class=\"section\"><h2 class=\"subhead\">Сыгранные турниры ({})</h2>",
        games.len()
    );
    if games.is_empty() {
        out.push_str("<p class=\"empty\">Рейтинговых турниров нет.</p></section>");
        return Some(out);
    }
    out.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th>Дата</th><th>Турнир</th><th>Тип</th><th>Команда</th>\
         <th class=\"num\">Место</th><th class=\"num\">Взято</th>\
         <th class=\"num\">Было</th><th class=\"num\">Стало</th><th class=\"num\">Δ</th></tr></thead><tbody>",
    );
    for g in &games {
        let _ = write!(
            out,
            "<tr><td class=\"nowrap\">{}</td><td><a href=\"/tournaments/{}/\">{}</a>{}</td>\
             <td class=\"muted\">{}</td><td><a href=\"/teams/{}/\">{}</a></td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td>{}</tr>",
            e(&g.2),
            g.0,
            e(if g.1.is_empty() { "без названия" } else { &g.1 }),
            unrated_tag(g.11),
            e(tour_type(&g.3)),
            g.4,
            e(if g.5.is_empty() { "—" } else { &g.5 }),
            place(g.6),
            g.7.map(|x| if g.8 > 0 { format!("{x}/{}", g.8) } else { x.to_string() })
                .unwrap_or_else(|| "—".into()),
            step_cells(s.d.a_player, s.d.b_player, g.9, g.10),
        );
    }
    out.push_str("</tbody></table></div></section>");
    Some(out)
}

pub fn team_page(s: &Site, c: &Connection, id: i64) -> Option<String> {
    let (name, town): (String, String) = c
        .query_row("select name, town from x.team where id = ?1", [id], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        })
        .ok()?;
    let name = if name.is_empty() { format!("Команда {id}") } else { name };

    let now: Option<(f64, f64, f64, f64)> = c
        .query_row(
            "select display, sigma, evidence, core_strength from team_ratings \
             where team_id = ?1 order by release_id desc limit 1",
            [id],
            |r| {
                Ok((
                    r.get::<_, Option<f64>>(0)?.unwrap_or(0.0),
                    r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                    r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                    r.get::<_, Option<f64>>(3)?.unwrap_or(f64::NAN),
                ))
            },
        )
        .ok();
    let rank: Option<(i64, Option<f64>)> = c
        .query_row("select rank, change from x.team_rank where team_id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .ok();
    let provisional = now.map(|n| n.1 > s.d.provisional_sigma).unwrap_or(true);

    let mut out = String::with_capacity(8192);
    let _ = write!(
        out,
        "<header class=\"page-head\"><h1>{}{}</h1><p class=\"muted\">{}</p></header>",
        e(&name),
        if rank.is_some() && provisional { prov_badge() } else { "" },
        match &rank {
            Some(r) => format!("№{} · {}", r.0, e(if town.is_empty() { "город не указан" } else { &town })),
            None => format!(
                "Не в текущем релизе · {}",
                e(if town.is_empty() { "город не указан" } else { &town })
            ),
        }
    );
    // ── the achievement rating, when the database carries it ─────────────────
    let rules = achieve_rules(c);
    let team_row: Option<(f64, f64, i64)> = c
        .query_row(
            "select rating, divisor, n_results from achieve_team where team_id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();

    out.push_str("<dl class=\"stats\">");
    match now {
        Some((display, _, _, _)) => out.push_str(&stat("Рейтинг", &e(&disp(display)))),
        None => out.push_str(&stat("Рейтинг", "—")),
    }
    out.push_str(&stat(
        "Место",
        &rank.map(|r| format!("№{}", r.0)).unwrap_or_else(|| "—".into()),
    ));
    out.push_str(&stat(
        "За неделю",
        &rank.and_then(|r| r.1).map(signed).unwrap_or_else(|| "—".into()),
    ));
    if let (Some((_, _, n)), Some(x)) = (team_row, rules.get("top_x")) {
        out.push_str(&stat("Результатов в рейтинге", &format!("{n} из {x}")));
    }
    out.push_str("</dl>");

    if let Some((rating, divisor, _)) = team_row {
        out.push_str(&achieve_breakdown(c, id, rating, divisor, &rules));
    }

    out.push_str(&chart_section(
        s,
        c,
        1,
        id,
        "select release_id, mu, sigma from team_ratings where team_id = ?1",
        s.d.a_team,
        s.d.b_team,
        false,
    ));

    // The base roster in force at the last release, strongest first; a player
    // with no current rating goes last.
    let base: Vec<(i64, String, Option<f64>, bool)> = c
        .prepare(
            "select b.player_id, b.name, k.display, coalesce(k.provisional, 0) from x.base_roster b \
             left join x.player_rank k on k.player_id = b.player_id \
             where b.team_id = ?1 order by k.display is null, k.display desc, b.name",
        )
        .and_then(|mut st| {
            st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0)))?
                .collect()
        })
        .unwrap_or_default();
    if !base.is_empty() {
        let _ = write!(
            out,
            "<section class=\"section\"><h2 class=\"subhead\">Базовый состав ({})</h2><ul class=\"roster base-roster\">",
            base.len()
        );
        for (pid, pname, display, prov) in &base {
            let _ = write!(
                out,
                "<li><a href=\"/players/{pid}/\">{}</a>{} <span class=\"num\">{}</span></li>",
                e(pname),
                if *prov { prov_badge() } else { "" },
                display.map(disp).unwrap_or_else(|| "—".into()),
            );
        }
        out.push_str("</ul></section>");
    }

    // ── every result, with what it was worth and whether it counts today ─────
    let have_values = crate::data::table_exists(c, "achieve_values");
    let sql = format!(
        "select v.tournament_id, t.name, coalesce(v.date, t.date) as d, v.actual_place, \
         v.mu_before, v.mu_after, v.rated, t.type, {}, {}, {}, {}, {} \
         from team_events v left join x.tournament t on t.id = v.tournament_id {} \
         where v.team_id = ?1 order by d desc, v.tournament_id desc",
        if have_values { "a.peak" } else { "null" },
        if have_values { "a.n_teams" } else { "null" },
        if have_values { "a.gain" } else { "null" },
        if have_values { "a.strength, a.w, a.place" } else { "null, null, null" },
        if have_values {
            "exists(select 1 from achieve_parts p where p.team_id = v.team_id and p.tournament_id = v.tournament_id)"
        } else {
            "0"
        },
        if have_values {
            "left join achieve_values a on a.tournament_id = v.tournament_id and a.team_id = v.team_id"
        } else {
            ""
        },
    );
    let mut st = c.prepare(&sql).unwrap();
    type Ev = (i64, String, String, Option<f64>, Option<f64>, Option<f64>, bool, String, Option<f64>, Option<i64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>, bool);
    let evs: Vec<Ev> = st
        .query_map([id], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get::<_, Option<i64>>(6)?.unwrap_or(1) != 0,
                r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
                r.get(13)?,
                r.get::<_, Option<i64>>(14)?.unwrap_or(0) != 0,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    let _ = write!(
        out,
        "<section class=\"section\"><h2 class=\"subhead\">Все результаты ({})</h2>",
        evs.len()
    );
    if evs.is_empty() {
        out.push_str("<p class=\"empty\">Рейтинговых турниров нет.</p></section>");
        return Some(out);
    }
    out.push_str(
        "<p class=\"hint\">Ценность — сколько результат стоил в день турнира. Прибавка — насколько \
         вырос рейтинг, когда результат вошёл в семь лучших; результат, не вошедший в них, рейтинг \
         не меняет. Отмеченные ● входят в рейтинг сейчас. Прочерк в ценности — результат вне \
         рейтинга или без данных о взятых вопросах. Как получена ценность, видно при наведении.</p>",
    );
    // The last three years show; older results open with the switch.
    let cutoff = {
        let y: i32 = s.release_date.get(..4).and_then(|x| x.parse().ok()).unwrap_or(0);
        format!("{}{}", y - 3, s.release_date.get(4..).unwrap_or(""))
    };
    let older = evs.iter().filter(|v| v.2 < cutoff).count();
    if older > 0 {
        let _ = write!(
            out,
            "<label class=\"checkbox older-toggle\"><input type=\"checkbox\"> Показать и старые результаты \
             <span class=\"prov-count\">ещё {older}, до {}</span></label>",
            e(&cutoff)
        );
    }
    out.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th>Дата</th><th>Турнир</th><th>Тип</th><th class=\"num\">Место</th>\
         <th class=\"num\" title=\"Сколько результат стоил в день турнира\">Ценность</th>\
         <th class=\"num\" title=\"Входит ли результат в рейтинг сейчас\">В рейтинге</th>\
         <th class=\"num\" title=\"Насколько вырос рейтинг, когда результат вошёл в семь лучших\">Прибавка</th>\
         </tr></thead><tbody>",
    );
    for v in &evs {
        let delta = match v.10 {
            Some(g) if g.round() > 0.0 => format!("<td class=\"num delta-up\">{}</td>", signed(g)),
            _ => "<td class=\"num muted\"></td>".into(),
        };
        let field = v.9.map(|n| format!(" <span class=\"muted\">из {n}</span>")).unwrap_or_default();
        let _ = write!(
            out,
            "<tr{}><td class=\"nowrap\">{}</td><td><a href=\"/tournaments/{}/\">{}</a>{}</td>\
             <td class=\"muted\">{}</td><td class=\"num nowrap\">{}{field}</td><td class=\"num\"{}>{}</td>\
             <td class=\"num\">{}</td>{delta}</tr>",
            match (v.14, v.2 < cutoff) {
                (true, _) => " class=\"counts\"",
                (false, true) => " class=\"older-row\"",
                _ => "",
            },
            e(&v.2),
            v.0,
            e(if v.1.is_empty() { "без названия" } else { &v.1 }),
            unrated_tag(v.6),
            e(tour_type(&v.7)),
            place(v.3),
            match (v.11, v.12, v.13) {
                (Some(st), Some(w), Some(pl)) => format!(
                    " title=\"игра {}{} × вес {}\"",
                    disp(st),
                    if pl != 1.0 { format!(" ÷ место {}^0,6", place(Some(pl))) } else { String::new() },
                    w2(w)
                ),
                _ => String::new(),
            },
            v.8.map(disp).unwrap_or_else(|| "—".into()),
            if v.14 { "<span class=\"dot-in\" title=\"входит в рейтинг\">●</span>" } else { "" },
        );
    }
    out.push_str("</tbody></table></div></section>");
    Some(out)
}

/// The knobs the achievement rating was published with, by name.
fn achieve_rules(c: &Connection) -> HashMap<String, String> {
    if !crate::data::table_exists(c, "achieve_meta") {
        return HashMap::new();
    }
    c.prepare("select key, value from achieve_meta")
        .and_then(|mut st| st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect())
        .unwrap_or_default()
}

/// A number from the rules, formatted for a sentence: 0.1 → "10%".
fn rule_pct(rules: &HashMap<String, String>, key: &str) -> String {
    rules
        .get(key)
        .and_then(|v| v.parse::<f64>().ok())
        .map(|x| format!("{:.0}%", 100.0 * x))
        .unwrap_or_else(|| "?".into())
}

/// The rating taken apart: the results it is the sum of, each with what it was
/// worth on the day, how much of that is left, and what it adds.
fn achieve_breakdown(
    c: &Connection,
    id: i64,
    rating: f64,
    divisor: f64,
    rules: &HashMap<String, String>,
) -> String {
    type Part = (i64, String, String, f64, f64, i64, i64, f64, f64, f64, f64, f64, f64, f64, f64, String);
    let parts: Vec<Part> = c
        .prepare(
            "select p.tournament_id, coalesce(t.name, ''), a.date, a.place, a.venue_place, a.n_teams, \
             a.n_venues, a.strength, a.w_format, a.w_field, a.w_reach, p.peak, p.fade, p.overlap, \
             p.contribution, a.format \
             from achieve_parts p join achieve_values a on a.tournament_id = p.tournament_id and a.team_id = p.team_id \
             left join x.tournament t on t.id = p.tournament_id \
             where p.team_id = ?1 order by p.slot",
        )
        .and_then(|mut st| {
            st.query_map([id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                    r.get(12)?,
                    r.get(13)?,
                    r.get(14)?,
                    r.get(15)?,
                ))
            })?
            .collect()
        })
        .unwrap_or_default();
    let x = rules.get("top_x").cloned().unwrap_or_else(|| format!("{divisor:.0}"));
    let pow = rules.get("place_pow").cloned().unwrap_or_else(|| "0.6".into());
    let mut o = String::new();
    o.push_str("<section class=\"section\"><h2 class=\"subhead\">Из чего складывается рейтинг</h2>");
    let _ = write!(
        o,
        "<p class=\"hint\">Рейтинг — сумма {x} лучших результатов команды, делённая на {x}. \
         Результат стоит столько, насколько сильно команда сыграла, делённое на место<sup>{pow}</sup> и \
         умноженное на вес турнира. Затем ценность убывает: очный результат теряет {} в год, синхронный — {}. \
         Если из того состава в базе осталась только часть игроков, результат стоит меньше. Подробности — \
         в <a href=\"/method/\">методике</a>.</p>",
        rule_pct(rules, "loss_ip"),
        rule_pct(rules, "loss_sync"),
    );
    o.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table breakdown\"><thead><tr>\
         <th>Турнир</th><th class=\"num\">Место</th>\
         <th class=\"num\" title=\"Сколько результат стоил в день турнира\">Ценность</th>\
         <th class=\"num\" title=\"Доля ценности, оставшаяся сегодня\">Осталось</th>\
         <th class=\"num\" title=\"Ценность сегодня: ценность × осталось\">Сейчас</th>\
         </tr></thead><tbody>",
    );
    let mut sum = 0.0;
    for p in &parts {
        let (tid, name, date, place_all, place_own, n, nv, strength, wfmt, wfield, wreach, peak, fade, ov, contrib, fmt) =
            (p.0, &p.1, &p.2, p.3, p.4, p.5, p.6, p.7, p.8, p.9, p.10, p.11, p.12, p.13, p.14, &p.15);
        sum += contrib;
        let place_txt = |x: f64| if x.fract() == 0.0 { format!("{x:.0}") } else { format!("{x:.1}") };
        // How the value on the day was made, in one line under the name.
        let mut how = vec![format!("игра {}", disp(strength))];
        if place_all != 1.0 {
            how.push(format!("÷ место<sup>{pow}</sup>"));
        }
        let w = wfmt * wfield * wreach;
        if (w - 1.0).abs() >= 0.005 {
            let mut why = Vec::new();
            if wfmt < 0.999 {
                why.push(format!("{} ×{}", if fmt == "S" { "строгий синхрон" } else { "синхрон" }, w2(wfmt)));
            }
            if wfield < 0.995 {
                why.push(format!("поле ×{}", w2(wfield)));
            }
            if wreach < 0.995 {
                why.push(format!("охват ×{}", w2(wreach)));
            }
            how.push(format!("× вес {} ({})", w2(w), why.join(", ")));
        }
        if how.len() > 1 {
            how.push(format!("= {}", disp(peak)));
        }
        let left = fade * ov;
        let left_title = if ov < 0.995 {
            format!(
                " title=\"затухание {:.0}% × доля нынешнего состава {:.0}%\"",
                100.0 * fade,
                100.0 * ov
            )
        } else {
            format!(" title=\"затухание: осталось {:.0}%\"", 100.0 * fade)
        };
        let merged = if nv > 1 {
            format!(
                "<span class=\"muted\"> · сводный зачёт {} {}, на своей — {}</span>",
                nv,
                plural(nv, "площадки", "площадки", "площадок"),
                place_txt(place_own)
            )
        } else {
            String::new()
        };
        let _ = write!(
            o,
            "<tr><td><a href=\"/tournaments/{tid}/\">{}</a> <span class=\"muted nowrap\">{}</span>{merged}\
             <div class=\"how\">{}</div></td>\
             <td class=\"num nowrap\">{} <span class=\"muted\">из {n}</span></td>\
             <td class=\"num\">{}</td><td class=\"num muted\"{left_title}>{:.0}%</td>\
             <td class=\"num strong\">{}</td></tr>",
            e(if name.is_empty() { "без названия" } else { name }),
            e(date),
            how.join(" "),
            place_txt(place_all),
            disp(peak),
            100.0 * left,
            disp(contrib * divisor),
        );
    }
    let slots: usize = x.parse().unwrap_or(parts.len());
    for _ in parts.len()..slots {
        o.push_str(
            "<tr class=\"empty-slot\"><td class=\"muted\">пустой слот</td><td></td><td></td><td></td>\
             <td class=\"num muted\">0</td></tr>",
        );
    }
    let _ = write!(
        o,
        "</tbody><tfoot><tr><td colspan=\"4\">Сумма {} ÷ {divisor:.0} = рейтинг</td>\
         <td class=\"num strong\">{}</td></tr></tfoot></table></div></section>",
        disp(sum * divisor),
        disp(rating)
    );
    o
}

/// The Russian noun form after a count: 2 площадки, 5 площадок.
fn plural<'a>(n: i64, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
    match (n % 10, n % 100) {
        (1, x) if x != 11 => one,
        (2..=4, x) if !(12..=14).contains(&x) => few,
        _ => many,
    }
}

/// A weight to two places, with a decimal comma.
fn w2(x: f64) -> String {
    format!("{x:.2}").replace('.', ",")
}

pub fn tournament_page(s: &Site, c: &Connection, id: i64) -> Option<String> {
    let (name, date, ttype, weight, n_teams, n_q, n_obs, ll_in, ll_cf, sp_f, sp_o, truedl): (
        String,
        String,
        String,
        f64,
        i64,
        i64,
        i64,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
    ) = c
        .query_row(
            "select name, date, type, weight, n_teams, n_q, n_obs, ll_in, ll_cf, sp_forecast, \
             sp_old, truedl from x.tournament where id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                    r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                ))
            },
        )
        .ok()?;

    let mut out = String::with_capacity(16384);
    let _ = write!(
        out,
        "<header class=\"page-head\"><h1>{}</h1><p class=\"muted\">{} · {} · вес {weight:.2}</p></header>",
        e(if name.is_empty() { "Турнир без названия" } else { &name }),
        e(&date),
        e(tour_type(&ttype))
    );
    out.push_str("<dl class=\"stats\">");
    out.push_str(&stat("Команд", &n_teams.to_string()));
    out.push_str(&stat("Вопросов", &if n_q > 0 { n_q.to_string() } else { "—".into() }));
    if let Some(dl) = truedl {
        out.push_str(&stat("Сложность", &format!("{dl:.1}")));
    }
    if ll_cf.is_some() || ll_in.is_some() {
        out.push_str(&stat("Log-loss (кросс-фит)", &e(&opt(ll_cf, 3))));
        out.push_str(&stat("Log-loss (по выборке)", &e(&opt(ll_in, 3))));
    }
    out.push_str(&stat("ρ прогноза", &e(&opt(sp_f, 3))));
    out.push_str(&stat("ρ старой системы", &e(&opt(sp_o, 3))));
    out.push_str("</dl>");
    out.push_str(
        "<p class=\"hint\">ρ — ранговая корреляция Спирмена между предсказанными и занятыми \
         местами; чем ближе к единице, тем точнее прогноз.",
    );
    if n_obs > 0 {
        let _ = write!(out, " Log-loss измерен на {n_obs} ответах.");
    } else {
        out.push_str(" Ответы по вопросам этого турнира не разобраны, поэтому log-loss не считался.");
    }
    out.push_str("</p>");

    // ── question difficulty ──────────────────────────────────────────────────
    let mut st = c
        .prepare("select q, b from question_params where tournament_id = ?1 order by q")
        .unwrap();
    let qs: Vec<(i64, f64)> = st
        .query_map([id], |r| Ok((r.get(0)?, r.get::<_, Option<f64>>(1)?.unwrap_or(0.0))))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let lo = qs.iter().map(|q| q.1).fold(f64::MAX, f64::min);
    let hi = qs.iter().map(|q| q.1).fold(f64::MIN, f64::max);
    // A pack whose difficulties never moved off the prior is every bar the same
    // height: a strip that says nothing. Say so instead of drawing it.
    if !qs.is_empty() && hi - lo > 1e-3 {
        out.push_str("<section class=\"section\"><h2 class=\"subhead\">Трудность вопросов</h2>");
        out.push_str("<div class=\"qbars\">");
        for (q, b) in &qs {
            let h = 12.0 + 88.0 * (b - lo) / (hi - lo);
            let _ = write!(out, "<i style=\"height:{h:.0}%\" title=\"Вопрос {q}: b = {b:.2}\"></i>");
        }
        out.push_str("</div>");
        let _ = write!(
            out,
            "<p class=\"hint\">Столбик — трудность вопроса b: выше значит сложнее. \
             От {lo:.2} до {hi:.2} по этому пакету.</p></section>"
        );
    }

    // ── the field ────────────────────────────────────────────────────────────
    struct Row {
        team: i64,
        name: String,
        forecast: Option<f64>,
        actual: Option<f64>,
        before: Option<f64>,
        after: Option<f64>,
        rated: bool,
        score: Option<i64>,
        played_as: String,
        lineup: Option<f64>,
        d_place: Option<f64>,
        d_pull: Option<f64>,
        value: Option<f64>,
        gain: Option<f64>,
    }
    let have = crate::data::columns(c, "team_events");
    let have_values = crate::data::table_exists(c, "achieve_values");
    let sql = format!(
        "select v.team_id, m.name, v.forecast_place, v.actual_place, v.mu_before, v.mu_after, \
         v.rated, r.score, r.name, v.s_r, {}, {}, {} \
         from team_events v left join x.team m on m.id = v.team_id \
         left join x.result_ r on r.tournament_id = v.tournament_id and r.team_id = v.team_id {} \
         where v.tournament_id = ?1 order by v.actual_place",
        crate::data::col(&have, "d_place"),
        crate::data::col(&have, "d_pull"),
        if have_values { "a.peak, a.gain" } else { "null, null" },
        if have_values {
            "left join achieve_values a on a.tournament_id = v.tournament_id and a.team_id = v.team_id"
        } else {
            ""
        },
    );
    let mut st = c.prepare(&sql).unwrap();
    let evs: Vec<Row> = st
        .query_map([id], |r| {
            Ok(Row {
                team: r.get(0)?,
                name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                forecast: r.get(2)?,
                actual: r.get(3)?,
                before: r.get(4)?,
                after: r.get(5)?,
                rated: r.get::<_, Option<i64>>(6)?.unwrap_or(1) != 0,
                score: r.get(7)?,
                played_as: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
                lineup: r.get(9)?,
                d_place: r.get(10)?,
                d_pull: r.get(11)?,
                value: r.get(12)?,
                gain: r.get(13)?,
            })
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    // ── the lineups, with each player's number on the morning of the game ──
    struct Member {
        player: i64,
        name: String,
        before: Option<f64>,
        after: Option<f64>,
        provisional: bool,
    }
    let mut st = c
        .prepare(
            "select g.team_id, g.player_id, p.name, v.theta_before, v.theta_after, v.phi_before, \
             v.sigma_before from x.player_game g left join x.player p on p.id = g.player_id \
             left join player_events v on v.player_id = g.player_id and v.tournament_id = g.tournament_id \
             where g.tournament_id = ?1",
        )
        .unwrap();
    let mut rosters: HashMap<i64, Vec<Member>> = HashMap::new();
    for m in st
        .query_map([id], |r| {
            let theta: Option<f64> = r.get(3)?;
            let after: Option<f64> = r.get(4)?;
            let phi: f64 = r.get::<_, Option<f64>>(5)?.unwrap_or(0.0);
            let sigma: Option<f64> = r.get(6)?;
            // The published player number is the strength at an in-person
            // table, theta + phi, and that is what the roster lists.
            Ok((
                r.get::<_, i64>(0)?,
                Member {
                    player: r.get(1)?,
                    name: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    before: theta.map(|t| s.d.a_player + s.d.b_player * (t + phi)),
                    after: after.map(|t| s.d.a_player + s.d.b_player * (t + phi)),
                    provisional: sigma.is_some_and(|x| x > s.d.provisional_sigma),
                },
            ))
        })
        .unwrap()
        .flatten()
    {
        rosters.entry(m.0).or_default().push(m.1);
    }
    for v in rosters.values_mut() {
        v.sort_by(|a, b| {
            b.before
                .unwrap_or(f64::NEG_INFINITY)
                .partial_cmp(&a.before.unwrap_or(f64::NEG_INFINITY))
                .unwrap()
                .then_with(|| a.name.cmp(&b.name))
        });
    }

    out.push_str("<section class=\"section\"><h2 class=\"subhead\">Итоги</h2>");
    let unrated = evs.iter().filter(|v| !v.rated).count();
    if unrated > 0 {
        let _ = write!(
            out,
            "<p class=\"hint\">Не зачтено результатов: {unrated} из {}. Такая команда получает \
             прогноз, но её рейтинг турнир не двигает — за старым номером вышел состав, \
             ничего этой команде не наигравший.</p>",
            evs.len()
        );
    }
    if evs.is_empty() {
        out.push_str("<p class=\"empty\">Данных по командам нет.</p></section>");
        return Some(out);
    }
    // The three parts of the step, in display points. The floor is whatever the
    // placement step and the pull leave unexplained, and only a winner has one.
    let parts = |v: &Row| -> Option<(f64, f64, f64)> {
        let (b, a, dp, dl) = (v.before?, v.after?, v.d_place?, v.d_pull?);
        let total = s.d.b_team * (a - b);
        let (dp, dl) = (s.d.b_team * dp, s.d.b_team * dl);
        Some((dp, dl, total - dp - dl))
    };
    // Shown only when a pull moved some rating; otherwise Δ is the placement step.
    let split = evs.iter().filter_map(parts).any(|(_, dl, _)| dl.round() != 0.0);
    let floor = evs.iter().filter_map(parts).any(|(_, _, f)| f.round() != 0.0);
    if !rosters.is_empty() {
        out.push_str(
            "<label class=\"checkbox roster-toggle\"><input type=\"checkbox\"> Показывать составы</label>",
        );
    }
    let _ = write!(
        out,
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">Место</th><th class=\"num\">Прогноз</th><th>Команда</th>\
         <th class=\"num\">Взято</th>\
         <th class=\"num\" title=\"Сила вышедшего состава по рейтингам игроков на утро турнира\">Состав</th>\
         <th class=\"num\" title=\"Сколько результат стоит в рейтинге команды в день турнира\">Ценность</th>\
         {}{}{}\
         </tr></thead><tbody>",
        if have_values {
            "<th class=\"num\" title=\"Насколько вырос рейтинг команды, когда результат вошёл в семь её лучших\">Прибавка</th>"
        } else {
            "<th class=\"num\">Было</th><th class=\"num\">Стало</th><th class=\"num\">Δ</th>"
        },
        if split {
            "<th class=\"num\" title=\"Шаг за место: против ожидания по силе команд в поле\">за место</th>\
             <th class=\"num\" title=\"Подтягивание рейтинга к силе вышедшего состава\">к составу</th>"
        } else {
            ""
        },
        if floor {
            "<th class=\"num\" title=\"Победитель не теряет очков: возвращённая разница\">победитель</th>"
        } else {
            ""
        }
    );
    for v in &evs {
        let beat = match (v.forecast, v.actual) {
            (Some(f), Some(a)) if a < f - 0.5 => " place-better",
            (Some(f), Some(a)) if a > f + 0.5 => " place-worse",
            _ => "",
        };
        // The link says the team's own name; the name it entered under, when it
        // is a different one, goes next to it.
        let shown = if v.name.is_empty() { &v.played_as } else { &v.name };
        let played_as = if !v.played_as.is_empty()
            && crate::index::fold(&v.played_as) != crate::index::fold(shown)
        {
            format!(" <span class=\"muted\">(как {})</span>", e(&v.played_as))
        } else {
            String::new()
        };
        let roster = rosters.get(&v.team).filter(|r| !r.is_empty());
        // The lineup sits inside the team's own cell, one player to a line, and
        // opens under the name: by the switch beside it or the one above the table.
        let open = match roster {
            Some(roster) => {
                let mut s = String::from(
                    " <label class=\"roster-open\"><input type=\"checkbox\"><span>состав</span></label>\
                     <ul class=\"cell-roster\">",
                );
                for m in roster {
                    let _ = write!(
                        s,
                        "<li><a href=\"/players/{}/\">{}</a>{} <span class=\"num\">{}</span>{}</li>",
                        m.player,
                        e(if m.name.is_empty() { "—" } else { &m.name }),
                        if m.provisional { prov_badge() } else { "" },
                        m.before.map(disp).unwrap_or_else(|| "—".into()),
                        match (m.before, m.after) {
                            (Some(b), Some(a)) if (a - b).round() != 0.0 => format!(
                                " <span class=\"{}\">{}</span>",
                                delta_class(a - b),
                                signed(a - b)
                            ),
                            _ => String::new(),
                        }
                    );
                }
                s.push_str("</ul>");
                s
            }
            None => String::new(),
        };
        let p = parts(v);
        let _ = write!(
            out,
            "<tr><td class=\"num{beat}\">{}</td><td class=\"num muted\">{}</td>\
             <td><a href=\"/teams/{}/\">{}</a>{}{}{}</td><td class=\"num\">{}</td>\
             <td class=\"num muted\">{}</td><td class=\"num\">{}</td>{}{}{}</tr>",
            place(v.actual),
            place(v.forecast),
            v.team,
            e(if shown.is_empty() { "—" } else { shown }),
            played_as,
            unrated_tag(v.rated),
            open,
            v.score.map(|x| x.to_string()).unwrap_or_else(|| "—".into()),
            v.lineup.map(|q| disp(s.d.a_team + s.d.b_team * q)).unwrap_or_else(|| "—".into()),
            v.value.map(disp).unwrap_or_else(|| "—".into()),
            if have_values {
                match v.gain {
                    Some(g) if g.round() > 0.0 => format!("<td class=\"num delta-up\">{}</td>", signed(g)),
                    _ => "<td class=\"num muted\"></td>".into(),
                }
            } else {
                step_cells(s.d.a_team, s.d.b_team, v.before, v.after)
            },
            if split {
                match p {
                    Some((dp, dl, _)) => format!(
                        "<td class=\"num {}\">{}</td><td class=\"num {}\">{}</td>",
                        delta_class(dp),
                        signed(dp),
                        delta_class(dl),
                        signed(dl)
                    ),
                    None => "<td class=\"num muted\">—</td><td class=\"num muted\">—</td>".into(),
                }
            } else {
                String::new()
            },
            if floor {
                match p {
                    Some((_, _, f)) if f.round() != 0.0 => {
                        format!("<td class=\"num {}\">{}</td>", delta_class(f), signed(f))
                    }
                    _ => "<td class=\"num muted\"></td>".into(),
                }
            } else {
                String::new()
            },
        );
    }
    out.push_str("</tbody></table></div>");
    out.push_str(
        "<p class=\"hint\">Состав — сила вышедшего состава по рейтингам игроков на утро турнира; \
         по ней и строится прогноз, рейтинг команды в прогноз не входит.",
    );
    if have_values {
        out.push_str(
            " Ценность — сколько результат стоит в рейтинге команды: сила игры, делённая на место \
             в степени 0,6 и умноженная на вес турнира. Прочерк — результат не идёт в зачёт команде. \
             Прибавка — насколько вырос рейтинг команды, когда результат вошёл в семь её лучших; \
             результат, не вошедший в них, рейтинг не меняет.",
        );
    }
    if split {
        out.push_str(
            " Δ складывается из двух шагов. За место — насколько команда обошла ожидание \
             по силе команд в поле или не дотянула до него; чем больше поле, тем меньше \
             весит одно место. К составу — доля разрыва между рейтингом и силой состава, \
             которую рейтинг проходит за каждую зачтённую игру: так рейтинг узнаёт, кто \
             сидит за столом, а не только кто кого обошёл.",
        );
    }
    if floor {
        out.push_str(
            " Победитель не теряет очков: если шаги в сумме ушли в минус, разница \
             возвращается.",
        );
    }
    if !rosters.is_empty() {
        out.push_str(
            " В составе у каждого игрока — его рейтинг на утро турнира и сдвиг по итогам.",
        );
    }
    out.push_str("</p></section>");
    Some(out)
}

pub fn releases_page(s: &Site, c: &Connection) -> String {
    let mut st = c
        .prepare("select id, date, n_players, n_teams, shift, shift_cum from x.release_ order by date desc, id desc")
        .unwrap();
    let rows: Vec<(i64, String, i64, i64, f64, f64)> = st
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let mut out = String::with_capacity(rows.len() * 200 + 512);
    out.push_str(&head(
        "Релизы",
        &format!(
            "Релиз — недельный снимок состояния, по четвергам. Опубликованное число никогда \
             не переписывается. Сдвиг — калибровка шкалы к нулевому среднему на этом релизе, \
             накопленный сдвиг показывает, насколько шкала ушла с начала счёта. \
             Всего релизов: {}.",
            s.rels.len()
        ),
    ));
    out.push_str(
        "<section class=\"section\"><div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">№</th><th>Дата</th><th class=\"num\">Игроков</th>\
         <th class=\"num\">Команд</th><th class=\"num\">Сдвиг</th>\
         <th class=\"num\">Накопленный сдвиг</th></tr></thead><tbody>",
    );
    for r in &rows {
        let _ = write!(
            out,
            "<tr><td class=\"num muted\">{}</td><td class=\"nowrap\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num muted\">{:+.4}</td><td class=\"num muted\">{:+.4}</td></tr>",
            r.0, e(&r.1), r.2, r.3, r.4, r.5
        );
    }
    out.push_str("</tbody></table></div></section>");
    out
}

// ── search ───────────────────────────────────────────────────────────────────

/// FTS5 takes the query as its own little language, so a name typed with a
/// quote or a hyphen would be a syntax error. Each word is requoted and given a
/// prefix star, which is what a reader typing half a surname expects.
fn fts_query(q: &str) -> String {
    let mut out = String::new();
    for w in q.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        let _ = write!(out, "\"{}\"*", w.replace('"', ""));
    }
    out
}

pub fn search_page(c: &Connection, q: &str) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str(&head("Поиск", &format!("Запрос: {}", e(q))));
    let query = fts_query(q);
    if query.is_empty() {
        out.push_str("<p class=\"empty\">Введите имя игрока или название команды.</p>");
        return out;
    }

    let mut st = c
        .prepare(
            "select f.rowid, p.name, k.rank, k.display, p.games, p.last_date \
             from x.player_fts f join x.player p on p.id = f.rowid \
             left join x.player_rank k on k.player_id = f.rowid \
             where player_fts match ?1 order by k.rank is null, k.rank, p.games desc limit 40",
        )
        .unwrap();
    let players: Vec<(i64, String, Option<i64>, Option<f64>, i64, String)> = st
        .query_map([&query], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get(2)?,
                r.get(3)?,
                r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                r.get::<_, Option<String>>(5)?.unwrap_or_default(),
            ))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    let mut st = c
        .prepare(
            "select f.rowid, t.name, t.town, k.rank, k.display, t.games, t.last_date \
             from x.team_fts f join x.team t on t.id = f.rowid \
             left join x.team_rank k on k.team_id = f.rowid \
             where team_fts match ?1 order by k.rank is null, k.rank, t.games desc limit 40",
        )
        .unwrap();
    let teams: Vec<(i64, String, String, Option<i64>, Option<f64>, i64, String)> = st
        .query_map([&query], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get(3)?,
                r.get(4)?,
                r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                r.get::<_, Option<String>>(6)?.unwrap_or_default(),
            ))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    if players.is_empty() && teams.is_empty() {
        out.push_str("<p class=\"empty\">Ничего не найдено.</p>");
        return out;
    }
    if !players.is_empty() {
        let _ = write!(
            out,
            "<section class=\"section\"><h2 class=\"subhead\">Игроки ({})</h2>\
             <div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
             <th class=\"num\">Место</th><th>Игрок</th><th class=\"num\">Рейтинг</th>\
             <th class=\"num\">Игр</th><th>Последняя игра</th></tr></thead><tbody>",
            players.len()
        );
        for p in &players {
            let _ = write!(
                out,
                "<tr><td class=\"num muted\">{}</td><td><a href=\"/players/{}/\">{}</a></td>\
                 <td class=\"num strong\">{}</td><td class=\"num\">{}</td>\
                 <td class=\"nowrap muted\">{}</td></tr>",
                p.2.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
                p.0,
                e(&p.1),
                p.3.map(disp).unwrap_or_else(|| "—".into()),
                p.4,
                e(&p.5)
            );
        }
        out.push_str("</tbody></table></div></section>");
    }
    if !teams.is_empty() {
        let _ = write!(
            out,
            "<section class=\"section\"><h2 class=\"subhead\">Команды ({})</h2>\
             <div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
             <th class=\"num\">Место</th><th>Команда</th><th>Город</th>\
             <th class=\"num\">Рейтинг</th><th class=\"num\">Турниров</th>\
             <th>Последняя игра</th></tr></thead><tbody>",
            teams.len()
        );
        for t in &teams {
            let _ = write!(
                out,
                "<tr><td class=\"num muted\">{}</td><td><a href=\"/teams/{}/\">{}</a></td>\
                 <td>{}</td><td class=\"num strong\">{}</td><td class=\"num\">{}</td>\
                 <td class=\"nowrap muted\">{}</td></tr>",
                t.3.map(|r| r.to_string()).unwrap_or_else(|| "—".into()),
                t.0,
                e(&t.1),
                e(&t.2),
                t.4.map(disp).unwrap_or_else(|| "—".into()),
                t.5,
                e(&t.6)
            );
        }
        out.push_str("</tbody></table></div></section>");
    }
    out
}

// ── anomalies ────────────────────────────────────────────────────────────────

pub fn anomaly_table(s: &Site, c: &Connection, page: usize) -> String {
    let pages = pages_of(s.n_anomaly);
    let page = page.clamp(1, pages);
    let mut st = c
        .prepare(
            "select a.rank, a.player_id, p.name, k.rank, k.display, a.k, a.m_gap, a.m_on, \
             a.m_ip, a.f_gap, a.f_on, a.f_ip from x.anomaly a join x.player p on p.id = a.player_id \
             left join x.player_rank k on k.player_id = a.player_id \
             where a.rank > ?1 and a.rank <= ?2 order by a.rank",
        )
        .unwrap();
    let lo = ((page - 1) * PAGE) as i64;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(i64, i64, String, Option<i64>, Option<f64>, f64, f64, i64, i64, f64, i64, i64)> =
        st.query_map(params![lo, lo + PAGE as i64], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get(3)?,
                r.get(4)?,
                r.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                r.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                r.get::<_, Option<i64>>(8)?.unwrap_or(0),
                r.get::<_, Option<f64>>(9)?.unwrap_or(0.0),
                r.get::<_, Option<i64>>(10)?.unwrap_or(0),
                r.get::<_, Option<i64>>(11)?.unwrap_or(0),
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    let mut out = String::with_capacity(rows.len() * 340 + 2048);
    out.push_str("<header class=\"page-head\"><h1>Аномалии</h1></header>");
    let _ = write!(
        out,
        "<section class=\"section\"><details class=\"note\"><summary>Как считается</summary>\
         <p>Коэффициент — это разница между тем, как команды игрока играют на синхронных \
         турнирах и на очных, по двум независимым величинам.</p>\
         <p><b>Прогноз</b> — во сколько раз выше команда финиширует по сравнению с местом, \
         которое ей предсказывали рейтинги её же состава. Считается по всем результатам, \
         включая незачтённые, где прогноз построен на одном составе; центрируется по тому, \
         в какой части таблицы прогноз поставил команду.</p>\
         <p><b>Трудные вопросы</b> — на сколько процентных пунктов команда обыгрывает поле \
         на той трети пакета, которую поле взяло хуже всего, сверх того, что даёт её \
         результат на самой лёгкой трети. Рейтинги в этой величине не участвуют вовсе: \
         шкалой трудности служит доля взятия в поле.</p>\
         <p>Обе сжимаются тем сильнее, чем меньше турниров за ними стоит, — но сжатие \
         ослабевает, когда разрыв велик: большинство игроков сидит около нуля, а меньшинство \
         далеко от него. Турниры одного игрока — это одни и те же партнёры против одного и \
         того же круга, поэтому независимыми считаются не больше тридцати из них. Обе \
         величины командные: они одинаковы для всех, кто играл в этом составе.</p>\
         <p>В списке {} игроков, у которых есть хотя бы восемь синхронных результатов.</p>\
         </details>",
        spaced(s.n_anomaly)
    );
    let url = |i: usize| if i == 1 { "/anomaly/".to_string() } else { format!("/anomaly/page/{i}/") };
    out.push_str(&pager(page, pages, &url));
    out.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">№</th><th>Игрок</th><th class=\"num\">Рейтинг</th>\
         <th class=\"num\">Место</th><th class=\"num\">Синхрон</th><th class=\"num\">Очно</th>\
         <th class=\"num\">Прогноз</th><th class=\"num\">Трудные вопросы</th>\
         <th class=\"num\">Коэффициент</th></tr></thead><tbody>",
    );
    for r in &rows {
        let name = if r.2.is_empty() { format!("Игрок {}", r.1) } else { r.2.clone() };
        let _ = write!(
            out,
            "<tr><td class=\"num muted\">{}</td><td><a href=\"/players/{}/\">{}</a></td>\
             <td class=\"num\">{}</td><td class=\"num muted\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num {}\">×{:.2}</td><td class=\"num {}\">{}</td>\
             <td class=\"num strong\">{:.2}</td></tr>",
            r.0,
            r.1,
            e(&name),
            r.4.map(disp).unwrap_or_else(|| "—".into()),
            r.3.map(|x| x.to_string()).unwrap_or_else(|| "—".into()),
            r.10,
            r.11,
            delta_class(r.9),
            r.9.exp(),
            delta_class(r.6),
            signed1(100.0 * r.6),
            r.5
        );
    }
    out.push_str("</tbody></table></div>");
    out.push_str(&pager(page, pages, &url));
    out.push_str("</section>");
    out
}
