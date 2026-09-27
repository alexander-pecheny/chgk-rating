//! Model S2, the shipped team engine of DESIGN 4: Elo in a field, kept on the
//! published scale. `src/bin/simple.rs` is the same rules as a research tool.

use crate::chemistry::Chemistry;
use crate::config::{Config, MarginShape, PullTarget, RankRule, S2Lineup, S2};
use crate::data::{Format, Tournament};
use crate::difficulty::Difficulties;
use crate::editor_memory::EditorMemory;
use crate::geography::{Geography, PlayerLoad};
use crate::question_model::Players;
use crate::team_model::TeamEvent;
use crate::util::{avg_ranks, Rng};

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
}

/// S2 carries no variance, so `sigma` is only the provisional flag in the shape
/// the schema and `display.provisional_sigma` expect.
pub fn sigma(games: f64) -> f64 {
    if games >= 5.0 {
        0.1
    } else {
        0.5
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
                - p.delta * (6.0 - rs.len() as f64).max(0.0)
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
            v.resize(6, fill);
            let w: f64 = (1..=6).map(|k| (7 - k) as f64).sum();
            v.iter()
                .take(6)
                .enumerate()
                .map(|(k, x)| (6 - k) as f64 * x)
                .sum::<f64>()
                / w
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
    let (chem, diff) = (ctx.chem, ctx.diff);
    let p = &cfg.team.s2;
    let vbar = players.editor_mean(t);
    let a = crate::question_model::a_of(t.format, cfg);
    let m = cfg.question.m_fixed;
    // In person the editor memory may enter a player's strength the way `phi`
    // does, at a rate of `ed_phi`, instead of being a display-point bonus on the
    // forecast alone. That is the one setting of the memory that reaches the
    // rating, and it is off by default.
    let ed_phi = p.ed_phi != 0.0 && ctx.edmem.on() && t.format == Format::InPerson;
    // Each lineup on the team scale, its members' uncertainties and the spread
    // of them. `base` is a seat before the pessimistic discount and `wide` the
    // discount's own width, so the same lineup can be read at another quantile
    // without touching the player states again.
    let seats: Vec<Seats> = t
        .teams
        .iter()
        .map(|res| {
            let mut sigmas = Vec::with_capacity(res.members.len());
            let mut wides = Vec::with_capacity(res.members.len());
            let mut ss = 0.0;
            let base: Vec<f64> = res
                .members
                .iter()
                .map(|&i| {
                    let iu = i as usize;
                    // `sigma_c > 0` reads each player at a pessimistic quantile,
                    // so an unknown in the lineup lowers the forecast.
                    // `sigma_ip > 0` widens the discount again for a player whose
                    // evidence is mostly not from an in-person table.
                    let sigma = players.sigma_at(iu, t.day, cfg);
                    let ip = players.state[iu].games_ip as f64;
                    sigmas.push(sigma);
                    wides.push(sigma * (1.0 + p.sigma_ip / (1.0 + ip)));
                    ss += sigma * sigma;
                    // `k_fast > 0` mixes in the fast estimate of the same
                    // strength, which is the only part of the state that tracks
                    // an established player's recent form.
                    let th = players.theta(iu);
                    let th = th + p.k_fast * (players.theta_fast(iu) - th);
                    let mem = if ed_phi && players.theta(iu) >= players.ed_gate_theta {
                        ctx.edmem
                            .player_bonus(i, res.team, &t.editors, t.day)
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    };
                    th + players.affinity(iu, &vbar) + p.ed_phi * mem
                })
                .collect();
            let n = base.len().max(1) as f64;
            Seats {
                base,
                sigmas,
                wides,
                spread: p.player_scale * (ss / n).sqrt(),
            }
        })
        .collect();
    // One lineup on the display scale at a given quantile.
    let at = |base: &[f64], wides: &[f64], c: f64| -> Vec<f64> {
        base.iter()
            .zip(wides)
            .map(|(&b, &w)| player_rating(b - c * w, cfg))
            .collect()
    };
    let lineups: Vec<(Vec<f64>, Vec<f64>, f64)> = seats
        .iter()
        .map(|s| (at(&s.base, &s.wides, p.sigma_c), s.sigmas.clone(), s.spread))
        .collect();
    // The temperature may depend on how far apart this field's lineups are.
    let tau = if p.tau_spread != 0.0 {
        let v: Vec<f64> = lineups
            .iter()
            .filter(|(rs, _, _)| !rs.is_empty())
            .map(|(rs, _, _)| lineup_strength(rs, p))
            .collect();
        field_tau(&v, p)
    } else {
        p.tau
    };
    let mut pt = p.clone();
    pt.tau = tau;
    let mut rows: Vec<Row> = t
        .teams
        .iter()
        .zip(&lineups)
        .zip(&seats)
        .map(|((res, (rs, sigmas, sd)), seat)| {
            let (base, wides) = (&seat.base, &seat.wides);
            let q = if rs.is_empty() {
                f64::NAN
            } else {
                // Chemistry and the lineup's own spread raise what the lineup is
                // worth and its roster sd lowers it; all three are display points
                // on top of `Q(L)`, so all three reach the rating through the
                // pull of DESIGN 4.3.
                lineup_strength(rs, &pt)
                    + p.sigma_v * sd
                    + p.chemistry * chem.t(&res.members)
                    - p.q_sigma_c * roster_sd(rs, sigmas, p)
            };
            // The shape of the lineup as the ranker of plan 8 reads it: the
            // weakest member's format effect and the spread of the table. Both
            // move the forecast alone, like the editor memory below.
            let shape = if (p.phi_min != 0.0 || p.phi_spread != 0.0) && !res.members.is_empty() {
                let z: Vec<f64> = res
                    .members
                    .iter()
                    .map(|&i| {
                        let i = i as usize;
                        players.theta(i) + players.phi(i)
                    })
                    .collect();
                let phi_min = res
                    .members
                    .iter()
                    .map(|&i| players.phi(i as usize))
                    .fold(f64::INFINITY, f64::min);
                let m = z.iter().sum::<f64>() / z.len() as f64;
                let sd = (z.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / z.len() as f64).sqrt();
                p.phi_min * phi_min + p.phi_spread * sd
            } else {
                0.0
            };
            // The editor memory is read at an in-person table and nowhere else --
            // it is made of in-person residuals -- and it moves the forecast
            // alone, so every synchronous forecast and every team rating stay
            // exactly what they were without it.
            let in_person = t.format == Format::InPerson && !rs.is_empty();
            let mut fc_bonus = if ctx.edmem.on() && p.ed_phi == 0.0 && in_person {
                let th: Vec<f64> = res
                    .members
                    .iter()
                    .map(|&i| players.theta(i as usize))
                    .collect();
                p.editor_memory
                    * ctx.edmem.bonus(
                        &res.members,
                        res.team,
                        &t.editors,
                        &th,
                        players.ed_gate_theta,
                        t.day,
                    )
            } else {
                0.0
            } + shape;
            // Where the lineup comes from, against where it is playing. Both
            // bonuses are display points on the forecast alone.
            if in_person && ctx.geo.on() {
                if p.home_bonus != 0.0
                    && ctx.geo.home_share(&res.members, t.town) >= p.home_min
                {
                    fc_bonus += p.home_bonus;
                }
                if p.home_team_bonus != 0.0 && res.town != u32::MAX && res.town == t.town {
                    fc_bonus += p.home_team_bonus;
                }
            }
            // How much the players have already played. `load_7` and `load_30`
            // are display points per tournament per player, so the shipped
            // negative reading is a fatigue penalty.
            if in_person && ctx.load.on() {
                let (l7, l30) = ctx.load.counts(&res.members, t.day);
                fc_bonus += p.load_7 * l7 + p.load_30 * l30;
            }
            // The same lineup at the two quantiles a field-conditional rule
            // chooses between. Built here because the seats are still to hand;
            // which one is used is decided in `step_rows`, where the field's own
            // spread is known.
            let (q_tight, q_loose) = if p.tight_gap > 0.0 && !rs.is_empty() {
                (
                    lineup_strength(&at(base, wides, p.sigma_c_tight), &pt),
                    lineup_strength(&at(base, wides, p.sigma_c_loose), &pt),
                )
            } else {
                (f64::NAN, f64::NAN)
            };
            // Filled whatever the switch says: `team_sweep` builds its rows once
            // from the config and replays them under other settings.
            let (members, member_r) = if !rs.is_empty() {
                let r = res
                    .members
                    .iter()
                    .zip(wides)
                    .map(|(&i, &w)| player_rating(players.theta(i as usize) - p.sigma_c * w, cfg))
                    .collect();
                (res.members.clone(), r)
            } else {
                (Vec::new(), Vec::new())
            };
            Row {
                member_base: if rs.is_empty() { Vec::new() } else { res.base.clone() },
                members,
                member_r,
                team: res.team,
                position: res.position,
                score: res.score,
                pred_pos: res.pred_pos,
                rated: res.rated,
                q,
                e_ans: f64::NAN,
                fc_bonus,
                sd: if rs.is_empty() {
                    0.0
                } else {
                    roster_sd(rs, sigmas, &pt)
                },
                q_tight,
                q_loose,
            }
        })
        .collect();
    if !diff.on() {
        return rows;
    }
    // The field's strength, for the bucket of difficulties to score it against.
    let mut top: Vec<f64> = rows.iter().map(|r| r.q).filter(|q| q.is_finite()).collect();
    top.sort_by(|a, b| b.total_cmp(a));
    let k = 10.min(top.len());
    if k == 0 {
        return rows;
    }
    let strength = top[..k].iter().sum::<f64>() / k as f64;
    let bs = diff.sample(strength);
    if bs.is_empty() {
        return rows;
    }
    // Each lineup's expected share of such a package. A result with no lineup
    // stands at six equal players worth its own `q`, so the two scales agree.
    for (r, res) in rows.iter_mut().zip(&t.teams) {
        let z: Vec<f64> = if res.members.is_empty() {
            Vec::new()
        } else {
            res.members
                .iter()
                .map(|&i| {
                    let i = i as usize;
                    let ip = players.state[i].games_ip as f64;
                    let pess = p.sigma_c
                        * players.sigma_at(i, t.day, cfg)
                        * (1.0 + p.sigma_ip / (1.0 + ip));
                    players.theta(i) + players.affinity(i, &vbar) - pess
                })
                .collect()
        };
        if z.is_empty() {
            continue;
        }
        let mut sum = 0.0;
        for &b in bs {
            let u: f64 = z
                .iter()
                .map(|&zi| (a * (zi - b)).clamp(cfg.numeric.exp_lo, cfg.numeric.exp_hi).exp())
                .sum();
            sum += (-(-u).exp_m1()).powf(m);
        }
        r.e_ans = sum / bs.len() as f64;
    }
    rows
}

/// The temperature this field is read at. A field whose lineups stand close
/// together is read more sharply, because that is where the order is decided by
/// the strongest player at each table rather than by the table as a whole.
fn field_tau(v: &[f64], p: &S2) -> f64 {
    if v.len() < 3 {
        return p.tau;
    }
    let m = v.iter().sum::<f64>() / v.len() as f64;
    let sd = (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt();
    let r = (sd / 1000.0).max(1e-3);
    (p.tau * r.powf(p.tau_spread)).clamp(0.2, 2.0)
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
    let mut rng = Rng::new(id as u64 ^ 0x5265_6C65_7661_6E63);
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
                let u = ((rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64).max(1e-12);
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
        *a = *a / d + 1e-9 * (es[k].fc - lo) / span;
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
    (p.field_gamma * (s - p.field_ref) / 1000.0).exp().clamp(0.2, 5.0)
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
        MarginShape::Linear => (0.5 + (sa - sb) / (2.0 * p.margin_g)).clamp(0.0, 1.0),
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
            seats: vec![6.0; n],
            pr: Vec::new(),
            guest_gap: Vec::new(),
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
        c.retain(|e| e.1 >= 0.01);
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
        // A quantile that depends on the field: the lineups are ordered once at
        // the shipped `sigma_c` to measure how bunched the top is, and the
        // forecast is then re-read at the quantile that tightness asks for.
        let gap = if p.tight_gap > 0.0 || p.rank_gap > 0.0 {
            let mut v: Vec<f64> = rows.iter().map(|r| r.q).filter(|x| x.is_finite()).collect();
            v.sort_by(|a, b| b.total_cmp(a));
            if v.len() >= 4 {
                v[0] - v[3]
            } else {
                f64::INFINITY
            }
        } else {
            f64::INFINITY
        };
        let tight = p.tight_gap > 0.0 && gap < p.tight_gap;
        {
            for r in rows {
                for (&m, &x) in r.members.iter().zip(&r.member_r) {
                    let m = m as usize;
                    if m >= self.pr.len() {
                        self.pr.resize(m + 1, f64::NAN);
                    }
                    self.pr[m] = x;
                }
            }
            // The live reading: the core at today's player ratings.
            if p.two_part && p.core_live {
                for r in rows {
                    let i = r.team as usize;
                    if self.r[i].is_finite() && !self.core[i].is_empty() {
                        self.r[i] = self.core_q(i, p) + self.b[i];
                    }
                }
            }
        }
        let mut es: Vec<Entry> = Vec::with_capacity(rows.len());
        for (ri, res) in rows.iter().enumerate() {
            let idx = res.team as usize;
            let known = self.r[idx].is_finite();
            // A team with neither a lineup nor a rating cannot be placed in the field.
            let Some(q) = res
                .q
                .is_finite()
                .then_some(res.q)
                .or(known.then_some(f64::NAN))
            else {
                continue;
            };
            let f = if known {
                if q.is_nan() {
                    self.r[idx]
                } else if p.two_part {
                    q + self.b[idx]
                } else {
                    let base = if p.ref_core && !self.core[idx].is_empty() {
                        self.core_q(idx, p)
                    } else {
                        self.qbar[idx]
                    };
                    if p.adj_mult {
                        let z = p.start_zero;
                        let ratio = ((q - z) / (base - z).max(1.0)).clamp(0.5, 2.0);
                        z + (self.r[idx] - z) * ratio
                    } else {
                        self.r[idx] + p.lineup_adj * (q - base)
                    }
                }
            } else {
                // An unrated result never opens a state; it still stands in the field.
                if res.rated {
                    self.r[idx] = q - p.start_drop - p.start_frac * (q - p.start_zero).max(0.0);
                    self.qbar[idx] = q;
                    self.b[idx] = 0.0;
                    self.core[idx].clear();
                }
                q
            };
            // The published forecast. `difficulty_forecast` replaces the lineup's
            // strength by what it is expected to answer; `kappa_s` adds the
            // team's recent over- or under-performance against its own forecast,
            // which is the one bonus that is not a property of today's lineup.
            let qf = if p.tight_gap > 0.0 {
                let alt = if tight { res.q_tight } else { res.q_loose };
                if alt.is_finite() {
                    alt
                } else {
                    q
                }
            } else {
                q
            };
            let fc = if res.e_ans.is_finite() {
                res.e_ans
            } else if q.is_nan() {
                f
            } else if p.forecast_rating {
                f + res.fc_bonus
            } else {
                qf + res.fc_bonus
            };
            let fc = fc
                + if p.kappa_s != 0.0 && format == Format::InPerson {
                    p.kappa_s * self.sbar(idx)
                } else {
                    0.0
                };
            // Under `expect_forecast` the field is judged at the published
            // forecast whenever the result lists a lineup.
            let f = if p.expect_forecast && fc.is_finite() && !q.is_nan() { fc } else { f };
            es.push(Entry {
                ri,
                idx,
                f,
                q,
                fc,
                qbar: self.qbar[idx],
                place: if res.position > 0.0 {
                    res.position
                } else {
                    f64::INFINITY
                },
                score: res.score,
                mu_before: self.r[idx],
                games: self.games[idx],
                gap_days: (self.last[idx] > i32::MIN / 2)
                    .then(|| day - self.last[idx])
                    .unwrap_or(0),
                rated: res.rated && res.position > 0.0,
            });
        }
        let n = es.len();
        self.guest_gap.clear();
        if n == 0 {
            return Vec::new();
        }

        // The order the field is published in. `rank_rule = "mean"` sorts by the
        // lineups' mean strength; the other three sort by the relevance the
        // metric actually pays, estimated from draws of every lineup's own
        // posterior. `fc` then carries that score rather than display points,
        // which is all the NDCG of `src/ndcg.rs` reads.
        let sample = p.rank_rule != RankRule::Mean
            && p.rank_draws > 0
            && (p.rank_gap <= 0.0 || gap < p.rank_gap);
        let score: Vec<f64> = if sample {
            expected_relevance(id, &es, rows, p)
        } else {
            es.iter().map(|e| e.fc).collect()
        };
        let fplace = avg_ranks(&score.iter().map(|x| -x).collect::<Vec<_>>());
        let c = std::f64::consts::LN_10 / p.d;
        let win = |a: f64, b: f64| 1.0 / (1.0 + (c * (b - a)).exp());
        let mut psum = vec![0.0; n];
        let mut pvar = vec![0.0; n];
        for i in 0..n {
            for j in i + 1..n {
                let pij = win(es[i].f, es[j].f);
                psum[i] += pij;
                psum[j] += 1.0 - pij;
                pvar[i] += pij * (1.0 - pij);
                pvar[j] += pij * (1.0 - pij);
            }
        }
        // Actual wins minus expected, over the field. A shared place says nothing
        // about which team is better, so its expectation is added back: a team
        // that finished first then has S >= 0 whether it shared the place or not.
        let mut ord: Vec<usize> = (0..n).collect();
        ord.sort_by(|&a, &b| es[a].place.total_cmp(&es[b].place));
        let mut spair = vec![0.0; n];
        let mut i = 0;
        while i < n {
            let mut j = i;
            while j + 1 < n && es[ord[j + 1]].place == es[ord[i]].place {
                j += 1;
            }
            let above = (n - j - 1) as f64;
            for &a in &ord[i..=j] {
                // Every pair that is not a shared place, by whichever outcome rule
                // is in force; the shipped one is 1 above, 0 below, which is just
                // the count of the teams that finished lower.
                let ysum: f64 = if p.margin_g <= 0.0 {
                    above
                } else {
                    (0..n)
                        .filter(|&b| !ord[i..=j].contains(&b))
                        .map(|b| outcome(&es[a], &es[b], p))
                        .sum()
                };
                let tied: f64 = ord[i..=j]
                    .iter()
                    .filter(|&&b| b != a)
                    .map(|&b| win(es[a].f, es[b].f))
                    .sum();
                let norm = if p.info_norm {
                    (4.0 * pvar[a]).max(p.info_min)
                } else {
                    ((n - 1) as f64).max(1.0)
                };
                spair[a] = (ysum - psum[a] + tied) / norm;
            }
            i = j + 1;
        }

        // The team side may weigh the formats differently from the player side.
        let w = match format {
            Format::Sync => p.w_sync.unwrap_or(weight),
            Format::StrictSync => p.w_strict_sync.unwrap_or(weight),
            _ => weight,
        };
        // A field with strong teams at the top moves ratings more.
        let w = w * field_factor(rows.iter().map(|r| r.q), format, p);
        // The field's first place, for the floor of principle 2.
        let best = es
            .iter()
            .filter(|e| e.place.is_finite())
            .map(|e| e.place)
            .fold(f64::INFINITY, f64::min);
        let mut out = Vec::with_capacity(n);
        for (k, e) in es.iter().enumerate() {
            let (mut d_place, mut d_pull) = (f64::NAN, f64::NAN);
            // Under `two_part`, the roster's share of the move: the old core at
            // today's player ratings, then the attendance update.
            let (mut d_drift, mut d_attend) = (0.0, 0.0);
            let mut gap = f64::NAN;
            if e.rated {
                let before = self.r[e.idx];
                if e.q.is_finite() && !self.core[e.idx].is_empty() {
                    gap = e.q - self.core_q(e.idx, p);
                }
                // A provisional team moves faster: K_n = K max(1, n0 / n).
                let kn = p.k * (p.n0 / e.games.max(1.0)).max(1.0);
                d_place = kn * w * p.d * spair[k];
                if p.place_sign {
                    d_place = if rows[e.ri].position <= fplace[k] {
                        d_place.max(0.0)
                    } else {
                        d_place.min(0.0)
                    };
                }
                self.r[e.idx] += d_place;
                if p.two_part {
                    // The team effect takes the placement step and nothing else;
                    // the rating is rebuilt from it and the core.
                    let q_prev = before - self.b[e.idx];
                    self.b[e.idx] += d_place;
                    self.b[e.idx] *= 1.0 - p.k_b * w;
                    let q0 = if self.core[e.idx].is_empty() {
                        q_prev
                    } else {
                        self.core_q(e.idx, p)
                    };
                    if e.q.is_finite() {
                        let (m, bf) = (rows[e.ri].members.clone(), rows[e.ri].member_base.clone());
                        self.attend(e.idx, &m, &bf, p.k_c);
                    }
                    if !self.core[e.idx].is_empty() {
                        d_drift = q0 - q_prev;
                        d_attend = self.core_q(e.idx, p) - q0;
                    }
                    if !self.core[e.idx].is_empty() {
                        self.r[e.idx] = self.core_q(e.idx, p) + self.b[e.idx];
                    }
                    if p.winner_floor && e.place == best && self.r[e.idx] < before {
                        self.b[e.idx] += before - self.r[e.idx];
                        self.r[e.idx] = before;
                    }
                    d_pull = self.r[e.idx] - before - d_place;
                }
                // The pull towards what the players who turned up are worth.
                if !p.two_part {
                    d_pull = 0.0;
                    if p.k_r > 0.0 {
                        let target = match p.k_r_target {
                            PullTarget::Lineup => e.q,
                            PullTarget::Usual => e.qbar,
                        };
                        let kr = match format {
                            Format::InPerson => p.k_r,
                            _ => p.k_r * p.k_r_sync,
                        };
                        if target.is_finite() {
                            d_pull = kr * w * (target - self.r[e.idx]);
                            self.r[e.idx] += d_pull;
                        }
                    }
                    if p.winner_floor && e.place == best && self.r[e.idx] < before {
                        self.r[e.idx] = before;
                    }
                }
                if e.q.is_finite() {
                    self.qbar[e.idx] += p.k_q * (e.q - self.qbar[e.idx]);
                    if !p.two_part {
                        let (m, bf) = (rows[e.ri].members.clone(), rows[e.ri].member_base.clone());
                        self.attend(e.idx, &m, &bf, p.k_c);
                    }
                }
                self.games[e.idx] += 1.0;
                self.last[e.idx] = day;
                // Today's surprise joins the memory only after today's forecast
                // has been made, so a result never predicts itself.
                if p.kappa_s != 0.0 && format == Format::InPerson && p.series_k > 0 {
                    let s = (fplace[k] - rows[e.ri].position) / (n as f64);
                    let v = &mut self.surprise[e.idx];
                    v.push(s);
                    if v.len() > p.series_k {
                        v.remove(0);
                    }
                }
            }
            self.guest_gap.push(gap);
            out.push(TeamEvent {
                d_place,
                d_pull,
                team: e.idx as u32,
                c: e.rated as u8 as f64,
                delta_l: if e.mu_before.is_finite() && e.q.is_finite() {
                    e.q - e.qbar
                } else {
                    0.0
                },
                x: e.f,
                // Under a sampled rule this column is the expected relevance,
                // not display points: it is what the order was made from, and
                // the metric reads the order alone.
                forecast: score[k],
                forecast_place: fplace[k],
                expected_place: n as f64 - psum[k],
                actual_place: rows[e.ri].position,
                pred_place: rows[e.ri].pred_pos,
                s_r: e.q,
                s_pair: spair[k],
                i_pair: 1.0,
                mu_before: e.mu_before,
                mu_after: self.r[e.idx],
                sigma_before: sigma(e.games),
                sigma_after: sigma(self.games[e.idx]),
                evidence: e.games,
                gap_days: e.gap_days,
                // S2 has no variance; under `two_part` these two carry the
                // roster's share of the move, for the sweep's diagnostics.
                var_diff: d_drift,
                lost_prev: d_attend,
                rated: e.rated,
                z: None,
            });
        }
        out
    }
}
