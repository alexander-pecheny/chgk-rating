use chgk_rating::config::{Config, FormatEffect};
use chgk_rating::data::{Arena, Format, Tournament};
use chgk_rating::features::{Features, Pre};
use chgk_rating::output::{PlayerEvent, PlayerRow, TournamentMetrics, Writer};
use chgk_rating::pass::{self, Engine, Evening, Morning, Observer, Release};
use chgk_rating::util::{format_date, parse_date, spearman};
use std::time::Instant;

const SCRAMBLE_SEED: u64 = 0x5C2A_3B1E;
/// A progress line every this many tournaments, a WAL checkpoint every this
/// many releases.
const PROGRESS_EVERY: usize = 200;
const CHECKPOINT_EVERY: i64 = 50;
const DAYS_PER_YEAR: f64 = 365.25;

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
    let mut rng = chgk_rating::util::Rng::new(SCRAMBLE_SEED);
    let t = arena
        .tournaments
        .iter_mut()
        .find(|t| t.day >= day && t.format == Format::InPerson && !t.train_only)
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
    let start = Instant::now();
    let cfg = config()?;
    let arena = arena(&cfg, start)?;
    // The wide feature table of plan 8. It is a research output: nothing in the
    // model reads it and asking for it changes no number the fit produces.
    let features = arg("--features").map(|p| Features::create(&p, &arena)).transpose()?;
    let opts = pass::Options {
        passes: arg("--passes").map(|v| v.parse().unwrap_or(1)),
        scores: true,
        chemistry: features.is_some(),
    };
    let mut fit = Fit::new(&arena, &cfg, features, start)?;
    let n = pass::run(&arena, &cfg, opts, &mut fit)?;
    if let Some(f) = fit.features {
        println!("features: {} rows", f.rows());
        f.finish()?;
    }
    fit.w.finish()?;
    println!("done: {n} releases, {:.1}s total", start.elapsed().as_secs_f64());
    Ok(())
}

/// The config, with the window, the output and the row cadence as the command
/// line overrides them.
fn config() -> Result<Config, Box<dyn std::error::Error>> {
    let mut cfg = Config::load(&arg("--config").unwrap_or_else(|| "config.toml".into()))?;
    for (flag, slot) in [("--from", &mut cfg.data.from), ("--to", &mut cfg.data.to), ("--out", &mut cfg.data.out_path)] {
        if let Some(v) = arg(flag) {
            *slot = v;
        }
    }
    if let Some(v) = arg("--rows-every") {
        cfg.data.rows_every = v.parse().map_err(|_| "bad --rows-every")?;
    }
    parse_date(&cfg.data.from).ok_or("bad --from")?;
    parse_date(&cfg.data.to).ok_or("bad --to")?;
    let rows = match cfg.data.rows_every {
        0 => "at the last release only".to_string(),
        1 => "every release".to_string(),
        n => format!("every {n} releases"),
    };
    println!("window {} to {}, rows {rows}", cfg.data.from, cfg.data.to);
    Ok(cfg)
}

/// The window's tournaments, with the acceptance tests' edits applied.
fn arena(cfg: &Config, start: Instant) -> Result<Arena, Box<dyn std::error::Error>> {
    let mut arena = chgk_rating::data::load(cfg, &cfg.data.from, &cfg.data.to)?;
    if let Some(s) = arg("--swap") {
        apply_swap(&mut arena, &s)?;
    }
    if let Some(d) = arg("--scramble") {
        apply_scramble(&mut arena, &d)?;
    }
    let counts = (arena.tournaments.len(), arena.n_players(), arena.n_teams(), arena.n_editors());
    let secs = start.elapsed().as_secs_f64();
    println!("loaded {} tournaments, {} players, {} teams, {} editors in {secs:.1}s", counts.0, counts.1, counts.2, counts.3);
    Ok(arena)
}

/// What the observed pass writes: `out/ratings.db`, the feature table when
/// asked, and the diagnostics on stdout.
struct Fit<'a> {
    arena: &'a Arena,
    cfg: &'a Config,
    w: Writer,
    features: Option<Features>,
    /// The team states the feature row of this tournament reads.
    pre: Option<Pre>,
    diag: Option<usize>,
    start: Instant,
    /// Last published raw value per entity, so `change` excludes the gauge shift.
    prev_p: Vec<f64>,
    /// The last published number per player, for `delta`.
    prev_d: Vec<f64>,
    /// The published scale at the previous release.
    shift_pub: f64,
    prev_t: Vec<f64>,
    /// The lineup of this tournament and the state the forecast read this
    /// morning, before the update moves it.
    before: Vec<(u32, PlayerEvent)>,
}

impl<'a> Fit<'a> {
    fn new(arena: &'a Arena, cfg: &'a Config, features: Option<Features>, start: Instant) -> rusqlite::Result<Fit<'a>> {
        let diag = arg("--diag-player").and_then(|v| v.parse::<i64>().ok());
        Ok(Fit {
            arena,
            cfg,
            w: Writer::create(&cfg.data.out_path)?,
            features,
            pre: None,
            diag: diag.and_then(|id| arena.player_ids.iter().position(|x| *x == id)),
            start,
            prev_p: vec![f64::NAN; arena.n_players()],
            prev_d: vec![f64::NAN; arena.n_players()],
            shift_pub: 0.0,
            prev_t: vec![f64::NAN; arena.n_teams()],
            before: Vec::new(),
        })
    }
}

impl Observer for Fit<'_> {
    fn pass_start(&mut self, pass: usize, passes: usize, warm: usize) {
        if pass > 0 {
            println!("pass {} of {passes}, {warm} players warm-started", pass + 1);
        }
    }

    fn before_step(&mut self, t: &Tournament, teams: &Engine) {
        self.pre = self.features.as_mut().and_then(|f| teams.s2().map(|e| f.pre(t, e)));
    }

    fn morning(&mut self, m: &Morning) -> pass::Result<()> {
        let (t, cfg) = (m.t, self.cfg);
        // The feature row is written after the field has been scored and before
        // any memory has absorbed it, so every column is the state of that morning.
        if let (Some(f), Some(pre)) = (self.features.as_mut(), self.pre.take()) {
            f.emit(t, self.arena, m.players, &m.memories.chem, &m.memories.edmem, &pre, m.events, cfg)?;
        }
        self.before = self.lineup_states(m);
        if t.train_only {
            return Ok(());
        }
        let qs: Vec<(f64, f64, f64)> = m.params.iter().map(|p| (p.b, p.a(), p.m())).collect();
        self.w.question_params(t.id, &qs)?;
        self.w.team_events(t.id, &format_date(t.day), &self.arena.team_ids, m.events)?;
        let actual: Vec<f64> = m.events.iter().map(|e| e.actual_place).collect();
        let forecast: Vec<f64> = m.events.iter().map(|e| e.forecast_place).collect();
        let (po, pa): (Vec<f64>, Vec<f64>) =
            m.events.iter().filter_map(|e| e.pred_place.map(|p| (p, e.actual_place))).unzip();
        let s = m.scores.as_ref().ok_or("fit runs the pass with scores")?;
        let mean = |(sum, n): (f64, u64)| (n > 0).then(|| sum / n as f64);
        let date = format_date(t.day);
        self.w.tournament_metrics(&TournamentMetrics {
            tid: t.id,
            date: &date,
            kind: t.format.name(),
            weight: t.weight,
            n_teams: t.teams.len(),
            n_q: t.n_q,
            n_obs: s.in_sample.1,
            insample: mean(s.in_sample),
            crossfit: mean(s.cross),
            sp_forecast: spearman(&forecast, &actual),
            sp_old: spearman(&po, &pa),
            truedl: chgk_rating::question_model::truedl(m.field, m.params, cfg),
        })?;
        Ok(())
    }

    fn evening(&mut self, e: &Evening) -> pass::Result<()> {
        let (t, cfg) = (e.t, self.cfg);
        if let Some(f) = self.features.as_mut() {
            f.absorb_teams(t, e.events);
            f.absorb_players(t, e.grads, e.players);
        }
        if !t.train_only {
            let rows: Vec<PlayerEvent> = std::mem::take(&mut self.before)
                .into_iter()
                .map(|(i, b)| PlayerEvent {
                    theta_after: e.players.published(i as usize),
                    sigma_after: e.players.sigma_at(i as usize, t.day, cfg),
                    ..b
                })
                .collect();
            self.w.player_events(t.id, &rows)?;
        }
        if e.index % PROGRESS_EVERY == 0 {
            println!(
                "{:6}/{} {} {:.0}s",
                e.index,
                self.arena.tournaments.len(),
                format_date(t.day),
                self.start.elapsed().as_secs_f64()
            );
        }
        Ok(())
    }

    fn release(&mut self, r: &Release) -> pass::Result<()> {
        let (cfg, arena) = (self.cfg, self.arena);
        if r.id % CHECKPOINT_EVERY == 0 {
            self.w.checkpoint()?;
        }
        let listed: Vec<usize> = (0..arena.n_players()).filter(|&i| r.activity.count[i] > 0).collect();
        if let Some(i) = self.diag {
            diag_player(r, i);
        }
        if r.last {
            release_diagnostics(r, &listed, cfg);
        }
        // The release records the scale as published, which is the gauge
        // itself unless `gauge.publish_step` holds it back.
        let moved = r.players.shift_pub - std::mem::replace(&mut self.shift_pub, r.players.shift_pub);
        let shift = if cfg.gauge.publish_step > 0.0 { moved } else { r.shift };
        self.w.release(r.id, &format_date(r.day), shift, r.players.shift_pub, listed.len())?;
        if !cfg.data.write_rows(r.id, r.last) {
            return Ok(());
        }
        let since = |now: f64, prev: &mut f64| {
            let d = if prev.is_finite() { now - *prev } else { 0.0 };
            *prev = now;
            d
        };
        let mut prows: Vec<PlayerRow> = Vec::with_capacity(listed.len());
        for &i in &listed {
            let change = since(r.players.state[i].theta_raw, &mut self.prev_p[i]);
            // The published number may carry a share of the format effect; the
            // forecast reads `theta` and `phi` separately and never this.
            let theta = r.players.published(i) + cfg.display.player_phi * r.players.phi(i);
            let delta = since(chgk_rating::display::player(theta, cfg), &mut self.prev_d[i]);
            prows.push((arena.player_ids[i], theta, r.players.sigma_at(i, r.day, cfg), r.activity.count[i], change, delta));
        }
        self.w.player_ratings(r.id, &prows, |t| chgk_rating::display::player(t, cfg))?;
        let trows: Vec<(i64, f64, f64, f64, f64, f64)> = r
            .teams
            .rows(r.day, cfg.gauge.window_days, cfg, r.players.shift_cum)
            .into_iter()
            .map(|(i, raw, mu, sigma, core, evidence)| {
                let change = if self.prev_t[i].is_finite() { raw - self.prev_t[i] } else { 0.0 };
                self.prev_t[i] = raw;
                (arena.team_ids[i], mu, sigma, core, evidence, change)
            })
            .collect();
        self.w.team_ratings(r.id, &trows, |m| chgk_rating::display::team(m, cfg))?;
        Ok(())
    }
}

impl Fit<'_> {
    /// Every player of the field once, with the state the forecast read.
    fn lineup_states(&self, m: &Morning) -> Vec<(u32, PlayerEvent)> {
        let (t, players, cfg) = (m.t, m.players, self.cfg);
        let mut lineup: Vec<u32> = t.teams.iter().flat_map(|r| r.members.iter().copied()).collect();
        lineup.sort_unstable();
        lineup.dedup();
        let state = |i: u32| {
            let (i, s) = (i as usize, &players.state[i as usize]);
            let ev = PlayerEvent {
                player: self.arena.player_ids[i],
                theta_before: players.published(i),
                theta_after: f64::NAN,
                sigma_after: f64::NAN,
                phi_before: players.phi(i),
                sigma_before: players.sigma_at(i, t.day, cfg),
                games_ip: s.games_ip as i64,
                days_ip: if s.games_ip > 0 { (t.day - s.last_ip) as i64 } else { -1 },
            };
            (i as u32, ev)
        };
        lineup.into_iter().map(state).collect()
    }
}

/// `--diag-player`: one player's uncertainty decomposition at a release.
fn diag_player(r: &Release, i: usize) {
    if r.activity.count[i] == 0 {
        return;
    }
    let s = &r.players.state[i];
    let dt = ((r.day - s.last_day).max(0) as f64) / DAYS_PER_YEAR;
    println!(
        "diag {} games {} fisher {:.2} sigma_from_fisher {:.3} sigma_at_game {:.3} vol {:.3} sigma {:.3}",
        format_date(r.day),
        r.activity.count[i],
        r.activity.fisher[i],
        (1.0 / r.activity.fisher[i].max(1e-9)).sqrt(),
        s.var.sqrt(),
        s.vol,
        (s.var + s.vol * s.vol * dt).sqrt()
    );
}

/// The summary lines printed with the last release.
fn release_diagnostics(r: &Release, listed: &[usize], cfg: &Config) {
    let players = r.players;
    let mut band: Vec<(f64, f64)> = listed
        .iter()
        .map(|&i| (players.theta(i), r.activity.fisher[i] / r.activity.count[i] as f64))
        .collect();
    band.sort_by(|a, b| b.0.total_cmp(&a.0));
    let med = |v: &[(f64, f64)]| {
        let mut x: Vec<f64> = v.iter().map(|p| p.1).collect();
        x.sort_by(f64::total_cmp);
        x.get(x.len() / 2).copied().unwrap_or(0.0)
    };
    let n = band.len();
    println!(
        "fisher per game: top 200 {:.3}, ranks 2000-5000 {:.3}",
        med(&band[..200.min(n)]),
        med(&band[2000.min(n)..5000.min(n)])
    );
    if cfg.question.format_effect != FormatEffect::None {
        let mut phi: Vec<f64> = listed.iter().map(|&i| players.state[i].aff[0]).collect();
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
        println!("player zvar {:.3} (n {n:.0}, mean {:.3})", s2 / n - (s1 / n) * (s1 / n), s1 / n);
    }
    let rated = listed.iter().filter(|&&i| players.sigma_at(i, r.day, cfg) <= cfg.display.provisional_sigma).count();
    println!("players rated {rated}/{} ({:.0}%)", listed.len(), 100.0 * rated as f64 / listed.len() as f64);
}
