//! Tuning the team rules against `tools/team_ndcg.py`'s metric, in one process.
//!
//! The question model never reads a team state, so the player pass can be run
//! once and the team engine replayed over its output as often as a grid needs.
//! One pass costs a couple of minutes, each replay about a second.
//!
//! ```text
//! ./target/release/team_sweep                       # the whole lever scan
//! ./target/release/team_sweep --axis "k_r=0,0.05,0.1" --boot 2000
//! ./target/release/team_sweep --base "k_r=0.05,n0=6" --point "K=0.3"
//! ```

use chgk_rating::config::{Config, MarginShape, PullTarget, S2};
use chgk_rating::data::{Arena, Format};
use chgk_rating::gauge::{self, Activity};
use chgk_rating::question_model as qm;
use chgk_rating::team_ndcg::{self as metric, QUALIFY, TEST, TUNE, WEIGHTS};
use chgk_rating::team_s2::{rows_of, Row, TeamsS2};
use chgk_rating::util::{next_thursday, parse_date};
use std::collections::{HashMap, HashSet};

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

// ------------------------------------------------------------------ the plan

/// A parameter set is the shipped `S2` plus the ranking-time idle penalty, which
/// is not a state and so is not a config key until it earns one.
#[derive(Clone)]
struct Point {
    s2: S2,
    idle_drop: f64,
}

fn set(p: &mut Point, key: &str, val: &str) -> Result<(), String> {
    let num = || val.parse::<f64>().map_err(|_| format!("{key}: not a number: {val}"));
    match key {
        "K" => p.s2.k = num()?,
        "k_q" => p.s2.k_q = num()?,
        "D" => p.s2.d = num()?,
        "margin_g" => p.s2.margin_g = num()?,
        "margin_shape" => {
            p.s2.margin_shape = match val {
                "linear" => MarginShape::Linear,
                "logistic" => MarginShape::Logistic,
                _ => return Err("margin_shape: linear or logistic".into()),
            }
        }
        "k_r" => p.s2.k_r = num()?,
        "k_r_target" => {
            p.s2.k_r_target = match val {
                "lineup" => PullTarget::Lineup,
                "usual" => PullTarget::Usual,
                _ => return Err("k_r_target: lineup or usual".into()),
            }
        }
        "n0" => p.s2.n0 = num()?,
        "w_sync" => p.s2.w_sync = Some(num()?),
        "w_strict_sync" => p.s2.w_strict_sync = Some(num()?),
        "winner_floor" => p.s2.winner_floor = val == "true",
        "idle_drop" => p.idle_drop = num()?,
        _ => return Err(format!("unknown parameter {key}")),
    }
    Ok(())
}

fn apply(p: &Point, spec: &str) -> Result<Point, String> {
    let mut q = p.clone();
    for part in spec.split(',').filter(|s| !s.trim().is_empty()) {
        let (k, v) = part.split_once('=').ok_or("expected name=value")?;
        set(&mut q, k.trim(), v.trim())?;
    }
    Ok(q)
}

/// The levers of plan 4, one at a time from the shipped defaults.
const LEVERS: &[&str] = &[
    "margin_g=2,4,8,16",
    "margin_g=4,8,16;margin_shape=logistic",
    "k_r=0.02,0.05,0.1,0.2",
    "k_r=0.02,0.05,0.1,0.2;k_r_target=usual",
    "n0=3,6,10",
    "w_sync=0.25,0.5,0.75",
    "w_strict_sync=0.25,0.5,0.75",
    "K=0.2,0.3,0.4,0.6,0.8",
    "k_q=0.003,0.01,0.03,0.1,0.3",
    "idle_drop=100,200",
];

// -------------------------------------------------------------- the metric

/// One tournament's field, as the metric sees it after the replay.
#[derive(Default)]
struct Field {
    team: Vec<i64>,
    mu: Vec<f64>,
    place: Vec<f64>,
}

struct Run {
    /// NDCG and NDCG@3 per tournament, per window.
    full: [HashMap<i64, f64>; 2],
    at3: [HashMap<i64, f64>; 2],
    sole: i64,
    shared: i64,
}

/// Everything about a tournament the replay needs, computed once.
struct Cached {
    id: i64,
    day: i32,
    format: Format,
    weight: f64,
    rows: Vec<Row>,
}

fn windows() -> [(i32, i32); 2] {
    [
        (parse_date(TUNE.0).unwrap(), parse_date(TUNE.1).unwrap()),
        (parse_date(TEST.0).unwrap(), parse_date(TEST.1).unwrap()),
    ]
}

fn replay(cache: &[Cached], arena: &Arena, qual: &HashSet<(i64, i64)>, p: &Point) -> Run {
    let mut teams = TeamsS2::new(arena.n_teams());
    let win = windows();
    let mut run = Run {
        full: [HashMap::new(), HashMap::new()],
        at3: [HashMap::new(), HashMap::new()],
        sole: 0,
        shared: 0,
    };
    for t in cache {
        let evs = teams.step_rows(t.day, t.weight, t.format, &t.rows, &p.s2);
        // The winner rule: a team that finished first never loses points.
        let best = evs
            .iter()
            .filter(|e| e.actual_place > 0.0)
            .map(|e| e.actual_place)
            .fold(f64::INFINITY, f64::min);
        let n_best = evs.iter().filter(|e| e.actual_place == best).count();
        for e in evs.iter().filter(|e| e.actual_place == best && e.rated) {
            if e.mu_after < e.mu_before - 1e-9 {
                if n_best == 1 {
                    run.sole += 1;
                } else {
                    run.shared += 1;
                }
            }
        }
        if t.format != Format::InPerson {
            continue;
        }
        let Some(w) = win.iter().position(|&(a, b)| t.day >= a && t.day < b) else {
            continue;
        };
        let mut f = Field::default();
        for e in &evs {
            let team = arena.team_ids[e.team as usize];
            if !e.rated || e.actual_place <= 0.0 || !e.mu_before.is_finite() {
                continue;
            }
            if !qual.contains(&(t.id, team)) {
                continue;
            }
            f.team.push(team);
            // Lever 6: an untested rating counts lower in the ranking alone.
            f.mu.push(e.mu_before - p.idle_drop * (e.gap_days as f64 / 365.0).max(0.0));
            f.place.push(e.actual_place);
        }
        if f.team.len() < 5 {
            continue;
        }
        if let Some(v) = metric::score(t.id, &f.team, &f.mu, &f.place, usize::MAX) {
            run.full[w].insert(t.id, v);
        }
        if let Some(v) = metric::score(t.id, &f.team, &f.mu, &f.place, 3) {
            run.at3[w].insert(t.id, v);
        }
    }
    run
}

// ------------------------------------------------------------------ reports

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Paired bootstrap SE of the weighted difference a - b over tournaments.
fn boot(a: &HashMap<i64, f64>, b: &HashMap<i64, f64>, w: &HashMap<i64, f64>, n: usize) -> f64 {
    let keys: Vec<i64> = a.keys().filter(|k| b.contains_key(k)).copied().collect();
    if keys.len() < 5 || n == 0 {
        return f64::NAN;
    }
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut d = Vec::with_capacity(n);
    for _ in 0..n {
        let (mut num, mut den) = (0.0, 0.0);
        for _ in 0..keys.len() {
            let t = keys[(rng.next() % keys.len() as u64) as usize];
            let x = *w.get(&t).unwrap_or(&0.0);
            num += x * (a[&t] - b[&t]);
            den += x;
        }
        d.push(if den > 0.0 { num / den } else { 0.0 });
    }
    let m = d.iter().sum::<f64>() / d.len() as f64;
    (d.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / (d.len() - 1) as f64).sqrt()
}

fn header() -> String {
    let mut s = String::from(
        "| row | tune NDCG (w) | tune NDCG | tune @3 (w) | SE(w) | test NDCG (w) | test NDCG | test @3 (w) | winner rule |\n",
    );
    s.push_str("|---|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    s
}

fn line(name: &str, r: &Run, base: Option<&Run>, w: &HashMap<i64, f64>, nboot: usize) -> String {
    let se = base.map_or(f64::NAN, |b| boot(&r.at3[0], &b.at3[0], w, nboot));
    format!(
        "| {name} | {:.4} | {:.4} | {:.4} | {} | {:.4} | {:.4} | {:.4} | {} / {} |\n",
        metric::wmean(&r.full[0], w),
        metric::umean(&r.full[0]),
        metric::wmean(&r.at3[0], w),
        if se.is_finite() { format!("{se:.4}") } else { "-".into() },
        metric::wmean(&r.full[1], w),
        metric::umean(&r.full[1]),
        metric::wmean(&r.at3[1], w),
        r.sole,
        r.shared,
    )
}

// --------------------------------------------------------------------- main

fn build_cache(arena: &Arena, cfg: &Config) -> Vec<Cached> {
    let passes = cfg.data.passes;
    let from_day = parse_date(&cfg.data.from).unwrap();
    let warm_until = parse_date(&cfg.player.warm_until).unwrap();
    let zfrom = parse_date("2018-01-01").unwrap();
    let mut players = qm::Players::new(arena.n_players(), arena.n_editors(), cfg);
    let mut snap: Vec<Option<f64>> = Vec::new();
    let mut cache = Vec::new();
    let start = std::time::Instant::now();
    for pass in 0..passes {
        let last = pass + 1 == passes;
        if pass > 0 {
            players.warm_restart(&snap, from_day, cfg);
        }
        let mut ed_seen = vec![0.0f64; arena.n_editors()];
        let mut activity = Activity::new(arena.n_players());
        let mut next_rel = next_thursday(from_day);
        cache.clear();
        for (n, t) in arena.tournaments.iter().enumerate() {
            if snap.is_empty() && t.day >= warm_until {
                snap = players.snapshot();
            }
            // The weekly gauge re-centres every theta the team model then reads.
            while next_rel <= t.day {
                activity.expire_before(next_rel - cfg.gauge.window_days);
                players.shift_cum += gauge::shift(&players, &activity, cfg.gauge.n_est);
                next_rel += 7;
            }
            players.seed(t, cfg);
            let field = qm::build_field(t, &players, cfg);
            let params = qm::fit_all(&field, None, cfg);
            let grads = qm::player_grads(&field, &params, cfg);
            if last {
                cache.push(Cached {
                    id: t.id,
                    day: t.day,
                    format: t.format,
                    weight: t.weight,
                    rows: rows_of(t, &players, cfg),
                });
            }
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
    cache
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg_path = arg("--config").unwrap_or_else(|| "config.toml".into());
    let mut cfg = Config::load(&cfg_path)?;
    cfg.data.to = arg("--to").unwrap_or_else(|| "2026-09-05".into());
    let nboot: usize = arg("--boot").map_or(2000, |v| v.parse().unwrap_or(2000));
    let qual = metric::qualify(QUALIFY);
    let w = metric::weights(WEIGHTS);

    let arena = chgk_rating::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
    eprintln!(
        "loaded {} tournaments, {} teams; {} qualifying results, {} weights",
        arena.tournaments.len(),
        arena.n_teams(),
        qual.len(),
        w.len()
    );
    let cache = build_cache(&arena, &cfg);

    let base = apply(
        &Point { s2: cfg.team.s2.clone(), idle_drop: 0.0 },
        &arg("--base").unwrap_or_default(),
    )?;
    let base_run = replay(&cache, &arena, &qual, &base);

    let mut out = String::new();
    out.push_str(&header());
    out.push_str(&line("shipped (base)", &base_run, None, &w, 0));

    let axes = args_all("--axis");
    let points = args_all("--point");
    let specs: Vec<String> = if axes.is_empty() && points.is_empty() {
        LEVERS.iter().map(|s| s.to_string()).collect()
    } else {
        axes
    };
    for spec in &specs {
        // "k_r=0.02,0.05;k_r_target=usual": the first field is the axis, the rest
        // are held fixed for the whole scan.
        let mut parts = spec.split(';');
        let head = parts.next().unwrap();
        let fixed: Vec<&str> = parts.collect();
        let (key, vals) = head.split_once('=').ok_or("--axis expects name=v1,v2")?;
        for v in vals.split(',') {
            let mut p = apply(&base, &format!("{key}={v}"))?;
            for f in &fixed {
                p = apply(&p, f)?;
            }
            let name = if fixed.is_empty() {
                format!("`{key} = {v}`")
            } else {
                format!("`{key} = {v}`, `{}`", fixed.join("`, `"))
            };
            let r = replay(&cache, &arena, &qual, &p);
            out.push_str(&line(&name, &r, Some(&base_run), &w, nboot));
        }
    }
    for spec in &points {
        let p = apply(&base, spec)?;
        let r = replay(&cache, &arena, &qual, &p);
        out.push_str(&line(&format!("`{spec}`"), &r, Some(&base_run), &w, nboot));
    }

    print!("{out}");
    if let Some(path) = arg("--out") {
        std::fs::write(&path, &out)?;
        eprintln!("wrote {path}");
    }
    Ok(())
}
