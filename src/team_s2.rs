//! Model S2, the shipped team engine of DESIGN 4: Elo in a field, kept on the
//! published scale. `src/bin/simple.rs` is the same rules as a research tool.

use crate::chemistry::Chemistry;
use crate::config::{Config, MarginShape, PullTarget, RankRule, S2Lineup, S2};
use crate::data::{Format, TeamResult, Tournament};
use crate::difficulty::Difficulties;
use crate::editor_memory::EditorMemory;
use crate::geography::{Geography, PlayerLoad, YouthLoad};
use crate::question_model::Players;
use crate::team_event::TeamEvent;
use crate::util::{avg_ranks, Rng};

/// Six seats at a table.
const FULL_LINEUP: usize = 6;
const SEATS: f64 = 6.0;
/// Rated games after which a team is no longer provisional, and the sigma the
/// schema is given on either side of that.
const ESTABLISHED_GAMES: f64 = 5.0;
const SIGMA_ESTABLISHED: f64 = 0.1;
const SIGMA_PROVISIONAL: f64 = 0.5;
const HALF: f64 = 0.5;
/// Display points per unit of the spread and field-strength rules.
const SPREAD_UNIT: f64 = 1000.0;
const MIN_SPREAD: f64 = 1e-3;
/// `tau_spread` needs three lineups, and keeps the temperature in this range.
const MIN_TAU_FIELD: usize = 3;
const TAU_RANGE: (f64, f64) = (0.2, 2.0);
const FIELD_FACTOR_RANGE: (f64, f64) = (0.2, 5.0);
/// `adj_mult` keeps today's lineup within this ratio of the reference.
const LINEUP_RATIO: (f64, f64) = (0.5, 2.0);
/// The performance root is bracketed this many Elo units either side of the
/// field, and found to half a display point.
const BRACKET: f64 = 8.0;
const PERF_TOLERANCE: f64 = 0.5;
/// A core member whose attendance weight falls below this drops out.
const MIN_ATTENDANCE: f64 = 0.01;
/// The index of the fourth lineup, for the top-gap rules.
const FOURTH: usize = 3;
/// `info_norm`: the Fisher information of a pair, p(1 - p), is 1/4 at most.
const INFO_SCALE: f64 = 4.0;
/// Seeds the relevance draws apart from every other use of the tournament id.
const RELEVANCE_SALT: u64 = 0x5265_6C65_7661_6E63;
/// Breaks ties of the expected relevance by mean strength, far below one draw.
const TIE_NUDGE: f64 = 1e-9;
/// Keeps `ln u` finite in a Gumbel draw.
const GUMBEL_GUARD: f64 = 1e-12;

/// The forecast-side memories a tournament is read against, none of which is a
/// property of a single player: how long the lineup has played together, the
/// difficulties a field of this strength usually gets, how this lineup has
/// fared under today's editors, where its players come from and how much they
/// have already played this month.
pub struct Context<'a> {
    pub chem: &'a Chemistry,
    pub diff: &'a Difficulties,
    pub edmem: &'a EditorMemory,
    pub geo: &'a Geography,
    pub load: &'a PlayerLoad,
    pub youth: &'a YouthLoad,
}

/// S2 carries no variance, so `sigma` is only the provisional flag in the shape
/// the schema and `display.provisional_sigma` expect.
pub fn sigma(games: f64) -> f64 {
    if games >= ESTABLISHED_GAMES {
        SIGMA_ESTABLISHED
    } else {
        SIGMA_PROVISIONAL
    }
}

/// A player's strength on the team display scale: six equal players of that
/// rating are a lineup of that same rating.
pub fn player_rating(theta: f64, cfg: &Config) -> f64 {
    let s2 = &cfg.team.s2;
    s2.player_zero + s2.player_scale * (theta + 6f64.ln() + cfg.team.roster_bias)
}

/// The roster estimate of DESIGN 4.1 in display units. `ln sum exp(theta)` maps
/// to `scale * (ln sum exp(r/scale) - ln 6)` under `player_rating`, so the
/// bias and the zero cancel and only the slope is needed.
pub fn roster(rs: &[f64], scale: f64) -> f64 {
    let mx = rs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let sum: f64 = rs.iter().map(|&x| ((x - mx) / scale).exp()).sum();
    mx + scale * (sum.ln() - 6f64.ln())
}

/// Strength of the lineup that turned up, by whichever rule of DESIGN 4.1 is in
/// force. All four agree on a full lineup of six equal players.
pub fn lineup_strength(rs: &[f64], p: &S2) -> f64 {
    match p.lineup {
        S2Lineup::Mean => {
            rs.iter().sum::<f64>() / rs.len() as f64
                - p.delta * (SEATS - rs.len() as f64).max(0.0)
        }
        S2Lineup::Roster => roster(rs, p.player_scale),
        // The same soft maximum at temperature tau: tau -> 0 is the best player,
        // tau -> infinity the mean plus the log of the count.
        S2Lineup::Tempered => roster(rs, p.player_scale * p.tau),
        // chgk.gg's rule: the sorted lineup under weights 1, 5/6 ... 1/6,
        // normalised, with each seat short of six filled at the weakest player
        // less `delta`.
        S2Lineup::Best6 => {
            let mut v = rs.to_vec();
            v.sort_by(|a, b| b.total_cmp(a));
            let fill = v[v.len() - 1] - p.delta;
            let fill = if p.fill_max > 0.0 { fill.min(p.fill_max) } else { fill };
            v.resize(FULL_LINEUP, fill);
            // Weights 6, 5, ... 1 down the sorted lineup.
            let weight = |k: usize| (FULL_LINEUP - k) as f64;
            let w: f64 = (0..FULL_LINEUP).map(weight).sum();
            v.iter().take(FULL_LINEUP).enumerate().map(|(k, x)| weight(k) * x).sum::<f64>() / w
        }
    }
}

/// The sd of the tempered roster estimate, propagated from the players'
/// uncertainty: `sqrt(Σ w_i² (scale σ_i)²)` over the softmax weights the roster
/// estimate puts on the members. A lineup of six moderately uncertain players
/// and one with a single unknown have the same mean σ and different sd.
pub fn roster_sd(rs: &[f64], sigmas: &[f64], p: &S2) -> f64 {
    let t = p.player_scale * p.tau;
    let mx = rs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let e: Vec<f64> = rs.iter().map(|&x| ((x - mx) / t).exp()).collect();
    let s: f64 = e.iter().sum();
    if s <= 0.0 {
        return 0.0;
    }
    e.iter()
        .zip(sigmas)
        .map(|(&w, &sg)| {
            let w = w / s;
            let d = w * p.player_scale * sg;
            d * d
        })
        .sum::<f64>()
        .sqrt()
}

/// One lineup before the pessimistic discount is taken: what each seat is worth
/// on the `theta` scale, each member's uncertainty, the width of their own
/// discount, and the spread of the lineup. Keeping the parts lets the same
/// lineup be read at a second quantile without touching the player states again.
struct Seats {
    base: Vec<f64>,
    sigmas: Vec<f64>,
    wides: Vec<f64>,
    spread: f64,
}

/// The London Elo's pack and its shape: difficulties from `PACK_FROM - PACK_BELOW`
/// to `PACK_TO` every `PACK_STEP`, the standard sport `ln 10 / 400`, the slope's
/// rate and floor, and rho's floor, rate and ceiling.
const PACK_FROM: f64 = 2000.0;
const PACK_BELOW: f64 = 400.0;
const PACK_TO: f64 = 3600.0;
const PACK_STEP: f64 = 50.0;
const PACK_SPORT: f64 = std::f64::consts::LN_10 / 400.0;
const PACK_SLOPE: (f64, f64) = (0.002, 0.4);
const PACK_RHO: (f64, f64, f64) = (0.7, 0.0002, 0.9);

/// One result as the team engine sees it: no players left, only the number the
/// lineup that turned up is worth. `src/bin/team_sweep.rs` replays these.
#[derive(Clone)]
pub struct Row {
    pub team: u32,
    pub position: f64,
    /// Questions taken, for the margin rule; `None` when the mirror has none.
    pub score: Option<f64>,
    pub pred_pos: Option<f64>,
    pub rated: bool,
    /// Strength of the lineup that turned up; NaN when no lineup was listed.
    pub q: f64,
    /// The forecast, when it is not `q`: the expected number of answers on a
    /// package of the difficulty this field usually gets. NaN otherwise.
    pub e_ans: f64,
    /// Display points added to the forecast and to nothing else: the editor
    /// memory, the home-and-away bonus and the load penalty. `q` is left alone,
    /// so the rating, the pull and the usual lineup never see it and DESIGN 4.3
    /// is unchanged.
    pub fc_bonus: f64,
    /// The sd of the tempered roster estimate in display points, which is what
    /// `rank_rule` draws around. 0 when the lineup carries no uncertainty.
    pub sd: f64,
    /// The same lineup read at the two quantiles of `tight_gap`; NaN when the
    /// field-conditional quantile is off. The forecast picks one of them once
    /// the field's own spread is known, and `q` is untouched either way.
    pub q_tight: f64,
    pub q_loose: f64,
    /// The lineup and each member on the display scale at the `sigma_c`
    /// quantile, with no affinity: what the core of the two-part rating reads.
    /// Empty when no lineup was listed.
    pub members: Vec<u32>,
    pub member_r: Vec<f64>,
    /// Whether each member is on the team's base roster that day.
    pub member_base: Vec<bool>,
}

/// Everything the team rules of a tournament read off the player states, which
/// is one number per result. `team_sweep` computes these once and replays them.
pub fn rows_of(t: &Tournament, players: &Players, ctx: &Context, cfg: &Config) -> Vec<Row> {
    let reader = Reader::new(t, players, ctx, cfg);
    let p = reader.p;
    let seats: Vec<Seats> = t.teams.iter().map(|res| reader.seats(res)).collect();
    let lineups: Vec<Vec<f64>> = seats.iter().map(|s| reader.at(s, p.sigma_c)).collect();
    // The temperature may depend on how far apart this field's lineups are.
    let mut pt = p.clone();
    if p.tau_spread != 0.0 {
        let v: Vec<f64> = lineups.iter().filter(|rs| !rs.is_empty()).map(|rs| lineup_strength(rs, p)).collect();
        pt.tau = field_tau(&v, p);
    }
    let mut rows: Vec<Row> = t
        .teams
        .iter()
        .zip(&lineups)
        .zip(&seats)
        .map(|((res, rs), seat)| reader.row(res, rs, seat, &pt))
        .collect();
    reader.expected_answers(&mut rows);
    if p.pack_w > 0.0 {
        let h: Vec<f64> = t.teams.iter().map(|res| pack_score(&res.members, players, p)).collect();
        pack_forecast(&mut rows, &h, p.pack_w);
    }
    if p.k_score > 0.0 {
        let h: Vec<f64> = t.teams.iter().map(|res| {
            let rs: Vec<f64> = res.members.iter().map(|&i| player_rating(players.theta_score(i as usize) + players.affinity(i as usize, &reader.vbar), cfg)).collect();
            if rs.is_empty() { f64::NAN } else { lineup_strength(&rs, p) }
        }).collect();
        pack_forecast(&mut rows, &h, p.k_score);
    }
    rows
}

/// `pack_w`: the London Elo's expected score of a lineup on a fixed pack, its
/// players read at `pack_a + pack_b · theta` on its scale. A strong player's
/// chance falls off more slowly with the difficulty, and a strong table pools
/// its players' chances less, which is what lifts a deep lineup over one star.
fn pack_score(members: &[u32], players: &Players, p: &S2) -> f64 {
    if members.is_empty() {
        return f64::NAN;
    }
    let e: Vec<f64> = members.iter().map(|&i| p.pack_a + p.pack_b * players.theta(i as usize)).collect();
    let mean = e.iter().sum::<f64>() / e.len() as f64;
    let rho = (PACK_RHO.0 + PACK_RHO.1 * (mean - PACK_FROM)).clamp(PACK_RHO.0, PACK_RHO.2);
    let slope: Vec<f64> = e.iter().map(|&x| (1.0 / (1.0 + PACK_SLOPE.0 * (x - PACK_FROM).max(0.0))).max(PACK_SLOPE.1)).collect();
    let mut total = 0.0;
    let mut d = PACK_FROM - PACK_BELOW;
    while d <= PACK_TO {
        let sm: f64 = e.iter().zip(&slope).map(|(&x, &k)| {
            let z = k * PACK_SPORT * (x - d);
            let u = z.max(0.0) + (-z.abs()).exp().ln_1p();
            u.max(f64::MIN_POSITIVE).powf(1.0 / rho)
        }).sum();
        total += -(-sm.powf(rho)).exp_m1();
        d += PACK_STEP;
    }
    total
}

/// Moves each listed lineup's forecast `w` of the way to its pack score, put
/// on the field's own scale of `q` by matching the mean and the sd.
fn pack_forecast(rows: &mut [Row], h: &[f64], w: f64) {
    let pairs: Vec<(f64, f64)> = rows.iter().zip(h).filter(|(r, h)| r.q.is_finite() && h.is_finite()).map(|(r, &h)| (r.q, h)).collect();
    if pairs.len() < 2 {
        return;
    }
    let n = pairs.len() as f64;
    let (mq, mh) = (pairs.iter().map(|x| x.0).sum::<f64>() / n, pairs.iter().map(|x| x.1).sum::<f64>() / n);
    let sd = |f: &dyn Fn(&(f64, f64)) -> f64, m: f64| (pairs.iter().map(|x| (f(x) - m).powi(2)).sum::<f64>() / n).sqrt();
    let (sq, sh) = (sd(&|x| x.0, mq), sd(&|x| x.1, mh));
    if sh <= 0.0 {
        return;
    }
    for (r, &h) in rows.iter_mut().zip(h) {
        if r.q.is_finite() && h.is_finite() {
            r.fc_bonus += w * (mq + sq / sh * (h - mh) - r.q);
        }
    }
}

/// One tournament's field as the team rules read it.
struct Reader<'a> {
    t: &'a Tournament,
    players: &'a Players,
    ctx: &'a Context<'a>,
    cfg: &'a Config,
    p: &'a S2,
    vbar: Vec<f64>,
    /// In person the editor memory may enter a player's strength the way `phi`
    /// does, at a rate of `ed_phi`, instead of being a display-point bonus on
    /// the forecast alone. That is the one setting of the memory that reaches
    /// the rating, and it is off by default.
    ed_phi: bool,
}

impl<'a> Reader<'a> {
    fn new(t: &'a Tournament, players: &'a Players, ctx: &'a Context<'a>, cfg: &'a Config) -> Reader<'a> {
        let p = &cfg.team.s2;
        let ed_phi = p.ed_phi != 0.0 && ctx.edmem.on() && t.format == Format::InPerson;
        Reader { t, players, ctx, cfg, p, vbar: players.editor_mean(t), ed_phi }
    }

    /// Each seat of a lineup on the `theta` scale, its members' uncertainties
    /// and the spread of them. `sigma_c > 0` later reads each player at a
    /// pessimistic quantile, so an unknown in the lineup lowers the forecast;
    /// `sigma_ip > 0` widens that discount for a player whose evidence is mostly
    /// not from an in-person table; `k_fast > 0` mixes in the fast estimate,
    /// the only part of the state that tracks an established player's form.
    fn seats(&self, res: &TeamResult) -> Seats {
        let (players, t, p) = (self.players, self.t, self.p);
        let mut sigmas = Vec::with_capacity(res.members.len());
        let mut wides = Vec::with_capacity(res.members.len());
        let mut ss = 0.0;
        let mut seat = |i: u32| {
            let iu = i as usize;
            let sigma = players.sigma_at(iu, t.day, self.cfg);
            let ip = players.state[iu].games_ip as f64;
            sigmas.push(sigma);
            wides.push(sigma * (1.0 + p.sigma_ip / (1.0 + ip)));
            ss += sigma * sigma;
            let th = players.theta(iu);
            let th = th + p.k_fast * (players.theta_fast(iu) - th);
            let mem = if self.ed_phi && players.theta(iu) >= players.ed_gate_theta {
                self.ctx.edmem.player_bonus(i, res.team, &t.editors, t.day).unwrap_or(0.0)
            } else {
                0.0
            };
            th + players.affinity(iu, &self.vbar) + p.ed_phi * mem
        };
        let mut base: Vec<f64> = res.members.iter().map(|&i| seat(i)).collect();
        if p.k_mates > 0.0 && base.len() > 1 {
            let (sum, n) = (base.iter().sum::<f64>(), base.len() as f64);
            let new_var = self.cfg.player.sigma_new * self.cfg.player.sigma_new;
            for (b, s) in base.iter_mut().zip(&sigmas) {
                let mates = (sum - *b) / (n - 1.0);
                *b += p.k_mates * (s * s / new_var).min(1.0) * (mates - *b).max(0.0);
            }
        }
        let n = base.len().max(1) as f64;
        Seats { base, sigmas, wides, spread: p.player_scale * (ss / n).sqrt() }
    }

    /// One lineup on the display scale at the quantile `c`.
    fn at(&self, s: &Seats, c: f64) -> Vec<f64> {
        s.base.iter().zip(&s.wides).map(|(&b, &w)| player_rating(b - c * w, self.cfg)).collect()
    }

    fn row(&self, res: &TeamResult, rs: &[f64], seat: &Seats, pt: &S2) -> Row {
        let (p, players) = (self.p, self.players);
        let listed = !rs.is_empty();
        // Chemistry and the lineup's own spread raise what the lineup is worth
        // and its roster sd lowers it; all three are display points on top of
        // `Q(L)`, so all three reach the rating through the pull of DESIGN 4.3.
        let q = if listed {
            lineup_strength(rs, pt) + p.sigma_v * seat.spread + p.chemistry * self.ctx.chem.t(&res.members)
                - p.q_sigma_c * roster_sd(rs, &seat.sigmas, p)
                + p.player_scale * players.team_effect(res.team)
        } else {
            f64::NAN
        };
        // The same lineup at the two quantiles a field-conditional rule chooses
        // between; which one is used is decided in `step_rows`, where the
        // field's own spread is known.
        let (q_tight, q_loose) = if p.tight_gap > 0.0 && listed {
            (
                lineup_strength(&self.at(seat, p.sigma_c_tight), pt),
                lineup_strength(&self.at(seat, p.sigma_c_loose), pt),
            )
        } else {
            (f64::NAN, f64::NAN)
        };
        // Filled whatever the switch says: `team_sweep` builds its rows once
        // from the config and replays them under other settings.
        let member_r = |(&i, &w): (&u32, &f64)| player_rating(players.theta(i as usize) - p.sigma_c * w, self.cfg);
        let (members, member_r, member_base) = if listed {
            (res.members.clone(), res.members.iter().zip(&seat.wides).map(member_r).collect(), res.base.clone())
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };
        Row {
            member_base,
            members,
            member_r,
            team: res.team,
            position: res.position,
            score: res.score,
            pred_pos: res.pred_pos,
            rated: res.rated,
            q,
            e_ans: f64::NAN,
            fc_bonus: self.forecast_bonus(res, listed),
            sd: if listed { roster_sd(rs, &seat.sigmas, pt) } else { 0.0 },
            q_tight,
            q_loose,
        }
    }

    /// Display points on the forecast alone, so every team rating stays what
    /// it was without them: the shape of the lineup as the ranker of plan 8
    /// reads it, and at an in-person table the editor memory, the home bonuses
    /// and the load penalty.
    fn forecast_bonus(&self, res: &TeamResult, listed: bool) -> f64 {
        let (p, ctx, t) = (self.p, self.ctx, self.t);
        let in_person = t.format == Format::InPerson && listed;
        // The editor memory is made of in-person residuals and is read nowhere else.
        let mut b = if ctx.edmem.on() && p.ed_phi == 0.0 && in_person {
            let th: Vec<f64> = res.members.iter().map(|&i| self.players.theta(i as usize)).collect();
            p.editor_memory
                * ctx.edmem.bonus(&res.members, res.team, &t.editors, &th, self.players.ed_gate_theta, t.day)
        } else {
            0.0
        } + self.shape(res);
        if in_person && ctx.geo.on() {
            if p.home_bonus != 0.0 && ctx.geo.home_share(&res.members, t.town) >= p.home_min {
                b += p.home_bonus;
            }
            if p.home_team_bonus != 0.0 && res.town != u32::MAX && res.town == t.town {
                b += p.home_team_bonus;
            }
        }
        // `load_7` and `load_30` are display points per tournament per player,
        // so a negative reading is a fatigue penalty.
        if in_person && ctx.load.on() {
            let (l7, l30) = ctx.load.counts(&res.members, t.day);
            b += p.load_7 * l7 + p.load_30 * l30;
        }
        if in_person && p.k_ip_new != 0.0 && !res.members.is_empty() {
            let fresh: f64 = res.members.iter().map(|&i| 1.0 / (1.0 + self.players.state[i as usize].games_ip as f64)).sum();
            b += p.k_ip_new * fresh / res.members.len() as f64;
        }
        if listed && ctx.youth.on() {
            b += p.k_youth * ctx.youth.share(&res.members, t.day);
        }
        b
    }

    /// The weakest member's format effect and the spread of the table.
    fn shape(&self, res: &TeamResult) -> f64 {
        let (p, players) = (self.p, self.players);
        if (p.phi_min == 0.0 && p.phi_spread == 0.0) || res.members.is_empty() {
            return 0.0;
        }
        let z: Vec<f64> = res.members.iter().map(|&i| players.theta(i as usize) + players.phi(i as usize)).collect();
        let phi_min = res.members.iter().map(|&i| players.phi(i as usize)).fold(f64::INFINITY, f64::min);
        let m = z.iter().sum::<f64>() / z.len() as f64;
        let sd = (z.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / z.len() as f64).sqrt();
        p.phi_min * phi_min + p.phi_spread * sd
    }

    /// `difficulty_forecast`: each lineup's expected share of a package of the
    /// difficulty a field of this strength usually gets. A result with no lineup
    /// stands at six equal players worth its own `q`, so the two scales agree.
    fn expected_answers(&self, rows: &mut [Row]) {
        let (p, players, t, cfg) = (self.p, self.players, self.t, self.cfg);
        if !self.ctx.diff.on() {
            return;
        }
        let mut top: Vec<f64> = rows.iter().map(|r| r.q).filter(|q| q.is_finite()).collect();
        top.sort_by(|a, b| b.total_cmp(a));
        let k = 10.min(top.len());
        if k == 0 {
            return;
        }
        let bs = self.ctx.diff.sample(top[..k].iter().sum::<f64>() / k as f64);
        if bs.is_empty() {
            return;
        }
        let a = crate::question_model::a_of(t.format, cfg);
        let m = cfg.question.m_fixed;
        for (r, res) in rows.iter_mut().zip(&t.teams) {
            if res.members.is_empty() {
                continue;
            }
            let z: Vec<f64> = res
                .members
                .iter()
                .map(|&i| {
                    let i = i as usize;
                    let ip = players.state[i].games_ip as f64;
                    let pess = p.sigma_c * players.sigma_at(i, t.day, cfg) * (1.0 + p.sigma_ip / (1.0 + ip));
                    players.theta(i) + players.affinity(i, &self.vbar) - pess
                })
                .collect();
            let mut sum = 0.0;
            for &b in bs {
                let u: f64 =
                    z.iter().map(|&zi| (a * (zi - b)).clamp(cfg.numeric.exp_lo, cfg.numeric.exp_hi).exp()).sum();
                sum += (-(-u).exp_m1()).powf(m);
            }
            r.e_ans = sum / bs.len() as f64;
        }
    }
}

/// The temperature this field is read at. A field whose lineups stand close
/// together is read more sharply, because that is where the order is decided by
/// the strongest player at each table rather than by the table as a whole.
fn field_tau(v: &[f64], p: &S2) -> f64 {
    if v.len() < MIN_TAU_FIELD {
        return p.tau;
    }
    let m = v.iter().sum::<f64>() / v.len() as f64;
    let sd = (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt();
    let r = (sd / SPREAD_UNIT).max(MIN_SPREAD);
    (p.tau * r.powf(p.tau_spread)).clamp(TAU_RANGE.0, TAU_RANGE.1)
}

/// Team state of DESIGN 4.2: a rating, the usual lineup it was earned with, and
/// the count and date of the rated games behind it.
pub struct TeamsS2 {
    r: Vec<f64>,
    qbar: Vec<f64>,
    games: Vec<f64>,
    last: Vec<i32>,
    /// The last `series_k` in-person surprises of each team: how far its own
    /// lineup forecast put it from where it finished, in fields of the field's
    /// own size. Teams that peak at a championship do so every year, and this is
    /// the only quantity in the engine that carries that and is not the rating.
    surprise: Vec<Vec<f64>>,
    /// The two-part rating: the team effect, and the core as (player,
    /// attendance weight) pairs.
    b: Vec<f64>,
    core: Vec<Vec<(u32, f64, bool)>>,
    /// The seats the team usually fills, at the same memory as the weights.
    seats: Vec<f64>,
    /// Every player's latest display rating seen in any field.
    pr: Vec<f64>,
    /// Per event of the last `step_rows`: how far today's lineup stood above
    /// the team's own core at today's ratings, NaN when either is unknown. The
    /// core is kept under every setting so the sweep can find guest-heavy
    /// results whichever rule moves the rating.
    pub guest_gap: Vec<f64>,
    /// Under `best_n`: every counted performance still in the pool, as (day,
    /// points above the published zero), oldest first.
    pool: Vec<Vec<(i32, f64)>>,
    /// Under `best_n`: the performance and the legionnaire cut of each event of
    /// the last `step_rows`, NaN elsewhere.
    pub perf: Vec<(f64, f64)>,
}

/// The strength at which a team's place is what the field expects: the root of
/// `sum_j P(x beats f_j) + m P(x beats own) = wins + m/2`, by bisection.
fn performance(own: f64, others: &[f64], wins: f64, m: f64, c: f64) -> f64 {
    let g = |x: f64| {
        others.iter().map(|&f| 1.0 / (1.0 + (c * (f - x)).exp())).sum::<f64>()
            + m / (1.0 + (c * (own - x)).exp())
            - wins
            - m * HALF
    };
    let lo0 = others.iter().copied().fold(own, f64::min) - BRACKET / c;
    let hi0 = others.iter().copied().fold(own, f64::max) + BRACKET / c;
    let (mut lo, mut hi) = (lo0, hi0);
    while hi - lo > PERF_TOLERANCE {
        let mid = HALF * (lo + hi);
        if g(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    HALF * (lo + hi)
}

/// What the pairwise stage needs, read off the pre-tournament states.
struct Entry {
    ri: usize,
    idx: usize,
    /// Field strength: rating plus today's lineup offset.
    f: f64,
    q: f64,
    /// The published forecast: the lineup's strength, or the field strength when
    /// the result lists no lineup at all.
    fc: f64,
    qbar: f64,
    place: f64,
    score: Option<f64>,
    mu_before: f64,
    games: f64,
    gap_days: i32,
    rated: bool,
}

/// The relevance the metric expects from each lineup, by simulation.
///
/// NDCG pays `1/place`, which is convex, so of two lineups with the same mean
/// strength the one with the wider posterior is worth more — it has the better
/// chance of the first place, and the places it loses instead are cheap. The
/// mean ordering of DESIGN 4.1 cannot say that, and the pessimistic quantile
/// `sigma_c` says the opposite of it. Here every lineup is drawn `rank_draws`
/// times at `fc ± sd`, each draw is ranked, and the field is ordered by the mean
/// of `1/place` over the draws (`erel`), by the share of draws it led
/// (`p_first`), or by `1/place` with the pairwise noise of the Elo scale `D`
/// added as a Gumbel, which makes one draw a Plackett-Luce ranking (`erel_d`).
///
/// The seed is the tournament id, so the order does not move between runs.
fn expected_relevance(id: i64, es: &[Entry], rows: &[Row], p: &S2) -> Vec<f64> {
    let n = es.len();
    let mut acc = vec![0.0f64; n];
    let mut rng = Rng::new(id as u64 ^ RELEVANCE_SALT);
    // Gumbel(0, beta) noise reproduces the pairwise win probability of DESIGN
    // 4.3 exactly: beta = D / ln 10.
    let beta = if p.rank_rule == RankRule::ErelD {
        p.d / std::f64::consts::LN_10
    } else {
        0.0
    };
    let mut x: Vec<(f64, usize)> = Vec::with_capacity(n);
    for _ in 0..p.rank_draws {
        x.clear();
        for (k, e) in es.iter().enumerate() {
            let sd = p.rank_sd * rows[e.ri].sd;
            let mut v = e.fc + sd * rng.normal();
            if beta > 0.0 {
                // -ln(-ln u) is a standard Gumbel.
                let u = rng.uniform().max(GUMBEL_GUARD);
                v += beta * -(-u.ln()).ln();
            }
            x.push((v, k));
        }
        x.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (place, &(_, k)) in x.iter().enumerate() {
            acc[k] += match p.rank_rule {
                RankRule::PFirst => (place == 0) as u8 as f64,
                _ => 1.0 / (place + 1) as f64,
            };
        }
    }
    let d = p.rank_draws as f64;
    // A field of tied scores would leave the order to the metric's own
    // tie-break; the mean strength breaks those ties instead, at a scale far
    // below one draw's worth of relevance.
    let lo = es
        .iter()
        .map(|e| e.fc)
        .fold(f64::INFINITY, f64::min);
    let hi = es
        .iter()
        .map(|e| e.fc)
        .fold(f64::NEG_INFINITY, f64::max);
    let span = (hi - lo).max(1.0);
    for (k, a) in acc.iter_mut().enumerate() {
        *a = *a / d + TIE_NUDGE * (es[k].fc - lo) / span;
    }
    acc
}

/// The multiplier `field_gamma` puts on a tournament's steps, from the lineup
/// strengths of its field; 1 when the switch is off or does not reach the format.
pub fn field_factor(q: impl Iterator<Item = f64>, format: Format, p: &S2) -> f64 {
    if p.field_gamma == 0.0 || !(format == Format::InPerson || p.field_sync) {
        return 1.0;
    }
    let mut v: Vec<f64> = q.filter(|x| x.is_finite()).collect();
    v.sort_by(|a, b| b.total_cmp(a));
    let k = p.field_k.min(v.len());
    if k == 0 {
        return 1.0;
    }
    let s = v[..k].iter().sum::<f64>() / k as f64;
    (p.field_gamma * (s - p.field_ref) / SPREAD_UNIT).exp().clamp(FIELD_FACTOR_RANGE.0, FIELD_FACTOR_RANGE.1)
}

/// The outcome of one pair, `y_AB` of DESIGN 4.3 step 2. Win/lose when
/// `margin_g` is zero, otherwise the score gap on a scale of `margin_g`
/// questions.
fn outcome(a: &Entry, b: &Entry, p: &S2) -> f64 {
    let plain = if a.place < b.place { 1.0 } else { 0.0 };
    if p.margin_g <= 0.0 {
        return plain;
    }
    let (Some(sa), Some(sb)) = (a.score, b.score) else {
        return plain;
    };
    match p.margin_shape {
        MarginShape::Linear => (HALF + HALF * (sa - sb) / p.margin_g).clamp(0.0, 1.0),
        MarginShape::Logistic => 1.0 / (1.0 + 10f64.powf(-(sa - sb) / p.margin_g)),
    }
}

impl TeamsS2 {
    pub fn new(n: usize) -> TeamsS2 {
        TeamsS2 {
            r: vec![f64::NAN; n],
            qbar: vec![f64::NAN; n],
            games: vec![0.0; n],
            last: vec![i32::MIN / 2; n],
            surprise: vec![Vec::new(); n],
            b: vec![0.0; n],
            core: vec![Vec::new(); n],
            seats: vec![SEATS; n],
            pr: Vec::new(),
            guest_gap: Vec::new(),
            pool: vec![Vec::new(); n],
            perf: Vec::new(),
        }
    }

    /// `Q(core)`: the tempered roster estimate over the core with each player
    /// counting for their attendance weight, rescaled so the weights fill the
    /// seats the team usually fills; a guest who came once is a tenth of a seat
    /// and displaces the regulars rather than joining them. Under `core_top`
    /// it is the lineup rule over the `round(seats)` most frequent players.
    fn core_q(&self, i: usize, p: &S2) -> f64 {
        let all = &self.core[i];
        // Members only; a team with none yet above the bar reads everyone.
        let kept: Vec<(u32, f64, bool)> = all
            .iter()
            .copied()
            .filter(|e| e.1 >= p.core_min_w && (!p.core_base || e.2))
            .collect();
        let c = if kept.is_empty() { all } else { &kept };
        if p.core_top {
            let mut v = c.clone();
            v.sort_by(|a, b| b.1.total_cmp(&a.1));
            let n = (self.seats[i].round() as usize).clamp(1, v.len());
            let rs: Vec<f64> = v[..n].iter().map(|&(j, _, _)| self.pr[j as usize]).collect();
            return lineup_strength(&rs, p);
        }
        let t = p.player_scale * p.tau;
        let wsum: f64 = c.iter().map(|e| e.1).sum();
        let mx = c
            .iter()
            .map(|&(j, _, _)| self.pr[j as usize])
            .fold(f64::NEG_INFINITY, f64::max);
        let sum: f64 = c
            .iter()
            .map(|&(j, w, _)| w * ((self.pr[j as usize] - mx) / t).exp())
            .sum();
        mx + t * (sum.ln() - wsum.ln() + self.seats[i].ln() - 6f64.ln())
    }

    /// One rated game's attendance: present players move towards weight 1,
    /// absent ones towards 0 and drop out below 0.01.
    fn attend(&mut self, i: usize, members: &[u32], base: &[bool], k_c: f64) {
        let is_base = |k: usize| base.get(k).copied().unwrap_or(false);
        let s = &mut self.seats[i];
        let c = &mut self.core[i];
        if c.is_empty() {
            *s = members.len() as f64;
            c.extend(members.iter().enumerate().map(|(k, &m)| (m, 1.0, is_base(k))));
            return;
        }
        *s += k_c * (members.len() as f64 - *s);
        for e in c.iter_mut() {
            e.1 *= 1.0 - k_c;
        }
        for (k, &m) in members.iter().enumerate() {
            match c.iter_mut().find(|e| e.0 == m) {
                Some(e) => {
                    e.1 += k_c;
                    e.2 = is_base(k);
                }
                None => c.push((m, k_c, is_base(k))),
            }
        }
        c.retain(|e| e.1 >= MIN_ATTENDANCE);
    }

    /// The mean of a team's stored surprises, 0 when it has none.
    fn sbar(&self, i: usize) -> f64 {
        let v = &self.surprise[i];
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    }

    /// `(Q(core), b)` of one team as it stood after its last rated game, for
    /// diagnostics; `None` before the team has a core.
    pub fn parts_of(&self, i: usize, p: &S2) -> Option<(f64, f64)> {
        (!self.core[i].is_empty()).then(|| (self.core_q(i, p), self.b[i]))
    }

    /// Under `best_n`, the results a team's rating is drawn from, as (day,
    /// points above the published zero), oldest first.
    pub fn pool_of(&self, i: usize) -> &[(i32, f64)] {
        &self.pool[i]
    }

    pub fn n_teams(&self) -> usize {
        self.r.len()
    }

    /// `(rating, usual lineup, rated games, day of the last rated game)` of one
    /// team, or `None` before its first rated result. Read-only, for the
    /// feature table of plan 8; nothing in the engine calls it.
    pub fn state_of(&self, i: usize) -> Option<(f64, f64, f64, i32)> {
        self.r[i]
            .is_finite()
            .then(|| (self.r[i], self.qbar[i], self.games[i], self.last[i]))
    }

    /// `(index, rating, sigma, usual lineup, rated games)` for every team with a
    /// rated game in the trailing window.
    pub fn rows(&self, day: i32, window: i32) -> Vec<(usize, f64, f64, f64, f64)> {
        (0..self.r.len())
            .filter(|&i| self.r[i].is_finite() && self.last[i] >= day - window)
            .map(|i| (i, self.r[i], sigma(self.games[i]), self.qbar[i], self.games[i]))
            .collect()
    }

    /// One tournament: DESIGN 4.3. Player states must be the pre-tournament ones.
    pub fn step(
        &mut self,
        t: &Tournament,
        players: &Players,
        ctx: &Context,
        cfg: &Config,
    ) -> Vec<TeamEvent> {
        let rows = rows_of(t, players, ctx, cfg);
        self.step_rows(t.id, t.day, t.weight, t.format, &rows, &cfg.team.s2)
    }

    /// The team rules alone, over lineup strengths already computed. `fit` and
    /// the parameter sweep run this same code. `id` seeds the draws of
    /// `rank_rule` and is read for nothing else, so a field's order is the same
    /// on every run and on every pass.
    pub fn step_rows(
        &mut self,
        id: i64,
        day: i32,
        weight: f64,
        format: Format,
        rows: &[Row],
        p: &S2,
    ) -> Vec<TeamEvent> {
        let gap = top_gap(rows, p);
        let tight = p.tight_gap > 0.0 && gap < p.tight_gap;
        self.remember(rows, p);
        let es: Vec<Entry> = rows
            .iter()
            .enumerate()
            .filter_map(|(ri, res)| self.enter(ri, res, day, format, tight, p))
            .collect();
        self.guest_gap.clear();
        self.perf.clear();
        if es.is_empty() {
            return Vec::new();
        }
        let field = FieldStep::new(id, &es, rows, weight, format, gap, p);
        let mut out = Vec::with_capacity(es.len());
        for (k, e) in es.iter().enumerate() {
            let mv = if e.rated { self.update(e, k, &field, rows, day, p) } else { Moves::idle() };
            self.guest_gap.push(mv.gap);
            self.perf.push(mv.shown);
            out.push(self.event(e, k, &mv, &field, rows));
        }
        out
    }

    /// Every player's latest display rating, and under `core_live` every
    /// listed team re-read at today's player ratings.
    fn remember(&mut self, rows: &[Row], p: &S2) {
        for r in rows {
            for (&m, &x) in r.members.iter().zip(&r.member_r) {
                let m = m as usize;
                if m >= self.pr.len() {
                    self.pr.resize(m + 1, f64::NAN);
                }
                self.pr[m] = x;
            }
        }
        if p.two_part && p.core_live {
            for r in rows {
                let i = r.team as usize;
                if self.r[i].is_finite() && !self.core[i].is_empty() {
                    self.r[i] = self.core_q(i, p) + self.b[i];
                }
            }
        }
    }

    /// One result as it stands in the field, opening the team's state on its
    /// first rated result. `None` for a team with neither a lineup nor a rating,
    /// which cannot be placed.
    fn enter(&mut self, ri: usize, res: &Row, day: i32, format: Format, tight: bool, p: &S2) -> Option<Entry> {
        let idx = res.team as usize;
        let known = self.r[idx].is_finite();
        let q = res.q.is_finite().then_some(res.q).or(known.then_some(f64::NAN))?;
        let f = if known { self.field_strength(idx, q, p) } else { self.open(idx, res, q, p) };
        let fc = self.forecast(idx, res, q, f, format, tight, p);
        // Under `expect_forecast` the field is judged at the published
        // forecast whenever the result lists a lineup.
        let f = if p.expect_forecast && fc.is_finite() && !q.is_nan() { fc } else { f };
        Some(Entry {
            ri,
            idx,
            f,
            q,
            fc,
            qbar: self.qbar[idx],
            place: if res.position > 0.0 { res.position } else { f64::INFINITY },
            score: res.score,
            mu_before: self.r[idx],
            games: self.games[idx],
            gap_days: if self.last[idx] > i32::MIN / 2 { day - self.last[idx] } else { 0 },
            rated: res.rated && res.position > 0.0,
        })
    }

    /// A rated team in today's field: its rating shifted by how much stronger
    /// today's lineup is than the one it is measured against (step 1 of DESIGN
    /// 4.3), or the rating alone when no lineup was listed.
    fn field_strength(&self, idx: usize, q: f64, p: &S2) -> f64 {
        if q.is_nan() {
            return self.r[idx];
        }
        if p.two_part {
            return q + self.b[idx];
        }
        let base = if p.ref_core && !self.core[idx].is_empty() { self.core_q(idx, p) } else { self.qbar[idx] };
        if p.adj_mult {
            let z = p.start_zero;
            let ratio = ((q - z) / (base - z).max(1.0)).clamp(LINEUP_RATIO.0, LINEUP_RATIO.1);
            z + (self.r[idx] - z) * ratio
        } else {
            self.r[idx] + p.lineup_adj * (q - base)
        }
    }

    /// A team with no rating stands in the field at its lineup. A rated result
    /// opens its state at the starting rule; an unrated one never does.
    fn open(&mut self, idx: usize, res: &Row, q: f64, p: &S2) -> f64 {
        if res.rated {
            self.r[idx] = if p.best_n > 0 {
                p.start_zero
            } else {
                q - p.start_drop - p.start_frac * (q - p.start_zero).max(0.0)
            };
            self.qbar[idx] = q;
            self.b[idx] = 0.0;
            self.core[idx].clear();
        }
        q
    }

    /// The published forecast. `difficulty_forecast` replaces the lineup's
    /// strength by what it is expected to answer; `kappa_s` adds the team's
    /// recent over- or under-performance against its own forecast, which is the
    /// one bonus that is not a property of today's lineup.
    #[allow(clippy::too_many_arguments)]
    fn forecast(&self, idx: usize, res: &Row, q: f64, f: f64, format: Format, tight: bool, p: &S2) -> f64 {
        let alt = if tight { res.q_tight } else { res.q_loose };
        let qf = if p.tight_gap > 0.0 && alt.is_finite() { alt } else { q };
        let fc = if res.e_ans.is_finite() {
            res.e_ans
        } else if q.is_nan() {
            f
        } else if p.forecast_rating {
            f + res.fc_bonus
        } else {
            qf + res.fc_bonus
        };
        let series = if p.kappa_s != 0.0 && format == Format::InPerson { p.kappa_s * self.sbar(idx) } else { 0.0 };
        let team = if q.is_finite() && self.r[idx].is_finite() {
            p.k_usual * (self.qbar[idx] - q) + p.k_team * (self.r[idx] - q)
        } else {
            0.0
        };
        fc + series + team
    }

    /// One rated result: the placement step, then either the pull or the
    /// two-part rebuild, then the usual lineup, the core and the counters.
    fn update(&mut self, e: &Entry, k: usize, field: &FieldStep, rows: &[Row], day: i32, p: &S2) -> Moves {
        let i = e.idx;
        let before = self.r[i];
        let mut mv = Moves::idle();
        if e.q.is_finite() && !self.core[i].is_empty() {
            mv.gap = e.q - self.core_q(i, p);
        }
        mv.place = self.place_step(e, k, field, rows, day, before, &mut mv.shown, p);
        self.r[i] += mv.place;
        if p.two_part {
            self.rebuild(e, field, rows, before, &mut mv, p);
        } else {
            mv.pull = self.pull(e, field, before, p);
        }
        if e.q.is_finite() {
            self.qbar[i] += p.k_q * (e.q - self.qbar[i]);
            if !p.two_part {
                let r = &rows[e.ri];
                self.attend(i, &r.members, &r.member_base, p.k_c);
            }
        }
        self.games[i] += 1.0;
        self.last[i] = day;
        // Today's surprise joins the memory only after today's forecast has been
        // made, so a result never predicts itself.
        if p.kappa_s != 0.0 && field.format == Format::InPerson && p.series_k > 0 {
            let s = (field.fplace[k] - rows[e.ri].position) / (field.n as f64);
            let v = &mut self.surprise[i];
            v.push(s);
            if v.len() > p.series_k {
                v.remove(0);
            }
        }
        mv
    }

    /// The placement step: `K · w · D` times actual minus expected wins, faster
    /// for a provisional team (`K_n = K max(1, n0 / n)`). Under `best_n` the
    /// rating is instead the mean of the best recent performances; under
    /// `place_sign` the step cannot go against the forecast place.
    #[allow(clippy::too_many_arguments)]
    fn place_step(
        &mut self,
        e: &Entry,
        k: usize,
        field: &FieldStep,
        rows: &[Row],
        day: i32,
        before: f64,
        shown: &mut (f64, f64),
        p: &S2,
    ) -> f64 {
        let kn = p.k * (p.n0 / e.games.max(1.0)).max(1.0);
        let d = kn * field.w * p.d * field.spair[k];
        if p.best_n > 0 && field.perf[k].is_finite() {
            // The lineup's lift over the base roster is not the team's.
            let cut = if e.q.is_finite() && !self.core[e.idx].is_empty() {
                (e.q - self.core_q(e.idx, p)).max(0.0)
            } else {
                0.0
            };
            *shown = (field.perf[k], cut);
            return self.best_of(e.idx, day, field.perf[k] - cut, p) - before;
        }
        if !p.place_sign {
            return d;
        }
        if rows[e.ri].position <= field.fplace[k] {
            d.max(0.0)
        } else {
            d.min(0.0)
        }
    }

    /// `best_n`: the performance joins the pool, and the rating is the mean of
    /// the best of the year before this game. A short year is topped up with the
    /// most recent results before it, so a returning team's new results push
    /// out its oldest ones.
    fn best_of(&mut self, i: usize, day: i32, perf: f64, p: &S2) -> f64 {
        let pool = &mut self.pool[i];
        pool.push((day, (perf - p.start_zero).max(0.0)));
        let from = day - p.best_window;
        let mut v: Vec<f64> = pool.iter().filter(|x| x.0 >= from).map(|x| x.1).collect();
        v.sort_by(|a, b| b.total_cmp(a));
        v.truncate(p.best_n);
        let need = p.best_n - v.len();
        v.extend(pool.iter().rev().filter(|x| x.0 < from).take(need).map(|x| x.1));
        let sum: f64 = v.iter().sum();
        p.start_zero + sum / p.best_n as f64
    }

    /// The two-part rating of DESIGN 4.9: the team effect takes the placement
    /// step and nothing else, the core follows attendance, and the rating is
    /// rebuilt from the two.
    fn rebuild(&mut self, e: &Entry, field: &FieldStep, rows: &[Row], before: f64, mv: &mut Moves, p: &S2) {
        let i = e.idx;
        let q_prev = before - self.b[i];
        self.b[i] += mv.place;
        self.b[i] *= 1.0 - p.k_b * field.w;
        let q0 = if self.core[i].is_empty() { q_prev } else { self.core_q(i, p) };
        if e.q.is_finite() {
            let r = &rows[e.ri];
            self.attend(i, &r.members, &r.member_base, p.k_c);
        }
        if !self.core[i].is_empty() {
            mv.drift = q0 - q_prev;
            mv.attend = self.core_q(i, p) - q0;
            self.r[i] = self.core_q(i, p) + self.b[i];
        }
        if p.winner_floor && e.place == field.best && self.r[i] < before {
            self.b[i] += before - self.r[i];
            self.r[i] = before;
        }
        mv.pull = self.r[i] - before - mv.place;
    }

    /// The pull of DESIGN 4.3 step 3 towards what the players who turned up are
    /// worth, then the floor of principle 2. Returns the pull alone.
    fn pull(&mut self, e: &Entry, field: &FieldStep, before: f64, p: &S2) -> f64 {
        let i = e.idx;
        let mut d = 0.0;
        if p.k_r > 0.0 {
            let target = match p.k_r_target {
                PullTarget::Lineup => e.q,
                PullTarget::Usual => e.qbar,
            };
            let kr = match field.format {
                Format::InPerson => p.k_r,
                _ => p.k_r * p.k_r_sync,
            };
            if target.is_finite() {
                d = kr * field.w * (target - self.r[i]);
                self.r[i] += d;
            }
        }
        if p.winner_floor && e.place == field.best && self.r[i] < before {
            self.r[i] = before;
        }
        d
    }

    fn event(&self, e: &Entry, k: usize, mv: &Moves, field: &FieldStep, rows: &[Row]) -> TeamEvent {
        let row = &rows[e.ri];
        TeamEvent {
            d_place: mv.place,
            d_pull: mv.pull,
            team: e.idx as u32,
            c: e.rated as u8 as f64,
            delta_l: if e.mu_before.is_finite() && e.q.is_finite() { e.q - e.qbar } else { 0.0 },
            x: e.f,
            // Under a sampled rule this column is the expected relevance, not
            // display points: it is what the order was made from, and the
            // metric reads the order alone.
            forecast: field.score[k],
            forecast_place: field.fplace[k],
            expected_place: field.n as f64 - field.psum[k],
            actual_place: row.position,
            pred_place: row.pred_pos,
            s_r: e.q,
            s_pair: field.spair[k],
            i_pair: 1.0,
            mu_before: e.mu_before,
            mu_after: self.r[e.idx],
            sigma_before: sigma(e.games),
            sigma_after: sigma(self.games[e.idx]),
            evidence: e.games,
            gap_days: e.gap_days,
            // S2 has no variance; under `two_part` these two carry the roster's
            // share of the move, for the sweep's diagnostics.
            var_diff: mv.drift,
            lost_prev: mv.attend,
            rated: e.rated,
            z: None,
        }
    }
}

/// How bunched the top of a field is: the gap from the first to the fourth
/// lineup, read only when a rule asks for it.
fn top_gap(rows: &[Row], p: &S2) -> f64 {
    if p.tight_gap <= 0.0 && p.rank_gap <= 0.0 {
        return f64::INFINITY;
    }
    let mut v: Vec<f64> = rows.iter().map(|r| r.q).filter(|x| x.is_finite()).collect();
    v.sort_by(|a, b| b.total_cmp(a));
    if v.len() > FOURTH {
        v[0] - v[FOURTH]
    } else {
        f64::INFINITY
    }
}

/// How one rated result moved its team: the placement step and the pull,
/// under `two_part` the roster's share of the move, the lineup's lift over its
/// core, and under `best_n` the performance and its legionnaire cut.
struct Moves {
    place: f64,
    pull: f64,
    drift: f64,
    attend: f64,
    gap: f64,
    shown: (f64, f64),
}

impl Moves {
    fn idle() -> Moves {
        Moves { place: f64::NAN, pull: f64::NAN, drift: 0.0, attend: 0.0, gap: f64::NAN, shown: (f64::NAN, f64::NAN) }
    }
}

/// What the whole field settles before any team moves: the published order,
/// each team's expected and actual wins over the rest, the weight of the field
/// and its first place.
struct FieldStep {
    n: usize,
    format: Format,
    score: Vec<f64>,
    fplace: Vec<f64>,
    psum: Vec<f64>,
    spair: Vec<f64>,
    w: f64,
    best: f64,
    /// Under `best_n`, every placed team's performance; empty otherwise.
    perf: Vec<f64>,
}

impl FieldStep {
    fn new(id: i64, es: &[Entry], rows: &[Row], weight: f64, format: Format, gap: f64, p: &S2) -> FieldStep {
        // The order the field is published in. `rank_rule = "mean"` sorts by the
        // lineups' mean strength; the other three sort by the relevance the
        // metric actually pays, estimated from draws of every lineup's own
        // posterior. `fc` then carries that score rather than display points,
        // which is all the NDCG of `src/ndcg.rs` reads.
        let sample = p.rank_rule != RankRule::Mean && p.rank_draws > 0 && (p.rank_gap <= 0.0 || gap < p.rank_gap);
        let score: Vec<f64> =
            if sample { expected_relevance(id, es, rows, p) } else { es.iter().map(|e| e.fc).collect() };
        let fplace = avg_ranks(&score.iter().map(|x| -x).collect::<Vec<_>>());
        let (psum, spair) = pair_scores(es, p);
        // The team side may weigh the formats differently from the player side,
        // and a field with strong teams at the top moves ratings more.
        let w = match format {
            Format::Sync => p.w_sync.unwrap_or(weight),
            Format::StrictSync => p.w_strict_sync.unwrap_or(weight),
            _ => weight,
        };
        let w = w * field_factor(rows.iter().map(|r| r.q), format, p);
        let best = es.iter().filter(|e| e.place.is_finite()).map(|e| e.place).fold(f64::INFINITY, f64::min);
        let perf = if p.best_n > 0 { performances(es, p) } else { Vec::new() };
        FieldStep { n: es.len(), format, score, fplace, psum, spair, w, best, perf }
    }
}

/// The win probability of a field strength `a` over `b` on the Elo scale `D`.
fn win(a: f64, b: f64, p: &S2) -> f64 {
    let c = std::f64::consts::LN_10 / p.d;
    1.0 / (1.0 + (c * (b - a)).exp())
}

/// Each team's expected wins over the field, and its actual wins minus those,
/// normalised. A shared place says nothing about which team is better, so its
/// expectation is added back: a team that finished first then has `S >= 0`
/// whether it shared the place or not.
fn pair_scores(es: &[Entry], p: &S2) -> (Vec<f64>, Vec<f64>) {
    let n = es.len();
    let mut psum = vec![0.0; n];
    let mut pvar = vec![0.0; n];
    for i in 0..n {
        for j in i + 1..n {
            let pij = win(es[i].f, es[j].f, p);
            psum[i] += pij;
            psum[j] += 1.0 - pij;
            pvar[i] += pij * (1.0 - pij);
            pvar[j] += pij * (1.0 - pij);
        }
    }
    let mut ord: Vec<usize> = (0..n).collect();
    ord.sort_by(|&a, &b| es[a].place.total_cmp(&es[b].place));
    let mut spair = vec![0.0; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && es[ord[j + 1]].place == es[ord[i]].place {
            j += 1;
        }
        let level = &ord[i..=j];
        let above = (n - j - 1) as f64;
        for &a in level {
            // Every pair that is not a shared place, by whichever outcome rule is
            // in force; the shipped one is 1 above, 0 below, which is just the
            // count of the teams that finished lower.
            let ysum: f64 = if p.margin_g <= 0.0 {
                above
            } else {
                (0..n).filter(|b| !level.contains(b)).map(|b| outcome(&es[a], &es[b], p)).sum()
            };
            let tied: f64 = level.iter().filter(|&&b| b != a).map(|&b| win(es[a].f, es[b].f, p)).sum();
            let norm = if p.info_norm { (INFO_SCALE * pvar[a]).max(p.info_min) } else { ((n - 1) as f64).max(1.0) };
            spair[a] = (ysum - psum[a] + tied) / norm;
        }
        i = j + 1;
    }
    (psum, spair)
}

/// Under `best_n`, every placed team's performance against the published
/// forecasts of the rest of the field.
fn performances(es: &[Entry], p: &S2) -> Vec<f64> {
    let c = std::f64::consts::LN_10 / p.d;
    let one = |(k, e): (usize, &Entry)| {
        if !e.place.is_finite() {
            return f64::NAN;
        }
        let mut others = Vec::with_capacity(es.len());
        let mut wins = 0.0;
        for (j, o) in es.iter().enumerate() {
            if j == k || !o.place.is_finite() || !o.fc.is_finite() {
                continue;
            }
            others.push(o.fc);
            wins += match o.place.total_cmp(&e.place) {
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Equal => HALF,
                std::cmp::Ordering::Less => 0.0,
            };
        }
        performance(e.fc, &others, wins, p.perf_prior, c)
    };
    es.iter().enumerate().map(one).collect()
}
