//! The achievement rating: a team's number is what its best recent results were
//! worth, and nothing else.
//!
//! Every rated result is given a value once, on the day it is played, and the
//! value never changes afterwards except by fading with age:
//!
//! ```text
//! perf   = the strength of six equal players who would expect to take the
//!          questions this lineup took, on this package (score), or to finish
//!          where it finished against this field's forecasts (place)
//! value  = w_format · w_field · max(0, perf)            published scale
//! w_field = clamp((mean of the field's top-k lineups / field_ref)^gamma, field_min, 1)
//! rating = Σ over the best top_x of value · fade(age) / top_x
//! fade   = 1 up to full_days, falling linearly to 0 at zero_days
//! ```
//!
//! A result counts for its team only when the lineup is the team's own by the
//! MAII rule (at least four base-roster players, at most two guests). A team is
//! listed only while it has a base roster for the season, or had one last
//! season and the new season is younger than `grace` days.
//!
//! ```text
//! ./target/release/achieve --build                      # player pass, writes the cache
//! ./target/release/achieve --point "top_x=8" --point "perf=place" --top 40
//! ```

use chgk_rating::chemistry::Chemistry;
use chgk_rating::config::Config;
use chgk_rating::data::{Arena, Format, CORRECT, SKIP};
use chgk_rating::difficulty::Difficulties;
use chgk_rating::editor_memory::EditorMemory;
use chgk_rating::gauge::{self, Activity};
use chgk_rating::question_model as qm;
use chgk_rating::team_ndcg::{self as metric, QUALIFY, TEST, TUNE, WEIGHTS};
use chgk_rating::team_s2::{rows_of, TeamsS2};
use chgk_rating::util::{format_date, next_thursday, parse_date};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

const CACHE: &str = "out/achieve/results.tsv";

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

/// One result as the aggregate sees it, every strength on the published team
/// scale. Nothing here changes after the day it was played.
#[derive(Clone)]
struct Res {
    tid: i64,
    day: i32,
    format: char,
    team: i64,
    place: f64,
    rated: bool,
    n: u32,
    n_base: u32,
    /// The lineup's strength as the forecast reads it; NaN with no lineup.
    q: f64,
    /// Performance from the questions taken on this package.
    perf_s: f64,
    /// Performance from the place against the field's forecasts.
    perf_p: f64,
    taken: f64,
    n_q: f64,
    /// The shipped fair rule's rating that morning, for the comparison.
    fair_mu: f64,
}

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

fn save(rs: &[Res], path: &str) -> std::io::Result<()> {
    let mut s = String::from("tid\tday\tformat\tteam\tplace\trated\tn\tn_base\tq\tperf_s\tperf_p\ttaken\tn_q\tfair_mu\n");
    for r in rs {
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.1}\t{:.1}\t{:.1}\t{}\t{}\t{:.1}",
            r.tid,
            format_date(r.day),
            r.format,
            r.team,
            r.place,
            r.rated as u8,
            r.n,
            r.n_base,
            r.q,
            r.perf_s,
            r.perf_p,
            r.taken,
            r.n_q,
            r.fair_mu
        );
    }
    std::fs::write(path, s)
}

fn load(path: &str) -> std::io::Result<Vec<Res>> {
    let s = std::fs::read_to_string(path)?;
    let f = |x: &str| x.parse::<f64>().unwrap_or(f64::NAN);
    Ok(s.lines()
        .skip(1)
        .filter_map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            Some(Res {
                tid: v[0].parse().ok()?,
                day: parse_date(v[1])?,
                format: v[2].chars().next()?,
                team: v[3].parse().ok()?,
                place: f(v[4]),
                rated: v[5] == "1",
                n: v[6].parse().ok()?,
                n_base: v[7].parse().ok()?,
                q: f(v[8]),
                perf_s: f(v[9]),
                perf_p: f(v[10]),
                taken: f(v[11]),
                n_q: f(v[12]),
                fair_mu: f(v[13]),
            })
        })
        .collect())
}

// --------------------------------------------------------------------- knobs

#[derive(Clone, Debug)]
struct Knobs {
    /// "score", "place", or a blend: `mix` of score and 1 - mix of place.
    perf: String,
    mix: f64,
    w_sync: f64,
    w_strict: f64,
    w_async: f64,
    /// Field strength is the mean of the top `field_k` lineups, empty seats at 0.
    field_k: usize,
    /// The field strength that weighs 0 and the one that weighs 1.
    field_zero: f64,
    field_ref: f64,
    field_gamma: f64,
    field_min: f64,
    full_days: i32,
    zero_days: i32,
    /// The same fade for a synchronous result; -1 is the in-person one.
    sync_full: i32,
    sync_zero: i32,
    /// A strong result fades slower: its age runs at (slow_ref / value)^slow_k
    /// of the calendar. 0 is one clock for every result.
    slow_ref: f64,
    slow_k: f64,
    /// After the full-value period a result halves every `half_life` days
    /// (`sync_half_life` for a synchronous one) instead of falling linearly to
    /// zero, and is dropped at `tail_days` of calendar age. 0 is the linear fade.
    half_life: f64,
    sync_half_life: f64,
    tail_days: i32,
    top_x: usize,
    /// The divisor is the number of results held, but never below this; empty
    /// slots below it count as zero. `top_x` is the pure sum over `top_x`.
    min_slots: usize,
    /// Count a result only when the lineup is the team's own by the MAII rule.
    base_rule: bool,
    /// Days into a season the last season's roster still lists a team; -1 lists
    /// every team whatever its roster.
    grace: i32,
    /// Published points added to a sole first place before the weights.
    win_bonus: f64,
    /// Added to every performance, to put it on the lineup scale.
    shift: f64,
}

impl Default for Knobs {
    fn default() -> Knobs {
        Knobs {
            perf: "score".into(),
            mix: 0.5,
            w_sync: 0.5,
            w_strict: 0.5,
            w_async: 0.0,
            field_k: 10,
            field_zero: 0.0,
            field_ref: 11000.0,
            field_gamma: 1.0,
            field_min: 0.0,
            full_days: 365,
            zero_days: 730,
            sync_full: -1,
            sync_zero: -1,
            slow_ref: 9000.0,
            slow_k: 0.0,
            half_life: 0.0,
            sync_half_life: 0.0,
            tail_days: 1826,
            top_x: 10,
            min_slots: 0,
            base_rule: true,
            grace: 60,
            win_bonus: 0.0,
            shift: 0.0,
        }
    }
}

fn apply(k: &Knobs, spec: &str) -> Result<Knobs, String> {
    let mut k = k.clone();
    for kv in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (key, val) = kv.split_once('=').ok_or(format!("expected key=value: {kv}"))?;
        let num = || val.parse::<f64>().map_err(|_| format!("{key}: not a number: {val}"));
        match key {
            "perf" => k.perf = val.into(),
            "mix" => k.mix = num()?,
            "w_sync" => {
                k.w_sync = num()?;
                k.w_strict = k.w_sync;
            }
            "w_strict" => k.w_strict = num()?,
            "w_async" => k.w_async = num()?,
            "field_k" => k.field_k = num()? as usize,
            "field_ref" => k.field_ref = num()?,
            "field_zero" => k.field_zero = num()?,
            "shift" => k.shift = num()?,
            "field_gamma" => k.field_gamma = num()?,
            "field_min" => k.field_min = num()?,
            "full_days" => k.full_days = num()? as i32,
            "zero_days" => k.zero_days = num()? as i32,
            "sync_full" => k.sync_full = num()? as i32,
            "sync_zero" => k.sync_zero = num()? as i32,
            "slow_ref" => k.slow_ref = num()?,
            "slow_k" => k.slow_k = num()?,
            "half_life" => k.half_life = num()?,
            "sync_half_life" => k.sync_half_life = num()?,
            "tail_days" => k.tail_days = num()? as i32,
            "top_x" => k.top_x = num()? as usize,
            "min_slots" => k.min_slots = num()? as usize,
            "base_rule" => k.base_rule = num()? != 0.0,
            "grace" => k.grace = num()? as i32,
            "win_bonus" => k.win_bonus = num()?,
            _ => return Err(format!("unknown knob {key}")),
        }
    }
    Ok(k)
}

// ------------------------------------------------------------------ aggregate

/// One counted result: its value before fading, and what it came from.
#[derive(Clone, Copy)]
struct Val {
    day: i32,
    tid: i64,
    base: f64,
    perf: f64,
    w: f64,
    sync: bool,
    place: f64,
    format: char,
    /// The format weight and the field's top-k strength behind `w`.
    wfmt: f64,
    fs: f64,
}

struct Book {
    /// Per team, every counted result, oldest first.
    vals: HashMap<i64, Vec<Val>>,
    /// Per tournament, the field weight.
    wf: HashMap<i64, f64>,
}

fn fade(age: i32, v: &Val, k: &Knobs) -> f64 {
    let (full, zero) = if v.sync && k.sync_zero >= 0 {
        (k.sync_full as f64, k.sync_zero as f64)
    } else {
        (k.full_days as f64, k.zero_days as f64)
    };
    let days = age;
    let age = if k.slow_k != 0.0 && v.base > 0.0 {
        age as f64 * (k.slow_ref / v.base).powf(k.slow_k)
    } else {
        age as f64
    };
    let hl = if v.sync && k.sync_half_life > 0.0 { k.sync_half_life } else { k.half_life };
    if hl > 0.0 {
        return if days >= k.tail_days {
            0.0
        } else if age <= full {
            1.0
        } else {
            0.5f64.powf((age - full) / hl)
        };
    }
    if age <= full {
        1.0
    } else if age >= zero {
        0.0
    } else {
        1.0 - (age - full) / (zero - full)
    }
}

fn perf_of(r: &Res, k: &Knobs) -> f64 {
    let (s, p) = (r.perf_s, r.perf_p);
    match k.perf.as_str() {
        "place" => p,
        "mix" if s.is_finite() && p.is_finite() => k.mix * s + (1.0 - k.mix) * p,
        _ if s.is_finite() => s,
        _ => p,
    }
}

fn own_lineup(r: &Res) -> bool {
    r.n_base >= 4 && r.n - r.n_base <= 2
}

fn book(rs: &[Res], k: &Knobs) -> Book {
    let mut by_t: HashMap<i64, Vec<&Res>> = HashMap::new();
    for r in rs {
        by_t.entry(r.tid).or_default().push(r);
    }
    let mut wf = HashMap::new();
    let mut fs_of = HashMap::new();
    for (&tid, v) in &by_t {
        let mut q: Vec<f64> = v.iter().map(|r| r.q).filter(|x| x.is_finite()).collect();
        q.sort_by(|a, b| b.total_cmp(a));
        let s: f64 = q.iter().take(k.field_k).map(|x| x.max(0.0)).sum::<f64>() / k.field_k.max(1) as f64;
        let w = if k.field_gamma == 0.0 {
            1.0
        } else {
            ((s - k.field_zero) / (k.field_ref - k.field_zero))
                .max(0.0)
                .powf(k.field_gamma)
                .clamp(k.field_min, if k.field_ref == 1.0 { f64::INFINITY } else { 1.0 })
        };
        wf.insert(tid, w);
        fs_of.insert(tid, s);
    }
    let mut vals: HashMap<i64, Vec<Val>> = HashMap::new();
    for r in rs {
        if !r.rated || r.place <= 0.0 || (k.base_rule && !own_lineup(r)) {
            continue;
        }
        let wfmt = match r.format {
            'i' => 1.0,
            's' => k.w_sync,
            'S' => k.w_strict,
            _ => k.w_async,
        };
        let perf = perf_of(r, k) + k.shift;
        if !perf.is_finite() || wfmt <= 0.0 {
            continue;
        }
        let sole = r.place == 1.0;
        let w = wfmt * wf[&r.tid];
        let base = w * (perf.max(0.0) + if sole { k.win_bonus } else { 0.0 });
        let sync = r.format != 'i';
        vals.entry(r.team).or_default().push(Val {
            day: r.day,
            tid: r.tid,
            base,
            perf,
            w,
            sync,
            place: r.place,
            format: r.format,
            wfmt,
            fs: fs_of[&r.tid],
        });
    }
    Book { vals, wf }
}

/// The rating on the morning of `day`: the best `top_x` faded values of the
/// results before it, over `top_x`. Also the ones that counted.
fn rating(b: &Book, team: i64, day: i32, k: &Knobs) -> (f64, Vec<(Val, f64)>) {
    let Some(v) = b.vals.get(&team) else { return (0.0, Vec::new()) };
    let mut cur: Vec<(Val, f64)> = v
        .iter()
        .filter(|x| x.day < day)
        .map(|x| (*x, x.base * fade(day - x.day, x, k)))
        .filter(|x| x.1 > 0.0)
        .collect();
    cur.sort_by(|a, b| b.1.total_cmp(&a.1));
    cur.truncate(k.top_x);
    // A faded result holds only its faded share of a slot, so an old result
    // near zero never drags the average down by taking a whole one.
    let held: f64 = cur.iter().map(|x| fade(day - x.0.day, &x.0, k)).sum();
    let slots = if k.min_slots == 0 {
        k.top_x as f64
    } else {
        held.clamp(k.min_slots as f64, k.top_x as f64)
    };
    (cur.iter().map(|x| x.1).sum::<f64>() / slots, cur)
}

// -------------------------------------------------------------------- rosters

struct Seasons {
    /// (id, first day, last day)
    list: Vec<(i64, i32, i32)>,
    has: HashSet<(i64, i64)>,
}

impl Seasons {
    fn load(conn: &Connection) -> Seasons {
        let mut list: Vec<(i64, i32, i32)> = conn
            .prepare("SELECT id, substr(date_start,1,10), substr(date_end,1,10) FROM seasons")
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .unwrap()
            .filter_map(|x| x.ok())
            .filter_map(|(i, a, b)| Some((i, parse_date(&a)?, parse_date(&b)?)))
            .collect();
        list.sort();
        let mut has = HashSet::new();
        for l in std::fs::read_to_string(chgk_rating::data::BASE_ROSTERS).unwrap_or_default().lines().skip(1) {
            let f: Vec<&str> = l.split(',').collect();
            if let (Some(Ok(t)), Some(Ok(s))) = (f.first().map(|x| x.parse()), f.get(2).map(|x| x.parse())) {
                has.insert((t, s));
            }
        }
        let rows: Vec<(i64, i64)> = conn
            .prepare("SELECT DISTINCT team_id, season_id FROM team_seasons WHERE player_id > 0")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .filter_map(|x| x.ok())
            .collect();
        has.extend(rows);
        Seasons { list, has }
    }

    /// Whether the team is listed on `day`, and why not when it is not.
    fn listed(&self, team: i64, day: i32, k: &Knobs) -> bool {
        if k.grace < 0 {
            return true;
        }
        let Some(&(s, start, _)) = self.list.iter().find(|x| day >= x.1 && day <= x.2) else {
            return true;
        };
        self.has.contains(&(team, s)) || (day - start <= k.grace && self.has.contains(&(team, s - 1)))
    }
}

// ------------------------------------------------------------------- metrics

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

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

/// The team NDCG of DESIGN 7.3 over the tune and test windows, for any rule
/// that gives a qualifying team a number on a tournament's morning.
fn team_ndcg(
    rs: &[Res],
    qual: &HashSet<(i64, i64)>,
    mut score: impl FnMut(&Res) -> f64,
) -> [HashMap<i64, f64>; 2] {
    let win = [
        (parse_date(TUNE.0).unwrap(), parse_date(TUNE.1).unwrap()),
        (parse_date(TEST.0).unwrap(), parse_date(TEST.1).unwrap()),
    ];
    let mut by_t: Vec<(i64, i32, Vec<&Res>)> = Vec::new();
    let mut ix: HashMap<i64, usize> = HashMap::new();
    for r in rs.iter().filter(|r| r.format == 'i') {
        let i = *ix.entry(r.tid).or_insert_with(|| {
            by_t.push((r.tid, r.day, Vec::new()));
            by_t.len() - 1
        });
        by_t[i].2.push(r);
    }
    let mut out = [HashMap::new(), HashMap::new()];
    for (tid, day, v) in &by_t {
        let Some(w) = win.iter().position(|&(a, b)| *day >= a && *day < b) else { continue };
        let (mut team, mut mu, mut place) = (Vec::new(), Vec::new(), Vec::new());
        for r in v {
            if !r.rated || r.place <= 0.0 || !qual.contains(&(*tid, r.team)) {
                continue;
            }
            let s = score(r);
            if !s.is_finite() {
                continue;
            }
            team.push(r.team);
            mu.push(s);
            place.push(r.place);
        }
        if let Some(x) = metric::score(*tid, &team, &mu, &place, usize::MAX) {
            out[w].insert(*tid, x);
        }
    }
    out
}

// ---------------------------------------------------------------------- main

/// Why a result's weight is below 1, for the hover text of its cell.
fn weight_title(v: &Val, k: &Knobs) -> String {
    if v.w >= 0.995 {
        return String::new();
    }
    let wf = v.w / v.wfmt.max(1e-9);
    let mut parts = Vec::new();
    if v.wfmt < 1.0 {
        let f = if v.format == 'a' { "асинхрон" } else { "синхрон" };
        parts.push(format!("{f}: ×{:.2}", v.wfmt));
    }
    if wf < 0.995 {
        parts.push(format!(
            "поле: средняя сила {} сильнейших составов {:.0}, вес ×{:.2} (0 при {:.0}, 1 при {:.0})",
            k.field_k, v.fs, wf, k.field_zero, k.field_ref
        ));
    }
    format!(
        " title=\"{}\" style=\"text-decoration: underline dotted; cursor: help\"",
        esc(&parts.join("; "))
    )
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The page body for the site's `/achieve/` route: every team of the top with
/// the results its rating is made of, each at its peak and as it stands today.
fn html_page(
    v: &[(i64, f64, Vec<(Val, f64)>)],
    k: &Knobs,
    at: i32,
    n: usize,
    was: &HashMap<i64, usize>,
    teams: &HashMap<i64, String>,
    tours: &HashMap<i64, String>,
) -> String {
    let mut o = String::new();
    let sync_rule = if k.sync_zero >= 0 {
        format!(
            "A synchronous result keeps its full value for {} days and is worth nothing after {}.",
            k.sync_full, k.sync_zero
        )
    } else {
        "Synchronous results fade like in-person ones.".to_string()
    };
    let slow = if k.slow_k != 0.0 {
        format!(
            " A strong result fades more slowly: its age runs at ({:.0} / peak value)<sup>{}</sup> of the calendar, \
             so a result worth 11,000 keeps its full value for about {:.0} months.",
            k.slow_ref,
            k.slow_k,
            k.full_days as f64 / (k.slow_ref / 11000.0).powf(k.slow_k) / 30.4
        )
    } else {
        String::new()
    };
    let _ = write!(
        o,
        "<header class=\"page-head\"><h1>Рейтинг достижений (эксперимент)</h1>\
         <p class=\"lede\">On {date}. A team's rating is the sum of its {x} best current results, divided by {x}; \
         an empty slot counts as zero. Each result is worth what the team played like: the strength of a lineup \
         of six equal players who would expect to take as many questions on that package. The peak value is \
         that strength times the format weight (in-person 1, synchronous {ws}) and the field weight \
         (0 when the mean of the ten strongest lineups is {fz:.0} or less, 1 at {fr:.0} or more). \
         {fade}{slow} \
         Only results played by the team's own lineup count (at least four base-roster players, at most two guests), \
         and a team without a base roster for the season leaves the table {g} days after the season starts. \
         Cities playing one package within two days of each other are one tournament. \
         \"Было\" is the team's place in the current official table of this site.</p></header>",
        date = format_date(at),
        x = k.top_x,
        ws = k.w_sync,
        fz = k.field_zero,
        fr = k.field_ref,
        fade = if k.half_life > 0.0 {
            format!(
                "An in-person result keeps its full value for {} days, then halves every {:.0} days; \
                 a synchronous one keeps it for {} days, then halves every {:.0} days. \
                 Anything older than {:.0} years is dropped.",
                k.full_days,
                k.half_life,
                if k.sync_zero >= 0 { k.sync_full } else { k.full_days },
                if k.sync_half_life > 0.0 { k.sync_half_life } else { k.half_life },
                k.tail_days as f64 / 365.25
            )
        } else {
            format!(
                "An in-person result keeps its full value for {} days and is worth nothing after {}. {}",
                k.full_days, k.zero_days, sync_rule
            )
        },
        slow = slow,
        g = k.grace,
    );
    o.push_str(
        "<section class=\"section\"><div class=\"table-scroll\"><table class=\"data-table\"><thead><tr>\
         <th class=\"num\">Место</th><th>Команда / турнир</th><th class=\"num\">Дата</th>\
         <th class=\"num\" title=\"Место в турнире\">Место</th>\
         <th class=\"num\" title=\"Сила игры: как играли бы шесть равных игроков\">Игра</th>\
         <th class=\"num\" title=\"Вес формата × вес поля\">Вес</th>\
         <th class=\"num\" title=\"Ценность результата в день игры\">Игра × Вес</th>\
         <th class=\"num\" title=\"Доля ценности, оставшаяся сегодня\">Осталось</th>\
         <th class=\"num\" title=\"Ценность сегодня\">Сейчас</th></tr></thead><tbody>",
    );
    for (i, (t, x, c)) in v.iter().take(n).enumerate() {
        let name = teams.get(t).cloned().unwrap_or_else(|| format!("Команда {t}"));
        let _ = write!(
            o,
            "<tr class=\"row-split\"><td class=\"num strong\">{}</td>\
             <td colspan=\"7\"><a href=\"/teams/{t}/\"><strong>{}</strong></a> \
             <span class=\"muted\">было {}</span></td><td class=\"num strong\">{:.0}</td></tr>",
            i + 1,
            esc(&name),
            was.get(t).map_or("—".into(), |r| r.to_string()),
            x
        );
        for (val, now) in c {
            let fmt = match val.format {
                'i' => "",
                'a' => " (асинхрон)",
                _ => " (синхрон)",
            };
            let place = if val.place.fract() == 0.0 {
                format!("{:.0}", val.place)
            } else {
                format!("{:.1}", val.place)
            };
            let _ = write!(
                o,
                "<tr><td></td><td><a href=\"/tournaments/{}/\">{}</a><span class=\"muted\">{}</span></td>\
                 <td class=\"num muted\">{}</td><td class=\"num\">{}</td><td class=\"num\">{:.0}</td>\
                 <td class=\"num muted\"{}>{:.2}</td><td class=\"num\">{:.0}</td>\
                 <td class=\"num muted\">{:.0}%</td><td class=\"num\">{:.0}</td></tr>",
                val.tid,
                esc(tours.get(&val.tid).map_or("?", |s| s.as_str())),
                fmt,
                format_date(val.day),
                place,
                val.perf,
                weight_title(val, k),
                val.w,
                val.base,
                100.0 * now / val.base.max(1e-9),
                now
            );
        }
        for _ in c.len()..k.top_x {
            o.push_str(
                "<tr><td></td><td class=\"muted\">пустой слот</td><td></td><td></td><td></td>\
                 <td></td><td></td><td></td><td class=\"num muted\">0</td></tr>",
            );
        }
    }
    o.push_str("</tbody></table></div></section>");
    o
}

fn names(conn: &Connection, sql: &str) -> HashMap<i64, String> {
    conn.prepare(sql)
        .unwrap()
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .filter_map(|x| x.ok())
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg_path = arg("--config").unwrap_or_else(|| "config.toml".into());
    let mut cfg = Config::load(&cfg_path)?;
    cfg.data.to = arg("--to").unwrap_or_else(|| chgk_rating::util::today_utc());
    cfg.data.merge_cities = true;
    cfg.data.merge_days = arg("--merge-days").map_or(2, |v| v.parse().unwrap_or(2));
    let cache = arg("--cache").unwrap_or_else(|| CACHE.into());
    let rs = if args().iter().any(|a| a == "--build") || !std::path::Path::new(&cache).exists() {
        let arena = chgk_rating::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
        eprintln!("loaded {} tournaments, {} teams", arena.tournaments.len(), arena.n_teams());
        let rs = build(&arena, &cfg);
        if let Some(dir) = std::path::Path::new(&cache).parent() {
            std::fs::create_dir_all(dir)?;
        }
        save(&rs, &cache)?;
        eprintln!("wrote {cache}: {} results", rs.len());
        rs
    } else {
        load(&cache)?
    };
    let conn = Connection::open(&cfg.data.db_path)?;
    let seasons = Seasons::load(&conn);
    let tnames = names(&conn, "SELECT id, name FROM tournaments");
    let team_names = names(&conn, "SELECT id, name FROM teams");
    let qual = metric::qualify(QUALIFY);
    let w = metric::weights(WEIGHTS);
    let nboot: usize = arg("--boot").map_or(1000, |v| v.parse().unwrap_or(1000));
    let last = rs.iter().map(|r| r.day).max().unwrap_or(0);
    let at = arg("--at").and_then(|d| parse_date(&d)).unwrap_or(last + 1);
    let top_n: usize = arg("--top").map_or(30, |v| v.parse().unwrap_or(30));
    let out_dir = arg("--out").unwrap_or_else(|| "out/achieve".into());
    std::fs::create_dir_all(&out_dir)?;

    // The field-strength distribution the weight is read against.
    {
        let k = Knobs::default();
        let b = book(&rs, &Knobs { field_zero: 0.0, field_ref: 1.0, field_gamma: 1.0, field_min: 0.0, ..k });
        let from = parse_date("2023-09-01").unwrap();
        let mut ip: Vec<f64> = rs
            .iter()
            .filter(|r| r.format == 'i' && r.day >= from)
            .map(|r| (r.tid, b.wf[&r.tid]))
            .collect::<HashMap<i64, f64>>()
            .into_values()
            .collect();
        ip.sort_by(|a, b| a.total_cmp(b));
        let qn = |p: f64| ip[((ip.len() - 1) as f64 * p) as usize];
        eprintln!(
            "in-person top-10 field strength since 2023-09 ({} fields): p50 {:.0}  p75 {:.0}  p90 {:.0}  p97 {:.0}  max {:.0}",
            ip.len(),
            qn(0.5),
            qn(0.75),
            qn(0.9),
            qn(0.97),
            qn(1.0)
        );
    }

    let base = apply(&Knobs::default(), &arg("--base").unwrap_or_default())?;
    let mut points = vec![String::new()];
    points.extend(args_all("--point"));

    let fair = team_ndcg(&rs, &qual, |r| r.fair_mu);
    let mut table = String::from(
        "| variant | tune | test | Δ test vs fair | SE |\n|---|---|---|---|---|\n",
    );
    let _ = writeln!(
        table,
        "| fair rule (shipped) | {:.4} | {:.4} | | |",
        metric::wmean(&fair[0], &w),
        metric::wmean(&fair[1], &w)
    );
    // The shipped table on the last release, for the "was" column.
    let shipped_rank: HashMap<i64, usize> = Connection::open("out/ratings.db")
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

    for (pi, spec) in points.iter().enumerate() {
        let k = apply(&base, spec)?;
        let b = book(&rs, &k);
        let nd = team_ndcg(&rs, &qual, |r| rating(&b, r.team, r.day, &k).0);
        let name = if spec.is_empty() { "achievement (base)".to_string() } else { format!("`{spec}`") };
        let _ = writeln!(
            table,
            "| {name} | {:.4} | {:.4} | {:+.4} | {:.4} |",
            metric::wmean(&nd[0], &w),
            metric::wmean(&nd[1], &w),
            metric::wmean(&nd[1], &w) - metric::wmean(&fair[1], &w),
            boot(&nd[1], &fair[1], &w, nboot)
        );

        // The table on `at`.
        let mut v: Vec<(i64, f64, Vec<(Val, f64)>)> = b
            .vals
            .keys()
            .filter(|&&t| seasons.listed(t, at, &k))
            .map(|&t| {
                let (x, c) = rating(&b, t, at, &k);
                (t, x, c)
            })
            .filter(|x| x.1 > 0.0)
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        let dropped: Vec<(i64, f64)> = {
            let mut d: Vec<(i64, f64)> = b
                .vals
                .keys()
                .filter(|&&t| !seasons.listed(t, at, &k))
                .map(|&t| (t, rating(&b, t, at, &k).0))
                .filter(|x| x.1 > v.get(top_n.saturating_sub(1)).map_or(0.0, |y| y.1))
                .collect();
            d.sort_by(|a, b| b.1.total_cmp(&a.1));
            d
        };
        let mut md = String::new();
        let _ = writeln!(md, "# Achievement rating on {}\n", format_date(at));
        let _ = writeln!(md, "Knobs: `{k:?}`\n");
        let _ = writeln!(md, "{} teams listed with a positive rating.\n", v.len());
        let _ = writeln!(
            md,
            "| # | was | team | rating | counted | last | best three results (performance × weight × fade) |\n|---|---|---|---|---|---|---|"
        );
        for (i, (t, x, c)) in v.iter().take(top_n).enumerate() {
            let best: Vec<String> = c
                .iter()
                .take(3)
                .map(|(val, f)| {
                    format!(
                        "{} {} {:.0}×{:.2}×{:.2}",
                        tnames.get(&val.tid).map_or("?", |s| s.as_str()),
                        &format_date(val.day)[..7],
                        val.perf,
                        val.w,
                        f / val.base.max(1e-9)
                    )
                })
                .collect();
            let lastd = b.vals[t].iter().map(|x| x.day).max().unwrap_or(0);
            let _ = writeln!(
                md,
                "| {} | {} | {} | {:.0} | {} | {} | {} |",
                i + 1,
                shipped_rank.get(t).map_or("–".into(), |r| r.to_string()),
                team_names.get(t).map_or("?", |s| s.as_str()),
                x,
                c.len(),
                format_date(lastd),
                best.join("; ")
            );
        }
        if !dropped.is_empty() {
            let _ = writeln!(md, "\nWould be in this top {top_n} but have no base roster for the season:\n");
            for (t, x) in dropped.iter().take(20) {
                let _ = writeln!(md, "- {} ({t}): {:.0}", team_names.get(t).map_or("?", |s| s.as_str()), x);
            }
        }
        let path = format!("{out_dir}/top{pi}.md");
        std::fs::write(&path, &md)?;
        if pi == 0 {
            if let Some(hp) = arg("--html") {
                let n_html: usize = arg("--html-top").map_or(100, |x| x.parse().unwrap_or(100));
                std::fs::write(&hp, html_page(&v, &k, at, n_html, &shipped_rank, &team_names, &tnames))?;
                eprintln!("wrote {hp}");
            }
        }
        eprintln!("wrote {path}");
    }
    print!("{table}");
    std::fs::write(format!("{out_dir}/ndcg.md"), &table)?;
    Ok(())
}
