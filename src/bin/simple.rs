//! Model S: the whole rating on the display scale, four constants, no algebra
//! beyond a weighted average. DESIGN.md describes the shipped model; this is the
//! spreadsheet-reproducible cousin, written to the same schema so `eval` reads it.
use chgk_rating::config::{Anchor, Config, Perf, PlayerRule, S2Lineup, Simple, S2};
use chgk_rating::data::{Arena, Format, Tournament};
use chgk_rating::gauge::Activity;
use chgk_rating::ndcg::{TEST, TUNE};
use chgk_rating::output::{PlayerEvent, TournamentMetrics, Writer};
use chgk_rating::rivals::{bare_player_event, blank_event, open_read_only, Args, Changes, PlayerRow, TeamRow};
use chgk_rating::team_event::TeamEvent;
use chgk_rating::team_s2::lineup_strength;
use chgk_rating::util::{avg_ranks, format_date, last_thursday, next_thursday, parse_date, spearman};
use std::collections::HashMap;

/// Where the anchor of rule 13 puts the established teams.
const ANCHOR_MEAN: f64 = 5000.0;
const ANCHOR_SD: f64 = 1000.0;
/// The anchor reads teams with this many games in the window, and only when
/// there are enough of them; a flatter spread than this is left unscaled.
const ANCHOR_MIN_GAMES: u32 = 10;
const ANCHOR_MIN_TEAMS: usize = 30;
const ANCHOR_MIN_SD: f64 = 1e-6;
/// The year whose mean R - Q(L) the report prints.
const RESIDUAL_YEAR: (&str, &str) = ("2019-01-01", "2020-01-01");
/// The nominal sigma of a team or player: settled from this many games on.
const SETTLED_GAMES: f64 = 5.0;
const SIGMA_SETTLED: f64 = 0.1;
const SIGMA_NEW: f64 = 0.5;
/// The activity window, in days.
const WINDOW_DAYS: i32 = 365;
const WEEK: i32 = 7;
const CHECKPOINT_EVERY: i64 = 50;
/// A full lineup; rule 1 charges `delta` per empty seat.
const FULL_LINEUP: f64 = 6.0;
/// Fields smaller than this carry no NDCG.
const NDCG_MIN_FIELD: usize = 5;
const NDCG_TOP: usize = 3;
const LOSS_FLOOR: f64 = 1e-12;
/// Deciles of the expected win.
const N_BINS: usize = 10;
const BIN_WIDTH: f64 = 0.1;
const HALF: f64 = 0.5;
const GRID_PASSES: u32 = 2;
const K_AXIS: &[f64] = &[0.05, 0.10, 0.20, 0.40, 0.80];
const K_P_AXIS: &[f64] = &[0.05, 0.10, 0.20, 0.40, 0.80];
const K_Q_AXIS: &[f64] = &[0.01, 0.05, 0.10, 0.30];
const DELTA_AXIS: &[f64] = &[150.0, 300.0, 600.0, 1200.0];
const R_NEW_AXIS: &[f64] = &[3000.0, 4000.0, 5000.0];
const D_AXIS: &[f64] = &[300.0, 600.0, 1200.0, 2400.0, 4800.0];
const K_R_AXIS: &[f64] = &[0.0, 0.1, 0.2, 0.3, 0.5];
const AXES: [(&str, &[f64]); 7] = [
    ("K", K_AXIS),
    ("k_p", K_P_AXIS),
    ("k_q", K_Q_AXIS),
    ("delta", DELTA_AXIS),
    ("r_new", R_NEW_AXIS),
    ("D", D_AXIS),
    ("k_r", K_R_AXIS),
];
const D_SCAN: &[f64] = &[100.0, 150.0, 225.0, 300.0, 450.0, 600.0, 900.0, 1200.0, 1800.0, 2400.0];
const K_GRID: &[f64] = &[0.2, 0.3, 0.4, 0.6];
const KQ_GRID: &[f64] = &[0.01, 0.03, 0.1];

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;
type Ext = HashMap<(i64, u32), f64>;

/// The DCG of `rel` taken in the order given, to depth `k`.
fn dcg(rel: &[f64], k: usize) -> f64 {
    rel[..k].iter().enumerate().map(|(i, r)| r / ((i + 2) as f64).log2()).sum()
}

/// NDCG of the forecast order against actual places, relevance 1/place.
fn ndcg(fc: &[f64], place: &[f64], k: usize) -> Option<f64> {
    if fc.len() < NDCG_MIN_FIELD || fc.iter().all(|v| *v == fc[0]) {
        return None;
    }
    let mut o: Vec<usize> = (0..fc.len()).collect();
    o.sort_by(|&a, &b| fc[b].total_cmp(&fc[a]));
    let mut rel: Vec<f64> = o.iter().map(|&i| 1.0 / place[i]).collect();
    let k = k.min(rel.len());
    let got = dcg(&rel, k);
    rel.sort_by(|a, b| b.total_cmp(a));
    let idcg = dcg(&rel, k);
    (idcg > 0.0).then(|| got / idcg)
}

#[derive(Default, Clone, Copy)]
struct Mean(f64, f64);

impl Mean {
    fn push(&mut self, v: f64) {
        self.0 += v;
        self.1 += 1.0;
    }
    fn push_some(&mut self, v: Option<f64>) {
        if let Some(v) = v {
            self.push(v);
        }
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
    bin_n: [f64; N_BINS],
    bin_w: [f64; N_BINS],
    new_level: f64,
}

impl Score {
    /// One decided pair: `y` is 1 when the first team finished ahead.
    fn pair(&mut self, pij: f64, y: f64) {
        self.ll.push(-(y * pij + (1.0 - y) * (1.0 - pij)).max(LOSS_FLOOR).ln());
        for (e, o) in [(pij, y), (1.0 - pij, 1.0 - y)] {
            let b = ((e * N_BINS as f64) as usize).min(N_BINS - 1);
            self.bin_n[b] += 1.0;
            self.bin_w[b] += o;
        }
    }
}

fn sigma(games: f64) -> f64 {
    if games >= SETTLED_GAMES {
        SIGMA_SETTLED
    } else {
        SIGMA_NEW
    }
}

/// The scoring windows as half-open day ranges.
struct Windows {
    tune: (i32, i32),
    test: (i32, i32),
    y19: (i32, i32),
}

impl Windows {
    fn new() -> Windows {
        let days = |w: (&str, &str)| (parse_date(w.0).unwrap(), parse_date(w.1).unwrap());
        Windows { tune: days(TUNE), test: days(TEST), y19: days(RESIDUAL_YEAR) }
    }
}

fn within(day: i32, w: (i32, i32)) -> bool {
    day >= w.0 && day < w.1
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
    prev_t: Changes,
    prev_p: Changes,
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
            prev_t: Changes::new(nt),
            prev_p: Changes::new(np),
            shift_cum: 0.0,
            new_level: p.r_new,
        }
    }

    /// The established teams' mean and sd, when there are enough of them.
    fn established(&self) -> Option<(f64, f64)> {
        let v: Vec<f64> = (0..self.r.len())
            .filter(|&i| self.tact.count[i] >= ANCHOR_MIN_GAMES && self.r[i].is_finite())
            .map(|i| self.r[i])
            .collect();
        if v.len() < ANCHOR_MIN_TEAMS {
            return None;
        }
        let m = v.iter().sum::<f64>() / v.len() as f64;
        let sd = (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt();
        Some((m, sd))
    }

    /// Rule 13: put the established teams back on the published scale. Every
    /// rating moves by the same affine map, so nothing about the field changes.
    fn anchor(&mut self, mode: Anchor) -> f64 {
        let Some((m, sd)) = (mode != Anchor::None).then(|| self.established()).flatten() else {
            return 0.0;
        };
        let k = if mode == Anchor::MeanSd && sd > ANCHOR_MIN_SD { ANCHOR_SD / sd } else { 1.0 };
        for x in (self.r.iter_mut())
            .chain(&mut self.qbar)
            .chain(&mut self.pr)
            .chain(&mut self.prev_t.0)
            .chain(&mut self.prev_p.0)
            .chain(std::iter::once(&mut self.new_level))
        {
            *x = ANCHOR_MEAN + (*x - m) * k;
        }
        m - ANCHOR_MEAN
    }

    fn player_rows(&mut self, arena: &Arena) -> Vec<PlayerRow> {
        let listed: Vec<usize> = (0..arena.n_players()).filter(|&i| self.pact.count[i] > 0).collect();
        let row = |s: &mut State, i: usize| {
            let change = s.prev_p.step(i, s.pr[i]);
            (arena.player_ids[i], s.pr[i], sigma(s.pgames[i]), s.pact.count[i], change)
        };
        listed.into_iter().map(|i| row(self, i)).collect()
    }

    fn team_rows(&mut self, arena: &Arena) -> Vec<TeamRow> {
        let listed: Vec<usize> =
            (0..arena.n_teams()).filter(|&i| self.tact.count[i] > 0 && self.r[i].is_finite()).collect();
        let row = |s: &mut State, i: usize| {
            let change = s.prev_t.step(i, s.r[i]);
            (arena.team_ids[i], s.r[i], sigma(s.tgames[i]), s.qbar[i], s.tgames[i], change)
        };
        listed.into_iter().map(|i| row(self, i)).collect()
    }
}

/// One tournament's field: the results that take part, their lineup strengths,
/// forecasts and places (infinite for a team without one).
#[derive(Default)]
struct Field {
    idx: Vec<usize>,
    q: Vec<f64>,
    f: Vec<f64>,
    place: Vec<f64>,
}

/// One chronological sweep. Without a writer nothing is stored, which is what
/// the tuning grid runs.
struct Sweep<'a> {
    arena: &'a Arena,
    cfg: &'a Config,
    p: &'a Simple,
    ext: Option<&'a Ext>,
    w: Option<&'a Writer>,
    st: State,
    s2: S2,
    sc: Score,
    win: Windows,
    anchor: Anchor,
    next_rel: i32,
    release_id: i64,
    final_rel: i32,
}

impl<'a> Sweep<'a> {
    fn new(arena: &'a Arena, cfg: &'a Config, p: &'a Simple, ext: Option<&'a Ext>, w: Option<&'a Writer>) -> Fallible<Sweep<'a>> {
        let mut s2 = cfg.team.s2.clone();
        (s2.delta, s2.lineup) = (p.delta, p.lineup);
        let from_day = parse_date(&cfg.data.from).ok_or("bad from")?;
        let to_day = parse_date(&cfg.data.to).ok_or("bad to")?;
        // The hybrid's players are anchored by the question model's own gauge, and
        // rescaling the team side would leave the two scales inconsistent.
        let anchor = if ext.is_some() { Anchor::None } else { p.anchor };
        let (st, sc, win) = (State::new(arena, p), Score::default(), Windows::new());
        let (next_rel, final_rel) = (next_thursday(from_day), last_thursday(to_day));
        Ok(Sweep { arena, cfg, p, ext, w, st, s2, sc, win, anchor, next_rel, release_id: 0, final_rel })
    }

    fn run(mut self) -> Fallible<Score> {
        for t in &self.arena.tournaments {
            while self.next_rel <= t.day {
                self.release()?;
                if let Some(w) = self.w.filter(|_| self.release_id % CHECKPOINT_EVERY == 0) {
                    w.checkpoint()?;
                }
            }
            self.tournament(t)?;
        }
        self.sc.new_level = self.st.new_level;
        if self.w.is_some() {
            let to_day = parse_date(&self.cfg.data.to).unwrap();
            while self.next_rel <= to_day {
                self.release()?;
            }
        }
        Ok(self.sc)
    }

    fn release(&mut self) -> rusqlite::Result<()> {
        self.release_id += 1;
        let (id, day) = (self.release_id, self.next_rel);
        self.next_rel += WEEK;
        let st = &mut self.st;
        st.pact.expire_before(day - WINDOW_DAYS);
        st.tact.expire_before(day - WINDOW_DAYS);
        let shift = st.anchor(self.anchor);
        st.shift_cum += shift;
        let Some(w) = self.w else { return Ok(()) };
        let listed = (0..self.arena.n_players()).filter(|&i| st.pact.count[i] > 0).count();
        w.release(id, &format_date(day), shift, st.shift_cum, listed)?;
        if !self.cfg.data.write_rows(id, day == self.final_rel) {
            return Ok(());
        }
        let (prows, trows) = (st.player_rows(self.arena), st.team_rows(self.arena));
        w.player_ratings(id, &prows, |t| chgk_rating::display::player(t, self.cfg))?;
        w.team_ratings(id, &trows, |m| chgk_rating::display::team(m, self.cfg))
    }

    /// A lineup's strength from its players, ours or the question model's.
    fn lineup(&self, tid: i64, members: &[u32]) -> f64 {
        let rating = |m: u32| match self.ext {
            Some(e) => *e.get(&(tid, m)).unwrap_or(&self.p.r_new),
            None => self.st.pr[m as usize],
        };
        lineup_strength(&members.iter().map(|&m| rating(m)).collect::<Vec<f64>>(), &self.s2)
    }

    /// The field, opening a rated team the first time its lineup is known.
    fn field(&mut self, t: &Tournament) -> Field {
        let mut fd = Field::default();
        for (ri, res) in t.teams.iter().enumerate() {
            let ti = res.team as usize;
            let ql = (!res.members.is_empty()).then(|| self.lineup(t.id, &res.members));
            let known = self.st.r[ti].is_finite();
            let Some(qv) = ql.or(known.then_some(f64::NAN)) else { continue };
            if !known && res.rated {
                (self.st.r[ti], self.st.qbar[ti]) = (qv, qv);
            }
            fd.f.push(if !known { qv } else { self.st.r[ti] + if qv.is_nan() { 0.0 } else { qv - self.st.qbar[ti] } });
            fd.idx.push(ri);
            fd.q.push(qv);
            fd.place.push(if res.position > 0.0 { res.position } else { f64::INFINITY });
        }
        fd
    }

    fn win(&self, a: f64, b: f64) -> f64 {
        let c = std::f64::consts::LN_10 / self.p.d;
        1.0 / (1.0 + (c * (b - a)).exp())
    }

    /// Each team's expected wins over the field; the tuning window's in-person
    /// pairs also score the log-loss.
    fn expected_wins(&mut self, t: &Tournament, fd: &Field) -> Vec<f64> {
        let n = fd.f.len();
        let mut psum = vec![0.0; n];
        if self.p.perf != Perf::Pairwise && self.w.is_none() {
            return psum;
        }
        let score_ll = t.format == Format::InPerson && within(t.day, self.win.tune);
        for i in 0..n {
            for j in i + 1..n {
                let pij = self.win(fd.f[i], fd.f[j]);
                psum[i] += pij;
                psum[j] += 1.0 - pij;
                if score_ll && fd.place[i] != fd.place[j] {
                    self.sc.pair(pij, (fd.place[i] < fd.place[j]) as u8 as f64);
                }
            }
        }
        psum
    }

    /// Pairwise: actual wins less expected, per opponent, ties counting the
    /// expected share of the tie.
    fn pairwise_surprise(&self, fd: &Field, psum: &[f64]) -> Vec<f64> {
        let (n, place) = (fd.f.len(), &fd.place);
        let mut ord: Vec<usize> = (0..n).collect();
        ord.sort_by(|&a, &b| place[a].total_cmp(&place[b]));
        let (mut spair, mut start) = (vec![0.0; n], 0);
        for tie in ord.chunk_by(|&a, &b| place[a] == place[b]) {
            start += tie.len();
            let above = (n - start) as f64;
            for &a in tie {
                let tied: f64 = tie.iter().filter(|&&b| b != a).map(|&b| self.win(fd.f[a], fd.f[b])).sum();
                spair[a] = (above - psum[a] + tied) / ((n - 1) as f64).max(1.0);
            }
        }
        spair
    }

    /// Played like: the forecast at the place taken, interpolated, less the team's own.
    fn played_like(fd: &Field) -> Vec<f64> {
        let n = fd.f.len();
        let rank = avg_ranks(&fd.place);
        let mut fs = fd.f.clone();
        fs.sort_by(|a, b| b.total_cmp(a));
        let at = |k: usize, pl: f64| {
            let x = (pl - 1.0).clamp(0.0, (n - 1) as f64);
            let (lo, hi) = (x.floor() as usize, x.ceil() as usize);
            fs[lo] + (x - lo as f64) * (fs[hi] - fs[lo]) - fd.f[k]
        };
        rank.iter().enumerate().map(|(k, &pl)| at(k, pl)).collect()
    }

    fn tournament(&mut self, t: &Tournament) -> Fallible<()> {
        let fd = self.field(t);
        let n = fd.idx.len();
        if n == 0 {
            return Ok(());
        }
        let fplace = avg_ranks(&fd.f.iter().map(|v| -v).collect::<Vec<_>>());
        let psum = self.expected_wins(t, &fd);
        let pairwise = self.p.perf == Perf::Pairwise;
        let spair = if pairwise { self.pairwise_surprise(&fd, &psum) } else { Self::played_like(&fd) };
        let steps: Vec<f64> = if pairwise { spair.iter().map(|s| self.p.d * s).collect() } else { spair.clone() };
        let ctx = Step { t, fd: &fd, fplace: &fplace, psum: &psum, spair: &spair, steps: &steps, n };
        let (evs, prows) = self.update(&ctx);
        let sp = self.score(t, &fd, &fplace);
        if let Some(w) = self.w {
            write_tournament(w, t, &self.arena.team_ids, &evs, &prows, sp)?;
        }
        Ok(())
    }

    fn update(&mut self, c: &Step) -> (Vec<TeamEvent>, Vec<PlayerEvent>) {
        // The field's first place, for the floor of rule 9.
        let best = c.fd.place.iter().cloned().fold(f64::INFINITY, f64::min);
        let mut evs = Vec::with_capacity(if self.w.is_some() { c.n } else { 0 });
        let mut prows: Vec<PlayerEvent> = Vec::new();
        for k in 0..c.n {
            let res = &c.t.teams[c.fd.idx[k]];
            let rated = res.rated && res.position > 0.0;
            let ev = self.event_before(c, k, rated);
            self.update_players(c, k, rated, &mut prows);
            if rated {
                self.update_team(c, k, best);
            }
            if let Some(mut e) = ev {
                let ti = res.team as usize;
                (e.mu_after, e.sigma_after) = (self.st.r[ti], sigma(self.st.tgames[ti]));
                evs.push(e);
            }
        }
        (evs, prows)
    }

    /// The team's event as it stood before the update, when there is a writer;
    /// also scores the 2019 residual.
    fn event_before(&mut self, c: &Step, k: usize, rated: bool) -> Option<TeamEvent> {
        let res = &c.t.teams[c.fd.idx[k]];
        let ti = res.team as usize;
        let (mu_before, q) = (self.st.r[ti], c.fd.q[k]);
        if mu_before.is_finite() && within(c.t.day, self.win.y19) && q.is_finite() {
            self.sc.resid19.push(mu_before - q);
        }
        self.w?;
        let tlast = self.st.tlast[ti];
        Some(TeamEvent {
            c: rated as u8 as f64,
            delta_l: if mu_before.is_finite() && q.is_finite() { q - self.st.qbar[ti] } else { 0.0 },
            x: c.fd.f[k],
            forecast: c.fd.f[k],
            forecast_place: c.fplace[k],
            expected_place: (c.n as f64) - c.psum[k],
            actual_place: res.position,
            pred_place: res.pred_pos,
            s_r: q,
            s_pair: c.spair[k],
            i_pair: 1.0,
            mu_before,
            mu_after: f64::NAN,
            sigma_before: sigma(self.st.tgames[ti]),
            evidence: self.st.tgames[ti],
            gap_days: if tlast > i32::MIN / 2 { c.t.day - tlast } else { 0 },
            rated,
            ..blank_event(res.team)
        })
    }

    fn update_players(&mut self, c: &Step, k: usize, rated: bool, prows: &mut Vec<PlayerEvent>) {
        let (t, p) = (c.t, self.p);
        let res = &t.teams[c.fd.idx[k]];
        let (fk, step) = (c.fd.f[k], c.steps[k]);
        let short = p.delta * (FULL_LINEUP - res.members.len() as f64).max(0.0);
        for &m in &res.members {
            let m = m as usize;
            let before = self.st.pr[m];
            let pr = &mut self.st.pr[m];
            match (self.ext, p.player_rule) {
                (Some(e), _) => *pr = e.get(&(t.id, m as u32)).copied().unwrap_or(*pr),
                (None, PlayerRule::Surprise) => *pr += p.k_p * t.weight * step,
                // Inverting rule 1: a short lineup that played like `perf` must be
                // that much stronger per head, so the shortfall is added back.
                (None, PlayerRule::Level) => *pr += p.k_p * t.weight * (fk + step + short - *pr),
            }
            if self.w.is_some() {
                prows.push(bare_player_event(self.arena.player_ids[m], before, self.st.pr[m]));
            }
            if rated {
                self.st.pgames[m] += 1.0;
                self.st.pact.push(t.day, m as u32, 0.0);
            }
        }
    }

    fn update_team(&mut self, c: &Step, k: usize, best: f64) {
        let (t, p, q) = (c.t, self.p, c.fd.q[k]);
        let team = t.teams[c.fd.idx[k]].team;
        let (st, ti) = (&mut self.st, team as usize);
        let r_before = st.r[ti];
        st.r[ti] += p.k * t.weight * c.steps[k];
        // Rule 8: the rating drifts towards what the players are worth.
        if p.k_r > 0.0 && q.is_finite() {
            st.r[ti] += p.k_r * t.weight * (q - st.r[ti]);
        }
        // Rule 9: a team that finished first keeps at least what it came with.
        if c.fd.place[k] == best && st.r[ti] < r_before {
            st.r[ti] = r_before;
        }
        if q.is_finite() {
            st.qbar[ti] += p.k_q * (q - st.qbar[ti]);
        }
        st.tgames[ti] += 1.0;
        st.tlast[ti] = t.day;
        st.tact.push(t.day, team, 0.0);
    }

    /// The tournament's forecast Spearman, added to the tuning and test means.
    fn score(&mut self, t: &Tournament, fd: &Field, fplace: &[f64]) -> Option<f64> {
        let placed: Vec<usize> = (0..fd.f.len()).filter(|&k| fd.place[k].is_finite()).collect();
        let pick = |v: &[f64]| placed.iter().map(|&k| v[k]).collect::<Vec<f64>>();
        let (pv, sp) = (pick(&fd.place), spearman(&pick(fplace), &pick(&fd.place)));
        let (tune, test) = (within(t.day, self.win.tune), within(t.day, self.win.test));
        let sc = &mut self.sc;
        if t.format == Format::InPerson {
            let fv = pick(&fd.f);
            if tune {
                sc.tune.push_some(sp);
                sc.tune_ndcg.push_some(ndcg(&fv, &pv, usize::MAX));
                sc.tune_ndcg3.push_some(ndcg(&fv, &pv, NDCG_TOP));
            }
            if test {
                sc.test.push_some(sp);
            }
        } else if t.format == Format::Sync && test {
            sc.test_sync.push_some(sp);
        }
        sp
    }
}

/// What the update of one tournament reads.
struct Step<'a> {
    t: &'a Tournament,
    fd: &'a Field,
    fplace: &'a [f64],
    psum: &'a [f64],
    spair: &'a [f64],
    steps: &'a [f64],
    n: usize,
}

fn write_tournament(w: &Writer, t: &Tournament, team_ids: &[i64], evs: &[TeamEvent], prows: &[PlayerEvent], sp: Option<f64>) -> rusqlite::Result<()> {
    let date = format_date(t.day);
    w.team_events(t.id, &date, team_ids, evs)?;
    w.player_events(t.id, prows)?;
    let (po, pa): (Vec<f64>, Vec<f64>) = (evs.iter())
        .filter(|e| e.actual_place > 0.0)
        .filter_map(|e| e.pred_place.map(|p| (p, e.actual_place)))
        .unzip();
    let field = TournamentMetrics::field(t.id, &date, t.format.name(), t.weight, t.teams.len(), t.n_q);
    w.tournament_metrics(&TournamentMetrics { sp_forecast: sp, sp_old: spearman(&po, &pa), ..field })
}

fn run(arena: &Arena, cfg: &Config, p: &Simple, ext: Option<&Ext>, w: Option<&Writer>) -> Fallible<Score> {
    Sweep::new(arena, cfg, p, ext, w)?.run()
}

/// Player strengths from the shipped question model: theta as it stood before
/// each tournament, mapped to the team scale.
fn load_ext(path: &str, arena: &Arena, cfg: &Config) -> rusqlite::Result<Ext> {
    let conn = open_read_only(path)?;
    let pidx: HashMap<i64, u32> = arena.player_ids.iter().enumerate().map(|(i, &id)| (id, i as u32)).collect();
    let mut st = conn.prepare("SELECT tournament_id, player_id, theta_before FROM player_events")?;
    let mut rows = st.query([])?;
    let mut out = Ext::default();
    while let Some(r) = rows.next()? {
        if let Some(&i) = pidx.get(&r.get::<_, i64>("player_id")?) {
            let th: f64 = r.get("theta_before")?;
            out.insert((r.get("tournament_id")?, i), chgk_rating::team_s2::player_rating(th, cfg));
        }
    }
    Ok(out)
}

fn axis(p: &mut Simple, i: usize) -> &mut f64 {
    [&mut p.k, &mut p.k_p, &mut p.k_q, &mut p.delta, &mut p.r_new, &mut p.d, &mut p.k_r].into_iter().nth(i).unwrap()
}

/// D by maximum likelihood of the pairwise outcomes; Spearman cannot see it.
fn calibrate_d(arena: &Arena, cfg: &Config, base: &Simple) -> Fallible<()> {
    let mut p = base.clone();
    println!("| D | pairs | log-loss | tune spearman | NDCG@3 |");
    println!("|---:|---:|---:|---:|---:|");
    for &d in D_SCAN {
        p.d = d;
        let s = run(arena, cfg, &p, None, None)?;
        println!("| {d} | {:.0} | {:.4} | {:.4} | {:.4} |", s.ll.1, s.ll.get(), s.tune.get(), s.tune_ndcg3.get());
    }
    Ok(())
}

fn print_bins(sc: &Score) {
    println!("\n| expected win | pairs | mean expected | actual |");
    println!("|---|---:|---:|---:|");
    for b in (0..N_BINS).filter(|&b| sc.bin_n[b] > 0.0) {
        let (lo, hi) = (b as f64 * BIN_WIDTH, (b + 1) as f64 * BIN_WIDTH);
        let mid = (b as f64 + HALF) * BIN_WIDTH;
        println!("| {lo:.1}-{hi:.1} | {:.0} | {mid:.3} | {:.3} |", sc.bin_n[b], sc.bin_w[b] / sc.bin_n[b]);
    }
}

/// The K x K_q sweep, run per lineup rule; the rest of the constants are the
/// likelihood-fixed pair D and delta.
fn grid_kq(arena: &Arena, cfg: &Config, base: &Simple, ext: Option<&Ext>) -> Fallible<()> {
    let mut p = base.clone();
    println!("| lineup | K | k_q | tune spearman | NDCG | NDCG@3 |");
    println!("|---|---:|---:|---:|---:|---:|");
    for lineup in [S2Lineup::Mean, S2Lineup::Roster] {
        p.lineup = lineup;
        for (&k, &kq) in K_GRID.iter().flat_map(|k| KQ_GRID.iter().map(move |kq| (k, kq))) {
            (p.k, p.k_q) = (k, kq);
            let s = run(arena, cfg, &p, ext, None)?;
            let (a, b, c) = (s.tune.get(), s.tune_ndcg.get(), s.tune_ndcg3.get());
            println!("| {lineup:?} | {k} | {kq} | {a:.4} | {b:.4} | {c:.4} |");
        }
    }
    Ok(())
}

/// One run of the coordinate search: (point, tune Spearman, NDCG, NDCG@3).
type GridRun = (String, f64, f64, f64);

/// Coordinate ascent on the tuning Spearman, two passes over the axes.
fn grid(arena: &Arena, cfg: &Config, base: &Simple) -> Fallible<()> {
    let mut p = base.clone();
    let mut runs: Vec<GridRun> = Vec::new();
    println!("| pass | axis | value | tune spearman | NDCG | NDCG@3 |");
    println!("|---:|---|---:|---:|---:|---:|");
    for pass in 1..=GRID_PASSES {
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
    print_grid(&runs, &p);
    Ok(())
}

fn print_grid(runs: &[GridRun], p: &Simple) {
    let lo = runs.iter().map(|r| r.1).fold(f64::INFINITY, f64::min);
    let hi = runs.iter().map(|r| r.1).fold(f64::NEG_INFINITY, f64::max);
    let b3 = runs.iter().max_by(|a, b| a.3.total_cmp(&b.3)).cloned().unwrap_or_default();
    println!("\n{} runs, tune spearman {lo:.4} to {hi:.4}", runs.len());
    println!("best by NDCG@3: {} ({:.4}, spearman {:.4})", b3.0, b3.3, b3.1);
    println!(
        "chosen: D {} K {} k_p {} delta {} k_q {} r_new {} k_r {}",
        p.d, p.k, p.k_p, p.delta, p.k_q, p.r_new, p.k_r
    );
}

/// `flag` read as one of the named `options`, if given.
fn choice<T: Copy>(args: &Args, flag: &str, options: &[(&str, T)]) -> Fallible<Option<T>> {
    let Some(v) = args.get(flag) else { return Ok(None) };
    match options.iter().find(|o| o.0 == v) {
        Some(o) => Ok(Some(o.1)),
        None => {
            let names: Vec<&str> = options.iter().map(|o| o.0).collect();
            Err(format!("{flag}: one of {}", names.join(", ")).into())
        }
    }
}

/// `[simple]` with the command line's overrides.
fn params(args: &Args, cfg: &Config) -> Fallible<Simple> {
    let mut p = cfg.simple.clone().ok_or("config has no [simple] section")?;
    let perf = [("played_like", Perf::PlayedLike), ("pairwise", Perf::Pairwise)];
    let anchor = [("none", Anchor::None), ("mean", Anchor::Mean), ("mean_sd", Anchor::MeanSd)];
    let lineup =
        [("mean", S2Lineup::Mean), ("roster", S2Lineup::Roster), ("tempered", S2Lineup::Tempered), ("best6", S2Lineup::Best6)];
    let rule = [("surprise", PlayerRule::Surprise), ("level", PlayerRule::Level)];
    p.perf = choice(args, "--perf", &perf)?.unwrap_or(p.perf);
    p.anchor = choice(args, "--anchor", &anchor)?.unwrap_or(p.anchor);
    p.lineup = choice(args, "--lineup", &lineup)?.unwrap_or(p.lineup);
    if let Some(v) = args.get("--k-r") {
        p.k_r = v.parse().map_err(|_| "--k-r: a number")?;
    }
    p.player_rule = choice(args, "--player-rule", &rule)?.unwrap_or(p.player_rule);
    Ok(p)
}

fn config(args: &Args) -> Fallible<Config> {
    let mut cfg = Config::load(&args.or("--config", "config_simple.toml"))?;
    if let Some(v) = args.get("--out") {
        cfg.data.out_path = v;
    }
    if let Some(v) = args.get("--rows-every") {
        cfg.data.rows_every = v.parse().map_err(|_| "bad --rows-every")?;
    }
    Ok(cfg)
}

fn report(sc: &Score, cfg: &Config, start: std::time::Instant) {
    let (tune, ndcg, ndcg3) = (sc.tune.get(), sc.tune_ndcg.get(), sc.tune_ndcg3.get());
    let (test, sync) = (sc.test.get(), sc.test_sync.get());
    println!("tune {tune:.4} (NDCG {ndcg:.4}, @3 {ndcg3:.4})  test in-person {test:.4}  test sync {sync:.4}");
    println!("newcomer level at the last release: {:.0}", sc.new_level);
    println!("pairwise log-loss {:.4} over {:.0} pairs", sc.ll.get(), sc.ll.1);
    print_bins(sc);
    let (resid, events, secs) = (sc.resid19.get(), sc.resid19.1, start.elapsed().as_secs_f64());
    println!("2019 mean R - Q(L): {resid:.1} over {events:.0} events; {} in {secs:.1}s", cfg.data.out_path);
}

fn main() -> Fallible<()> {
    let args = Args::from_env();
    let cfg = config(&args)?;
    let p = params(&args, &cfg)?;
    let start = std::time::Instant::now();
    let arena = chgk_rating::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
    let (nt, np, nteams) = (arena.tournaments.len(), arena.n_players(), arena.n_teams());
    println!("loaded {nt} tournaments, {np} players, {nteams} teams in {:.1}s", start.elapsed().as_secs_f64());
    if args.has("--grid") {
        return grid(&arena, &cfg, &p);
    }
    if args.has("--calibrate-d") {
        return calibrate_d(&arena, &cfg, &p);
    }
    let ext = args.get("--players-from").map(|path| load_ext(&path, &arena, &cfg)).transpose()?;
    if args.has("--grid-kq") {
        return grid_kq(&arena, &cfg, &p, ext.as_ref());
    }
    let w = (!args.has("--dry")).then(|| Writer::create(&cfg.data.out_path)).transpose()?;
    let sc = run(&arena, &cfg, &p, ext.as_ref(), w.as_ref())?;
    if let Some(w) = w {
        w.finish()?;
    }
    report(&sc, &cfg, start);
    Ok(())
}
