use chgk_rating::chemistry::Chemistry;
use chgk_rating::config::{Config, TeamEngine};
use chgk_rating::data::Arena;
use chgk_rating::difficulty::Difficulties;
use chgk_rating::editor_memory::EditorMemory;
use chgk_rating::features::Features;
use chgk_rating::gauge::{self, Activity};
use chgk_rating::output::Writer;
use chgk_rating::question_model as qm;
use chgk_rating::team_model::{TeamEvent, Teams};
use chgk_rating::team_s2::TeamsS2;
use chgk_rating::util::parse_date as crate_parse_date;
use chgk_rating::util::{format_date, next_thursday, parse_date, spearman};
use std::time::Instant;

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

/// `--swap PLAYER_A:TEAM_A,PLAYER_B:TEAM_B@DATE`: from DATE on, each player
/// takes the other's place. The acceptance test of DESIGN 7.5.
fn apply_swap(arena: &mut Arena, spec: &str) -> Result<(), String> {
    let (pairs, date) = spec.rsplit_once('@').ok_or("--swap needs @DATE")?;
    let day = parse_date(date).ok_or("--swap: bad date")?;
    let mut it = pairs.split(',').map(|p| {
        let (pl, tm) = p.split_once(':').ok_or("--swap needs PLAYER:TEAM")?;
        let pl: i64 = pl.trim().parse().map_err(|_| "--swap: bad player id")?;
        let tm: i64 = tm.trim().parse().map_err(|_| "--swap: bad team id")?;
        Ok::<_, String>((pl, tm))
    });
    let (pa, ta) = it.next().ok_or("--swap needs two pairs")??;
    let (pb, tb) = it.next().ok_or("--swap needs two pairs")??;
    let find = |ids: &[i64], v: i64| ids.iter().position(|x| *x == v).map(|i| i as u32);
    let (Some(ia), Some(ib)) = (find(&arena.player_ids, pa), find(&arena.player_ids, pb)) else {
        return Err("--swap: player not in range".into());
    };
    let (Some(ja), Some(jb)) = (find(&arena.team_ids, ta), find(&arena.team_ids, tb)) else {
        return Err("--swap: team not in range".into());
    };
    let mut n = 0;
    for t in arena.tournaments.iter_mut().filter(|t| t.day >= day) {
        for r in t.teams.iter_mut() {
            let (from, to) = match r.team {
                x if x == ja => (ia, ib),
                x if x == jb => (ib, ia),
                _ => continue,
            };
            for m in r.members.iter_mut() {
                if *m == from {
                    *m = to;
                    n += 1;
                }
            }
        }
    }
    println!("swap: {n} lineup slots changed from {date}");
    Ok(())
}

/// `--scramble DATE`: the **first in-person tournament** on or after DATE has
/// its finishing places and its answer masks permuted among its teams. Nothing
/// a lineup can be judged by on the morning of that tournament changes, so every
/// one of its feature rows must come out bit-identical, while the fields after
/// it must move. `tools/features_check.py` asserts both, and that is the leakage
/// test plan 8 turns on.
///
/// One tournament and not a suffix: the engine reads the tournaments in date
/// order, so scrambling a whole tail would move the player states that the
/// second scrambled field reads, and the test would fail for a reason that is
/// not leakage.
fn apply_scramble(arena: &mut Arena, date: &str) -> Result<(), String> {
    let day = parse_date(date).ok_or("--scramble: bad date")?;
    let mut rng = chgk_rating::util::Rng::new(0x5C2A_3B1E);
    let t = arena
        .tournaments
        .iter_mut()
        .find(|t| t.day >= day && t.format == chgk_rating::data::Format::InPerson && !t.train_only)
        .ok_or("--scramble: no in-person tournament on or after that date")?;
    let k = t.teams.len();
    if k < 2 {
        return Err("--scramble: that tournament has one team".into());
    }
    let mut ord: Vec<usize> = (0..k).collect();
    for i in (1..k).rev() {
        ord.swap(i, (rng.next_u64() % (i as u64 + 1)) as usize);
    }
    let pos: Vec<f64> = t.teams.iter().map(|r| r.position).collect();
    let masks: Vec<Vec<u8>> = t.teams.iter().map(|r| r.mask.clone()).collect();
    for (i, &j) in ord.iter().enumerate() {
        t.teams[i].position = pos[j];
        t.teams[i].mask = masks[j].clone();
    }
    println!(
        "scramble: tournament {} of {} permuted, {k} teams",
        t.id,
        format_date(t.day)
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg_path = arg("--config").unwrap_or_else(|| "config.toml".into());
    let mut cfg = Config::load(&cfg_path)?;
    for (flag, slot) in [
        ("--from", &mut cfg.data.from),
        ("--to", &mut cfg.data.to),
        ("--out", &mut cfg.data.out_path),
    ] {
        if let Some(v) = arg(flag) {
            *slot = v;
        }
    }
    if let Some(v) = arg("--rows-every") {
        cfg.data.rows_every = v.parse().map_err(|_| "bad --rows-every")?;
    }
    let start = Instant::now();
    let from_day = parse_date(&cfg.data.from).ok_or("bad --from")?;
    let to_day = parse_date(&cfg.data.to).ok_or("bad --to")?;
    println!(
        "window {} to {}, rows {}",
        cfg.data.from,
        cfg.data.to,
        match cfg.data.rows_every {
            0 => "at the last release only".to_string(),
            1 => "every release".to_string(),
            n => format!("every {n} releases"),
        }
    );

    let mut arena = chgk_rating::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
    if let Some(s) = arg("--swap") {
        apply_swap(&mut arena, &s)?;
    }
    if let Some(d) = arg("--scramble") {
        apply_scramble(&mut arena, &d)?;
    }
    println!(
        "loaded {} tournaments, {} players, {} teams, {} editors in {:.1}s",
        arena.tournaments.len(),
        arena.n_players(),
        arena.n_teams(),
        arena.n_editors(),
        start.elapsed().as_secs_f64()
    );

    let passes = arg("--passes").map_or(cfg.data.passes, |v| v.parse().unwrap_or(1));
    let warm_until = parse_date(&cfg.player.warm_until).ok_or("bad player.warm_until")?;
    let mut snap: Vec<Option<f64>> = Vec::new();
    // The wide feature table of plan 8, written on the last pass only. It is a
    // research output: nothing in the model reads it and asking for it changes
    // no number the fit produces.
    let features = arg("--features");
    let diag = arg("--diag-player").and_then(|v| v.parse::<i64>().ok());
    let diag = diag.and_then(|id| arena.player_ids.iter().position(|x| *x == id));
    let mut players = qm::Players::new(arena.n_players(), arena.n_editors(), &cfg);
    for p in 0..passes {
        let last = p + 1 == passes;
        if p > 0 {
            let n = snap.iter().filter(|x| x.is_some()).count();
            players.warm_restart(&snap, from_day, &cfg);
            println!("pass {} of {passes}, {n} players warm-started", p + 1);
        }
        let w = last
            .then(|| Writer::create(&cfg.data.out_path))
            .transpose()?;
        let n = run_pass(
            &arena,
            &cfg,
            &mut players,
            w.as_ref(),
            from_day,
            to_day,
            warm_until,
            &mut snap,
            diag,
            start,
            last.then_some(features.as_deref()).flatten(),
        )?;
        if let Some(w) = w {
            w.finish()?;
            println!(
                "done: {n} releases, {:.1}s total",
                start.elapsed().as_secs_f64()
            );
        }
    }
    Ok(())
}

/// The team side of the pass. Both engines write the same `team_events` and the
/// same release rows, so everything downstream reads one schema.
enum Engine {
    Kalman(Teams),
    S2(TeamsS2),
}

impl Engine {
    fn new(n: usize, cfg: &Config) -> Engine {
        match cfg.team.engine {
            TeamEngine::Kalman => Engine::Kalman(Teams::new(n)),
            TeamEngine::S2 => Engine::S2(TeamsS2::new(n)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        t: &chgk_rating::data::Tournament,
        players: &qm::Players,
        params: &[qm::QParams],
        ctx: &chgk_rating::team_s2::Context,
        cfg: &Config,
    ) -> Vec<TeamEvent> {
        match self {
            Engine::Kalman(e) => e.step(t, players, params, cfg),
            Engine::S2(e) => e.step(t, players, ctx, cfg),
        }
    }

    /// `(index, raw rating, published rating, sigma, core strength, evidence)`.
    /// The raw value is what `change` is measured on: for the Kalman engine that
    /// is the pre-gauge scale, and S2 has no gauge of its own to remove.
    fn rows(&self, day: i32, window: i32, cfg: &Config, shift: f64)
        -> Vec<(usize, f64, f64, f64, f64, f64)> {
        match self {
            Engine::Kalman(e) => e
                .listed(day, window)
                .map(|(i, s)| {
                    (i, s.mu_raw, s.mu_raw - shift, s.sigma_at(day, cfg), s.core_raw - shift, s.evidence())
                })
                .collect(),
            Engine::S2(e) => e
                .rows(day, window)
                .into_iter()
                .map(|(i, r, sg, q, n)| (i, r, r, sg, q, n))
                .collect(),
        }
    }
}

/// One chronological sweep. Without a writer nothing is stored and the trailing
/// releases are skipped, so the player state ends on the last tournament.
#[allow(clippy::too_many_arguments)]
fn run_pass(
    arena: &Arena,
    cfg: &Config,
    players: &mut qm::Players,
    w: Option<&Writer>,
    from_day: i32,
    to_day: i32,
    warm_until: i32,
    snap: &mut Vec<Option<f64>>,
    diag: Option<usize>,
    start: Instant,
    features: Option<&str>,
) -> Result<i64, Box<dyn std::error::Error>> {
    let zfrom = crate_parse_date("2018-01-01").unwrap();
    let mut teams = Engine::new(arena.n_teams(), cfg);
    let mut chem = Chemistry::new(
        cfg.question.chem_u != 0.0 || cfg.team.s2.chemistry != 0.0 || features.is_some(),
    );
    let mut diff = Difficulties::new(cfg.team.s2.difficulty_forecast);
    let mut edmem = EditorMemory::new(cfg.team.s2.editor_memory != 0.0, cfg.team.s2.ed_min_n);
    let mut feats = features
        .map(|p| Features::create(p, arena))
        .transpose()?;
    let mut activity = Activity::new(arena.n_players());
    // Last published raw value per entity, so `change` excludes the gauge shift.
    let mut prev_p = vec![f64::NAN; arena.n_players()];
    let mut prev_t = vec![f64::NAN; arena.n_teams()];
    let mut ed_seen = vec![0.0f64; arena.n_editors()];
    let mut release_id: i64 = 0;
    let mut next_rel = next_thursday(from_day);

    for (n, t) in arena.tournaments.iter().enumerate() {
        if snap.is_empty() && t.day >= warm_until {
            *snap = players.snapshot();
        }
        while next_rel <= t.day {
            release_id += 1;
            emit_release(
                w,
                release_id,
                next_rel,
                players,
                &mut activity,
                &teams,
                arena,
                cfg,
                diag,
                &mut prev_p,
                &mut prev_t,
                // The last release is the last Thursday before `to`, whether or
                // not a tournament follows it; the trailing loop uses the same rule.
                next_rel + 7 > to_day,
            )?;
            next_rel += 7;
            if release_id % 50 == 0 {
                if let Some(w) = w {
                    w.checkpoint()?;
                }
            }
        }
        players.seed(t, cfg);

        let field = qm::build_field(t, players, &chem, cfg);
        let mut n_obs = 0u64;
        let mut cross = 0.0;
        // A tournament kept for the player side alone is never scored and never
        // listed, so the cross-fit it would need is not run either.
        if !t.train_only {
            for half in 0..2u8 {
                let p = qm::fit_all(&field, Some(half), cfg);
                let (s, k) = qm::score(&field, &p, Some(1 - half), cfg);
                cross += s;
                n_obs += k;
            }
        }
        let params = qm::fit_all(&field, None, cfg);
        let (ins, n_all) = qm::score(&field, &params, None, cfg);
        let grads = qm::player_grads(&field, &params, cfg);
        // The team states the forecast was made against, before the step moves
        // them; the player states the row reads are untouched by it.
        let pre = feats.as_mut().map(|f| {
            let s2 = match &teams {
                Engine::S2(e) => Some(e),
                Engine::Kalman(_) => None,
            };
            s2.map(|e| f.pre(t, e))
        });
        let events = if t.train_only {
            Vec::new()
        } else {
            let ctx = chgk_rating::team_s2::Context {
                chem: &chem,
                diff: &diff,
                edmem: &edmem,
            };
            teams.step(t, players, &params, &ctx, cfg)
        };
        // The feature row is written here, after the field has been scored and
        // before any memory has absorbed it, so every column is the state of
        // that morning.
        if let (Some(f), Some(Some(pre))) = (feats.as_mut(), pre.as_ref()) {
            f.emit(t, arena, players, &chem, &edmem, pre, &events, cfg)?;
        }
        // The difficulty sample and the pair counter are advanced only once the
        // tournament has been forecast, so neither reads its own field.
        if diff.on() && t.format == chgk_rating::data::Format::InPerson && !t.train_only {
            let mut q: Vec<f64> = events.iter().map(|e| e.s_r).filter(|x| x.is_finite()).collect();
            q.sort_by(|a, b| b.total_cmp(a));
            let k = 10.min(q.len());
            if k > 0 {
                let s = q[..k].iter().sum::<f64>() / k as f64;
                diff.absorb(s, params.iter().map(|p| p.b));
            }
        }
        if !t.train_only {
            chem.absorb(t);
        }
        // The editor memory sees today's residuals only after today's forecast.
        if edmem.on() && t.format == chgk_rating::data::Format::InPerson && !t.train_only {
            let n = events.len().max(1) as f64;
            let res: Vec<(usize, f64)> = events
                .iter()
                .enumerate()
                .filter(|(_, e)| e.actual_place > 0.0)
                .map(|(k, e)| (k, (e.forecast_place - e.actual_place) / n))
                .collect();
            edmem.absorb(t, &res);
        }

        let w = if t.train_only { None } else { w };
        if let Some(w) = w {
            w.question_params(
                t.id,
                &params
                    .iter()
                    .map(|p| (p.b, p.a(), p.m()))
                    .collect::<Vec<_>>(),
            )?;
            w.team_events(t.id, &format_date(t.day), &arena.team_ids, &events)?;
            let actual: Vec<f64> = events.iter().map(|e| e.actual_place).collect();
            let sp_forecast = spearman(
                &events.iter().map(|e| e.forecast_place).collect::<Vec<_>>(),
                &actual,
            );
            let (po, pa): (Vec<f64>, Vec<f64>) = events
                .iter()
                .filter_map(|e| e.pred_place.map(|p| (p, e.actual_place)))
                .unzip();
            w.tournament_metrics(
                t.id,
                &format_date(t.day),
                t.format.name(),
                t.weight,
                t.teams.len(),
                t.n_q,
                n_all,
                (n_all > 0).then(|| ins / n_all as f64),
                (n_obs > 0).then(|| cross / n_obs as f64),
                sp_forecast,
                spearman(&po, &pa),
                qm::truedl(&field, &params, cfg),
            )?;
        }

        let collect = t.format == chgk_rating::data::Format::InPerson && t.day >= zfrom;
        let mut lineup: Vec<u32> = t
            .teams
            .iter()
            .flat_map(|r| r.members.iter().copied())
            .collect();
        lineup.sort_unstable();
        lineup.dedup();
        // The state the forecast read this morning, before the update moves it.
        let before: Vec<(f64, f64, f64, i64, i64)> = lineup
            .iter()
            .map(|&i| {
                let i = i as usize;
                let s = &players.state[i];
                (
                    players.theta(i),
                    players.phi(i),
                    players.sigma_at(i, t.day, cfg),
                    s.games_ip as i64,
                    if s.games_ip > 0 {
                        (t.day - s.last_ip) as i64
                    } else {
                        -1
                    },
                )
            })
            .collect();
        if let Some(f) = feats.as_mut() {
            f.absorb_teams(t, &events);
        }
        players.absorb(t, &grads, &mut ed_seen, collect, cfg);
        players.shrink_to_lineups(t, cfg);
        if let Some(f) = feats.as_mut() {
            f.absorb_players(t, &grads, players);
        }
        if let Some(w) = w {
            let rows: Vec<chgk_rating::output::PlayerEvent> = lineup
                .iter()
                .zip(&before)
                .map(|(&i, b)| {
                    let i = i as usize;
                    chgk_rating::output::PlayerEvent {
                        player: arena.player_ids[i],
                        theta_before: b.0,
                        theta_after: players.theta(i),
                        sigma_after: players.sigma_at(i, t.day, cfg),
                        phi_before: b.1,
                        sigma_before: b.2,
                        games_ip: b.3,
                        days_ip: b.4,
                    }
                })
                .collect();
            w.player_events(t.id, &rows)?;
        }
        for &(i, _, info) in &grads {
            activity.push(t.day, i, t.weight * info);
        }
        if n % 200 == 0 {
            println!(
                "{:6}/{} {} {:.0}s",
                n,
                arena.tournaments.len(),
                format_date(t.day),
                start.elapsed().as_secs_f64()
            );
        }
    }
    if let Some(f) = feats {
        println!("features: {} rows", f.rows());
        f.finish()?;
    }
    if snap.is_empty() {
        *snap = players.snapshot();
    }
    if w.is_some() {
        while next_rel <= to_day {
            release_id += 1;
            emit_release(
                w,
                release_id,
                next_rel,
                players,
                &mut activity,
                &teams,
                arena,
                cfg,
                diag,
                &mut prev_p,
                &mut prev_t,
                next_rel + 7 > to_day,
            )?;
            next_rel += 7;
        }
    }
    Ok(release_id)
}

#[allow(clippy::too_many_arguments)]
fn emit_release(
    w: Option<&Writer>,
    release_id: i64,
    day: i32,
    players: &mut qm::Players,
    activity: &mut Activity,
    teams: &Engine,
    arena: &Arena,
    cfg: &Config,
    diag: Option<usize>,
    prev_p: &mut [f64],
    prev_t: &mut [f64],
    last: bool,
) -> rusqlite::Result<()> {
    activity.expire_before(day - cfg.gauge.window_days);
    let shift = gauge::shift(players, activity, cfg.gauge.n_est);
    players.shift_cum += shift;
    // The affinity gate stands at a quantile of the same established panel the
    // gauge centres, so "a strong player" means strong against today's field.
    if cfg.question.aff_strength == chgk_rating::config::AffStrength::Gate
        || cfg.team.s2.ed_gate > 0.0
    {
        let mut panel: Vec<f64> = (0..arena.n_players())
            .filter(|&i| activity.count[i] >= cfg.gauge.n_est)
            .map(|i| players.theta(i))
            .collect();
        players.set_gate(&mut panel, cfg.question.aff_gate);
        players.set_ed_gate(&mut panel, cfg.team.s2.ed_gate);
    }
    let mut rows = Vec::new();
    for (i, &c) in activity.count.iter().enumerate() {
        if c > 0 {
            // The published number may carry a share of the format effect; the
            // forecast reads `theta` and `phi` separately and never this.
            rows.push((
                arena.player_ids[i],
                players.theta(i) + cfg.display.player_phi * players.phi(i),
                players.sigma_at(i, day, cfg),
                c,
                i,
            ));
        }
    }
    let Some(w) = w else { return Ok(()) };
    if let Some(i) = diag {
        if activity.count[i] > 0 {
            let s = &players.state[i];
            let dt = ((day - s.last_day).max(0) as f64) / 365.25;
            println!(
                "diag {} games {} fisher {:.2} sigma_from_fisher {:.3} sigma_at_game {:.3} vol {:.3} sigma {:.3}",
                format_date(day),
                activity.count[i],
                activity.fisher[i],
                (1.0 / activity.fisher[i].max(1e-9)).sqrt(),
                s.var.sqrt(),
                s.vol,
                (s.var + s.vol * s.vol * dt).sqrt()
            );
        }
    }
    if last {
        let mut band: Vec<(f64, f64)> = (0..arena.n_players())
            .filter(|&i| activity.count[i] > 0)
            .map(|i| {
                (
                    players.theta(i),
                    activity.fisher[i] / activity.count[i] as f64,
                )
            })
            .collect();
        band.sort_by(|a, b| b.0.total_cmp(&a.0));
        let med = |v: &[(f64, f64)]| {
            let mut x: Vec<f64> = v.iter().map(|p| p.1).collect();
            x.sort_by(f64::total_cmp);
            x.get(x.len() / 2).copied().unwrap_or(0.0)
        };
        println!(
            "fisher per game: top 200 {:.3}, ranks 2000-5000 {:.3}",
            med(&band[..200.min(band.len())]),
            med(&band[2000.min(band.len())..5000.min(band.len())])
        );
        if cfg.question.format_effect != chgk_rating::config::FormatEffect::None {
            let mut phi: Vec<f64> = (0..arena.n_players())
                .filter(|&i| activity.count[i] > 0)
                .map(|i| players.state[i].aff[0])
                .collect();
            phi.sort_by(f64::total_cmp);
            let q = |f: f64| phi[((phi.len() - 1) as f64 * f) as usize];
            println!(
                "phi over {} listed players: mean {:.3}, deciles {:.3} {:.3}, median {:.3}, {:.3} {:.3}",
                phi.len(),
                phi.iter().sum::<f64>() / phi.len() as f64,
                q(0.0),
                q(0.1),
                q(0.5),
                q(0.9),
                q(1.0)
            );
        }
        let (n, s1, s2) = players.zstat;
        if n > 0.0 {
            println!(
                "player zvar {:.3} (n {n:.0}, mean {:.3})",
                s2 / n - (s1 / n) * (s1 / n),
                s1 / n
            );
        }
        let np = (0..arena.n_players())
            .filter(|&i| activity.count[i] > 0)
            .filter(|&i| players.sigma_at(i, day, cfg) <= cfg.display.provisional_sigma)
            .count();
        let nl = activity.count.iter().filter(|&&c| c > 0).count();
        println!(
            "players rated {np}/{nl} ({:.0}%)",
            100.0 * np as f64 / nl as f64
        );
    }
    w.release(
        release_id,
        &format_date(day),
        shift,
        players.shift_cum,
        rows.len(),
    )?;
    if cfg.data.write_rows(release_id, last) {
        let prows: Vec<(i64, f64, f64, u32, f64)> = rows
            .iter()
            .map(|&(id, theta, sigma, c, i)| {
                let raw = players.state[i].theta_raw;
                let change = if prev_p[i].is_finite() {
                    raw - prev_p[i]
                } else {
                    0.0
                };
                prev_p[i] = raw;
                (id, theta, sigma, c, change)
            })
            .collect();
        w.player_ratings(release_id, &prows, |t| chgk_rating::display::player(t, cfg))?;
        let trows: Vec<(i64, f64, f64, f64, f64, f64)> = teams
            .rows(day, cfg.gauge.window_days, cfg, players.shift_cum)
            .into_iter()
            .map(|(i, raw, mu, sigma, core, evidence)| {
                let change = if prev_t[i].is_finite() { raw - prev_t[i] } else { 0.0 };
                prev_t[i] = raw;
                (arena.team_ids[i], mu, sigma, core, evidence, change)
            })
            .collect();
        w.team_ratings(release_id, &trows, |m| chgk_rating::display::team(m, cfg))?;
    }
    Ok(())
}
