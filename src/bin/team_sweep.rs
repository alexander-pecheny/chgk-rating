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
use chgk_rating::pass::{self, Morning, Observer};
use chgk_rating::team_event::TeamEvent;
use chgk_rating::team_ndcg::{self as metric, QUALIFY, TEST, TUNE, WEIGHTS};
use chgk_rating::team_s2::{lineup_strength, Row, TeamsS2};
use chgk_rating::util::{format_date, parse_date};
use std::collections::{HashMap, HashSet};

/// The sweep stops here unless `--to` says otherwise.
const TO: &str = "2026-09-05";
/// Bootstrap draws of the standard error, unless `--boot` says otherwise.
const BOOT: usize = 2000;
/// The guest and legionnaire checks read results from this day on.
const CHECKS_FROM: &str = "2018-09-01";
/// A loss counts as a guest loss when the lineup is this far above the team's core.
const GUEST_GAP: f64 = 600.0;
/// A loss counts as a legionnaire loss when the guests lift the lineup this far
/// over its base-roster players alone.
const LEGIONNAIRE_LIFT: f64 = 300.0;
/// A winner "lost points" only beyond rounding.
const LOSS_EPS: f64 = 1e-9;
/// A field needs this many qualifying teams to be scored, and NDCG@3 reads its podium.
const MIN_FIELD: usize = 5;
const PODIUM: usize = 3;
const YEAR: f64 = 365.0;
/// The table of `--top`: teams with this many rated games, active in the trailing
/// year (`TOP_ALL=1`: in the last century, so however long idle).
const TOP_MIN_GAMES: f64 = 5.0;
const TOP_WINDOW: i32 = 365;
const TOP_WINDOW_ALL: i32 = 100 * 365;
/// The bootstrap's seed, so its standard errors are the same on every run.
const SEED: u64 = 0x1234_5678_9ABC_DEF1;

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
    if set_switch(&mut p.s2, key, val == "true") || set_choice(&mut p.s2, key, val)? {
        return Ok(());
    }
    let x = val.parse::<f64>().map_err(|_| format!("{key}: not a number: {val}"))?;
    if key == "idle_drop" {
        p.idle_drop = x;
        return Ok(());
    }
    set_number(&mut p.s2, key, x)
}

/// The on/off rules; false when `key` is not one.
fn set_switch(s: &mut S2, key: &str, on: bool) -> bool {
    let field = match key {
        "winner_floor" => &mut s.winner_floor,
        "two_part" => &mut s.two_part,
        "core_live" => &mut s.core_live,
        "core_top" => &mut s.core_top,
        "core_base" => &mut s.core_base,
        "field_sync" => &mut s.field_sync,
        "place_sign" => &mut s.place_sign,
        "ref_core" => &mut s.ref_core,
        "info_norm" => &mut s.info_norm,
        "expect_forecast" => &mut s.expect_forecast,
        "forecast_rating" => &mut s.forecast_rating,
        "adj_mult" => &mut s.adj_mult,
        _ => return false,
    };
    *field = on;
    true
}

/// The rules with named options; false when `key` is not one.
fn set_choice(s: &mut S2, key: &str, val: &str) -> Result<bool, String> {
    match (key, val) {
        ("margin_shape", "linear") => s.margin_shape = MarginShape::Linear,
        ("margin_shape", "logistic") => s.margin_shape = MarginShape::Logistic,
        ("margin_shape", _) => return Err("margin_shape: linear or logistic".into()),
        ("k_r_target", "lineup") => s.k_r_target = PullTarget::Lineup,
        ("k_r_target", "usual") => s.k_r_target = PullTarget::Usual,
        ("k_r_target", _) => return Err("k_r_target: lineup or usual".into()),
        _ => return Ok(false),
    }
    Ok(true)
}

fn set_number(s: &mut S2, key: &str, x: f64) -> Result<(), String> {
    match key {
        "K" => s.k = x,
        "k_q" => s.k_q = x,
        "D" => s.d = x,
        "margin_g" => s.margin_g = x,
        "k_r" => s.k_r = x,
        "k_r_sync" => s.k_r_sync = x,
        "n0" => s.n0 = x,
        "w_sync" => s.w_sync = Some(x),
        "w_strict_sync" => s.w_strict_sync = Some(x),
        "k_c" => s.k_c = x,
        "k_b" => s.k_b = x,
        "core_min_w" => s.core_min_w = x,
        "field_gamma" => s.field_gamma = x,
        "field_k" => s.field_k = x as usize,
        "field_ref" => s.field_ref = x,
        "lineup_adj" => s.lineup_adj = x,
        "info_min" => s.info_min = x,
        "start_drop" => s.start_drop = x,
        "start_frac" => s.start_frac = x,
        "best_n" => s.best_n = x as usize,
        "best_window" => s.best_window = x as i32,
        "perf_prior" => s.perf_prior = x,
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

/// The levers of plan 4, one at a time from whatever the config holds.
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

#[derive(Default)]
struct Run {
    /// NDCG and NDCG@3 per tournament, per window.
    full: [HashMap<i64, f64>; 2],
    at3: [HashMap<i64, f64>; 2],
    /// The forecast NDCG of `eval --ndcg`, per window, over every in-person field.
    fc: [HashMap<i64, f64>; 2],
    sole: i64,
    shared: i64,
    /// Rated losses (a negative placement step) since 2018-09-01 at a lineup
    /// 600 points or more above the team's own core, and how many of them still
    /// raised the rating.
    guest_losses: i64,
    guest_up: i64,
    /// Mean parts of the move over those losses: placement, roster drift,
    /// attendance, and the rest (the shrink of `b` and the floor).
    guest_parts: [f64; 4],
    /// Rated losses since 2018-09-01 where the legionnaires lift the lineup by
    /// 300 points or more over its base-roster players alone, the share that
    /// still raised the rating, and the mean move.
    leg_losses: i64,
    leg_up: i64,
    leg_move: f64,
}

/// Everything about a tournament the replay needs, computed once.
struct Cached {
    id: i64,
    day: i32,
    format: Format,
    weight: f64,
    rows: Vec<Row>,
}

/// The pass's output and what every replay scores against.
struct Replay<'a> {
    cache: &'a [Cached],
    arena: &'a Arena,
    qual: &'a HashSet<(i64, i64)>,
    windows: [(i32, i32); 2],
    since: i32,
}

/// A rated loss: a negative placement step.
fn rated_loss(e: &TeamEvent) -> bool {
    e.rated && e.d_place < 0.0
}

fn rose(e: &TeamEvent) -> i64 {
    (e.mu_after > e.mu_before) as i64
}

impl Replay<'_> {
    fn new<'a>(cache: &'a [Cached], arena: &'a Arena, qual: &'a HashSet<(i64, i64)>) -> Replay<'a> {
        let day = |s: &str| parse_date(s).unwrap();
        let windows = [(day(TUNE.0), day(TUNE.1)), (day(TEST.0), day(TEST.1))];
        Replay { cache, arena, qual, windows, since: day(CHECKS_FROM) }
    }

    fn run(&self, p: &Point) -> Run {
        self.teams(p).0
    }

    fn teams(&self, p: &Point) -> (Run, TeamsS2) {
        let mut teams = TeamsS2::new(self.arena.n_teams());
        let mut run = Run::default();
        for t in self.cache {
            let evs = teams.step_rows(t.id, t.day, t.weight, t.format, &t.rows, &p.s2);
            winner_rule(&mut run, &evs);
            if t.day >= self.since {
                legionnaire_losses(&mut run, t, &evs, &p.s2);
                guest_losses(&mut run, &evs, &teams.guest_gap);
            }
            self.score(&mut run, t, &evs, p);
        }
        (run, teams)
    }

    /// The forecast NDCG and the team NDCG of an in-person field in a window.
    fn score(&self, run: &mut Run, t: &Cached, evs: &[TeamEvent], p: &Point) {
        if t.format != Format::InPerson {
            return;
        }
        let Some(w) = self.windows.iter().position(|&(a, b)| t.day >= a && t.day < b) else { return };
        let field: Vec<(f64, f64)> = evs
            .iter()
            .filter(|e| e.actual_place > 0.0 && e.forecast.is_finite())
            .map(|e| (e.forecast, e.actual_place))
            .collect();
        if let Some(s) = chgk_rating::ndcg::score_field(t.id, &field).filter(|s| s.ndcg.is_finite()) {
            run.fc[w].insert(t.id, s.ndcg);
        }
        let f = self.field(t, evs, p);
        if f.team.len() < MIN_FIELD {
            return;
        }
        if let Some(v) = metric::score(t.id, &f.team, &f.mu, &f.place, usize::MAX) {
            run.full[w].insert(t.id, v);
        }
        if let Some(v) = metric::score(t.id, &f.team, &f.mu, &f.place, PODIUM) {
            run.at3[w].insert(t.id, v);
        }
    }

    /// The qualifying rated placed teams of a field, ranked by rating alone.
    fn field(&self, t: &Cached, evs: &[TeamEvent], p: &Point) -> Field {
        let mut f = Field::default();
        for e in evs {
            let team = self.arena.team_ids[e.team as usize];
            if !e.rated || e.actual_place <= 0.0 || !e.mu_before.is_finite() || !self.qual.contains(&(t.id, team)) {
                continue;
            }
            f.team.push(team);
            // Lever 6: an untested rating counts lower in the ranking alone.
            f.mu.push(e.mu_before - p.idle_drop * (e.gap_days as f64 / YEAR).max(0.0));
            f.place.push(e.actual_place);
        }
        f
    }
}

/// The winner rule: a team that finished first never loses points.
fn winner_rule(run: &mut Run, evs: &[TeamEvent]) {
    let best = evs.iter().filter(|e| e.actual_place > 0.0).map(|e| e.actual_place).fold(f64::INFINITY, f64::min);
    let n_best = evs.iter().filter(|e| e.actual_place == best).count();
    for e in evs.iter().filter(|e| e.actual_place == best && e.rated) {
        if e.mu_after < e.mu_before - LOSS_EPS {
            if n_best == 1 {
                run.sole += 1;
            } else {
                run.shared += 1;
            }
        }
    }
}

fn legionnaire_losses(run: &mut Run, t: &Cached, evs: &[TeamEvent], s2: &S2) {
    let by_team: HashMap<u32, &Row> = t.rows.iter().map(|r| (r.team, r)).collect();
    for e in evs.iter().filter(|e| rated_loss(e)) {
        let Some(r) = by_team.get(&e.team) else { continue };
        let base: Vec<f64> = r.member_r.iter().zip(&r.member_base).filter(|(_, &b)| b).map(|(&x, _)| x).collect();
        if base.is_empty() || base.len() == r.member_r.len() {
            continue;
        }
        if lineup_strength(&r.member_r, s2) - lineup_strength(&base, s2) >= LEGIONNAIRE_LIFT {
            run.leg_losses += 1;
            run.leg_up += rose(e);
            run.leg_move += e.mu_after - e.mu_before;
        }
    }
}

fn guest_losses(run: &mut Run, evs: &[TeamEvent], guest_gap: &[f64]) {
    for (e, _) in evs.iter().zip(guest_gap).filter(|(e, &g)| rated_loss(e) && g > GUEST_GAP) {
        run.guest_losses += 1;
        run.guest_up += rose(e);
        let tot = e.mu_after - e.mu_before;
        let parts = [e.d_place, e.var_diff, e.lost_prev, tot - e.d_place - e.var_diff - e.lost_prev];
        for (a, x) in run.guest_parts.iter_mut().zip(parts) {
            *a += x;
        }
    }
}

/// The team table at the end of the replay: the `n` best teams with five rated
/// games or more and one in the trailing year, as `team_id<TAB>rating<TAB>games`.
fn top(teams: &TeamsS2, arena: &Arena, day: i32, n: usize) -> String {
    let window = if std::env::var("TOP_ALL").is_ok() { TOP_WINDOW_ALL } else { TOP_WINDOW };
    let mut v: Vec<_> = teams.rows(day, window).into_iter().filter(|r| r.4 >= TOP_MIN_GAMES).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    let line = |r: &(usize, f64, f64, f64, f64)| {
        let last = teams.state_of(r.0).map_or(0, |s| s.3);
        format!("{}\t{:.0}\t{:.0}\t{}\n", arena.team_ids[r.0], r.1, r.4, format_date(last))
    };
    v.iter().take(n).map(line).collect()
}

// ------------------------------------------------------------------ reports

/// Marsaglia's xorshift64 shifts.
const XORSHIFT: (u32, u32, u32) = (13, 7, 17);

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << XORSHIFT.0;
        self.0 ^= self.0 >> XORSHIFT.1;
        self.0 ^= self.0 << XORSHIFT.2;
        self.0
    }
}

/// Paired bootstrap SE of the weighted difference a - b over tournaments.
fn boot(a: &HashMap<i64, f64>, b: &HashMap<i64, f64>, w: &HashMap<i64, f64>, n: usize) -> f64 {
    let keys: Vec<i64> = a.keys().filter(|k| b.contains_key(k)).copied().collect();
    if keys.len() < MIN_FIELD || n == 0 {
        return f64::NAN;
    }
    let mut rng = Rng(SEED);
    let mut draw = || {
        let (mut num, mut den) = (0.0, 0.0);
        for _ in 0..keys.len() {
            let t = keys[(rng.next() % keys.len() as u64) as usize];
            let x = *w.get(&t).unwrap_or(&0.0);
            num += x * (a[&t] - b[&t]);
            den += x;
        }
        if den > 0.0 { num / den } else { 0.0 }
    };
    let d: Vec<f64> = (0..n).map(|_| draw()).collect();
    let m = d.iter().sum::<f64>() / d.len() as f64;
    (d.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / (d.len() - 1) as f64).sqrt()
}

fn header() -> String {
    let mut s = String::from(
        "| row | tune NDCG (w) | tune NDCG | tune @3 (w) | SE(w) | test NDCG (w) | test NDCG | test @3 (w) | winner rule | guest losses up | forecast tune (w) | forecast test (w) |\n",
    );
    s.push_str("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    s
}

fn line(name: &str, r: &Run, base: Option<&Run>, w: &HashMap<i64, f64>, nboot: usize) -> String {
    let se = base.map_or(f64::NAN, |b| boot(&r.at3[0], &b.at3[0], w, nboot));
    format!(
        "| {name} | {:.4} | {:.4} | {:.4} | {} | {:.4} | {:.4} | {:.4} | {} / {} | {:.1}% of {} | {:.4} | {:.4} |\n",
        metric::wmean(&r.full[0], w),
        metric::umean(&r.full[0]),
        metric::wmean(&r.at3[0], w),
        if se.is_finite() { format!("{se:.4}") } else { "-".into() },
        metric::wmean(&r.full[1], w),
        metric::umean(&r.full[1]),
        metric::wmean(&r.at3[1], w),
        r.sole,
        r.shared,
        100.0 * r.guest_up as f64 / r.guest_losses.max(1) as f64,
        r.guest_losses,
        metric::wmean(&r.fc[0], w),
        metric::wmean(&r.fc[1], w),
    ) + &loss_note(r)
}

/// The guest and legionnaire losses behind a row, as a Markdown comment.
fn loss_note(r: &Run) -> String {
    let n = r.guest_losses.max(1) as f64;
    let g = r.guest_parts.map(|x| x / n);
    let legs = r.leg_losses.max(1) as f64;
    format!(
        "<!-- guest-loss parts: place {:.0} drift {:.0} attend {:.0} rest {:.0}; legionnaire losses: {:.1}% up of {}, mean move {:.0} -->\n",
        g[0],
        g[1],
        g[2],
        g[3],
        100.0 * r.leg_up as f64 / legs,
        r.leg_losses,
        r.leg_move / legs
    )
}

// --------------------------------------------------------------------- main

/// The lineup strengths of every field as the shipped pass computed them, for
/// the team rules to be replayed over.
fn build_cache(arena: &Arena, cfg: &Config) -> pass::Result<Vec<Cached>> {
    struct Cache(Vec<Cached>);
    impl Observer for Cache {
        fn morning(&mut self, m: &Morning) -> pass::Result<()> {
            let t = m.t;
            let rows = if t.train_only { Vec::new() } else { m.rows.to_vec() };
            self.0.push(Cached { id: t.id, day: t.day, format: t.format, weight: t.weight, rows });
            Ok(())
        }
    }
    let mut cache = Cache(Vec::new());
    pass::run(arena, cfg, pass::Options::default(), &mut cache)?;
    Ok(cache.0)
}

/// What every row of the table is read against.
struct Sweep<'a> {
    replay: Replay<'a>,
    base: Point,
    base_run: Run,
    w: HashMap<i64, f64>,
    nboot: usize,
}

impl Sweep<'_> {
    /// "k_r=0.02,0.05;k_r_target=usual": the first field is the axis, the rest
    /// are held fixed for the whole scan.
    fn axis(&self, spec: &str, out: &mut String) -> Result<(), Box<dyn std::error::Error>> {
        let mut parts = spec.split(';');
        let head = parts.next().unwrap();
        let fixed: Vec<&str> = parts.collect();
        let (key, vals) = head.split_once('=').ok_or("--axis expects name=v1,v2")?;
        for v in vals.split(',') {
            let mut p = apply(&self.base, &format!("{key}={v}"))?;
            for f in &fixed {
                p = apply(&p, f)?;
            }
            let name = if fixed.is_empty() {
                format!("`{key} = {v}`")
            } else {
                format!("`{key} = {v}`, `{}`", fixed.join("`, `"))
            };
            out.push_str(&line(&name, &self.replay.run(&p), Some(&self.base_run), &self.w, self.nboot));
        }
        Ok(())
    }

    /// One `--point`: its row, `--explain`'s lines, and `--top`'s table.
    fn point(&self, k: usize, spec: &str, out: &mut String) -> Result<(), Box<dyn std::error::Error>> {
        let p = apply(&self.base, spec)?;
        let (r, teams) = self.replay.teams(&p);
        out.push_str(&line(&format!("`{spec}`"), &r, Some(&self.base_run), &self.w, self.nboot));
        for id in explained() {
            let Some(i) = self.replay.arena.team_ids.iter().position(|&x| x == id) else { continue };
            explain(spec, id, i, &teams, &p.s2);
        }
        self.write_top(&teams, &format!("point{k}.tsv"))
    }

    /// `--top n --top-dir dir`: the replay's table into `dir/name`.
    fn write_top(&self, teams: &TeamsS2, name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let top_n: Option<usize> = arg("--top").and_then(|v| v.parse().ok());
        if let (Some(n), Some(dir)) = (top_n, arg("--top-dir")) {
            let last_day = self.replay.cache.last().map_or(0, |t| t.day);
            std::fs::write(format!("{dir}/{name}"), top(teams, self.replay.arena, last_day, n))?;
        }
        Ok(())
    }
}

/// `--explain 49804,27177`: the teams whose two parts are shown at every point.
fn explained() -> Vec<i64> {
    arg("--explain").map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default()
}

fn explain(spec: &str, id: i64, i: usize, teams: &TeamsS2, s2: &S2) {
    if s2.best_n > 0 {
        let mut v: Vec<(i32, f64)> = teams.pool_of(i).to_vec();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        let s: Vec<String> = v.iter().map(|(d, x)| format!("{} {:.0}", format_date(*d), x)).collect();
        eprintln!("{spec} team {id}: pool {} — {}", v.len(), s.join(", "));
    }
    if let (Some((q, b)), Some((rt, _, g, _))) = (teams.parts_of(i, s2), teams.state_of(i)) {
        eprintln!("{spec} team {id}: R {rt:.0} = core {q:.0} + b {b:+.0}, games {g:.0}");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg_path = arg("--config").unwrap_or_else(|| "config.toml".into());
    let mut cfg = Config::load(&cfg_path)?;
    cfg.data.to = arg("--to").unwrap_or_else(|| TO.into());
    let qual = metric::qualify(QUALIFY);
    let w = metric::weights(WEIGHTS);
    let arena = chgk_rating::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
    let (nt, nteams) = (arena.tournaments.len(), arena.n_teams());
    eprintln!("loaded {nt} tournaments, {nteams} teams; {} qualifying results, {} weights", qual.len(), w.len());
    let cache = build_cache(&arena, &cfg)?;
    let replay = Replay::new(&cache, &arena, &qual);
    let base = apply(&Point { s2: cfg.team.s2.clone(), idle_drop: 0.0 }, &arg("--base").unwrap_or_default())?;
    let base_run = replay.run(&base);
    let nboot: usize = arg("--boot").map_or(BOOT, |v| v.parse().unwrap_or(BOOT));
    let sweep = Sweep { replay, base, base_run, w, nboot };
    let out = sweep.table()?;
    print!("{out}");
    if let Some(path) = arg("--out") {
        std::fs::write(&path, &out)?;
        eprintln!("wrote {path}");
    }
    Ok(())
}

impl Sweep<'_> {
    /// The base row, every axis (the levers of plan 4 when nothing is asked),
    /// then every point.
    fn table(&self) -> Result<String, Box<dyn std::error::Error>> {
        let mut out = header();
        out.push_str(&line("base (the config, plus --base)", &self.base_run, None, &self.w, 0));
        let (axes, points) = (args_all("--axis"), args_all("--point"));
        let specs: Vec<String> =
            if axes.is_empty() && points.is_empty() { LEVERS.iter().map(|s| s.to_string()).collect() } else { axes };
        for spec in &specs {
            self.axis(spec, &mut out)?;
        }
        if arg("--top").and_then(|v| v.parse::<usize>().ok()).is_some() && arg("--top-dir").is_some() {
            self.write_top(&self.replay.teams(&self.base).1, "base.tsv")?;
        }
        for (k, spec) in points.iter().enumerate() {
            self.point(k, spec, &mut out)?;
        }
        Ok(out)
    }
}
