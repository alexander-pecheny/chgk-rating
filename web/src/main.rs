mod data;
mod html;
mod index;
mod method;
mod pages;

use pages::{layout, Site};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tiny_http::{Header, Request, Response, Server};

struct Args {
    ratings: PathBuf,
    buff: PathBuf,
    config: PathBuf,
    web: PathBuf,
    assets: PathBuf,
    addr: String,
    threads: usize,
    rebuild: bool,
}

fn args() -> Args {
    let mut a = std::env::args().skip(1);
    let mut v = Args {
        ratings: "out/ratings.db".into(),
        buff: "buff.db".into(),
        config: "config.toml".into(),
        web: "out/web.db".into(),
        assets: "web/static".into(),
        addr: "127.0.0.1:8080".into(),
        threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8),
        rebuild: false,
    };
    while let Some(x) = a.next() {
        let mut next = || a.next().unwrap_or_else(|| panic!("{x} needs a value"));
        match x.as_str() {
            "--buff" => v.buff = next().into(),
            "--config" => v.config = next().into(),
            "--web" => v.web = next().into(),
            "--static" => v.assets = next().into(),
            "--addr" => v.addr = next(),
            "--threads" => v.threads = next().parse().expect("--threads takes a number"),
            "--rebuild" => v.rebuild = true,
            _ => v.ratings = x.into(),
        }
    }
    v
}

fn modified(p: &Path) -> std::time::SystemTime {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// One connection per worker: the ratings database, with the serving index
/// attached so a page can join names onto ratings in a single query.
fn connect(ratings: &Path, web: &Path) -> Connection {
    let c = Connection::open_with_flags(ratings, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|e| panic!("open {}: {e}", ratings.display()));
    c.execute("ATTACH DATABASE ?1 AS x", [web.to_str().expect("web path is not utf-8")])
        .unwrap_or_else(|e| panic!("attach {}: {e}", web.display()));
    c.execute_batch("PRAGMA cache_size = -65536; PRAGMA mmap_size = 1073741824;").ok();
    c
}

fn main() {
    let a = args();
    let d = data::read_display(&a.config);
    if a.rebuild || !a.web.exists() || modified(&a.web) < modified(&a.ratings) {
        eprintln!("building the serving index at {}", a.web.display());
        index::ensure_ratings_indexes(&a.ratings);
        index::build(&a.ratings, &a.buff, &a.web, &d);
    }

    let site = Arc::new(Site::load(&connect(&a.ratings, &a.web)));
    let server = Arc::new(Server::http(&a.addr).unwrap_or_else(|e| panic!("bind {}: {e}", a.addr)));
    eprintln!(
        "serving {} teams and {} players from release {} on http://{}",
        site.n_teams, site.n_players, site.release_date, a.addr
    );

    let mut workers = Vec::new();
    for _ in 0..a.threads.max(1) {
        let (server, site) = (server.clone(), site.clone());
        let c = connect(&a.ratings, &a.web);
        let assets = a.assets.clone();
        workers.push(std::thread::spawn(move || loop {
            let Ok(req) = server.recv() else { return };
            // A bad row or a query this schema cannot answer is one broken page,
            // not one fewer worker for the rest of the process's life.
            let url = req.url().to_string();
            let go = std::panic::AssertUnwindSafe(|| handle(req, &site, &c, &assets));
            if std::panic::catch_unwind(go).is_err() {
                eprintln!("panic serving {url}");
            }
        }));
    }
    for w in workers {
        let _ = w.join();
    }
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("header")
}

fn send_html(req: Request, status: u16, body: String) {
    let r = Response::from_string(body)
        .with_status_code(status)
        .with_header(header("Content-Type", "text/html; charset=utf-8"));
    let _ = req.respond(r);
}

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|x| x.to_str()).unwrap_or("") {
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "woff2" => "font/woff2",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn send_asset(req: Request, dir: &Path, rel: &str) {
    // Anything with a parent segment is a traversal attempt, not a typo.
    let safe = !rel.is_empty() && !rel.split('/').any(|s| s == ".." || s == ".");
    let path = dir.join(rel);
    match safe.then(|| std::fs::File::open(&path).ok()).flatten() {
        Some(f) => {
            let r = Response::from_file(f)
                .with_header(header("Content-Type", mime(&path)))
                .with_header(header("Cache-Control", "public, max-age=3600"));
            let _ = req.respond(r);
        }
        None => not_found(req),
    }
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| decode(v))
}

/// Splits a request target into its path segments and the raw query string.
fn route(url: &str) -> (Vec<String>, String) {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let segs = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(decode)
        .collect();
    (segs, query.to_string())
}

fn handle(req: Request, s: &Site, c: &Connection, assets: &Path) {
    let (segs, query) = route(req.url());
    let n = |i: usize| -> usize { segs.get(i).and_then(|x| x.parse().ok()).unwrap_or(1) };
    let id = |i: usize| -> Option<i64> { segs.get(i)?.parse().ok() };
    let parts: Vec<&str> = segs.iter().map(|x| x.as_str()).collect();

    let (title, body) = match parts.as_slice() {
        [] => ("Рейтинг ЧГК".into(), pages::team_table(s, c, 1)),
        ["page", _] => (format!("Команды — страница {}", n(1)), pages::team_table(s, c, n(1))),
        ["players"] => ("Игроки".into(), pages::player_table(s, c, 1)),
        ["players", "page", _] => {
            (format!("Игроки — страница {}", n(2)), pages::player_table(s, c, n(2)))
        }
        ["tournaments"] => ("Турниры".into(), pages::tournament_table(c, 1)),
        ["tournaments", "page", _] => {
            (format!("Турниры — страница {}", n(2)), pages::tournament_table(c, n(2)))
        }
        ["releases"] => ("Релизы".into(), pages::releases_page(s, c)),
        ["method"] => ("Методика".into(), method::BODY.to_string()),
        ["search"] => {
            let q = query_param(&query, "q").unwrap_or_default();
            (format!("Поиск: {q}"), pages::search_page(c, &q))
        }
        ["players", _] => match id(1).and_then(|i| pages::player_page(s, c, i)) {
            Some(b) => (title_of(&b), b),
            None => return not_found(req),
        },
        ["teams", _] => match id(1).and_then(|i| pages::team_page(s, c, i)) {
            Some(b) => (title_of(&b), b),
            None => return not_found(req),
        },
        ["tournaments", _] => match id(1).and_then(|i| pages::tournament_page(s, c, i)) {
            Some(b) => (title_of(&b), b),
            None => return not_found(req),
        },
        _ => return send_asset(req, assets, &segs.join("/")),
    };
    send_html(req, 200, layout(&format!("{title} — Рейтинг ЧГК"), &body));
}

/// The <h1> a page just rendered, reused as the browser title rather than
/// querying the same name twice.
fn title_of(body: &str) -> String {
    let Some(a) = body.find("<h1>") else { return "Рейтинг ЧГК".into() };
    let rest = &body[a + 4..];
    let end = rest.find("</h1>").unwrap_or(rest.len());
    let mut out = String::new();
    let mut depth = 0;
    for ch in rest[..end].chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn not_found(req: Request) {
    let body = "<header class=\"page-head\"><h1>Не найдено</h1>\
                <p class=\"lede\">Такой страницы нет. Попробуйте поиск в шапке.</p></header>";
    send_html(req, 404, layout("Не найдено — Рейтинг ЧГК", body));
}
