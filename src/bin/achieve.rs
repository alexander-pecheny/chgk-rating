//! The achievement rating from the command line: freeze each result's values,
//! publish the table into a fit's database, search the knobs, or score variants.
//! The rules and the knobs are in `chgk_rating::achieve`.
//!
//! ```text
//! ./target/release/achieve --build                  # player pass, writes out/achieve/
//! ./target/release/achieve --publish out/ratings.db # the site's team rating
//! ./target/release/achieve --point "top_x=10" --point "fade=none" --top 40
//! ./target/release/achieve --search 2000 --rounds 20 --fix "top_x=7"
//! ```
//!
//! `--build` writes `results.tsv`, `names.tsv` and `rosters.tsv`; every other
//! mode reads them.

use chgk_rating::achieve::{self, Knobs, Lab, SearchOpts};
use chgk_rating::config::Config;
use chgk_rating::util::{format_date, parse_date};
use rusqlite::Connection;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

const DIR: &str = "out/achieve";
/// Cities playing one package within this many days are one tournament.
const MERGE_DAYS: i32 = 2;
const THREADS: usize = 3;
const ROUNDS: usize = 20;
const TOP: usize = 30;
/// The shipped table, for the "was" column.
const RATINGS_DB: &str = "out/ratings.db";

fn args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

fn arg(name: &str) -> Option<String> {
    let a = args();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

fn args_all(name: &str) -> Vec<String> {
    let a = args();
    a.iter()
        .enumerate()
        .filter(|(_, x)| x.as_str() == name)
        .filter_map(|(i, _)| a.get(i + 1).cloned())
        .collect()
}

fn flag(name: &str) -> bool {
    args().iter().any(|a| a == name)
}

/// A numeric argument, or `default` when absent or unreadable.
fn num<T: std::str::FromStr>(name: &str, default: T) -> T {
    arg(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// The shipped table on its last release: each listed team's place.
fn shipped_places() -> HashMap<i64, usize> {
    let read = |c: Connection| -> Option<HashMap<i64, usize>> {
        let mut st = c
            .prepare(
                "SELECT team_id FROM team_ratings WHERE release_id = (SELECT max(release_id) FROM team_ratings)
                     AND listed = 1 ORDER BY display DESC",
            )
            .ok()?;
        let v: Vec<i64> = st.query_map([], |r| r.get(0)).ok()?.filter_map(|x| x.ok()).collect();
        Some(v.into_iter().enumerate().map(|(i, t)| (t, i + 1)).collect())
    };
    Connection::open(RATINGS_DB).ok().and_then(read).unwrap_or_default()
}

/// One variant's table of the top teams on `at`, as Markdown.
fn top_table(lab: &Lab, b: &achieve::Book, k: &Knobs, at: i32, top_n: usize, was: &HashMap<i64, usize>) -> String {
    let mut md = format!("# Achievement rating on {}\n\nKnobs: `{k:?}`\n\n", format_date(at));
    let _ = writeln!(md, "| # | was | team | rating | counted |\n|---|---|---|---|---|");
    for (i, s) in b.table(at).iter().take(top_n).enumerate() {
        let was = was.get(&s.team).map_or("–".into(), |r| r.to_string());
        let _ = writeln!(md, "| {} | {was} | {} | {:.0} | {} |", i + 1, lab.team_name(s.team), s.rating, s.counted);
    }
    md
}

/// The base knobs and every `--point` on top of them: each one's team metric
/// against the fair rule's, and its table as `top<i>.md` in `--out`.
fn variants(lab: &Lab, dir: &str) -> Result<String, Box<dyn std::error::Error>> {
    let at = arg("--at").and_then(|d| parse_date(&d)).unwrap_or(lab.last + 1);
    let top_n = num("--top", TOP);
    let out_dir = arg("--out").unwrap_or_else(|| dir.to_string());
    std::fs::create_dir_all(&out_dir)?;
    let was = shipped_places();
    let base = Knobs::default().apply(&arg("--base").unwrap_or_default())?;
    let mut table = String::from(
        "Test window; under each variant its difference from the base, ± the standard error of the paired difference over fields.\n\n| variant | NDCG | @3 | @10 | r | pair | pair@10% |\n|---|---|---|---|---|---|---|\n",
    );
    let fmt = |m: [f64; 6]| m.map(|x| format!("{x:.4}")).join(" | ");
    let _ = writeln!(table, "| fair rule (shipped) | {} |", fmt(lab.fair.1));
    let points = [vec![String::new()], args_all("--point")].concat();
    let mut first: Vec<achieve::Scored> = Vec::new();
    for (pi, spec) in points.iter().enumerate() {
        let k = base.apply(spec)?;
        let b = lab.book(&k);
        let fields = b.fields();
        let name = if spec.is_empty() { "achievement (base)".to_string() } else { format!("`{spec}`") };
        let _ = writeln!(table, "| {name} | {} |", fmt(achieve::mean(&fields, 1)));
        if pi == 0 {
            first = fields;
        } else {
            let d = achieve::paired(&first, &fields, 1).map(|(m, se)| format!("{m:+.4} ± {se:.4}"));
            let _ = writeln!(table, "| | {} |", d.join(" | "));
        }
        std::fs::write(format!("{out_dir}/top{pi}.md"), top_table(lab, &b, &k, at, top_n, &was))?;
    }
    Ok(table)
}

fn search_opts(dir: &str) -> Option<SearchOpts> {
    Some(SearchOpts {
        n: arg("--search")?.parse().ok()?,
        rounds: num("--rounds", ROUNDS),
        seed: num("--seed", 1),
        threads: num("--threads", THREADS),
        fixed: arg("--fix").unwrap_or_default(),
        path: arg("--search-out").unwrap_or_else(|| format!("{dir}/search.tsv")),
    })
}

/// Freeze the values when asked or missing, and the names and rosters likewise.
fn prepare(cfg: &Config, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    if flag("--build") || !Path::new(&format!("{dir}/results.tsv")).exists() {
        achieve::freeze(cfg, dir)?;
    }
    if flag("--build") || flag("--rosters") || !Path::new(&format!("{dir}/rosters.tsv")).exists() {
        achieve::save_lists(cfg, achieve::player_strengths(&cfg.data.out_path), dir)?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = Config::load(&arg("--config").unwrap_or_else(|| "config.toml".into()))?;
    if let Some(to) = arg("--to") {
        cfg.data.to = to;
    }
    cfg.data.merge_cities = true;
    cfg.data.merge_days = num("--merge-days", MERGE_DAYS);
    let dir = arg("--dir").unwrap_or_else(|| DIR.into());
    prepare(&cfg, &dir)?;
    let lab = Lab::load(&dir)?;
    if let Some(db) = arg("--publish") {
        let k = Knobs::default().apply(&arg("--base").unwrap_or_default())?;
        achieve::publish(&lab, &k, &db, &cfg, num("--threads", THREADS))?;
    } else if let Some(o) = search_opts(&dir) {
        achieve::search(&lab, &o)?;
        eprintln!("wrote {}", o.path);
    } else {
        print!("{}", variants(&lab, &dir)?);
    }
    Ok(())
}
