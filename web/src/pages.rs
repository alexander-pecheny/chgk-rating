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
            "Релиз от {}. В таблице {} команд, из них {} с установившимся рейтингом. \
             Команда попадает в список, если сыграла рейтинговый турнир за последние 365 дней.",
            e(&s.release_date),
            s.n_teams,
            firm
        ),
    ));
    out.push_str("<section class=\"section\">");
    out.push_str(&prov_toggle(
        rows.iter().filter(|r| r.8).count(),
        (firm < s.n_teams).then(|| firm / PAGE + 1),
        page,
    ));
    let url = |i: usize| if i == 1 { "/".to_string() } else { format!("/page/{i}/") };
    out.push_str(&pager(page, pages, &url));
    out.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">Место</th><th>Команда</th><th>Город</th>\
         <th class=\"num\">Рейтинг</th>\
         <th class=\"num\" title=\"Неопределённость оценки\">±</th>\
         <th class=\"num\">Изменение</th></tr></thead><tbody>",
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
            "<tr{}><td class=\"num\">{}</td><td><a href=\"/teams/{}/\">{}</a>{}</td>\
             <td>{}</td><td class=\"num strong\">{}</td><td class=\"num muted\">{}</td>{}</tr>",
            if r.8 { " class=\"prov-row\"" } else { "" },
            if r.8 { "—".into() } else { r.0.to_string() },
            r.1,
            e(&name),
            if r.8 { prov_badge() } else { "" },
            e(&r.3),
            disp(r.4),
            disp(r.5),
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
fn chart_section(s: &Site, c: &Connection, kind: i64, id: i64, sql: &str, a: f64, b: f64) -> String {
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
            text: format!("{} ± {}", disp(mid), disp(sigma)),
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
            band: true,
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
    ));

    let mut st = c
        .prepare(
            "select g.tournament_id, t.name, t.date, t.type, g.team_id, m.name, g.position, \
             g.score, t.n_q from x.player_game g join x.tournament t on t.id = g.tournament_id \
             left join x.team m on m.id = g.team_id where g.player_id = ?1 \
             order by t.date desc, t.id desc",
        )
        .unwrap();
    let games: Vec<(i64, String, String, String, i64, String, Option<f64>, Option<i64>, i64)> = st
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
         <th class=\"num\">Место</th><th class=\"num\">Взято</th></tr></thead><tbody>",
    );
    for g in &games {
        let _ = write!(
            out,
            "<tr><td class=\"nowrap\">{}</td><td><a href=\"/tournaments/{}/\">{}</a></td>\
             <td class=\"muted\">{}</td><td><a href=\"/teams/{}/\">{}</a></td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            e(&g.2),
            g.0,
            e(if g.1.is_empty() { "без названия" } else { &g.1 }),
            e(tour_type(&g.3)),
            g.4,
            e(if g.5.is_empty() { "—" } else { &g.5 }),
            place(g.6),
            g.7.map(|x| if g.8 > 0 { format!("{x}/{}", g.8) } else { x.to_string() })
                .unwrap_or_else(|| "—".into()),
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

    let now: Option<(f64, f64, f64)> = c
        .query_row(
            "select display, sigma, evidence from team_ratings where team_id = ?1 \
             order by release_id desc limit 1",
            [id],
            |r| {
                Ok((
                    r.get::<_, Option<f64>>(0)?.unwrap_or(0.0),
                    r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                    r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
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
    out.push_str("<dl class=\"stats\">");
    match now {
        Some((display, sigma, evidence)) => {
            out.push_str(&stat("Рейтинг", &e(&disp(display))));
            out.push_str(&stat("Неопределённость", &format!("±{}", e(&disp(s.d.b_team * sigma)))));
            out.push_str(&stat("Наигранность", &format!("{evidence:.1}")));
        }
        None => out.push_str(&stat("Рейтинг", "—")),
    }
    out.push_str(&stat(
        "Изменение за неделю",
        &rank.and_then(|r| r.1).map(signed).unwrap_or_else(|| "—".into()),
    ));
    out.push_str("</dl>");

    out.push_str(&chart_section(
        s,
        c,
        1,
        id,
        "select release_id, mu, sigma from team_ratings where team_id = ?1",
        s.d.a_team,
        s.d.b_team,
    ));

    let mut st = c
        .prepare(
            "select v.tournament_id, t.name, coalesce(v.date, t.date) as d, v.forecast_place, \
             v.actual_place, v.mu_before, v.mu_after, v.c, v.rated \
             from team_events v left join x.tournament t on t.id = v.tournament_id \
             where v.team_id = ?1 order by d desc, v.tournament_id desc",
        )
        .unwrap();
    let evs: Vec<(i64, String, String, Option<f64>, Option<f64>, f64, f64, f64, bool)> = st
        .query_map([id], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get(3)?,
                r.get(4)?,
                r.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                r.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
                r.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
                r.get::<_, Option<i64>>(8)?.unwrap_or(1) != 0,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    let _ = write!(
        out,
        "<section class=\"section\"><h2 class=\"subhead\">Сыгранные турниры ({})</h2>",
        evs.len()
    );
    if evs.is_empty() {
        out.push_str("<p class=\"empty\">Рейтинговых турниров нет.</p></section>");
        return Some(out);
    }
    out.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th>Дата</th><th>Турнир</th><th class=\"num\">Прогноз</th><th class=\"num\">Место</th>\
         <th class=\"num\">Было</th><th class=\"num\">Стало</th><th class=\"num\">Δ</th>\
         <th class=\"num\" title=\"Доля наигранного состава, вышедшая на этот турнир\">c</th>\
         </tr></thead><tbody>",
    );
    for v in &evs {
        let before = s.d.a_team + s.d.b_team * v.5;
        let after = s.d.a_team + s.d.b_team * v.6;
        let beat = match (v.3, v.4) {
            (Some(f), Some(a)) if a < f - 0.5 => " place-better",
            (Some(f), Some(a)) if a > f + 0.5 => " place-worse",
            _ => "",
        };
        let _ = write!(
            out,
            "<tr><td class=\"nowrap\">{}</td><td><a href=\"/tournaments/{}/\">{}</a>{}</td>\
             <td class=\"num muted\">{}</td><td class=\"num{beat}\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num {}\">{}</td><td class=\"num muted\">{:.2}</td></tr>",
            e(&v.2),
            v.0,
            e(if v.1.is_empty() { "без названия" } else { &v.1 }),
            unrated_tag(v.8),
            place(v.3),
            place(v.4),
            disp(before),
            disp(after),
            delta_class(after - before),
            signed(after - before),
            v.7
        );
    }
    out.push_str("</tbody></table></div></section>");
    Some(out)
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
    let mut st = c
        .prepare(
            "select v.team_id, m.name, v.forecast_place, v.actual_place, v.mu_before, v.mu_after, \
             v.rated, r.score from team_events v left join x.team m on m.id = v.team_id \
             left join x.result_ r on r.tournament_id = v.tournament_id and r.team_id = v.team_id \
             where v.tournament_id = ?1 order by v.actual_place",
        )
        .unwrap();
    let evs: Vec<(i64, String, Option<f64>, Option<f64>, f64, f64, bool, Option<i64>)> = st
        .query_map([id], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get(2)?,
                r.get(3)?,
                r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                r.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                r.get::<_, Option<i64>>(6)?.unwrap_or(1) != 0,
                r.get(7)?,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    out.push_str("<section class=\"section\"><h2 class=\"subhead\">Итоги</h2>");
    let unrated = evs.iter().filter(|v| !v.6).count();
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
    out.push_str(
        "<div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">Место</th><th class=\"num\">Прогноз</th><th>Команда</th>\
         <th class=\"num\">Взято</th><th class=\"num\">Было</th><th class=\"num\">Стало</th>\
         <th class=\"num\">Δ</th></tr></thead><tbody>",
    );
    for v in &evs {
        let before = s.d.a_team + s.d.b_team * v.4;
        let after = s.d.a_team + s.d.b_team * v.5;
        let beat = match (v.2, v.3) {
            (Some(f), Some(a)) if a < f - 0.5 => " place-better",
            (Some(f), Some(a)) if a > f + 0.5 => " place-worse",
            _ => "",
        };
        let _ = write!(
            out,
            "<tr><td class=\"num{beat}\">{}</td><td class=\"num muted\">{}</td>\
             <td><a href=\"/teams/{}/\">{}</a>{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num {}\">{}</td></tr>",
            place(v.3),
            place(v.2),
            v.0,
            e(if v.1.is_empty() { "—" } else { &v.1 }),
            unrated_tag(v.6),
            v.7.map(|x| x.to_string()).unwrap_or_else(|| "—".into()),
            disp(before),
            disp(after),
            delta_class(after - before),
            signed(after - before)
        );
    }
    out.push_str("</tbody></table></div></section>");
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
