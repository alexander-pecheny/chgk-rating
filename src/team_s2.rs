//! Model S2, the shipped team engine of DESIGN 4: Elo in a field, kept on the
//! published scale. `src/bin/simple.rs` is the same rules as a research tool.

use crate::config::{Config, MarginShape, PullTarget, S2Lineup, S2};
use crate::data::{Format, Tournament};
use crate::question_model::Players;
use crate::team_model::TeamEvent;
use crate::util::avg_ranks;

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

/// One result as the team engine sees it: no players left, only the number the
/// lineup that turned up is worth. `src/bin/team_sweep.rs` replays these.
#[derive(Clone, Copy)]
pub struct Row {
    pub team: u32,
    pub position: f64,
    /// Questions taken, for the margin rule; `None` when the mirror has none.
    pub score: Option<f64>,
    pub pred_pos: Option<f64>,
    pub rated: bool,
    /// Strength of the lineup that turned up; NaN when no lineup was listed.
    pub q: f64,
}

/// Everything the team rules of a tournament read off the player states, which
/// is one number per result. `team_sweep` computes these once and replays them.
pub fn rows_of(t: &Tournament, players: &Players, cfg: &Config) -> Vec<Row> {
    let p = &cfg.team.s2;
    let vbar = players.editor_mean(t);
    t.teams
        .iter()
        .map(|res| {
            let rs: Vec<f64> = res
                .members
                .iter()
                .map(|&i| {
                    let i = i as usize;
                    // `sigma_c > 0` reads each player at a pessimistic quantile,
                    // so an unknown in the lineup lowers the forecast.
                    // `sigma_ip > 0` widens the discount again for a player whose
                    // evidence is mostly not from an in-person table.
                    let ip = players.state[i].games_ip as f64;
                    let pess = p.sigma_c
                        * players.sigma_at(i, t.day, cfg)
                        * (1.0 + p.sigma_ip / (1.0 + ip));
                    player_rating(players.theta(i) + players.affinity(i, &vbar) - pess, cfg)
                })
                .collect();
            Row {
                team: res.team,
                position: res.position,
                score: res.score,
                pred_pos: res.pred_pos,
                rated: res.rated,
                q: if rs.is_empty() {
                    f64::NAN
                } else {
                    lineup_strength(&rs, p)
                },
            }
        })
        .collect()
}

/// Team state of DESIGN 4.2: a rating, the usual lineup it was earned with, and
/// the count and date of the rated games behind it.
pub struct TeamsS2 {
    r: Vec<f64>,
    qbar: Vec<f64>,
    games: Vec<f64>,
    last: Vec<i32>,
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
        }
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
    pub fn step(&mut self, t: &Tournament, players: &Players, cfg: &Config) -> Vec<TeamEvent> {
        let rows = rows_of(t, players, cfg);
        self.step_rows(t.day, t.weight, t.format, &rows, &cfg.team.s2)
    }

    /// The team rules alone, over lineup strengths already computed. `fit` and
    /// the parameter sweep run this same code.
    pub fn step_rows(
        &mut self,
        day: i32,
        weight: f64,
        format: Format,
        rows: &[Row],
        p: &S2,
    ) -> Vec<TeamEvent> {
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
                self.r[idx] + if q.is_nan() { 0.0 } else { q - self.qbar[idx] }
            } else {
                // An unrated result never opens a state; it still stands in the field.
                if res.rated {
                    self.r[idx] = q;
                    self.qbar[idx] = q;
                }
                q
            };
            es.push(Entry {
                ri,
                idx,
                f,
                q,
                fc: if q.is_nan() { f } else { q },
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
        if n == 0 {
            return Vec::new();
        }

        let fplace = avg_ranks(&es.iter().map(|e| -e.fc).collect::<Vec<_>>());
        let c = std::f64::consts::LN_10 / p.d;
        let win = |a: f64, b: f64| 1.0 / (1.0 + (c * (b - a)).exp());
        let mut psum = vec![0.0; n];
        for i in 0..n {
            for j in i + 1..n {
                let pij = win(es[i].f, es[j].f);
                psum[i] += pij;
                psum[j] += 1.0 - pij;
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
                spair[a] = (ysum - psum[a] + tied) / ((n - 1) as f64).max(1.0);
            }
            i = j + 1;
        }

        // The team side may weigh the formats differently from the player side.
        let w = match format {
            Format::Sync => p.w_sync.unwrap_or(weight),
            Format::StrictSync => p.w_strict_sync.unwrap_or(weight),
            _ => weight,
        };
        // The field's first place, for the floor of principle 2.
        let best = es
            .iter()
            .filter(|e| e.place.is_finite())
            .map(|e| e.place)
            .fold(f64::INFINITY, f64::min);
        let mut out = Vec::with_capacity(n);
        for (k, e) in es.iter().enumerate() {
            if e.rated {
                let before = self.r[e.idx];
                // A provisional team moves faster: K_n = K max(1, n0 / n).
                let kn = p.k * (p.n0 / e.games.max(1.0)).max(1.0);
                self.r[e.idx] += kn * w * p.d * spair[k];
                // The pull towards what the players who turned up are worth.
                if p.k_r > 0.0 {
                    let target = match p.k_r_target {
                        PullTarget::Lineup => e.q,
                        PullTarget::Usual => e.qbar,
                    };
                    if target.is_finite() {
                        self.r[e.idx] += p.k_r * w * (target - self.r[e.idx]);
                    }
                }
                if p.winner_floor && e.place == best && self.r[e.idx] < before {
                    self.r[e.idx] = before;
                }
                if e.q.is_finite() {
                    self.qbar[e.idx] += p.k_q * (e.q - self.qbar[e.idx]);
                }
                self.games[e.idx] += 1.0;
                self.last[e.idx] = day;
            }
            out.push(TeamEvent {
                team: e.idx as u32,
                c: e.rated as u8 as f64,
                delta_l: if e.mu_before.is_finite() && e.q.is_finite() {
                    e.q - e.qbar
                } else {
                    0.0
                },
                x: e.f,
                forecast: e.fc,
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
                var_diff: 0.0,
                lost_prev: 0.0,
                rated: e.rated,
                z: None,
            });
        }
        out
    }
}
