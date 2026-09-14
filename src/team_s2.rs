//! Model S2, the shipped team engine of DESIGN 4: Elo in a field, kept on the
//! published scale. `src/bin/simple.rs` is the same rules as a research tool.

use crate::config::{Config, S2Lineup, S2};
use crate::data::Tournament;
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
    mu_before: f64,
    games: f64,
    gap_days: i32,
    rated: bool,
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
        let p = &cfg.team.s2;
        let vbar = players.editor_mean(t);
        let mut es: Vec<Entry> = Vec::with_capacity(t.teams.len());
        for (ri, res) in t.teams.iter().enumerate() {
            let idx = res.team as usize;
            let rs: Vec<f64> = res
                .members
                .iter()
                .map(|&i| {
                    let i = i as usize;
                    // `sigma_c > 0` reads each player at a pessimistic quantile,
                    // so an unknown in the lineup lowers the forecast.
                    let pess = p.sigma_c * players.sigma_at(i, t.day, cfg);
                    player_rating(players.theta(i) + players.affinity(i, &vbar) - pess, cfg)
                })
                .collect();
            let known = self.r[idx].is_finite();
            // A team with neither a lineup nor a rating cannot be placed in the field.
            let Some(q) = (!rs.is_empty())
                .then(|| lineup_strength(&rs, p))
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
                mu_before: self.r[idx],
                games: self.games[idx],
                gap_days: (self.last[idx] > i32::MIN / 2)
                    .then(|| t.day - self.last[idx])
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
                let tied: f64 = ord[i..=j]
                    .iter()
                    .filter(|&&b| b != a)
                    .map(|&b| win(es[a].f, es[b].f))
                    .sum();
                spair[a] = (above - psum[a] + tied) / ((n - 1) as f64).max(1.0);
            }
            i = j + 1;
        }

        let mut out = Vec::with_capacity(n);
        for (k, e) in es.iter().enumerate() {
            if e.rated {
                self.r[e.idx] += p.k * t.weight * p.d * spair[k];
                if e.q.is_finite() {
                    self.qbar[e.idx] += p.k_q * (e.q - self.qbar[e.idx]);
                }
                self.games[e.idx] += 1.0;
                self.last[e.idx] = t.day;
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
                actual_place: t.teams[e.ri].position,
                pred_place: t.teams[e.ri].pred_pos,
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
