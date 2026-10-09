//! What one result is worth under a set of knobs: its peak value, from the
//! performance and the format, field and reach weights, and how that fades.

use super::frozen::Res;
use super::knobs::Knobs;
use super::lab::{Field, Lab};
use std::collections::HashMap;

/// The MAII rule's own lineup: at least four base-roster players.
const OWN_BASE: u32 = 4;
/// The yearly fade counts age in years of this many days; exp halves.
const YEAR: f64 = 365.25;
const HALF: f64 = 0.5;
/// Six play at a time.
const LINEUP: usize = 6;
const UNKNOWN_SEASON: i64 = -1;

/// One counted result: its value before fading, and what it came from.
#[derive(Clone, Copy)]
pub(super) struct Val {
    pub day: i32,
    pub tid: i64,
    pub venue: i64,
    pub place: f64,
    pub vplace: f64,
    pub format: char,
    pub sync: bool,
    /// Peak value, performance and the weights behind it.
    pub base: f64,
    pub perf: f64,
    pub by_score: f64,
    pub w: f64,
    pub wfmt: f64,
    pub fs: f64,
    pub nt: usize,
    pub nv: usize,
    /// The season it was played in.
    pub season: i64,
    /// The reach factor and the town count behind it.
    pub geo: f64,
    pub n_towns: usize,
}

fn own_lineup(r: &Res) -> bool {
    r.n_base >= OWN_BASE && r.n - r.n_base <= 2
}

/// The performance the knobs read, and the score and place numbers behind it.
fn perf_of(r: &Res, k: &Knobs) -> (f64, f64, f64) {
    let (s, p) = (r.perf_s + k.shift, r.perf_p);
    let v = match k.perf {
        "strength_place" if r.place > 0.0 && k.fallback && !s.is_finite() => p,
        "strength_place" if r.place > 0.0 => s / r.place.powf(k.place_pow),
        "place" => p,
        "mix" if s.is_finite() && p.is_finite() => k.mix * s + (1.0 - k.mix) * p,
        _ if s.is_finite() => s,
        _ => p,
    };
    (v, s, p)
}

fn format_weight(format: char, k: &Knobs) -> f64 {
    match format {
        'i' => 1.0,
        's' => k.w_sync,
        'S' => k.w_strict,
        _ => k.w_async,
    }
}

/// The field weight and the field strength it was read from.
fn field_weight(f: &Field, k: &Knobs) -> (f64, f64) {
    let sum: f64 = f.top_q.iter().take(k.field_k).map(|x| x.max(0.0)).sum();
    let seats = if k.field_pad { k.field_k } else { f.top_q.len().clamp(1, k.field_k) };
    let s = sum / seats.max(1) as f64;
    let size = if k.field_pad { 1.0 } else { (seats as f64 / k.field_k as f64).powf(k.size_pow) };
    if k.field_gamma == 0.0 {
        return (1.0, s);
    }
    if k.field_mode == "sum" && f.in_person {
        let w = (f.sum_all / k.sum_ref).powf(k.sum_gamma);
        return (w.max(k.field_min.min(1.0)), s);
    }
    let w = if k.field_mode == "world" || k.field_mode == "sum" {
        if f.ref20 > 0.0 {
            (sum / f.ref20).min(1.0).powf(k.field_gamma)
        } else {
            1.0
        }
    } else if k.field_ref <= k.field_zero {
        1.0
    } else {
        ((s - k.field_zero) / (k.field_ref - k.field_zero)).max(0.0).powf(k.field_gamma).min(1.0) * size
    };
    (w.max(k.field_min.min(1.0)), s)
}

/// The in-person reach factor and the country and town counts behind it.
fn reach(f: &Field, k: &Knobs) -> (f64, usize, usize) {
    let nc = f.countries.iter().filter(|&&c| c >= k.country_min).count();
    let nt = f.towns.iter().filter(|&&c| c >= k.town_min).count();
    if !f.in_person {
        return (1.0, nc, nt);
    }
    let x = (nc.max(1) as f64 / k.country_ref).powf(k.country_pow) * (nt.max(1) as f64 / k.town_ref).powf(k.town_pow);
    (if k.geo_cap { x.min(1.0) } else { x }, nc, nt)
}

/// Per team, every counted result.
pub(super) fn values(lab: &Lab, k: &Knobs) -> HashMap<i64, Vec<Val>> {
    let mut fw: HashMap<i64, (f64, f64)> = HashMap::new();
    let mut vals: HashMap<i64, Vec<Val>> = HashMap::new();
    for r in &lab.rows {
        if k.base_rule && !own_lineup(r) {
            continue;
        }
        let Some(f) = lab.fields.get(&r.tid) else { continue };
        if let Some(v) = value(lab, r, f, k, &mut fw) {
            vals.entry(r.team).or_default().push(v);
        }
    }
    vals
}

/// One result's value; none when it has no performance or its format is not
/// counted. `fw` keeps each field's weight once read.
fn value(lab: &Lab, r: &Res, f: &Field, k: &Knobs, fw: &mut HashMap<i64, (f64, f64)>) -> Option<Val> {
    let wfmt = format_weight(r.format, k);
    let (perf, by_score, _) = perf_of(r, k);
    if !perf.is_finite() || wfmt <= 0.0 {
        return None;
    }
    let (wf, fs) = *fw.entry(r.tid).or_insert_with(|| field_weight(f, k));
    let (geo, _, n_towns) = reach(f, k);
    let w = wfmt * wf * geo;
    let base = w * (perf.max(0.0) + if r.place == 1.0 { k.win_bonus } else { 0.0 });
    Some(Val {
        day: r.day,
        tid: r.tid,
        venue: r.venue,
        place: r.place,
        vplace: r.vplace,
        format: r.format,
        sync: r.format != 'i',
        base,
        perf,
        by_score,
        w,
        wfmt,
        fs,
        nt: f.n_teams,
        nv: f.n_venues,
        season: lab.seasons.season_of(r.day).unwrap_or(UNKNOWN_SEASON),
        geo,
        n_towns,
    })
}

/// The share of its peak value a result keeps at `age` days.
pub(super) fn fade(age: i32, v: &Val, k: &Knobs) -> f64 {
    if k.fade == "none" {
        return 1.0;
    }
    let days = age;
    let age = if k.slow_k != 0.0 && v.base > 0.0 { age as f64 * (k.slow_ref / v.base).powf(k.slow_k) } else { age as f64 };
    match k.fade {
        "yearly" => {
            let loss = if v.sync { k.loss_sync } else { k.loss_ip };
            (1.0 - loss).max(0.0).powf(age / YEAR)
        }
        "exp" => exp_fade(days, age, v, k),
        _ => linear_fade(age, v, k),
    }
}

/// The in-person or the synchronous fade's (full, zero) days.
fn fade_days(v: &Val, k: &Knobs) -> (f64, f64) {
    if v.sync && k.sync_zero >= 0 {
        (k.sync_full as f64, k.sync_zero as f64)
    } else {
        (k.full_days as f64, k.zero_days as f64)
    }
}

fn exp_fade(days: i32, age: f64, v: &Val, k: &Knobs) -> f64 {
    let (full, _) = fade_days(v, k);
    let hl = if v.sync && k.sync_half_life > 0.0 { k.sync_half_life } else { k.half_life };
    if days >= k.tail_days {
        0.0
    } else if age <= full {
        1.0
    } else if hl <= 0.0 {
        0.0
    } else {
        HALF.powf((age - full) / hl)
    }
}

fn linear_fade(age: f64, v: &Val, k: &Knobs) -> f64 {
    let (full, zero) = fade_days(v, k);
    if age <= full {
        1.0
    } else if age >= zero {
        0.0
    } else {
        1.0 - (age - full) / (zero - full)
    }
}

/// The share of the base roster a result was earned with that is still on
/// the team's roster on `day`, by seats or by rating points over the six
/// strongest, to the power `overlap_k`. 1 when either roster is unknown.
pub(super) fn overlap(lab: &Lab, team: i64, season: i64, now: Option<&Vec<i64>>, k: &Knobs) -> f64 {
    if k.overlap_k == 0.0 {
        return 1.0;
    }
    let (Some(now), Some(then)) = (now, lab.seasons.players.get(&(team, season))) else { return 1.0 };
    if then.is_empty() {
        return 1.0;
    }
    // Six play at a time: both sides are read over their six strongest, so a
    // roster of eight that keeps its best six has lost nothing it needed.
    let weight = |p: &i64| if k.overlap_by == "count" { 1.0 } else { lab.seasons.strength.get(p).copied().unwrap_or(0.0) };
    let top6 = |mut v: Vec<f64>| -> f64 {
        v.sort_by(|a, b| b.total_cmp(a));
        v.iter().take(LINEUP).sum()
    };
    let all = top6(then.iter().map(weight).collect());
    let kept = top6(then.iter().filter(|p| now.binary_search(p).is_ok()).map(weight).collect());
    if all <= 0.0 {
        return 1.0;
    }
    (kept / all).min(1.0).powf(k.overlap_k)
}
