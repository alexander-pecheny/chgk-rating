//! The achievement rating: a team's number is what its best recent results were
//! worth, and nothing else. The rules and the knobs are in
//! `chgk_rating::achieve_core`; this binary runs the player pass that freezes
//! each result's values, and scores variants from the command line.
//!
//! ```text
//! ./target/release/achieve --build                  # player pass, writes out/achieve/
//! ./target/release/achieve --point "top_x=10" --point "fade=none" --top 40
//! ```
//!
//! `--build` writes `results.tsv`, `names.tsv` and `rosters.tsv`; the site's
//! `/achieve/` lab reads the same three files from `out/achieve/`.

use chgk_rating::achieve_core::{self as ac, Knobs, Lab, Names, Res, Seasons};
use chgk_rating::chemistry::Chemistry;
use chgk_rating::config::Config;
use chgk_rating::data::{Arena, Format, CORRECT, SKIP};
use chgk_rating::difficulty::Difficulties;
use chgk_rating::editor_memory::EditorMemory;
use chgk_rating::gauge::{self, Activity};
use chgk_rating::question_model as qm;
use chgk_rating::team_ndcg as metric;
use chgk_rating::team_s2::{rows_of, TeamsS2};
use chgk_rating::util::{next_thursday, parse_date};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

const DIR: &str = "out/achieve";

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

// ------------------------------------------------------------ the frozen values

/// The strength at which a team's place is what the field expects: the root of
/// `sum_j P(x beats f_j) + m P(x beats own) = wins + m/2`, by bisection. The
/// same function as the `best_n` rule in `team_s2`.
fn place_perf(own: f64, others: &[f64], wins: f64, m: f64, c: f64) -> f64 {
    let g = |x: f64| {
        others.iter().map(|&f| 1.0 / (1.0 + (c * (f - x)).exp())).sum::<f64>()
            + m / (1.0 + (c * (own - x)).exp())
            - wins
            - m / 2.0
    };
    let lo0 = others.iter().copied().fold(own, f64::min) - 8.0 / c;
    let hi0 = others.iter().copied().fold(own, f64::max) + 8.0 / c;
    let (mut lo, mut hi) = (lo0, hi0);
    while hi - lo > 0.5 {
        let mid = 0.5 * (lo + hi);
        if g(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The theta at which six equal players expect to take `k` of the package's
/// questions, `k` kept half a question inside both ends so a clean sheet stays
/// finite.
fn score_perf(mask: &[u8], params: &[qm::QParams]) -> (f64, f64, f64) {
    let qs: Vec<(usize, u8)> = mask
        .iter()
        .enumerate()
        .filter(|(q, &y)| y != SKIP && *q < params.len())
        .map(|(q, &y)| (q, y))
        .collect();
    let n = qs.len() as f64;
    let k = qs.iter().filter(|x| x.1 == CORRECT).count() as f64;
    if n < 1.0 {
        return (f64::NAN, k, n);
    }
    let target = k.clamp(0.5, n - 0.5);
    let e = |th: f64| {
        qs.iter()
            .map(|&(q, _)| {
                let p = &params[q];
                let u = 6.0 * (p.a() * (th - p.b)).min(30.0).exp();
                (-(-u).exp_m1()).powf(p.m())
            })
            .sum::<f64>()
    };
    let (mut lo, mut hi) = (-20.0, 20.0);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if e(mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (0.5 * (lo + hi), k, n)
}

fn fmt_char(f: Format) -> char {
    match f {
        Format::InPerson => 'i',
        Format::Sync => 's',
        Format::StrictSync => 'S',
        Format::Async => 'a',
    }
}

/// The player pass of `team_sweep`, with each result's performance read off the
/// question fit of its own tournament and the fair rule replayed beside it.
fn build(arena: &Arena, cfg: &Config) -> Vec<Res> {
    let qual = metric::qualify(metric::QUALIFY);
    let tw = metric::weights(metric::WEIGHTS);
    let passes = cfg.data.passes;
    let from_day = parse_date(&cfg.data.from).unwrap();
    let warm_until = parse_date(&cfg.player.warm_until).unwrap();
    let zfrom = parse_date("2018-01-01").unwrap();
    let s2 = &cfg.team.s2;
    let disp = |x: f64| cfg.display.a_team + cfg.display.b_team * x;
    let c = std::f64::consts::LN_10 / s2.d;
    let mut players = qm::Players::new(arena.n_players(), arena.n_editors(), cfg);
    let mut snap: Vec<Option<f64>> = Vec::new();
    let mut out = Vec::new();
    let start = std::time::Instant::now();
    for pass in 0..passes {
        let last = pass + 1 == passes;
        if pass > 0 {
            players.warm_restart(&snap, from_day, cfg);
        }
        let mut ed_seen = vec![0.0f64; arena.n_editors()];
        let mut activity = Activity::new(arena.n_players());
        let mut next_rel = next_thursday(from_day);
        let mut teams = TeamsS2::new(arena.n_teams());
        out.clear();
        let mut chem = Chemistry::new(cfg.question.chem_u != 0.0 || s2.chemistry != 0.0);
        let diff = Difficulties::new(false);
        let edmem = EditorMemory::new(false, 0.0, s2.ed_key, 0.0);
        let geo = chgk_rating::geography::Geography::new(false);
        let load = chgk_rating::geography::PlayerLoad::new(false);
        for (n, t) in arena.tournaments.iter().enumerate() {
            if snap.is_empty() && t.day >= warm_until {
                snap = players.snapshot();
            }
            while next_rel <= t.day {
                activity.expire_before(next_rel - cfg.gauge.window_days);
                players.shift_cum += gauge::shift(&players, &activity, cfg.gauge.n_est);
                next_rel += 7;
            }
            players.seed(t, cfg);
            let field = qm::build_field(t, &players, &chem, cfg);
            let params = qm::fit_all(&field, None, cfg);
            let grads = qm::player_grads(&field, &params, cfg);
            if last && !t.train_only {
                let ctx = chgk_rating::team_s2::Context {
                    chem: &chem,
                    diff: &diff,
                    edmem: &edmem,
                    geo: &geo,
                    load: &load,
                };
                let rows = rows_of(t, &players, &ctx, cfg);
                let evs = teams.step_rows(t.id, t.day, t.weight, t.format, &rows, s2);
                let mu: HashMap<u32, f64> = evs.iter().map(|e| (e.team, e.mu_before)).collect();
                let q: HashMap<u32, f64> = rows.iter().map(|r| (r.team, r.q)).collect();
                let placed: Vec<(f64, f64)> = t
                    .teams
                    .iter()
                    .filter(|r| r.position > 0.0)
                    .map(|r| (r.position, q.get(&r.team).copied().unwrap_or(f64::NAN)))
                    .collect();
                let known: Vec<f64> = placed.iter().map(|x| x.1).filter(|x| x.is_finite()).collect();
                let mid = {
                    let mut v = known.clone();
                    v.sort_by(|a, b| a.total_cmp(b));
                    v.get(v.len() / 2).copied().unwrap_or(0.0)
                };
                for r in &t.teams {
                    let qi = q.get(&r.team).copied().unwrap_or(f64::NAN);
                    let perf_p = if r.position > 0.0 {
                        let mut others = Vec::with_capacity(placed.len());
                        let mut wins = 0.0;
                        let mut seen_self = false;
                        for &(pos, fq) in &placed {
                            if !seen_self && pos == r.position && (fq == qi || (fq.is_nan() && qi.is_nan())) {
                                seen_self = true;
                                continue;
                            }
                            if !fq.is_finite() {
                                continue;
                            }
                            others.push(fq);
                            wins += if pos > r.position {
                                1.0
                            } else if pos == r.position {
                                0.5
                            } else {
                                0.0
                            };
                        }
                        let own = if qi.is_finite() { qi } else { mid };
                        if others.is_empty() {
                            f64::NAN
                        } else {
                            place_perf(own, &others, wins, 1.0, c)
                        }
                    } else {
                        f64::NAN
                    };
                    let (th, taken, n_q) = score_perf(&r.mask, &params);
                    let perf_s = if th.is_finite() {
                        s2.player_zero + s2.player_scale * (th + 6f64.ln() + cfg.team.roster_bias)
                    } else {
                        f64::NAN
                    };
                    out.push(Res {
                        tid: t.id,
                        day: t.day,
                        format: fmt_char(t.format),
                        team: arena.team_ids[r.team as usize],
                        place: r.position,
                        rated: r.rated,
                        n: r.members.len() as u32,
                        n_base: r.base.iter().filter(|&&b| b).count() as u32,
                        q: disp(qi),
                        perf_s: disp(perf_s),
                        perf_p: disp(perf_p),
                        taken,
                        n_q,
                        fair_mu: mu.get(&r.team).map_or(f64::NAN, |&m| disp(m)),
                        venue: r.venue,
                        vplace: r.venue_position,
                        qual: qual.contains(&(t.id, arena.team_ids[r.team as usize])),
                        tw: tw.get(&t.id).copied().unwrap_or(0.0),
                    });
                }
            }
            chem.absorb(t);
            let collect = t.format == Format::InPerson && t.day >= zfrom;
            players.absorb(t, &grads, &mut ed_seen, collect, cfg);
            players.shrink_to_lineups(t, cfg);
            for &(i, _, info) in &grads {
                activity.push(t.day, i, t.weight * info);
            }
            if n % 2000 == 0 {
                eprintln!(
                    "pass {}/{passes} {n}/{} {:.0}s",
                    pass + 1,
                    arena.tournaments.len(),
                    start.elapsed().as_secs_f64()
                );
            }
        }
        if snap.is_empty() {
            snap = players.snapshot();
        }
    }
    out
}

// ---------------------------------------------------------------------- main

fn names(conn: &Connection, sql: &str) -> HashMap<i64, String> {
    conn.prepare(sql)
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .filter_map(|x| x.ok())
        .collect()
}

/// Seasons and every base roster, from the chgk.gg history and `buff.db`.
fn seasons(conn: &Connection) -> Seasons {
    let list = conn
        .prepare("SELECT id, substr(date_start,1,10), substr(date_end,1,10) FROM seasons")
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
        .unwrap()
        .filter_map(|x| x.ok())
        .filter_map(|(i, a, b)| Some((i, parse_date(&a)?, parse_date(&b)?)))
        .collect();
    let mut players: HashMap<(i64, i64), Vec<i64>> = HashMap::new();
    for l in std::fs::read_to_string(chgk_rating::data::BASE_ROSTERS).unwrap_or_default().lines().skip(1) {
        let f: Vec<&str> = l.split(',').collect();
        if let (Some(Ok(t)), Some(Ok(pl)), Some(Ok(s))) =
            (f.first().map(|x| x.parse()), f.get(1).map(|x| x.parse()), f.get(2).map(|x| x.parse()))
        {
            players.entry((t, s)).or_default().push(pl);
        }
    }
    let rows: Vec<(i64, i64, i64)> = conn
        .prepare("SELECT team_id, season_id, player_id FROM team_seasons WHERE player_id > 0")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .filter_map(|x| x.ok())
        .collect();
    for (t, s, pl) in rows {
        players.entry((t, s)).or_default().push(pl);
    }
    for v in players.values_mut() {
        v.sort_unstable();
        v.dedup();
    }
    let has: HashSet<(i64, i64)> = players.keys().copied().collect();
    Seasons { list, has, players }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg_path = arg("--config").unwrap_or_else(|| "config.toml".into());
    let mut cfg = Config::load(&cfg_path)?;
    cfg.data.to = arg("--to").unwrap_or_else(chgk_rating::util::today_utc);
    cfg.data.merge_cities = true;
    cfg.data.merge_days = arg("--merge-days").map_or(2, |v| v.parse().unwrap_or(2));
    let dir = arg("--dir").unwrap_or_else(|| DIR.into());
    std::fs::create_dir_all(&dir)?;
    let results = format!("{dir}/results.tsv");
    if args().iter().any(|a| a == "--build") || !std::path::Path::new(&results).exists() {
        let arena = chgk_rating::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
        eprintln!("loaded {} tournaments, {} teams", arena.tournaments.len(), arena.n_teams());
        let rs = build(&arena, &cfg);
        ac::save_results(&rs, &results)?;
        eprintln!("wrote {results}: {} results", rs.len());
    }
    if args().iter().any(|a| a == "--build" || a == "--rosters")
        || !std::path::Path::new(&format!("{dir}/rosters.tsv")).exists()
    {
        let conn = Connection::open(&cfg.data.db_path)?;
        Names {
            teams: names(&conn, "SELECT id, name FROM teams"),
            tours: names(&conn, "SELECT id, name FROM tournaments"),
        }
        .save(&format!("{dir}/names.tsv"))?;
        seasons(&conn).save(&format!("{dir}/rosters.tsv"))?;
    }
    let lab = Lab::load(&dir)?;
    let at = arg("--at").and_then(|d| parse_date(&d)).unwrap_or(lab.last + 1);
    let top_n: usize = arg("--top").map_or(30, |v| v.parse().unwrap_or(30));
    let out_dir = arg("--out").unwrap_or_else(|| dir.clone());
    std::fs::create_dir_all(&out_dir)?;
    // The shipped table on the last release, for the "was" column.
    let was: HashMap<i64, usize> = Connection::open("out/ratings.db")
        .ok()
        .and_then(|c| {
            let mut st = c
                .prepare(
                    "SELECT team_id FROM team_ratings WHERE release_id = (SELECT max(release_id) FROM team_ratings)
                     AND listed = 1 ORDER BY display DESC",
                )
                .ok()?;
            let v: Vec<i64> = st.query_map([], |r| r.get(0)).ok()?.filter_map(|x| x.ok()).collect();
            Some(v.into_iter().enumerate().map(|(i, t)| (t, i + 1)).collect())
        })
        .unwrap_or_default();

    let base = Knobs::default().apply(&arg("--base").unwrap_or_default())?;
    let mut points = vec![String::new()];
    points.extend(args_all("--point"));
    let mut table = String::from("| variant | tune | test | Δ test vs fair |\n|---|---|---|---|\n");
    let _ = writeln!(table, "| fair rule (shipped) | {:.4} | {:.4} | |", lab.fair.0, lab.fair.1);
    for (pi, spec) in points.iter().enumerate() {
        let k = base.apply(spec)?;
        let b = ac::book(&lab, &k);
        let (tune, test) = ac::ndcg_of(&lab, &b, &k);
        let name = if spec.is_empty() { "achievement (base)".to_string() } else { format!("`{spec}`") };
        let _ = writeln!(table, "| {name} | {tune:.4} | {test:.4} | {:+.4} |", test - lab.fair.1);
        let v = ac::table(&lab, &b, at, &k);
        let mut md = format!("# Achievement rating on {}\n\nKnobs: `{k:?}`\n\n", ac::show_day(at));
        let _ = writeln!(md, "| # | was | team | rating | counted |\n|---|---|---|---|---|");
        for (i, (t, x, c)) in v.iter().take(top_n).enumerate() {
            let _ = writeln!(
                md,
                "| {} | {} | {} | {x:.0} | {} |",
                i + 1,
                was.get(t).map_or("–".into(), |r| r.to_string()),
                lab.names.teams.get(t).map_or("?", |s| s.as_str()),
                c.len()
            );
        }
        std::fs::write(format!("{out_dir}/top{pi}.md"), &md)?;
        if pi == 0 {
            if let Some(hp) = arg("--html") {
                std::fs::write(&hp, ac::render_lab(&lab, &k, at, top_n, &was, &[]))?;
                eprintln!("wrote {hp}");
            }
        }
    }
    print!("{table}");
    Ok(())
}
