//! The chronological pass of DESIGN 3: per tournament the questions, then the
//! teams, then the players, and every Thursday a release. `fit`, `achieve`,
//! `team_sweep` and `calibrate` all run this one loop and differ only in what
//! they do with it, which an [`Observer`] says.

use crate::chemistry::Chemistry;
use crate::config::{AffStrength, Config, TeamEngine};
use crate::data::{Arena, Format, Tournament};
use crate::difficulty::Difficulties;
use crate::editor_memory::EditorMemory;
use crate::gauge::{self, Activity};
use crate::geography::{Geography, PlayerLoad};
use crate::question_model::{self as qm, Field, Players, QParams};
use crate::team_event::TeamEvent;
use crate::team_model::Teams;
use crate::team_s2::{self, Context, Row, TeamsS2};
use crate::util::{next_thursday, parse_date};
use std::collections::HashMap;

const WEEK: i32 = 7;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The team side of the pass. Both engines write the same `team_events` and the
/// same release rows, so everything downstream reads one schema.
pub enum Engine {
    Kalman(Teams),
    S2(TeamsS2),
}

/// One team in a release: `(index, raw rating, published rating, sigma, core
/// strength, evidence)`. The raw value is what `change` is measured on: for the
/// Kalman engine that is the pre-gauge scale, and S2 has no gauge of its own.
pub type TeamRow = (usize, f64, f64, f64, f64, f64);

impl Engine {
    fn new(n: usize, cfg: &Config) -> Engine {
        match cfg.team.engine {
            TeamEngine::Kalman => Engine::Kalman(Teams::new(n)),
            TeamEngine::S2 => Engine::S2(TeamsS2::new(n)),
        }
    }

    /// The S2 engine, for the readers that only make sense under it.
    pub fn s2(&self) -> Option<&TeamsS2> {
        match self {
            Engine::S2(e) => Some(e),
            Engine::Kalman(_) => None,
        }
    }

    /// One tournament. Under S2 the lineup strengths the rules read come back
    /// too; the Kalman engine has none.
    fn step(
        &mut self,
        t: &Tournament,
        players: &Players,
        params: &[QParams],
        ctx: &Context,
        cfg: &Config,
    ) -> (Vec<Row>, Vec<TeamEvent>) {
        match self {
            Engine::Kalman(e) => (Vec::new(), e.step(t, players, params, cfg)),
            Engine::S2(e) => {
                let rows = team_s2::rows_of(t, players, ctx, cfg);
                let events = e.step_rows(t.id, t.day, t.weight, t.format, &rows, &cfg.team.s2);
                (rows, events)
            }
        }
    }

    /// Every team with a rated game in the trailing window.
    pub fn rows(&self, day: i32, window: i32, cfg: &Config, shift: f64) -> Vec<TeamRow> {
        match self {
            Engine::Kalman(e) => e
                .listed(day, window)
                .map(|(i, s)| (i, s.mu_raw, s.mu_raw - shift, s.sigma_at(day, cfg), s.core_raw - shift, s.evidence()))
                .collect(),
            Engine::S2(e) => e.rows(day, window).into_iter().map(|(i, r, sg, q, n)| (i, r, r, sg, q, n)).collect(),
        }
    }
}

/// The forecast-side memories: read before a field is forecast, told about it
/// only afterwards, so nothing predicts itself.
pub struct Memories {
    pub chem: Chemistry,
    pub diff: Difficulties,
    pub edmem: EditorMemory,
    pub geo: Geography,
    pub load: PlayerLoad,
    /// `team.s2.ed_from`.
    ed_from: i32,
}

impl Memories {
    fn new(cfg: &Config, chemistry: bool) -> Memories {
        let s2 = &cfg.team.s2;
        Memories {
            chem: Chemistry::new(cfg.question.chem_u != 0.0 || s2.chemistry != 0.0 || chemistry),
            diff: Difficulties::new(s2.difficulty_forecast),
            edmem: EditorMemory::new(s2.editor_memory != 0.0 || s2.ed_phi != 0.0, s2.ed_min_n, s2.ed_key, s2.ed_half_life),
            geo: Geography::new(s2.home_bonus != 0.0 || s2.home_team_bonus != 0.0),
            load: PlayerLoad::new(s2.load_7 != 0.0 || s2.load_30 != 0.0),
            ed_from: parse_date(&s2.ed_from).unwrap_or(i32::MIN),
        }
    }

    pub fn context(&self) -> Context<'_> {
        Context { chem: &self.chem, diff: &self.diff, edmem: &self.edmem, geo: &self.geo, load: &self.load }
    }

    /// Everything the memories learn from one tournament once it has been
    /// forecast. A tournament kept for the player side alone teaches them nothing.
    fn absorb(&mut self, t: &Tournament, events: &[TeamEvent], params: &[QParams]) {
        if t.train_only {
            return;
        }
        let in_person = t.format == Format::InPerson;
        if self.diff.on() && in_person {
            let mut q: Vec<f64> = events.iter().map(|e| e.s_r).filter(|x| x.is_finite()).collect();
            q.sort_by(|a, b| b.total_cmp(a));
            let k = 10.min(q.len());
            if k > 0 {
                let s = q[..k].iter().sum::<f64>() / k as f64;
                self.diff.absorb(s, params.iter().map(|p| p.b));
            }
        }
        self.chem.absorb(t);
        if self.edmem.on() && in_person && t.day >= self.ed_from {
            let n = events.len().max(1) as f64;
            let res: Vec<(usize, f64)> = events
                .iter()
                .enumerate()
                .filter(|(_, e)| e.actual_place > 0.0)
                .map(|(k, e)| (k, (e.forecast_place - e.actual_place) / n))
                .collect();
            self.edmem.absorb(t, &res);
        }
        self.geo.absorb(t);
        if in_person {
            self.load.absorb(t);
        }
    }
}

/// The in-sample and cross-fitted log-likelihood of a field's questions, as
/// `(sum, observations)`; the cross-fit is skipped for a train-only tournament.
pub struct FitScores {
    pub in_sample: (f64, u64),
    pub cross: (f64, u64),
}

/// One tournament after the forecast and the team step, before anything has
/// learned from it.
pub struct Morning<'a> {
    pub index: usize,
    pub t: &'a Tournament,
    pub players: &'a Players,
    pub memories: &'a Memories,
    pub field: &'a Field,
    pub params: &'a [QParams],
    pub rows: &'a [Row],
    pub events: &'a [TeamEvent],
    pub scores: Option<FitScores>,
    /// The day of the next release, which is the week this tournament counts in.
    pub next_release: i32,
}

/// The same tournament once the players have absorbed it.
pub struct Evening<'a> {
    pub index: usize,
    pub t: &'a Tournament,
    pub players: &'a Players,
    pub grads: &'a [(u32, f64, f64)],
    pub events: &'a [TeamEvent],
}

/// A Thursday, after the gauge has been re-centred.
pub struct Release<'a> {
    pub id: i64,
    pub day: i32,
    /// The last Thursday before `data.to`.
    pub last: bool,
    pub shift: f64,
    pub players: &'a Players,
    pub activity: &'a Activity,
    pub teams: &'a Engine,
}

/// What a caller does with the pass. Only the last pass is observed; the
/// earlier ones exist to warm-start the players.
pub trait Observer {
    /// A pass is starting; `warm` players were carried over from the one before.
    fn pass_start(&mut self, _pass: usize, _passes: usize, _warm: usize) {}
    /// The team states the forecast is about to be made against.
    fn before_step(&mut self, _t: &Tournament, _teams: &Engine) {}
    fn morning(&mut self, _m: &Morning) -> Result<()> {
        Ok(())
    }
    fn evening(&mut self, _e: &Evening) -> Result<()> {
        Ok(())
    }
    fn release(&mut self, _r: &Release) -> Result<()> {
        Ok(())
    }
}

#[derive(Default, Clone, Copy)]
pub struct Options {
    /// Passes over the window; `data.passes` when `None`.
    pub passes: Option<usize>,
    /// Score every field in sample and cross-fitted, for `tournament_metrics`.
    pub scores: bool,
    /// Keep the pair counter even when the model does not read it.
    pub chemistry: bool,
}

/// The players are snapshotted for the warm restart on the first tournament on
/// or after `player.warm_until`. With `data.publish_from` on or after
/// `warm_until`, every published number still depends only on results before
/// its date: the snapshot holds nothing later than `warm_until`.
pub fn run(arena: &Arena, cfg: &Config, opts: Options, obs: &mut dyn Observer) -> Result<i64> {
    let passes = opts.passes.unwrap_or(cfg.data.passes);
    let from_day = parse_date(&cfg.data.from).ok_or("bad data.from")?;
    let warm_until = parse_date(&cfg.player.warm_until).ok_or("bad player.warm_until")?;
    let publish_from = parse_date(&cfg.data.publish_from).unwrap_or(i32::MIN);
    if passes > 1 && publish_from < warm_until {
        eprintln!("note: the warm start publishes before player.warm_until; those releases read results after their date");
    }
    let mut players = Players::new(arena.n_players(), arena.n_editors(), cfg);
    let mut snap: Vec<Option<f64>> = Vec::new();
    let mut releases = 0;
    for p in 0..passes {
        let last = p + 1 == passes;
        let mut warm = 0;
        if p > 0 {
            warm = snap.iter().filter(|x| x.is_some()).count();
            players.warm_restart(&snap, from_day, cfg);
        }
        obs.pass_start(p, passes, warm);
        let mut quiet = Quiet;
        let obs: &mut dyn Observer = if last { &mut *obs } else { &mut quiet };
        let mut sweep = Sweep::new(arena, cfg, opts, &mut players, last);
        let mut stopped = false;
        let mut reset = cfg.player.burn_reset_phi;
        let places_until = parse_date(&cfg.player.places_until).unwrap_or(i32::MAX);
        let mut handoff = cfg.player.places_handoff_var > 0.0;
        for (n, t) in arena.tournaments.iter().enumerate() {
            // An unobserved pass exists only for its snapshot, and nothing it
            // does after `warm_until` reaches the next one: it stops there, so
            // each further pass restarts from the one before.
            if !last && t.day >= warm_until {
                snap = sweep.players.snapshot();
                stopped = true;
                break;
            }
            if handoff && t.day >= places_until {
                sweep.players.handoff(cfg.player.places_handoff_var, cfg);
                handoff = false;
            }
            if reset && t.day >= publish_from {
                sweep.players.reset_pins(cfg);
                reset = false;
            }
            sweep.releases_through(t.day, obs)?;
            sweep.tournament(n, t, obs)?;
        }
        if !last && !stopped {
            snap = sweep.players.snapshot();
        }
        if last {
            sweep.releases_through(sweep.to_day, obs)?;
        }
        releases = sweep.release_id;
    }
    Ok(releases)
}

struct Quiet;
impl Observer for Quiet {}

/// Two observers of one pass, each told everything in turn.
pub struct Tee<'a>(pub &'a mut dyn Observer, pub &'a mut dyn Observer);

impl Observer for Tee<'_> {
    fn pass_start(&mut self, pass: usize, passes: usize, warm: usize) {
        self.0.pass_start(pass, passes, warm);
        self.1.pass_start(pass, passes, warm);
    }
    fn before_step(&mut self, t: &Tournament, teams: &Engine) {
        self.0.before_step(t, teams);
        self.1.before_step(t, teams);
    }
    fn morning(&mut self, m: &Morning) -> Result<()> {
        self.0.morning(m)?;
        self.1.morning(m)
    }
    fn evening(&mut self, e: &Evening) -> Result<()> {
        self.0.evening(e)?;
        self.1.evening(e)
    }
    fn release(&mut self, r: &Release) -> Result<()> {
        self.0.release(r)?;
        self.1.release(r)
    }
}

/// The state of one pass.
struct Sweep<'a> {
    arena: &'a Arena,
    cfg: &'a Config,
    opts: Options,
    players: &'a mut Players,
    teams: Engine,
    mem: Memories,
    activity: Activity,
    ed_seen: Vec<f64>,
    release_id: i64,
    next_rel: i32,
    to_day: i32,
    /// The first day whose in-person fields feed the residual statistics.
    zfrom: i32,
    /// `player.places` reads no field on or after this day.
    places_until: i32,
}

impl<'a> Sweep<'a> {
    fn new(arena: &'a Arena, cfg: &'a Config, opts: Options, players: &'a mut Players, last: bool) -> Sweep<'a> {
        let from_day = parse_date(&cfg.data.from).unwrap_or(0);
        // Only the observed pass runs the releases past the last tournament.
        let to_day = if last { parse_date(&cfg.data.to).unwrap_or(i32::MAX) } else { i32::MIN };
        Sweep {
            arena,
            cfg,
            opts,
            players,
            teams: Engine::new(arena.n_teams(), cfg),
            mem: Memories::new(cfg, opts.chemistry && last),
            activity: Activity::new(arena.n_players()),
            ed_seen: vec![0.0; arena.n_editors()],
            release_id: 0,
            next_rel: next_thursday(from_day),
            to_day,
            zfrom: parse_date("2018-01-01").unwrap_or(0),
            places_until: parse_date(&cfg.player.places_until).unwrap_or(i32::MAX),
        }
    }

    /// Every Thursday on or before `day` that has not been released yet.
    fn releases_through(&mut self, day: i32, obs: &mut dyn Observer) -> Result<()> {
        while self.next_rel <= day {
            self.release_id += 1;
            let rel = self.next_rel;
            let shift = self.recentre(rel);
            obs.release(&Release {
                id: self.release_id,
                day: rel,
                // The last release is the last Thursday before `to`, whether or
                // not a tournament follows it.
                last: rel + WEEK > self.to_day,
                shift,
                players: self.players,
                activity: &self.activity,
                teams: &self.teams,
            })?;
            self.next_rel += WEEK;
        }
        Ok(())
    }

    /// The weekly gauge of DESIGN 3.5, and the gates that stand at quantiles of
    /// the same established panel, so "a strong player" means strong against
    /// today's field.
    fn recentre(&mut self, day: i32) -> f64 {
        let cfg = self.cfg;
        self.activity.expire_before(day - cfg.gauge.window_days);
        let shift = gauge::shift(self.players, &self.activity, &cfg.gauge);
        self.players.shift_cum += shift;
        let p = &mut *self.players;
        let held = parse_date(&cfg.gauge.publish_hold_from).is_some_and(|from| day >= from)
            && parse_date(&cfg.gauge.publish_hold_until).is_some_and(|until| day < until);
        p.shift_pub = match cfg.gauge.publish_step {
            _ if held => p.shift_pub,
            step if step > 0.0 => p.shift_pub + (p.shift_cum - p.shift_pub).clamp(-step, step),
            _ => p.shift_cum,
        };
        if cfg.question.aff_strength == AffStrength::Gate || cfg.team.s2.ed_gate > 0.0 {
            let mut panel: Vec<f64> = (0..self.arena.n_players())
                .filter(|&i| self.activity.count[i] >= cfg.gauge.n_est)
                .map(|i| self.players.theta(i))
                .collect();
            self.players.set_gate(&mut panel, cfg.question.aff_gate);
            self.players.set_ed_gate(&mut panel, cfg.team.s2.ed_gate);
        }
        shift
    }

    fn tournament(&mut self, n: usize, t: &Tournament, obs: &mut dyn Observer) -> Result<()> {
        let cfg = self.cfg;
        let q = self.questions(t);
        obs.before_step(t, &self.teams);
        let (rows, events) = if t.train_only {
            (Vec::new(), Vec::new())
        } else {
            self.teams.step(t, self.players, &q.params, &self.mem.context(), cfg)
        };
        obs.morning(&Morning {
            index: n,
            t,
            players: self.players,
            memories: &self.mem,
            field: &q.field,
            params: &q.params,
            rows: &rows,
            events: &events,
            scores: q.scores,
            next_release: self.next_rel,
        })?;
        self.learn(t, &q.grads, &q.blocks, &events, &q.params);
        if cfg.player.score_k > 0.0 {
            let own = cfg.player.score_own_fit.then(|| {
                let vbar = self.players.editor_mean(t);
                let field = qm::with_strengths(&q.field, |i| self.players.theta_score(i as usize) + self.players.affinity(i as usize, &vbar), cfg);
                qm::fit_all(&field, None, cfg)
            });
            self.players.absorb_score(t, &q.field, own.as_deref().unwrap_or(&q.params), cfg);
        }
        self.learn_teams(t, &q.team_grads, &events);
        obs.evening(&Evening { index: n, t, players: self.players, grads: &q.grads, events: &events })
    }

    /// Step 1 of a tournament: its players seeded, its questions fitted, and
    /// what they say about each player.
    fn questions(&mut self, t: &Tournament) -> Questions {
        let cfg = self.cfg;
        self.players.seed(t, cfg);
        let field = qm::build_field(t, self.players, &self.mem.chem, cfg);
        // The cross-fit is only scored, never learnt from, so it runs beside
        // the fit that is.
        let scores = self.opts.scores;
        let (cross, params) = rayon::join(|| scores.then(|| cross_fit(&field, t, cfg)), || qm::fit_all(&field, None, cfg));
        let scores = cross.map(|cross| FitScores { in_sample: qm::score(&field, &params, None, cfg), cross });
        let grads = qm::player_grads(&field, &params, cfg);
        let team_grads = qm::team_grads_by_id(&field, &params, cfg);
        let blocks = if cfg.player.pair_cov.is_empty() { Vec::new() } else { qm::lineup_fisher(&field, &params, cfg) };
        Questions { field, params, scores, grads, team_grads, blocks }
    }

    /// Everything that learns from a tournament once it has been forecast: the
    /// memories, then the players, then the gauge's activity record.
    fn learn(&mut self, t: &Tournament, grads: &[(u32, f64, f64)], blocks: &[(Vec<u32>, Vec<f64>)], events: &[TeamEvent], params: &[QParams]) {
        let cfg = self.cfg;
        self.mem.absorb(t, events, params);
        let p = &cfg.player;
        let by_places =
            p.places && t.day < self.places_until && (p.places_masked || t.teams.iter().all(|r| r.mask.is_empty()));
        if by_places {
            for (i, info) in self.players.absorb_places(t, cfg) {
                self.activity.push(t.day, i, info);
            }
            if p.places_masked {
                return;
            }
        }
        let collect = t.format == Format::InPerson && t.day >= self.zfrom;
        let steered = steer(grads, events, t, cfg);
        let floor = self.floored(t, events);
        let before: HashMap<u32, f64> = if cfg.player.star_cap > 0.0 {
            grads.iter().map(|&(i, _, _)| (i, self.players.state[i as usize].theta_raw)).collect()
        } else {
            HashMap::new()
        };
        self.players.absorb(t, steered.as_deref().unwrap_or(grads), blocks, &mut self.ed_seen, collect, cfg);
        self.players.shrink_to_lineups(t, cfg);
        for (i, before) in floor {
            let s = &mut self.players.state[i as usize];
            s.theta_raw = s.theta_raw.max(before);
        }
        self.players.cap_stars(t, &before, cfg);
        self.players.pull_partners(t, cfg);
        for &(i, _, info) in grads {
            self.activity.push(t.day, i, t.weight * info);
        }
    }
}

impl Sweep<'_> {
    /// `question.team_effect_sd`: each team's strength steps on its lineup's
    /// gradient, steered by its forecast place as the players are.
    fn learn_teams(&mut self, t: &Tournament, team_grads: &[(u32, f64, f64)], events: &[TeamEvent]) {
        let cfg = self.cfg;
        if team_grads.is_empty() || t.train_only {
            return;
        }
        let dir: HashMap<u32, f64> = if cfg.question.rel_place {
            events
                .iter()
                .filter(|e| e.actual_place > 0.0 && e.forecast_place > 0.0)
                .map(|e| (e.team, e.forecast_place - e.actual_place))
                .collect()
        } else {
            HashMap::new()
        };
        let steered: Vec<(u32, f64, f64)> = team_grads
            .iter()
            .map(|&(team, g, info)| match dir.get(&team) {
                Some(&s) if s >= 0.0 => (team, g.max(0.0), info),
                Some(_) => (team, g.min(0.0), info),
                None => (team, g, info),
            })
            .collect();
        self.players.absorb_teams(t, &steered, cfg);
    }

    /// Under `player.shrink_keeps_floor`, every member of a lineup a floor
    /// protects, with the strength they came in with: under `rel_place` a
    /// lineup at or above its forecast place, under `rel_winner` the winners.
    fn floored(&self, t: &Tournament, events: &[TeamEvent]) -> Vec<(u32, f64)> {
        let (cfg, q) = (self.cfg, &self.cfg.question);
        if !cfg.player.shrink_keeps_floor || !(q.rel_place || q.rel_winner) {
            return Vec::new();
        }
        let held: Vec<&crate::data::TeamResult> = if q.rel_place {
            let met: HashMap<u32, bool> = events
                .iter()
                .filter(|e| e.actual_place > 0.0 && e.forecast_place > 0.0)
                .map(|e| (e.team, e.forecast_place >= e.actual_place))
                .collect();
            t.teams.iter().filter(|r| met.get(&r.team) == Some(&true)).collect()
        } else {
            let best = t.teams.iter().map(|r| r.position).filter(|&p| p > 0.0).fold(f64::INFINITY, f64::min);
            t.teams.iter().filter(|r| r.position == best).collect()
        };
        held.iter().flat_map(|r| &r.members).map(|&m| (m, self.players.state[m as usize].theta_raw)).collect()
    }
}

/// One tournament's questions as the model fitted them.
struct Questions {
    field: Field,
    params: Vec<QParams>,
    scores: Option<FitScores>,
    grads: Vec<(u32, f64, f64)>,
    /// `question.team_effect_sd`: `(team, gradient, information)` per lineup.
    team_grads: Vec<(u32, f64, f64)>,
    /// `player.pair_cov`: each lineup's members and information matrix.
    blocks: Vec<(Vec<u32>, Vec<f64>)>,
}

/// The question log-likelihood of each of the two halves of the field,
/// fitted on the other.
fn cross_fit(field: &Field, t: &Tournament, cfg: &Config) -> (f64, u64) {
    if t.train_only {
        return (0.0, 0);
    }
    let half = |h: u8| {
        let p = qm::fit_all(field, Some(h), cfg);
        qm::score(field, &p, Some(1 - h), cfg)
    };
    let ((s0, k0), (s1, k1)) = rayon::join(|| half(0), || half(1));
    (0.0 + s0 + s1, k0 + k1)
}

/// The player gradients as the shipped rules take them. `question.rel_place`:
/// the place sets the direction of every member's step, the questions its
/// size, so a lineup at or above its forecast place cannot lose from this
/// tournament and one below it cannot gain. `field_players`: the field-strength
/// factor reaches the players as a larger weight. `None` when neither applies.
fn steer(grads: &[(u32, f64, f64)], events: &[TeamEvent], t: &Tournament, cfg: &Config) -> Option<Vec<(u32, f64, f64)>> {
    let s2 = &cfg.team.s2;
    let pf = if s2.field_players { team_s2::field_factor(events.iter().map(|e| e.s_r), t.format, s2) } else { 1.0 };
    let dir: HashMap<u32, f64> = if cfg.question.rel_place {
        let by_team: HashMap<u32, f64> = events
            .iter()
            .filter(|e| e.actual_place > 0.0 && e.forecast_place > 0.0)
            .map(|e| (e.team, e.forecast_place - e.actual_place))
            .collect();
        t.teams
            .iter()
            .filter_map(|r| by_team.get(&r.team).map(|&s| (r, s)))
            .flat_map(|(r, s)| r.members.iter().map(move |&m| (m, s)))
            .collect()
    } else {
        HashMap::new()
    };
    if pf == 1.0 && dir.is_empty() {
        return None;
    }
    let steer_one = |&(i, g, info): &(u32, f64, f64)| {
        let g = match dir.get(&i) {
            Some(&s) if s >= 0.0 => g.max(0.0),
            Some(_) => g.min(0.0),
            None => g,
        };
        (i, g * pf, info * pf)
    };
    Some(grads.iter().map(steer_one).collect())
}
