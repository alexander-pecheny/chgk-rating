//! Model S: the whole rating on the display scale, four constants, no algebra
//! beyond a weighted average. DESIGN.md describes the shipped model; this is the
//! spreadsheet-reproducible cousin, written to the same schema so `eval` reads it.
use chgk_rating::config::{Anchor, Config, Perf, PlayerRule, S2Lineup, Simple};
use chgk_rating::data::{Arena, Format};
use chgk_rating::gauge::Activity;
use chgk_rating::output::Writer;
use chgk_rating::team_model::TeamEvent;
use chgk_rating::team_s2::lineup_strength;
use chgk_rating::util::{
    avg_ranks, format_date, last_thursday, next_thursday, parse_date, spearman,
};
use rusqlite::Connection;
use std::collections::HashMap;

const TUNE: (&str, &str) = ("2018-09-01", "2024-08-29");
const TEST: (&str, &str) = ("2024-08-30", "2026-08-27");
/// Where the anchor of rule 13 puts the established teams.
const ANCHOR_MEAN: f64 = 5000.0;
const ANCHOR_SD: f64 = 1000.0;

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

/// NDCG of the forecast order against actual places, relevance 1/place.
fn ndcg(fc: &[f64], place: &[f64], k: usize) -> Option<f64> {
    if fc.len() < 5 || fc.iter().all(|v| *v == fc[0]) {
        return None;
    }
    let mut o: Vec<usize> = (0..fc.len()).collect();
    o.sort_by(|&a, &b| fc[b].total_cmp(&fc[a]));
    let mut rel: Vec<f64> = o.iter().map(|&i| 1.0 / place[i]).collect();
    let k = k.min(rel.len());
    let dcg: f64 = rel[..k]
        .iter()
        .enumerate()
        .map(|(i, r)| r / ((i + 2) as f64).log2())
        .sum();
    rel.sort_by(|a, b| b.total_cmp(a));
    let idcg: f64 = rel[..k]
        .iter()
        .enumerate()
        .map(|(i, r)| r / ((i + 2) as f64).log2())
        .sum();
    (idcg > 0.0).then(|| dcg / idcg)
}

#[derive(Default, Clone, Copy)]
struct Mean(f64, f64);

impl Mean {
    fn push(&mut self, v: f64) {
        self.0 += v;
        self.1 += 1.0;
    }
    fn get(self) -> f64 {
        self.0 / self.1.max(1.0)
    }
}

#[derive(Default, Clone, Copy)]
struct Score {
    tune: Mean,
    tune_ndcg: Mean,
    tune_ndcg3: Mean,
    test: Mean,
    test_sync: Mean,
    resid19: Mean,
    /// Pairwise log-loss and the decile calibration of the expected wins.
    ll: Mean,
    bin_n: [f64; 10],
    bin_w: [f64; 10],
    new_level: f64,
}

fn sigma(games: f64) -> f64 {
    if games >= 5.0 {
        0.1
    } else {
        0.5
    }
}

struct State {
    r: Vec<f64>,
    qbar: Vec<f64>,
    tgames: Vec<f64>,
    tlast: Vec<i32>,
    pr: Vec<f64>,
    pgames: Vec<f64>,
    tact: Activity,
    pact: Activity,
    prev_t: Vec<f64>,
    prev_p: Vec<f64>,
    shift_cum: f64,
    /// What a player with no history is worth once the anchor has moved the scale.
    new_level: f64,
}

impl State {
    fn new(arena: &Arena, p: &Simple) -> State {
        let (nt, np) = (arena.n_teams(), arena.n_players());
        State {
            r: vec![f64::NAN; nt],
            qbar: vec![f64::NAN; nt],
            tgames: vec![0.0; nt],
            tlast: vec![i32::MIN / 2; nt],
            pr: vec![p.r_new; np],
            pgames: vec![0.0; np],
            tact: Activity::new(nt),
            pact: Activity::new(np),
            prev_t: vec![f64::NAN; nt],
            prev_p: vec![f64::NAN; np],
            shift_cum: 0.0,
            new_level: p.r_new,
        }
    }

    /// Rule 13: put the established teams back on the published scale. Every
    /// rating moves by the same affine map, so nothing about the field changes.
    fn anchor(&mut self, mode: Anchor) -> f64 {
        if mode == Anchor::None {
            return 0.0;
        }
        let v: Vec<f64> = (0..self.r.len())
            .filter(|&i| self.tact.count[i] >= 10 && self.r[i].is_finite())
            .map(|i| self.r[i])
            .collect();
        if v.len() < 30 {
            return 0.0;
        }
        let m = v.iter().sum::<f64>() / v.len() as f64;
        let sd = (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt();
        let k = if mode == Anchor::MeanSd && sd > 1e-6 {
            ANCHOR_SD / sd
        } else {
            1.0
        };
        for x in self
            .r
            .iter_mut()
            .chain(&mut self.qbar)
            .chain(&mut self.pr)
            .chain(&mut self.prev_t)
            .chain(&mut self.prev_p)
            .chain(std::iter::once(&mut self.new_level))
        {
            *x = ANCHOR_MEAN + (*x - m) * k;
        }
        m - ANCHOR_MEAN
    }

    fn release(
        &mut self,
        w: Option<&Writer>,
        id: i64,
        day: i32,
        arena: &Arena,
        cfg: &Config,
        anchor: Anchor,
        rows: bool,
    ) -> rusqlite::Result<()> {
        self.pact.expire_before(day - 365);
        self.tact.expire_before(day - 365);
        let shift = self.anchor(anchor);
        self.shift_cum += shift;
        let Some(w) = w else { return Ok(()) };
        let listed = (0..arena.n_players())
            .filter(|&i| self.pact.count[i] > 0)
            .count();
        w.release(id, &format_date(day), shift, self.shift_cum, listed)?;
        if !rows {
            return Ok(());
        }
        let prows: Vec<(i64, f64, f64, u32, f64)> = (0..arena.n_players())
            .filter(|&i| self.pact.count[i] > 0)
            .map(|i| {
                let change = if self.prev_p[i].is_finite() {
                    self.pr[i] - self.prev_p[i]
                } else {
                    0.0
                };
                self.prev_p[i] = self.pr[i];
                (
                    arena.player_ids[i],
                    self.pr[i],
                    sigma(self.pgames[i]),
                    self.pact.count[i],
                    change,
                )
            })
            .collect();
        let trows: Vec<(i64, f64, f64, f64, f64, f64)> = (0..arena.n_teams())
            .filter(|&i| self.tact.count[i] > 0 && self.r[i].is_finite())
            .map(|i| {
                let change = if self.prev_t[i].is_finite() {
                    self.r[i] - self.prev_t[i]
                } else {
                    0.0
                };
                self.prev_t[i] = self.r[i];
                (
                    arena.team_ids[i],
                    self.r[i],
                    sigma(self.tgames[i]),
                    self.qbar[i],
                    self.tgames[i],
                    change,
                )
            })
            .collect();
        w.player_ratings(id, &prows, |t| chgk_rating::display::player(t, cfg))?;
        w.team_ratings(id, &trows, |m| chgk_rating::display::team(m, cfg))
    }
}

type Ext = HashMap<(i64, u32), f64>;

/// One chronological sweep. Without a writer nothing is stored, which is what
/// the tuning grid runs.
fn run(
    arena: &Arena,
    cfg: &Config,
    p: &Simple,
    ext: Option<&Ext>,
    w: Option<&Writer>,
) -> Result<Score, Box<dyn std::error::Error>> {
    let mut st = State::new(arena, p);
    let mut s2 = cfg.team.s2.clone();
    (s2.delta, s2.lineup) = (p.delta, p.lineup);
    let mut sc = Score::default();
    let (tune_a, tune_b) = (parse_date(TUNE.0).unwrap(), parse_date(TUNE.1).unwrap());
    let (test_a, test_b) = (parse_date(TEST.0).unwrap(), parse_date(TEST.1).unwrap());
    let (y19a, y19b) = (parse_date("2019-01-01").unwrap(), parse_date("2020-01-01").unwrap());
    let from_day = parse_date(&cfg.data.from).ok_or("bad from")?;
    let to_day = parse_date(&cfg.data.to).ok_or("bad to")?;
    let mut next_rel = next_thursday(from_day);
    let mut release_id: i64 = 0;
    let final_rel = last_thursday(to_day);
    // The hybrid's players are anchored by the question model's own gauge, and
    // rescaling the team side would leave the two scales inconsistent.
    let anchor = if ext.is_some() { Anchor::None } else { p.anchor };

    // Scratch, reused across tournaments.
    let (mut idx, mut q, mut f, mut place) = (vec![], vec![], vec![], vec![]);

    for t in &arena.tournaments {
        while next_rel <= t.day {
            release_id += 1;
            let rows = cfg.data.write_rows(release_id, next_rel == final_rel);
            st.release(w, release_id, next_rel, arena, cfg, anchor, rows)?;
            next_rel += 7;
            if let Some(w) = w {
                if release_id % 50 == 0 {
                    w.checkpoint()?;
                }
            }
        }
        idx.clear();
        q.clear();
        f.clear();
        place.clear();
        for (ri, res) in t.teams.iter().enumerate() {
            let ti = res.team as usize;
            let ql = (!res.members.is_empty()).then(|| {
                let rs: Vec<f64> = res
                    .members
                    .iter()
                    .map(|&m| match ext {
                        Some(e) => *e.get(&(t.id, m)).unwrap_or(&p.r_new),
                        None => st.pr[m as usize],
                    })
                    .collect();
                lineup_strength(&rs, &s2)
            });
            let known = st.r[ti].is_finite();
            let Some(qv) = ql.or(known.then_some(f64::NAN)) else {
                continue;
            };
            if !known {
                if res.rated {
                    st.r[ti] = qv;
                    st.qbar[ti] = qv;
                }
                f.push(qv);
            } else {
                f.push(st.r[ti] + if qv.is_nan() { 0.0 } else { qv - st.qbar[ti] });
            }
            idx.push(ri);
            q.push(qv);
            place.push(if res.position > 0.0 {
                res.position
            } else {
                f64::INFINITY
            });
        }
        let n = idx.len();
        if n == 0 {
            continue;
        }
        let fplace = avg_ranks(&f.iter().map(|v| -v).collect::<Vec<_>>());
        let c = std::f64::consts::LN_10 / p.d;
        let win = |a: f64, b: f64| 1.0 / (1.0 + (c * (b - a)).exp());
        let pairwise = p.perf == Perf::Pairwise;
        let mut psum = vec![0.0; n];
        let score_ll =
            t.format == Format::InPerson && t.day >= tune_a && t.day < tune_b;
        if pairwise || w.is_some() {
            for i in 0..n {
                for j in i + 1..n {
                    let pij = win(f[i], f[j]);
                    psum[i] += pij;
                    psum[j] += 1.0 - pij;
                    if score_ll && place[i] != place[j] {
                        let y = (place[i] < place[j]) as u8 as f64;
                        sc.ll.push(-(y * pij + (1.0 - y) * (1.0 - pij)).max(1e-12).ln());
                        for (e, o) in [(pij, y), (1.0 - pij, 1.0 - y)] {
                            let b = ((e * 10.0) as usize).min(9);
                            sc.bin_n[b] += 1.0;
                            sc.bin_w[b] += o;
                        }
                    }
                }
            }
        }
        let mut spair = vec![0.0; n];
        if pairwise {
            let mut ord: Vec<usize> = (0..n).collect();
            ord.sort_by(|&a, &b| place[a].total_cmp(&place[b]));
            let mut i = 0;
            while i < n {
                let mut j = i;
                while j + 1 < n && place[ord[j + 1]] == place[ord[i]] {
                    j += 1;
                }
                let above = (n - j - 1) as f64;
                for &a in &ord[i..=j] {
                    let tied: f64 = ord[i..=j]
                        .iter()
                        .filter(|&&b| b != a)
                        .map(|&b| win(f[a], f[b]))
                        .sum();
                    spair[a] = (above - psum[a] + tied) / ((n - 1) as f64).max(1.0);
                }
                i = j + 1;
            }
        } else {
            let rank = avg_ranks(&place);
            let mut fs = f.clone();
            fs.sort_by(|a, b| b.total_cmp(a));
            for (k, &pl) in rank.iter().enumerate() {
                let x = (pl - 1.0).clamp(0.0, (n - 1) as f64);
                let (lo, hi) = (x.floor() as usize, x.ceil() as usize);
                spair[k] = fs[lo] + (x - lo as f64) * (fs[hi] - fs[lo]) - f[k];
            }
        }
        let steps: Vec<f64> = if pairwise {
            spair.iter().map(|s| p.d * s).collect()
        } else {
            spair.clone()
        };

        // The field's first place, for the floor of rule 9.
        let best = place.iter().cloned().fold(f64::INFINITY, f64::min);
        let mut evs = Vec::with_capacity(if w.is_some() { n } else { 0 });
        let mut prows: Vec<(i64, f64, f64, f64)> = Vec::new();
        for (k, &ri) in idx.iter().enumerate() {
            let res = &t.teams[ri];
            let ti = res.team as usize;
            let (fk, step) = (f[k], steps[k]);
            let rated = res.rated && res.position > 0.0;
            let short = p.delta * (6.0 - res.members.len() as f64).max(0.0);
            let mu_before = st.r[ti];
            let gap = if st.tlast[ti] > i32::MIN / 2 {
                t.day - st.tlast[ti]
            } else {
                0
            };
            if mu_before.is_finite() && t.day >= y19a && t.day < y19b && q[k].is_finite() {
                sc.resid19.push(mu_before - q[k]);
            }
            if w.is_some() {
                let expected = (n as f64) - psum[k];
                evs.push(TeamEvent {
                    team: res.team,
                    c: rated as u8 as f64,
                    delta_l: if mu_before.is_finite() && q[k].is_finite() {
                        q[k] - st.qbar[ti]
                    } else {
                        0.0
                    },
                    x: fk,
                    forecast: fk,
                    forecast_place: fplace[k],
                    expected_place: expected,
                    actual_place: res.position,
                    pred_place: res.pred_pos,
                    s_r: q[k],
                    s_pair: spair[k],
                    i_pair: 1.0,
                    mu_before,
                    mu_after: f64::NAN,
                    sigma_before: sigma(st.tgames[ti]),
                    sigma_after: 0.0,
                    evidence: st.tgames[ti],
                    gap_days: gap,
                    var_diff: 0.0,
                    lost_prev: 0.0,
                    rated,
                    z: None,
                });
            }
            for &m in &res.members {
                let m = m as usize;
                let before = st.pr[m];
                match (ext, p.player_rule) {
                    (Some(e), _) => st.pr[m] = *e.get(&(t.id, m as u32)).unwrap_or(&st.pr[m]),
                    (None, PlayerRule::Surprise) => st.pr[m] += p.k_p * t.weight * step,
                    // Inverting rule 1: a short lineup that played like `perf` must be
                    // that much stronger per head, so the shortfall is added back.
                    (None, PlayerRule::Level) => {
                        st.pr[m] += p.k_p * t.weight * (fk + step + short - st.pr[m])
                    }
                }
                if w.is_some() {
                    prows.push((arena.player_ids[m], before, st.pr[m], 0.0));
                }
                if rated {
                    st.pgames[m] += 1.0;
                    st.pact.push(t.day, m as u32, 0.0);
                }
            }
            if rated {
                let r_before = st.r[ti];
                st.r[ti] += p.k * t.weight * step;
                // Rule 8: the rating drifts towards what the players are worth.
                if p.k_r > 0.0 && q[k].is_finite() {
                    st.r[ti] += p.k_r * t.weight * (q[k] - st.r[ti]);
                }
                // Rule 9: a team that finished first keeps at least what it came with.
                if place[k] == best && st.r[ti] < r_before {
                    st.r[ti] = r_before;
                }
                if q[k].is_finite() {
                    st.qbar[ti] += p.k_q * (q[k] - st.qbar[ti]);
                }
                st.tgames[ti] += 1.0;
                st.tlast[ti] = t.day;
                st.tact.push(t.day, res.team, 0.0);
            }
            if let Some(e) = evs.last_mut() {
                e.mu_after = st.r[ti];
                e.sigma_after = sigma(st.tgames[ti]);
            }
        }

        let (fp, ap): (Vec<f64>, Vec<f64>) = (0..n)
            .filter(|&k| place[k].is_finite())
            .map(|k| (fplace[k], place[k]))
            .unzip();
        let sp = spearman(&fp, &ap);
        if t.format == Format::InPerson {
            let (fv, pv): (Vec<f64>, Vec<f64>) = (0..n)
                .filter(|&k| place[k].is_finite())
                .map(|k| (f[k], place[k]))
                .unzip();
            if t.day >= tune_a && t.day < tune_b {
                if let Some(v) = sp {
                    sc.tune.push(v);
                }
                if let Some(v) = ndcg(&fv, &pv, usize::MAX) {
                    sc.tune_ndcg.push(v);
                }
                if let Some(v) = ndcg(&fv, &pv, 3) {
                    sc.tune_ndcg3.push(v);
                }
            }
            if t.day >= test_a && t.day < test_b {
                if let Some(v) = sp {
                    sc.test.push(v);
                }
            }
        } else if t.format == Format::Sync && t.day >= test_a && t.day < test_b {
            if let Some(v) = sp {
                sc.test_sync.push(v);
            }
        }

        if let Some(w) = w {
            w.team_events(t.id, &format_date(t.day), &arena.team_ids, &evs)?;
            w.player_events(t.id, &prows)?;
            let (po, pa): (Vec<f64>, Vec<f64>) = evs
                .iter()
                .filter(|e| e.actual_place > 0.0)
                .filter_map(|e| e.pred_place.map(|p| (p, e.actual_place)))
                .unzip();
            w.tournament_metrics(
                t.id,
                &format_date(t.day),
                t.format.name(),
                t.weight,
                t.teams.len(),
                t.n_q,
                0,
                None,
                None,
                sp,
                spearman(&po, &pa),
                None,
            )?;
        }
    }
    sc.new_level = st.new_level;
    if w.is_some() {
        while next_rel <= to_day {
            release_id += 1;
            let rows = cfg.data.write_rows(release_id, next_rel == final_rel);
            st.release(w, release_id, next_rel, arena, cfg, anchor, rows)?;
            next_rel += 7;
        }
    }
    Ok(sc)
}

/// Player strengths from the shipped question model: theta as it stood before
/// each tournament, mapped to the team scale.
fn load_ext(path: &str, arena: &Arena, cfg: &Config) -> rusqlite::Result<Ext> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let pidx: HashMap<i64, u32> = arena
        .player_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i as u32))
        .collect();
    let mut st = conn.prepare("SELECT tournament_id, player_id, theta_before FROM player_events")?;
    let mut rows = st.query([])?;
    let mut out = Ext::default();
    while let Some(r) = rows.next()? {
        if let Some(&i) = pidx.get(&r.get::<_, i64>(1)?) {
            let th: f64 = r.get(2)?;
            out.insert((r.get(0)?, i), chgk_rating::team_s2::player_rating(th, cfg));
        }
    }
    Ok(out)
}

const AXES: [(&str, &[f64]); 7] = [
    ("K", &[0.05, 0.10, 0.20, 0.40, 0.80]),
    ("k_p", &[0.05, 0.10, 0.20, 0.40, 0.80]),
    ("k_q", &[0.01, 0.05, 0.10, 0.30]),
    ("delta", &[150.0, 300.0, 600.0, 1200.0]),
    ("r_new", &[3000.0, 4000.0, 5000.0]),
    ("D", &[300.0, 600.0, 1200.0, 2400.0, 4800.0]),
    ("k_r", &[0.0, 0.1, 0.2, 0.3, 0.5]),
];

fn axis(p: &mut Simple, i: usize) -> &mut f64 {
    match i {
        0 => &mut p.k,
        1 => &mut p.k_p,
        2 => &mut p.k_q,
        3 => &mut p.delta,
        4 => &mut p.r_new,
        5 => &mut p.d,
        _ => &mut p.k_r,
    }
}

const D_SCAN: &[f64] = &[
    100.0, 150.0, 225.0, 300.0, 450.0, 600.0, 900.0, 1200.0, 1800.0, 2400.0,
];

/// D by maximum likelihood of the pairwise outcomes; Spearman cannot see it.
fn calibrate_d(arena: &Arena, cfg: &Config, base: &Simple) -> Result<(), Box<dyn std::error::Error>> {
    let mut p = base.clone();
    println!("| D | pairs | log-loss | tune spearman | NDCG@3 |");
    println!("|---:|---:|---:|---:|---:|");
    for &d in D_SCAN {
        p.d = d;
        let s = run(arena, cfg, &p, None, None)?;
        println!(
            "| {d} | {:.0} | {:.4} | {:.4} | {:.4} |",
            s.ll.1,
            s.ll.get(),
            s.tune.get(),
            s.tune_ndcg3.get()
        );
    }
    Ok(())
}

fn print_bins(sc: &Score) {
    println!("\n| expected win | pairs | mean expected | actual |");
    println!("|---|---:|---:|---:|");
    for b in 0..10 {
        if sc.bin_n[b] > 0.0 {
            println!(
                "| {:.1}-{:.1} | {:.0} | {:.3} | {:.3} |",
                b as f64 / 10.0,
                (b + 1) as f64 / 10.0,
                sc.bin_n[b],
                (b as f64 + 0.5) / 10.0,
                sc.bin_w[b] / sc.bin_n[b]
            );
        }
    }
}

const K_GRID: &[f64] = &[0.2, 0.3, 0.4, 0.6];
const KQ_GRID: &[f64] = &[0.01, 0.03, 0.1];

/// The K x K_q sweep, run per lineup rule; the rest of the constants are the
/// likelihood-fixed pair D and delta.
fn grid_kq(arena: &Arena, cfg: &Config, base: &Simple, ext: Option<&Ext>)
    -> Result<(), Box<dyn std::error::Error>> {
    let mut p = base.clone();
    println!("| lineup | K | k_q | tune spearman | NDCG | NDCG@3 |");
    println!("|---|---:|---:|---:|---:|---:|");
    for lineup in [S2Lineup::Mean, S2Lineup::Roster] {
        p.lineup = lineup;
        for &k in K_GRID {
            for &kq in KQ_GRID {
                (p.k, p.k_q) = (k, kq);
                let s = run(arena, cfg, &p, ext, None)?;
                println!(
                    "| {lineup:?} | {k} | {kq} | {:.4} | {:.4} | {:.4} |",
                    s.tune.get(),
                    s.tune_ndcg.get(),
                    s.tune_ndcg3.get()
                );
            }
        }
    }
    Ok(())
}

fn grid(arena: &Arena, cfg: &Config, base: &Simple) -> Result<(), Box<dyn std::error::Error>> {
    let mut p = base.clone();
    let mut runs: Vec<(String, f64, f64, f64)> = Vec::new();
    println!("| pass | axis | value | tune spearman | NDCG | NDCG@3 |");
    println!("|---:|---|---:|---:|---:|---:|");
    for pass in 1..=2 {
        for (i, (name, vals)) in AXES.iter().enumerate() {
            let mut best = (f64::NEG_INFINITY, *axis(&mut p, i));
            for &v in vals.iter() {
                *axis(&mut p, i) = v;
                let s = run(arena, cfg, &p, None, None)?;
                let (a, b, c) = (s.tune.get(), s.tune_ndcg.get(), s.tune_ndcg3.get());
                println!("| {pass} | {name} | {v} | {a:.4} | {b:.4} | {c:.4} |");
                runs.push((format!("{name}={v}"), a, b, c));
                if a > best.0 {
                    best = (a, v);
                }
            }
            *axis(&mut p, i) = best.1;
        }
    }
    let lo = runs.iter().map(|r| r.1).fold(f64::INFINITY, f64::min);
    let hi = runs.iter().map(|r| r.1).fold(f64::NEG_INFINITY, f64::max);
    let b3 = runs
        .iter()
        .max_by(|a, b| a.3.total_cmp(&b.3))
        .cloned()
        .unwrap_or_default();
    println!("\n{} runs, tune spearman {lo:.4} to {hi:.4}", runs.len());
    println!(
        "best by NDCG@3: {} ({:.4}, spearman {:.4})",
        b3.0, b3.3, b3.1
    );
    println!(
        "chosen: D {} K {} k_p {} delta {} k_q {} r_new {} k_r {}",
        p.d, p.k, p.k_p, p.delta, p.k_q, p.r_new, p.k_r
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg_path = arg("--config").unwrap_or_else(|| "config_simple.toml".into());
    let mut cfg = Config::load(&cfg_path)?;
    if let Some(v) = arg("--out") {
        cfg.data.out_path = v;
    }
    if let Some(v) = arg("--rows-every") {
        cfg.data.rows_every = v.parse().map_err(|_| "bad --rows-every")?;
    }
    let mut p = cfg.simple.clone().ok_or("config has no [simple] section")?;
    if let Some(v) = arg("--perf") {
        p.perf = match v.as_str() {
            "played_like" => Perf::PlayedLike,
            "pairwise" => Perf::Pairwise,
            _ => return Err("--perf: played_like or pairwise".into()),
        };
    }
    if let Some(v) = arg("--anchor") {
        p.anchor = match v.as_str() {
            "none" => Anchor::None,
            "mean" => Anchor::Mean,
            "mean_sd" => Anchor::MeanSd,
            _ => return Err("--anchor: none, mean or mean_sd".into()),
        };
    }
    if let Some(v) = arg("--lineup") {
        p.lineup = match v.as_str() {
            "mean" => S2Lineup::Mean,
            "roster" => S2Lineup::Roster,
            "tempered" => S2Lineup::Tempered,
            "best6" => S2Lineup::Best6,
            _ => return Err("--lineup: mean, roster, tempered or best6".into()),
        };
    }
    if let Some(v) = arg("--k-r") {
        p.k_r = v.parse().map_err(|_| "--k-r: a number")?;
    }
    if let Some(v) = arg("--player-rule") {
        p.player_rule = match v.as_str() {
            "surprise" => PlayerRule::Surprise,
            "level" => PlayerRule::Level,
            _ => return Err("--player-rule: surprise or level".into()),
        };
    }
    let start = std::time::Instant::now();
    let arena = chgk_rating::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
    println!(
        "loaded {} tournaments, {} players, {} teams in {:.1}s",
        arena.tournaments.len(),
        arena.n_players(),
        arena.n_teams(),
        start.elapsed().as_secs_f64()
    );
    if std::env::args().any(|a| a == "--grid") {
        return grid(&arena, &cfg, &p);
    }
    if std::env::args().any(|a| a == "--calibrate-d") {
        return calibrate_d(&arena, &cfg, &p);
    }
    let ext = arg("--players-from")
        .map(|path| load_ext(&path, &arena, &cfg))
        .transpose()?;
    if std::env::args().any(|a| a == "--grid-kq") {
        return grid_kq(&arena, &cfg, &p, ext.as_ref());
    }
    let w = (!std::env::args().any(|a| a == "--dry"))
        .then(|| Writer::create(&cfg.data.out_path))
        .transpose()?;
    let sc = run(&arena, &cfg, &p, ext.as_ref(), w.as_ref())?;
    if let Some(w) = w {
        w.finish()?;
    }
    println!(
        "tune {:.4} (NDCG {:.4}, @3 {:.4})  test in-person {:.4}  test sync {:.4}",
        sc.tune.get(),
        sc.tune_ndcg.get(),
        sc.tune_ndcg3.get(),
        sc.test.get(),
        sc.test_sync.get()
    );
    println!("newcomer level at the last release: {:.0}", sc.new_level);
    println!(
        "pairwise log-loss {:.4} over {:.0} pairs",
        sc.ll.get(),
        sc.ll.1
    );
    print_bins(&sc);
    println!(
        "2019 mean R - Q(L): {:.1} over {:.0} events; {} in {:.1}s",
        sc.resid19.get(),
        sc.resid19.1,
        cfg.data.out_path,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
