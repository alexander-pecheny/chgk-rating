mod data;
mod model;
mod refdl;
mod util;

use data::*;
use model::*;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use util::*;

const FROM: &str = "2013-01-01";
const REPS: usize = 6;
const ACTIVE_WINDOW: i64 = 365;

struct Row {
    id: i64,
    date: String,
    ty: String,
    n_teams: usize,
    n_teams_dl: usize,
    n_q: usize,
    place_src: &'static str,
    truedl: f64,
    truedl_rounded: f64,
    truedl_trunc: f64,
    truedl_sd: f64,
    truedl_rms: f64,
    med_place: f64,
    top100: f64,
    clamp_share: f64,
    // stored-parameter measures, filled once theta_ref is known
    b: Vec<f64>,
    a: Vec<f64>,
    m: Vec<f64>,
    // refits: index 0 = full field, 1.. = random halves
    fit_b: Vec<Vec<f64>>,
    fit_a: Vec<Vec<f64>>,
}

fn main() {
    let root = "/home/pecheny/new_chgk_rating";
    let buff = open_ro(&format!("{root}/buff.db"));
    let snap = open_ro(&format!("{root}/truedl/ratings_snapshot.db"));

    eprintln!("loading tournaments...");
    let tourns = load_tournaments(&buff, FROM);
    eprintln!("  {} rated tournaments", tourns.len());
    eprintln!("loading results...");
    let mut results = load_results(&buff, FROM);
    eprintln!("loading question params...");
    let qparams = load_question_params(&snap);
    eprintln!("loading real B releases...");
    let releases = load_releases(&buff);
    let first_release_day = releases.first().map(|r| r.0).unwrap_or(i64::MAX);
    let snap_releases = load_snapshot_releases(&snap);

    eprintln!("reconstructing places...");
    let (min_games, window) = (RECON_MIN_GAMES, RECON_WINDOW);
    let recon = recon_places(&tourns, &results, min_games, window, RECON_AGG);
    if std::env::args().any(|a| a == "tune") {
        tune_places(&tourns, &results, &releases);
        return;
    }

    // ---- main pass ----------------------------------------------------------
    let mut rows: Vec<Row> = Vec::new();
    let mut ref_strengths: Vec<f64> = Vec::new(); // log(sum exp theta / 6) of B-place 251..500 lineups
    let mut place_agree: Vec<(f64, f64)> = Vec::new();
    let mut cur_rel = usize::MAX;
    let mut thetas_now: HashMap<i64, f64> = HashMap::new();

    let total = tourns.len();
    for (ti, t) in tourns.iter().enumerate() {
        if ti % 250 == 0 {
            eprintln!("  {ti}/{total}");
        }
        let rs = match results.remove(&t.id) {
            Some(r) => r,
            None => continue,
        };
        let qp = match qparams.get(&t.id) {
            Some(q) => q,
            None => continue,
        };
        // teams that carry a mask and a lineup, the ones the question model sees
        let teams: Vec<&Res> = rs
            .iter()
            .filter(|r| !r.mask.is_empty() && !r.lineup.is_empty())
            .collect();
        if teams.len() < 2 {
            continue;
        }
        let n_q = teams.iter().map(|r| r.mask.len()).max().unwrap();
        if n_q == 0 || n_q > qp.b.len() {
            continue;
        }
        // questions actually played: at least one team answered them
        let mut played = vec![false; n_q];
        for r in &teams {
            for (q, &c) in r.mask.iter().enumerate() {
                if c != 2 {
                    played[q] = true;
                }
            }
        }
        let qidx: Vec<usize> = (0..n_q).filter(|&q| played[q]).collect();
        if qidx.len() < 6 {
            continue;
        }
        let n_played = qidx.len() as f64;

        // ---- reference TrueDL ----
        let real = if t.day >= first_release_day {
            releases
                .iter()
                .rev()
                .find(|(d, _)| *d <= t.day)
                .map(|(_, m)| m)
        } else {
            None
        };
        let rec = recon.get(&t.id);
        let place_of = |team: i64| -> Option<f64> {
            if let Some(m) = real {
                m.get(&team).copied()
            } else {
                rec.and_then(|m| m.get(&team).copied())
            }
        };
        if let (Some(m), Some(rc)) = (real, rec) {
            for r in &rs {
                if let (Some(p1), Some(p2)) = (m.get(&r.team), rc.get(&r.team)) {
                    place_agree.push((*p1, *p2));
                }
            }
        }
        let mut dl_vals: Vec<f64> = Vec::new();
        let mut dl_raw: Vec<f64> = Vec::new();
        let mut dl_trunc: Vec<f64> = Vec::new();
        let mut dl_by_team: HashMap<i64, f64> = HashMap::new();
        let mut places: Vec<f64> = Vec::new();
        let mut clamped: usize = 0;
        for r in &rs {
            if !r.in_rating || r.mask.is_empty() {
                continue;
            }
            let p = match place_of(r.team) {
                Some(p) => p,
                None => continue,
            };
            let q = r.mask.iter().filter(|&&c| c == 1).count() as f64;
            if let Some(v) = refdl::team_truedl(q, n_played, p, 0) {
                dl_vals.push(v);
                let raw = refdl::team_truedl(q, n_played, p, 1).unwrap();
                dl_by_team.insert(r.team, raw);
                dl_raw.push(raw);
                dl_trunc.push(refdl::team_truedl(q, n_played, p, 2).unwrap());
                places.push(p);
                if q / refdl::coeff(p).unwrap() >= n_played {
                    clamped += 1;
                }
            }
        }
        let truedl_rounded = if dl_vals.is_empty() { f64::NAN } else { round1(mean(&dl_vals)) };
        let truedl = if dl_raw.is_empty() { f64::NAN } else { mean(&dl_raw) };
        let truedl_trunc = if dl_trunc.is_empty() { f64::NAN } else { mean(&dl_trunc) };

        // ---- player strengths at the release before the tournament ----
        let mut ridx = 0usize;
        for (k, (_, d)) in snap_releases.iter().enumerate() {
            if *d <= t.day {
                ridx = k;
            } else {
                break;
            }
        }
        if ridx != cur_rel {
            thetas_now = load_thetas(&snap, snap_releases[ridx].0);
            cur_rel = ridx;
        }
        let th_tab: Vec<Vec<f64>> = teams
            .iter()
            .map(|r| {
                r.lineup
                    .iter()
                    .map(|p| *thetas_now.get(p).unwrap_or(&MU_NEW))
                    .collect()
            })
            .collect();

        // reference-strength sample: teams sitting at B places 251..500
        if t.date.as_str() >= "2024-01-01" {
            for (k, r) in teams.iter().enumerate() {
                if let Some(p) = place_of(r.team) {
                    if p >= 251.0 && p <= 500.0 {
                        let s: f64 = th_tab[k].iter().map(|x| x.exp()).sum();
                        if s > 0.0 {
                            ref_strengths.push((s / REF_SIZE).ln());
                        }
                    }
                }
            }
        }

        // ---- refits: full field and REPS random halves ----
        let do_sub = teams.len() >= 8 && qidx.len() >= 12;
        let n_fits = if do_sub { 1 + REPS } else { 1 };
        let mut keeps: Vec<Vec<bool>> = Vec::with_capacity(n_fits);
        keeps.push(vec![true; teams.len()]);
        let mut rng = Rng(0x5EED ^ (t.id as u64).wrapping_mul(0x9E3779B1));
        for _ in 1..n_fits {
            let mut order: Vec<usize> = (0..teams.len()).collect();
            for k in (1..order.len()).rev() {
                let j = rng.below(k + 1);
                order.swap(k, j);
            }
            let half = teams.len() / 2;
            let mut keep = vec![false; teams.len()];
            for &o in &order[..half] {
                keep[o] = true;
            }
            keeps.push(keep);
        }

        let mut fit_b: Vec<Vec<f64>> = Vec::with_capacity(n_fits);
        let mut fit_a: Vec<Vec<f64>> = Vec::with_capacity(n_fits);
        for keep in &keeps {
            let obs_by_q: Vec<Vec<Obs>> = qidx
                .iter()
                .map(|&q| {
                    let mut v = Vec::new();
                    for (k, r) in teams.iter().enumerate() {
                        if keep[k] && q < r.mask.len() && r.mask[q] != 2 {
                            v.push(Obs { team: k as u32, y: r.mask[q] });
                        }
                    }
                    v
                })
                .collect();
            let fitted: Vec<(f64, f64)> = obs_by_q
                .par_iter()
                .map(|o| fit_question(o, &th_tab, t.w))
                .collect();
            fit_b.push(fitted.iter().map(|x| x.0).collect());
            fit_a.push(fitted.iter().map(|x| x.1).collect());
        }

        // TrueDL on the same halves
        let mut dl_sub: Vec<f64> = Vec::new();
        if do_sub {
            for keep in keeps.iter().skip(1) {
                let mut v = Vec::new();
                for (k, r) in teams.iter().enumerate() {
                    if keep[k] {
                        if let Some(x) = dl_by_team.get(&r.team) {
                            v.push(*x);
                        }
                    }
                }
                if v.len() >= 2 {
                    dl_sub.push(mean(&v));
                }
            }
        }

        rows.push(Row {
            id: t.id,
            date: t.date.clone(),
            ty: t.ty.clone(),
            n_teams: teams.len(),
            n_teams_dl: dl_vals.len(),
            n_q: qidx.len(),
            place_src: if real.is_some() { "release" } else { "recon" },
            truedl,
            truedl_rounded,
            truedl_trunc,
            truedl_sd: if dl_sub.len() >= 2 { sd(&dl_sub) } else { f64::NAN },
            truedl_rms: if dl_sub.len() >= 2 {
                (dl_sub.iter().map(|x| (x - truedl) * (x - truedl)).sum::<f64>() / dl_sub.len() as f64).sqrt()
            } else {
                f64::NAN
            },
            med_place: if places.is_empty() { f64::NAN } else { median(&mut places.clone()) },
            top100: places.iter().filter(|&&p| p <= 100.0).count() as f64 / places.len().max(1) as f64,
            clamp_share: clamped as f64 / places.len().max(1) as f64,
            b: qidx.iter().map(|&q| qp.b[q]).collect(),
            a: qidx.iter().map(|&q| qp.a[q]).collect(),
            m: qidx.iter().map(|&q| qp.m[q]).collect(),
            fit_b,
            fit_a,
        });
    }

    // ---- reference strength -------------------------------------------------
    let theta_ref = median(&mut ref_strengths.clone());
    eprintln!("theta_ref (median over {} B-place 251-500 lineups) = {:.4}", ref_strengths.len(), theta_ref);

    let mut out = File::create(format!("{root}/truedl/out.csv")).unwrap();
    writeln!(
        out,
        "tournament_id,date,type,n_teams,n_teams_truedl,n_q,place_source,truedl,improved,\
truedl_sub_sd,improved_sub_sd,truedl_sub_rms,improved_sub_rms,theta_half,improved_refit,\
median_place,share_top100,share_clamped"
    )
    .unwrap();

    let mut rep_lines: Vec<String> = Vec::new();
    let mut all: Vec<(f64, f64, f64, f64, String, String, i64, usize, &'static str)> = Vec::new();
    let mut fld: Vec<(f64, f64, f64, f64, f64)> = Vec::new(); // truedl, improved, med_place, top100, clamp
    let mut stored_vs_refit: (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    let mut improved_by_id: HashMap<i64, (f64, f64, f64)> = HashMap::new();

    for r in &rows {
        let ones = vec![1.0; r.b.len()];
        let m = if r.m.len() == r.b.len() { r.m.clone() } else { ones };
        let improved = 10.0 * (1.0 - ref_rate(theta_ref, &r.b, &r.a, &m));
        let th = theta_half(&r.b, &r.a, &m);
        let m1 = vec![1.0; r.fit_b[0].len()];
        let improved_refit = 10.0 * (1.0 - ref_rate(theta_ref, &r.fit_b[0], &r.fit_a[0], &m1));
        let mut subs: Vec<f64> = Vec::new();
        for k in 1..r.fit_b.len() {
            let mk = vec![1.0; r.fit_b[k].len()];
            subs.push(10.0 * (1.0 - ref_rate(theta_ref, &r.fit_b[k], &r.fit_a[k], &mk)));
        }
        let isd = if subs.len() >= 2 { sd(&subs) } else { f64::NAN };
        let irms = if subs.len() >= 2 {
            (subs.iter().map(|x| (x - improved_refit).powi(2)).sum::<f64>() / subs.len() as f64).sqrt()
        } else {
            f64::NAN
        };
        stored_vs_refit.0.push(improved);
        stored_vs_refit.1.push(improved_refit);
        improved_by_id.insert(r.id, (improved, isd, th));
        writeln!(
            out,
            "{},{},{},{},{},{},{},{},{:.4},{},{},{},{},{:.4},{:.4},{},{:.4},{:.4}",
            r.id,
            r.date,
            r.ty,
            r.n_teams,
            r.n_teams_dl,
            r.n_q,
            r.place_src,
            fmt(r.truedl),
            improved,
            fmt(r.truedl_sd),
            fmt(isd),
            fmt(r.truedl_rms),
            fmt(irms),
            th,
            improved_refit,
            fmt(r.med_place),
            r.top100,
            r.clamp_share
        )
        .unwrap();
        all.push((r.truedl, improved, r.truedl_rms, irms, r.date.clone(), r.ty.clone(), r.id, r.n_teams, r.place_src));
        if r.truedl.is_finite() && r.med_place.is_finite() {
            fld.push((r.truedl, improved, r.med_place, r.top100, r.clamp_share));
        }
    }
    drop(out);

    // ---- agreement with the values published on rating.chgk.gg -------------
    {
        let pubv = read_published(&format!("{root}/truedl/published_truedl.csv"));
        let byid: HashMap<i64, &Row> = rows.iter().map(|r| (r.id, r)).collect();
        let mut p: Vec<f64> = Vec::new();
        let mut v0: Vec<f64> = Vec::new();
        let mut v1: Vec<f64> = Vec::new();
        let mut v2: Vec<f64> = Vec::new();
        for (id, val) in &pubv {
            if let Some(r) = byid.get(id) {
                if r.truedl.is_finite() {
                    p.push(*val);
                    v0.push(r.truedl_rounded);
                    v1.push(r.truedl);
                    v2.push(r.truedl_trunc);
                }
            }
        }
        if p.len() > 3 {
            for (lab, v) in [("rounded (CLI)", &v0), ("unrounded", &v1), ("truncated (batch)", &v2)] {
                let rmse = (p.iter().zip(v).map(|(x, y)| (x - y) * (x - y)).sum::<f64>() / p.len() as f64).sqrt();
                let bias = v.iter().zip(&p).map(|(x, y)| x - y).sum::<f64>() / p.len() as f64;
                let mut ad: Vec<f64> = p.iter().zip(v).map(|(x, y)| (x - y).abs()).collect();
                rep_lines.push(format!(
                    "published vs my reimplementation [{lab}]: n={}, pearson={:.4}, spearman={:.4}, rmse={:.3}, bias={:+.3}, median|diff|={:.3}",
                    p.len(), pearson(&p, v), spearman(&p, v), rmse, bias, median(&mut ad)
                ));
            }
        }
    }

    // ---- report -------------------------------------------------------------
    rep_lines.push(format!("tournaments written: {}", rows.len()));
    rep_lines.push(format!("theta_ref = {:.4} (median roster strength / 6 of B-place 251-500 lineups, n={})", theta_ref, ref_strengths.len()));
    if !place_agree.is_empty() {
        let a: Vec<f64> = place_agree.iter().map(|x| x.0).collect();
        let b: Vec<f64> = place_agree.iter().map(|x| x.1).collect();
        rep_lines.push(format!(
            "reconstructed vs real B place: n={}, spearman={:.4}, median |diff|={:.1}",
            a.len(),
            spearman(&a, &b),
            median(&mut place_agree.iter().map(|x| (x.0 - x.1).abs()).collect::<Vec<f64>>())
        ));
    }
    rep_lines.push(format!(
        "stored params vs my full-field refit: pearson={:.4}, spearman={:.4}, rmse={:.3}",
        pearson(&stored_vs_refit.0, &stored_vs_refit.1),
        spearman(&stored_vs_refit.0, &stored_vs_refit.1),
        (stored_vs_refit
            .0
            .iter()
            .zip(&stored_vs_refit.1)
            .map(|(x, y)| (x - y) * (x - y))
            .sum::<f64>()
            / stored_vs_refit.0.len() as f64)
            .sqrt()
    ));

    // scale of the two measures
    let ok: Vec<usize> = (0..all.len()).filter(|&i| all[i].0.is_finite() && all[i].1.is_finite()).collect();
    let x: Vec<f64> = ok.iter().map(|&i| all[i].1).collect();
    let y: Vec<f64> = ok.iter().map(|&i| all[i].0).collect();
    let (a0, b0) = ols(&x, &y);
    rep_lines.push(format!(
        "n with both = {}; truedl = {:.4} + {:.4} * improved; pearson={:.4}, spearman={:.4}",
        x.len(), a0, b0, pearson(&x, &y), spearman(&x, &y)
    ));
    rep_lines.push(format!(
        "mean truedl={:.3} sd={:.3}; mean improved={:.3} sd={:.3}",
        mean(&y), sd(&y), mean(&x), sd(&x)
    ));

    {
        let sel: Vec<usize> = ok.iter().cloned().filter(|&i| all[i].8 == "release").collect();
        let xx: Vec<f64> = sel.iter().map(|&i| all[i].1).collect();
        let yy: Vec<f64> = sel.iter().map(|&i| all[i].0).collect();
        rep_lines.push(format!(
            "release-backed places only (2021-09 on): n={}, spearman={:.4}, pearson={:.4}",
            sel.len(), spearman(&xx, &yy), pearson(&xx, &yy)
        ));
    }
    for (label, lo, hi) in [("2024-2026", "2024-01-01", "2027-01-01"), ("2013-2019", "2013-01-01", "2020-01-01")] {
        let sel: Vec<usize> = ok.iter().cloned().filter(|&i| all[i].4.as_str() >= lo && all[i].4.as_str() < hi).collect();
        if sel.len() > 10 {
            let xx: Vec<f64> = sel.iter().map(|&i| all[i].1).collect();
            let yy: Vec<f64> = sel.iter().map(|&i| all[i].0).collect();
            rep_lines.push(format!("{label}: n={}, spearman(truedl, improved)={:.4}, pearson={:.4}", sel.len(), spearman(&xx, &yy), pearson(&xx, &yy)));
        }
    }

    // stability
    let stab: Vec<usize> = (0..all.len()).filter(|&i| all[i].2.is_finite() && all[i].3.is_finite()).collect();
    let mut ts: Vec<f64> = stab.iter().map(|&i| all[i].2).collect();
    let mut is_: Vec<f64> = stab.iter().map(|&i| all[i].3 * b0).collect();
    let mut is_raw: Vec<f64> = stab.iter().map(|&i| all[i].3).collect();
    let sd_t = sd(&stab.iter().map(|&i| all[i].0).collect::<Vec<f64>>());
    let sd_i = sd(&stab.iter().map(|&i| all[i].1).collect::<Vec<f64>>());
    rep_lines.push(format!("subsampling (half the field, {REPS} draws; deviation is rms against the full-field value), n={}", stab.len()));
    rep_lines.push(format!("  truedl   : median rms={:.4}, mean rms={:.4}, sd across tournaments={:.4}, noise/signal={:.4}",
        median(&mut ts.clone()), mean(&ts), sd_t, median(&mut ts) / sd_t));
    rep_lines.push(format!("  improved : median rms={:.4} (raw {:.4}), mean rms={:.4}, sd across tournaments={:.4}, noise/signal={:.4}",
        median(&mut is_.clone()), median(&mut is_raw), mean(&is_), sd_i, median(&mut is_raw.clone()) / sd_i));

    // by field size
    for (lab, lo, hi) in [("n_teams 8-20", 8, 21), ("n_teams 21-60", 21, 61), ("n_teams 61-200", 61, 201), ("n_teams 201+", 201, 100000)] {
        let s: Vec<usize> = stab.iter().cloned().filter(|&i| all[i].7 >= lo && all[i].7 < hi).collect();
        if s.len() > 5 {
            let mut a: Vec<f64> = s.iter().map(|&i| all[i].2).collect();
            let mut b: Vec<f64> = s.iter().map(|&i| all[i].3 * b0).collect();
            rep_lines.push(format!("  {lab}: n={}, truedl median rms={:.4}, improved median rms={:.4}", s.len(), median(&mut a), median(&mut b)));
        }
    }

    // ---- does TrueDL move with the composition of the field? ---------------
    {
        let resid: Vec<f64> = fld.iter().map(|f| f.0 - (a0 + b0 * f.1)).collect();
        let lp: Vec<f64> = fld.iter().map(|f| f.2.max(1.0).ln()).collect();
        let tp: Vec<f64> = fld.iter().map(|f| f.3).collect();
        let cl: Vec<f64> = fld.iter().map(|f| f.4).collect();
        rep_lines.push(format!(
            "field composition, n={}: corr(truedl residual, log median B place)={:+.4}, corr(residual, share of top-100 teams)={:+.4}, corr(residual, share of clamped teams)={:+.4}",
            fld.len(), pearson(&resid, &lp), pearson(&resid, &tp), pearson(&resid, &cl)
        ));
        rep_lines.push(format!(
            "clamped rows (hypothetical result capped at n_q): mean share={:.4}, tournaments with any={:.4}, with over 10%={:.4}",
            mean(&cl),
            cl.iter().filter(|&&x| x > 0.0).count() as f64 / cl.len() as f64,
            cl.iter().filter(|&&x| x > 0.1).count() as f64 / cl.len() as f64
        ));
    }

    // ---- worked examples ----------------------------------------------------
    {
        let names: HashMap<i64, &str> = tourns.iter().map(|t| (t.id, t.name.as_str())).collect();
        let mut by_inst: Vec<&Row> = rows
            .iter()
            .filter(|r| r.truedl_rms.is_finite() && r.n_teams_dl >= 8)
            .collect();
        by_inst.sort_by(|a, b| b.truedl_rms.partial_cmp(&a.truedl_rms).unwrap());
        rep_lines.push("least stable under half-field subsampling (by TrueDL rms):".into());
        for r in by_inst.iter().take(8) {
            let (imp, _, _) = improved_by_id[&r.id];
            rep_lines.push(format!(
                "  {} {} [{}]: n_teams_dl={} truedl={:.2} rms={:.2}; improved={:.2} rms(refit)={:.2}; median place={:.0}",
                r.id, names.get(&r.id).unwrap_or(&""), r.date, r.n_teams_dl, r.truedl, r.truedl_rms, imp,
                {
                    let m1 = vec![1.0; r.fit_b[0].len()];
                    let full = 10.0 * (1.0 - ref_rate(theta_ref, &r.fit_b[0], &r.fit_a[0], &m1));
                    let mut s = 0.0;
                    for k in 1..r.fit_b.len() {
                        let mk = vec![1.0; r.fit_b[k].len()];
                        let v = 10.0 * (1.0 - ref_rate(theta_ref, &r.fit_b[k], &r.fit_a[k], &mk));
                        s += (v - full) * (v - full);
                    }
                    (s / (r.fit_b.len() - 1) as f64).sqrt()
                },
                r.med_place
            ));
        }
        let mut by_clamp: Vec<&Row> = rows.iter().filter(|r| r.n_teams_dl >= 20).collect();
        by_clamp.sort_by(|a, b| b.clamp_share.partial_cmp(&a.clamp_share).unwrap());
        rep_lines.push("largest share of teams whose hypothetical result is capped:".into());
        for r in by_clamp.iter().take(6) {
            let (imp, _, _) = improved_by_id[&r.id];
            rep_lines.push(format!(
                "  {} {} [{}]: clamped={:.2} of {} teams, truedl={:.2}, improved={:.2}",
                r.id, names.get(&r.id).unwrap_or(&""), r.date, r.clamp_share, r.n_teams_dl, r.truedl, imp
            ));
        }
    }

    // ---- package pairs: live tournament vs its synchronous mirror -----------
    let pairs = find_pairs(&buff);
    let mut dt: Vec<f64> = Vec::new();
    let mut di: Vec<f64> = Vec::new();
    let mut pl: Vec<String> = Vec::new();
    let idx: HashMap<i64, usize> = all.iter().enumerate().map(|(k, r)| (r.6, k)).collect();
    for (x, y, nx, ny) in &pairs {
        if let (Some(&i), Some(&j)) = (idx.get(x), idx.get(y)) {
            if all[i].0.is_finite() && all[j].0.is_finite() {
                dt.push((all[i].0 - all[j].0).abs());
                di.push((all[i].1 - all[j].1).abs() * b0);
                pl.push(format!("  {} {} [{:.1} / {:.2}]  vs  {} {} [{:.1} / {:.2}]",
                    x, nx, all[i].0, all[i].1, y, ny, all[j].0, all[j].1));
            }
        }
    }
    // pairs restricted to the release-backed era
    let mut dtr: Vec<f64> = Vec::new();
    let mut dir: Vec<f64> = Vec::new();
    for (x, y, _, _) in &pairs {
        if let (Some(&i), Some(&j)) = (idx.get(x), idx.get(y)) {
            if all[i].0.is_finite() && all[j].0.is_finite() && all[i].8 == "release" && all[j].8 == "release" {
                dtr.push((all[i].0 - all[j].0).abs());
                dir.push((all[i].1 - all[j].1).abs() * b0);
            }
        }
    }
    if dtr.len() > 2 {
        rep_lines.push(format!(
            "package pairs, release-backed era only: n={}, truedl median |diff|={:.4} (mean {:.4}), improved median |diff|={:.4} (mean {:.4})",
            dtr.len(), median(&mut dtr.clone()), mean(&dtr), median(&mut dir.clone()), mean(&dir)
        ));
    }
    {
        // signed: mirror minus live
        let mut st: Vec<f64> = Vec::new();
        let mut si: Vec<f64> = Vec::new();
        for (x, y, _, _) in &pairs {
            if let (Some(&i), Some(&j)) = (idx.get(x), idx.get(y)) {
                if all[i].0.is_finite() && all[j].0.is_finite() {
                    st.push(all[j].0 - all[i].0);
                    si.push((all[j].1 - all[i].1) * b0);
                }
            }
        }
        if st.len() > 2 {
            rep_lines.push(format!(
                "package pairs, signed (mirror minus live): truedl mean={:+.4} median={:+.4}; improved mean={:+.4} median={:+.4}",
                mean(&st), median(&mut st.clone()), mean(&si), median(&mut si.clone())
            ));
        }
    }
    rep_lines.push(format!("package pairs (live vs mirror): n={}", dt.len()));
    if dt.len() > 2 {
        rep_lines.push(format!("  truedl   : median |diff|={:.4}, mean={:.4}", median(&mut dt.clone()), mean(&dt)));
        rep_lines.push(format!("  improved : median |diff|={:.4}, mean={:.4}", median(&mut di.clone()), mean(&di)));
        let xa: Vec<f64> = pairs.iter().filter_map(|(x, y, _, _)| idx.get(x).zip(idx.get(y)).map(|(&i, &j)| (all[i].0, all[j].0))).filter(|v| v.0.is_finite() && v.1.is_finite()).map(|v| v.0).collect();
        let ya: Vec<f64> = pairs.iter().filter_map(|(x, y, _, _)| idx.get(x).zip(idx.get(y)).map(|(&i, &j)| (all[i].0, all[j].0))).filter(|v| v.0.is_finite() && v.1.is_finite()).map(|v| v.1).collect();
        if xa.len() > 2 { rep_lines.push(format!("  truedl pair correlation r={:.4}", pearson(&xa, &ya))); }
        let xb: Vec<f64> = pairs.iter().filter_map(|(x, y, _, _)| idx.get(x).zip(idx.get(y)).map(|(&i, &j)| (all[i].1, all[j].1))).collect::<Vec<_>>().iter().map(|v| v.0).collect();
        let yb: Vec<f64> = pairs.iter().filter_map(|(x, y, _, _)| idx.get(x).zip(idx.get(y)).map(|(&i, &j)| (all[i].1, all[j].1))).collect::<Vec<_>>().iter().map(|v| v.1).collect();
        if xb.len() > 2 { rep_lines.push(format!("  improved pair correlation r={:.4}", pearson(&xb, &yb))); }
    }
    rep_lines.extend(pl);

    let mut f = File::create(format!("{root}/truedl/report.txt")).unwrap();
    for l in &rep_lines {
        println!("{l}");
        writeln!(f, "{l}").unwrap();
    }
    // pair list for the analysis document
    let mut pf = File::create(format!("{root}/truedl/pairs.csv")).unwrap();
    writeln!(pf, "live_id,live_name,mirror_id,mirror_name,truedl_live,truedl_mirror,improved_live,improved_mirror").unwrap();
    for (x, y, nx, ny) in &pairs {
        if let (Some(&i), Some(&j)) = (idx.get(x), idx.get(y)) {
            writeln!(pf, "{},{},{},{},{},{},{:.4},{:.4}", x, nx.replace(',', ";"), y, ny.replace(',', ";"), fmt(all[i].0), fmt(all[j].0), all[i].1, all[j].1).unwrap();
        }
    }
}

fn fmt(x: f64) -> String {
    if x.is_finite() {
        format!("{:.4}", x)
    } else {
        String::new()
    }
}

fn norm_tokens(s: &str) -> Vec<String> {
    let stop = ["синхрон", "синхронный", "синхронная", "зеркало", "полный", "некривое", "с", "и", "в", "на", "по", "the"];
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 3)
        .filter(|t| !stop.contains(t))
        .map(|t| t.to_string())
        .collect()
}

/// Same editors, same tour structure, within 60 days, live vs synchronous, names overlapping.
fn find_pairs(buff: &rusqlite::Connection) -> Vec<(i64, i64, String, String)> {
    let ts = load_tournaments(buff, FROM);
    let mut out = Vec::new();
    for a in &ts {
        if !(a.ty == "Обычный" || a.ty == "Региональный") {
            continue;
        }
        if a.editors.is_empty() || a.editors == "[]" || a.qbt.is_empty() {
            continue;
        }
        for b in &ts {
            if !(b.ty == "Синхрон" || b.ty == "Строго синхронный") {
                continue;
            }
            if b.editors != a.editors || b.qbt != a.qbt {
                continue;
            }
            if (b.day - a.day).abs() > 60 {
                continue;
            }
            let ta = norm_tokens(&a.name);
            let tb = norm_tokens(&b.name);
            if ta.is_empty() || tb.is_empty() {
                continue;
            }
            let inter = ta.iter().filter(|x| tb.contains(x)).count();
            let jac = inter as f64 / (ta.len() + tb.len() - inter) as f64;
            if jac >= PAIR_JACCARD {
                out.push((a.id, b.id, a.name.clone(), b.name.clone()));
            }
        }
    }
    out
}

const RECON_MIN_GAMES: usize = 6;
const RECON_AGG: u8 = 0;
const RECON_WINDOW: i64 = 270;

/// Rank teams by their latest rating from the old system among those with at least
/// `min_games` rated games in the trailing `window` days, and read off each
/// tournament's participants' places from that ranking.
fn recon_places(
    tourns: &[Tourn],
    results: &HashMap<i64, Vec<Res>>,
    min_games: usize,
    window: i64,
    agg: u8,
) -> HashMap<i64, HashMap<i64, f64>> {
    let mut games: HashMap<i64, Vec<(i64, f64)>> = HashMap::new();
    let mut out: HashMap<i64, HashMap<i64, f64>> = HashMap::new();
    let mut i = 0;
    while i < tourns.len() {
        let day = tourns[i].day;
        let mut j = i;
        while j < tourns.len() && tourns[j].day == day {
            j += 1;
        }
        let mut pool: Vec<(i64, f64)> = Vec::new();
        for (t, ds) in games.iter() {
            let recent: Vec<f64> = ds
                .iter()
                .filter(|(d, _)| day - *d <= window && *d < day)
                .map(|(_, r)| *r)
                .collect();
            if recent.len() < min_games {
                continue;
            }
            let r = match agg {
                1 => recent.iter().cloned().fold(0.0f64, f64::max),
                2 => {
                    let mut v = recent.clone();
                    v.sort_by(|a, b| b.partial_cmp(a).unwrap());
                    let k = v.len().min(3);
                    v[..k].iter().sum::<f64>() / k as f64
                }
                _ => *recent.last().unwrap(),
            };
            if r > 0.0 {
                pool.push((*t, r));
            }
        }
        pool.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap());
        let mut place: HashMap<i64, f64> = HashMap::with_capacity(pool.len());
        for (k, (t, _)) in pool.iter().enumerate() {
            place.insert(*t, (k + 1) as f64);
        }
        for t in &tourns[i..j] {
            if let Some(rs) = results.get(&t.id) {
                let mut mine = HashMap::new();
                for r in rs {
                    if let Some(p) = place.get(&r.team) {
                        mine.insert(r.team, *p);
                    }
                }
                out.insert(t.id, mine);
            }
        }
        for t in &tourns[i..j] {
            if let Some(rs) = results.get(&t.id) {
                for r in rs {
                    if r.rg > 0.0 {
                        games.entry(r.team).or_default().push((day, r.rg));
                    }
                }
            }
        }
        i = j;
    }
    out
}

fn tune_places(tourns: &[Tourn], results: &HashMap<i64, Vec<Res>>, releases: &[(i64, HashMap<i64, f64>)]) {
    let first = releases[0].0;
    for agg in [0u8, 1, 2] {
    for min_games in [2usize, 4, 6] {
        for window in [270i64, 365] {
            let rec = recon_places(tourns, results, min_games, window, agg);
            let mut a: Vec<f64> = Vec::new();
            let mut b: Vec<f64> = Vec::new();
            let mut same_bucket = 0usize;
            let mut n = 0usize;
            for t in tourns {
                if t.day < first || t.date.as_str() < "2022-01-01" {
                    continue;
                }
                let real = match releases.iter().rev().find(|(d, _)| *d <= t.day) {
                    Some((_, m)) => m,
                    None => continue,
                };
                let rc = match rec.get(&t.id) {
                    Some(m) => m,
                    None => continue,
                };
                for (team, p1) in real.iter() {
                    if let Some(p2) = rc.get(team) {
                        a.push(*p1);
                        b.push(*p2);
                        n += 1;
                        if refdl::coeff(*p1) == refdl::coeff(*p2) {
                            same_bucket += 1;
                        }
                    }
                }
            }
            if n == 0 {
                continue;
            }
            let mut d: Vec<f64> = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).collect();
            println!(
                "agg={agg} min_games={min_games} window={window}: n={n} spearman={:.4} median|diff|={:.1} same_bucket={:.4}",
                spearman(&a, &b),
                median(&mut d),
                same_bucket as f64 / n as f64
            );
        }
    }
    }
}

fn read_published(path: &str) -> Vec<(i64, f64)> {
    let mut out = Vec::new();
    if let Ok(s) = std::fs::read_to_string(path) {
        for line in s.lines().skip(1) {
            let mut it = line.split(',');
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                if let (Ok(id), Ok(v)) = (a.trim().parse::<i64>(), b.trim().parse::<f64>()) {
                    out.push((id, v));
                }
            }
        }
    }
    out
}

const PAIR_JACCARD: f64 = 0.5;
