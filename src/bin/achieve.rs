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
use chgk_rating::team_s2::{lineup_strength, player_rating, rows_of, TeamsS2};
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
/// The combined strength of the 20 strongest base rosters of the season on
/// `day`, on the published scale, each roster read over its six strongest
/// known players the way a lineup is: in-person strength when `ip`, the plain
/// rating otherwise.
fn ref20(rosters: &[Vec<u32>], players: &qm::Players, day: i32, ip: bool, cfg: &Config) -> f64 {
    let p = &cfg.team.s2;
    let disp = |x: f64| cfg.display.a_team + cfg.display.b_team * x;
    let mut qs: Vec<f64> = rosters
        .iter()
        .filter_map(|r| {
            let mut rs: Vec<f64> = r
                .iter()
                .map(|&i| i as usize)
                .filter(|&i| players.state[i].seen)
                .map(|i| {
                    let th = players.theta(i) + if ip { players.phi(i) } else { 0.0 };
                    player_rating(th - p.sigma_c * players.sigma_at(i, day, cfg), cfg)
                })
                .collect();
            if rs.len() < 4 {
                return None;
            }
            rs.sort_by(|a, b| b.total_cmp(a));
            rs.truncate(6);
            Some(disp(lineup_strength(&rs, p)).max(0.0))
        })
        .collect();
    qs.sort_by(|a, b| b.total_cmp(a));
    qs.iter().take(20).sum()
}

fn build(arena: &Arena, cfg: &Config, seasons: &Seasons, geo: &HashMap<String, (i64, i64)>) -> Vec<Res> {
    // Each town of the arena as (town id, country id), -1 where unknown.
    let town_geo: Vec<(i64, i64)> =
        arena.towns.iter().map(|n| geo.get(n).copied().unwrap_or((-1, -1))).collect();
    // Every season's base rosters as player indices of this arena.
    let pidx: HashMap<i64, u32> = arena.player_ids.iter().enumerate().map(|(i, &p)| (p, i as u32)).collect();
    let mut by_season: HashMap<i64, Vec<Vec<u32>>> = HashMap::new();
    for ((_, s), v) in &seasons.players {
        let r: Vec<u32> = v.iter().filter_map(|p| pidx.get(p).copied()).collect();
        if r.len() >= 4 {
            by_season.entry(*s).or_default().push(r);
        }
    }
    let empty = Vec::new();
    // Recomputed once a week, like a release.
    let mut refs: Option<(i32, f64, f64)> = None;
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
                let week = next_rel;
                if refs.map_or(true, |r| r.0 != week) {
                    let season = seasons.season_of(t.day).unwrap_or(-1);
                    let rs = by_season.get(&season).unwrap_or(&empty);
                    refs = Some((
                        week,
                        ref20(rs, &players, t.day, true, cfg),
                        ref20(rs, &players, t.day, false, cfg),
                    ));
                }
                let (_, ref_ip, ref_sync) = refs.unwrap();
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
                        ref20: if t.format == Format::InPerson { ref_ip } else { ref_sync },
                        town: town_geo.get(r.town as usize).map_or(-1, |g| g.0),
                        country: town_geo.get(r.town as usize).map_or(-1, |g| g.1),
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
    // Each player's last published rating, from the fit the site serves.
    let strength: HashMap<i64, f64> = Connection::open("out/ratings.db")
        .and_then(|c| {
            c.prepare("SELECT player_id, display, max(release_id) FROM player_ratings GROUP BY player_id")?
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))?
                .collect::<Result<_, _>>()
        })
        .unwrap_or_default();
    eprintln!("player strengths: {}", strength.len());
    Seasons { list, has, players, strength }
}

// ------------------------------------------------------------------- search

/// xorshift, seeded, so a search is reproducible.
struct Rand(u64);

impl Rand {
    fn u(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.u()
    }
    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v[((self.u() * v.len() as f64) as usize).min(v.len() - 1)]
    }
}

/// Every knob that can change the order of a field, with how to draw it. The
/// listing rules (grace, activity) and the display scale cannot, so they are
/// not searched.
fn draw(key: &str, r: &mut Rand) -> String {
    let int = |r: &mut Rand, a: f64, b: f64| (r.range(a, b + 1.0).floor()).to_string();
    match key {
        "top_x" => int(r, 3.0, 15.0),
        "min_slots" => int(r, 0.0, 15.0),
        "perf" => r.pick(&["strength_place", "strength_place", "mix", "score", "place"]).into(),
        "place_pow" => format!("{:.2}", r.range(0.0, 1.2)),
        "mix" => format!("{:.2}", r.range(0.0, 1.0)),
        "w_sync" => format!("{:.2}", r.range(0.0, 1.2)),
        "w_strict" => format!("{:.2}", r.range(0.0, 1.2)),
        "field_mode" => r.pick(&["sum", "world", "anchors"]).into(),
        "field_k" => r.pick(&["5", "10", "20", "30"]).into(),
        "field_gamma" => format!("{:.2}", r.range(0.0, 5.0)),
        "field_zero" => format!("{:.0}", r.range(0.0, 6000.0)),
        "field_ref" => format!("{:.0}", r.range(8000.0, 11000.0)),
        "sum_ref" => format!("{:.0}", (r.range(12.0, 14.6)).exp()),
        "sum_gamma" => format!("{:.2}", r.range(0.0, 1.5)),
        "country_pow" => format!("{:.2}", if r.u() < 0.4 { 0.0 } else { r.range(0.0, 1.0) }),
        "country_min" => int(r, 1.0, 3.0),
        "town_pow" => format!("{:.2}", if r.u() < 0.4 { 0.0 } else { r.range(0.0, 0.8) }),
        "town_min" => int(r, 1.0, 3.0),
        "country_ref" => format!("{:.1}", r.range(1.0, 10.0)),
        "town_ref" => format!("{:.1}", r.range(1.0, 25.0)),
        "geo_cap" => r.pick(&["0", "1", "1"]).into(),
        "fade" => r.pick(&["yearly", "yearly", "exp", "linear", "none"]).into(),
        "loss_ip" => format!("{:.2}", r.range(0.0, 0.7)),
        "loss_sync" => format!("{:.2}", r.range(0.0, 0.95)),
        "full_days" => int(r, 0.0, 730.0),
        "zero_days" => int(r, 400.0, 2000.0),
        "half_life" => int(r, 60.0, 1500.0),
        "sync_full" => int(r, 0.0, 365.0),
        "sync_zero" => int(r, 200.0, 1000.0),
        "sync_half_life" => int(r, 30.0, 700.0),
        "tail_days" => int(r, 730.0, 5000.0),
        "slow_k" => format!("{:.2}", r.range(0.0, 2.0)),
        "overlap_k" => format!("{:.2}", if r.u() < 0.3 { 0.0 } else { r.range(0.0, 2.0) }),
        "overlap_by" => r.pick(&["strength", "count"]).into(),
        "base_rule" => r.pick(&["0", "1"]).into(),
        "win_bonus" => format!("{:.0}", if r.u() < 0.5 { 0.0 } else { r.range(0.0, 2000.0) }),
        _ => String::new(),
    }
}

const SEARCHED: &[&str] = &[
    "top_x", "min_slots", "perf", "place_pow", "mix", "w_sync", "w_strict", "field_mode", "field_k",
    "field_gamma", "field_zero", "field_ref", "sum_ref", "sum_gamma", "country_pow", "country_min",
    "town_pow", "town_min", "country_ref", "town_ref", "geo_cap", "fade", "loss_ip", "loss_sync", "full_days", "zero_days", "half_life",
    "sync_full", "sync_zero", "sync_half_life", "tail_days", "slow_k", "overlap_k", "overlap_by",
    "base_rule", "win_bonus",
];

/// A drawn point, with the knobs `fixed` holds on top of it.
fn knobs_of(vals: &[String], fixed: &str) -> Knobs {
    let mut k = Knobs::default();
    for (key, v) in SEARCHED.iter().zip(vals) {
        let _ = k.set(key, v);
    }
    k.apply(fixed).unwrap_or(k)
}

/// Random search, then local refinement of the best on the tune window; every
/// point scored on both windows and written as a row of `path`.
fn search(lab: &Lab, n: usize, rounds: usize, seed: u64, threads: usize, fixed: &str, path: &str) -> std::io::Result<()> {
    use rayon::prelude::*;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    let score = |pts: &[Vec<String>]| -> Vec<(f64, f64)> {
        pool.install(|| {
            pts.par_iter()
                .map(|v| {
                    let k = knobs_of(v, fixed);
                    let b = ac::book(lab, &k);
                    ac::ndcg_of(lab, &b, &k)
                })
                .collect()
        })
    };
    let mut out = String::new();
    let _ = writeln!(out, "phase\ttune\ttest\t{}", SEARCHED.join("\t"));
    let mut all: Vec<(Vec<String>, f64, f64)> = Vec::new();
    type Pt = (Vec<String>, f64, f64);
    let shown = |v: &Vec<String>| -> Vec<String> {
        let k = knobs_of(v, fixed);
        SEARCHED.iter().map(|key| k.get(key)).collect()
    };
    fn record(all: &mut Vec<Pt>, phase: &str, pts: Vec<Vec<String>>, sc: Vec<(f64, f64)>, out: &mut String) {
        for (v, (a, b)) in pts.into_iter().zip(sc) {
            let _ = writeln!(out, "{phase}\t{a:.5}\t{b:.5}\t{}", v.join("\t"));
            all.push((v, a, b));
        }
    }
    // The default first, as the reference row.
    let d = Knobs::default();
    let def: Vec<String> = SEARCHED.iter().map(|k| d.get(k)).collect();
    let sc = score(&[def.clone()]);
    record(&mut all, "default", vec![def], sc, &mut out);
    let mut r = Rand(seed | 1);
    let t0 = std::time::Instant::now();
    for chunk in 0..n.div_ceil(200) {
        let pts: Vec<Vec<String>> = (0..200.min(n - chunk * 200))
            .map(|_| SEARCHED.iter().map(|k| draw(k, &mut r)).collect())
            .collect();
        let sc = score(&pts);
        let pts: Vec<Vec<String>> = pts.iter().map(&shown).collect();
        record(&mut all, "random", pts, sc, &mut out);
        std::fs::write(path, &out)?;
        let best = all.iter().map(|x| x.1).fold(f64::MIN, f64::max);
        eprintln!("random {} / {n}, best tune {best:.4}, {:.0}s", all.len() - 1, t0.elapsed().as_secs_f64());
    }
    // Local refinement: mutate a few knobs of the best eight at a time.
    for round in 0..rounds {
        let mut best: Vec<&(Vec<String>, f64, f64)> = all.iter().collect();
        best.sort_by(|a, b| b.1.total_cmp(&a.1));
        let parents: Vec<Vec<String>> = best.iter().take(8).map(|x| x.0.clone()).collect();
        let pts: Vec<Vec<String>> = (0..96)
            .map(|i| {
                let mut v = parents[i % parents.len()].clone();
                let m = 1 + (r.u() * 3.0) as usize;
                for _ in 0..m {
                    let j = ((r.u() * SEARCHED.len() as f64) as usize).min(SEARCHED.len() - 1);
                    v[j] = draw(SEARCHED[j], &mut r);
                }
                v
            })
            .collect();
        let sc = score(&pts);
        let pts: Vec<Vec<String>> = pts.iter().map(&shown).collect();
        record(&mut all, "local", pts, sc, &mut out);
        std::fs::write(path, &out)?;
        let b = all.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        eprintln!("local {}/{rounds}, best tune {:.4} (test {:.4}), {:.0}s", round + 1, b.1, b.2, t0.elapsed().as_secs_f64());
    }
    Ok(())
}

// ------------------------------------------------------------------ publish

/// Replace the team side of a fit's database with the achievement rating:
/// `team_ratings` at every release, each team's rating before and after every
/// event in `team_events`, and two tables the site explains a rating from —
/// `achieve_values`, what every counted result was worth on the day, and
/// `achieve_parts`, which of them make each rating at the last release.
fn publish(lab: &Lab, k: &Knobs, db: &str, cfg: &Config, threads: usize) -> rusqlite::Result<()> {
    use rayon::prelude::*;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    let (a, bt) = (cfg.display.a_team, cfg.display.b_team);
    let mu = |x: f64| (x - a) / bt;
    let t0 = std::time::Instant::now();
    let b = ac::book(lab, k);
    let mut conn = Connection::open(db)?;
    let releases: Vec<(i64, i32)> = conn
        .prepare("SELECT id, date FROM releases ORDER BY date")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|x| x.ok())
        .filter_map(|(i, d)| Some((i, parse_date(&d)?)))
        .collect();
    let written: HashSet<i64> = conn
        .prepare("SELECT DISTINCT release_id FROM team_ratings")?
        .query_map([], |r| r.get(0))?
        .filter_map(|x| x.ok())
        .collect();
    // Only the releases the fit wrote team rows for (every one in production,
    // the last one in a research config).
    let releases: Vec<(i64, i32)> = releases.into_iter().filter(|r| written.contains(&r.0)).collect();
    let teams: Vec<i64> = b.vals.keys().copied().collect();
    let per_release: Vec<Vec<(i64, f64, usize)>> = pool.install(|| {
        releases
            .par_iter()
            .map(|&(_, d)| {
                teams
                    .iter()
                    .filter(|&&t| ac::listed(lab, &b, t, d, k))
                    .filter_map(|&t| {
                        let (x, c) = ac::rating(lab, &b, t, d, k);
                        (x > 0.0).then_some((t, x, c.len()))
                    })
                    .collect()
            })
            .collect()
    });
    eprintln!("publish: {} releases rated [{:.0}s]", releases.len(), t0.elapsed().as_secs_f64());

    // Every event's rating before and after, on the mornings of its day and the next.
    let events: Vec<(i64, i64, i32)> = conn
        .prepare("SELECT rowid, team_id, date FROM team_events")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?
        .filter_map(|x| x.ok())
        .filter_map(|(i, t, d)| Some((i, t, parse_date(&d)?)))
        .collect();
    let steps: Vec<(i64, f64, f64)> = pool.install(|| {
        events
            .par_iter()
            .map(|&(i, t, d)| (i, ac::rating(lab, &b, t, d, k).0, ac::rating(lab, &b, t, d + 1, k).0))
            .collect()
    });
    eprintln!("publish: {} events stepped [{:.0}s]", steps.len(), t0.elapsed().as_secs_f64());

    // What each result added: the day's rise, credited only to the results
    // that entered the counted set the next morning, split by what they add.
    let gains: HashMap<(i64, i64), f64> = pool.install(|| {
        teams
            .par_iter()
            .flat_map_iter(|&t| {
                let mut days: Vec<i32> = b.vals[&t].iter().map(|v| v.day).collect();
                days.sort_unstable();
                days.dedup();
                let mut out = Vec::new();
                for d in days {
                    let x0 = ac::rating(lab, &b, t, d, k).0;
                    let (x1, c, slots) = ac::rating_slots(lab, &b, t, d + 1, k);
                    let rise = x1 - x0;
                    let entered: Vec<(i64, f64)> =
                        c.iter().filter(|(v, _)| v.day == d).map(|(v, now)| (v.venue, now / slots)).collect();
                    let total: f64 = entered.iter().map(|e| e.1).sum();
                    if rise > 0.0 && total > 0.0 {
                        for (venue, part) in entered {
                            out.push(((venue, t), rise * part / total));
                        }
                    }
                }
                out
            })
            .collect()
    });

    let tx = conn.transaction()?;
    tx.execute("DELETE FROM team_ratings", [])?;
    {
        let mut ins = tx.prepare(
            "INSERT INTO team_ratings (release_id, team_id, mu, sigma, core_strength, evidence, display, listed, change)
             VALUES (?1, ?2, ?3, 0.1, NULL, ?4, ?5, 1, ?6)",
        )?;
        let mut prev: HashMap<i64, f64> = HashMap::new();
        for (&(rid, _), rows) in releases.iter().zip(&per_release) {
            let mut next = HashMap::with_capacity(rows.len());
            for &(t, x, n) in rows {
                let change = prev.get(&t).map(|p| x - p);
                ins.execute(rusqlite::params![rid, t, mu(x), n as i64, x, change])?;
                next.insert(t, x);
            }
            prev = next;
        }
    }
    {
        let mut up = tx.prepare(
            "UPDATE team_events SET mu_before = ?2, mu_after = ?3, d_place = ?4, d_pull = NULL WHERE rowid = ?1",
        )?;
        for &(i, x0, x1) in &steps {
            // No rating yet reads as a blank, not as the map's zero.
            let m0 = (x0 > 0.0).then(|| mu(x0));
            let m1 = (x1 > 0.0).then(|| mu(x1));
            up.execute(rusqlite::params![i, m0, m1, (x1 - x0) / bt])?;
        }
    }
    tx.execute_batch(
        "DROP TABLE IF EXISTS achieve_values;
         CREATE TABLE achieve_values (tournament_id INTEGER, field_id INTEGER, team_id INTEGER, date TEXT,
           format TEXT, place REAL, venue_place REAL, n_teams INTEGER, n_venues INTEGER, field REAL,
           perf REAL, strength REAL, w REAL, w_format REAL, w_field REAL, w_reach REAL, n_towns INTEGER, peak REAL,
           gain REAL);
         DROP TABLE IF EXISTS achieve_parts;
         CREATE TABLE achieve_parts (team_id INTEGER, slot INTEGER, tournament_id INTEGER, field_id INTEGER,
           peak REAL, fade REAL, overlap REAL, share REAL, contribution REAL);
         DROP TABLE IF EXISTS achieve_team;
         CREATE TABLE achieve_team (team_id INTEGER PRIMARY KEY, rating REAL, divisor REAL, n_results INTEGER,
           empty_share REAL, date TEXT);
         DROP TABLE IF EXISTS achieve_meta;
         CREATE TABLE achieve_meta (key TEXT PRIMARY KEY, value TEXT);",
    )?;
    {
        let mut ins = tx.prepare(
            "INSERT INTO achieve_values VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
        )?;
        for (&t, vals) in &b.vals {
            for v in vals {
                let wf = v.w / (v.wfmt * v.geo).max(1e-9);
                ins.execute(rusqlite::params![
                    v.venue, v.tid, t, ac::show_day(v.day), v.format.to_string(), v.place, v.vplace,
                    v.nt as i64, v.nv as i64, v.fs, v.perf, v.by_score, v.w, v.wfmt, wf, v.geo,
                    v.n_towns as i64, v.base, gains.get(&(v.venue, t))
                ])?;
            }
        }
    }
    if let Some(&(_, d)) = releases.last() {
        let mut ins = tx.prepare("INSERT INTO achieve_parts VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)")?;
        let mut team = tx.prepare("INSERT INTO achieve_team VALUES (?1,?2,?3,?4,?5,?6)")?;
        for &t in &teams {
            if !ac::listed(lab, &b, t, d, k) {
                continue;
            }
            let (x, c, slots) = ac::rating_slots(lab, &b, t, d, k);
            if x <= 0.0 {
                continue;
            }
            let mut share_sum = 0.0;
            for (i, (v, now)) in c.iter().enumerate() {
                let f = ac::fade(d - v.day, v, k);
                let share = now / v.base.max(1e-9) / slots;
                share_sum += share;
                ins.execute(rusqlite::params![
                    t, i as i64 + 1, v.venue, v.tid, v.base, f, now / (v.base * f).max(1e-9), share, now / slots
                ])?;
            }
            team.execute(rusqlite::params![t, x, slots, c.len() as i64, (1.0 - share_sum).max(0.0), ac::show_day(d)])?;
        }
    }
    {
        let mut ins = tx.prepare("INSERT INTO achieve_meta VALUES (?1, ?2)")?;
        for (key, _, _) in ac::KNOBS {
            ins.execute(rusqlite::params![key, k.get(key)])?;
        }
    }
    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_achieve_values_team ON achieve_values(team_id);
         CREATE INDEX IF NOT EXISTS idx_achieve_values_tt ON achieve_values(tournament_id, team_id);
         CREATE INDEX IF NOT EXISTS idx_achieve_parts_team ON achieve_parts(team_id);",
    )?;
    tx.commit()?;
    eprintln!("publish: written to {db} [{:.0}s]", t0.elapsed().as_secs_f64());
    Ok(())
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
        let conn = Connection::open(&cfg.data.db_path)?;
        // Town name to (town id, country id): results carry the name only.
        let geo: HashMap<String, (i64, i64)> = conn
            .prepare("SELECT name, min(id), coalesce(max(country_id), -1) FROM towns GROUP BY name")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))))?
            .collect::<Result<_, _>>()?;
        let rs = build(&arena, &cfg, &seasons(&conn), &geo);
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
    if let Some(db) = arg("--publish") {
        let k = Knobs::default().apply(&arg("--base").unwrap_or_default())?;
        let threads = arg("--threads").map_or(3, |v| v.parse().unwrap_or(3));
        publish(&lab, &k, &db, &cfg, threads)?;
        return Ok(());
    }
    if let Some(n) = arg("--search").and_then(|v| v.parse::<usize>().ok()) {
        let rounds = arg("--rounds").map_or(20, |v| v.parse().unwrap_or(20));
        let seed = arg("--seed").map_or(1, |v| v.parse().unwrap_or(1));
        let threads = arg("--threads").map_or(3, |v| v.parse().unwrap_or(3));
        let path = arg("--search-out").unwrap_or_else(|| format!("{dir}/search.tsv"));
        let fixed = arg("--fix").unwrap_or_default();
        search(&lab, n, rounds, seed, threads, &fixed, &path)?;
        eprintln!("wrote {path}");
        return Ok(());
    }
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
        let (v, _) = ac::table(&lab, &b, at, &k);
        let mut md = format!("# Achievement rating on {}\n\nKnobs: `{k:?}`\n\n", ac::show_day(at));
        let _ = writeln!(md, "| # | was | team | rating | counted |\n|---|---|---|---|---|");
        for (i, (t, x, c, _)) in v.iter().take(top_n).enumerate() {
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
