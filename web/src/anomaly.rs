//! The anomaly score behind `/anomaly/`.
//!
//! Cheating is close to impossible at an in-person tournament, awkward at a
//! synchronous one and trivial when a synchronous tournament is played at home.
//! So a player whose results depend on the format has a rating their play does
//! not support. Two independent measurements of that, per player:
//!
//! * **forecast** — `ln(forecast place) − ln(actual place)`, how far the team
//!   finished above the place its own lineup's ratings predicted. Available for
//!   every result, including the ones flagged `inRating = 0`, where the forecast
//!   comes from the lineup alone.
//! * **hard questions** — how far the team's take rate sat above the field's on
//!   the third of the package the field found hardest, given where it sat on the
//!   third it found easiest. The easy third is the part a cheat has no reason to
//!   touch, because everybody already knows it. This uses no ratings at all: the
//!   field's own take rate is the difficulty scale.
//!
//! Each is taken as a gap — the player online minus the same player in person,
//! against zero when they have no in-person record — then shrunk towards zero by
//! its own noise, so a large gap measured over eight games does not outrank a
//! smaller one measured over eight hundred. The score is the two shrunk gaps in
//! units of their own spread across players.
//!
//! A high score is a reason to look, not a verdict. Both statistics are team
//! statistics attributed to everyone at the table, and both are noisy.

use rusqlite::{params, Connection};
use std::collections::HashMap;

/// Minimum online results before a player is scored at all.
const MIN_ONLINE: f64 = 8.0;
/// Fewer in-person results than this and the comparison is against zero.
const MIN_IN_PERSON: f64 = 5.0;

#[derive(Default, Clone, Copy)]
struct Acc {
    n: f64,
    sum: f64,
    sum2: f64,
}

impl Acc {
    fn push(&mut self, x: f64) {
        self.n += 1.0;
        self.sum += x;
        self.sum2 += x * x;
    }
    fn mean(&self) -> f64 {
        if self.n > 0.0 {
            self.sum / self.n
        } else {
            0.0
        }
    }
    fn sd(&self) -> f64 {
        if self.n < 2.0 {
            return f64::NAN;
        }
        (self.sum2 / self.n - self.mean() * self.mean()).max(0.0).sqrt()
    }
    /// Squared standard error of the mean, with the spread floored at half the
    /// population's so a player with three results cannot look certain.
    fn var_mean(&self, pop_sd: f64) -> f64 {
        if self.n < 1.0 {
            return pop_sd * pop_sd;
        }
        let sd = if self.n < 4.0 { pop_sd } else { self.sd().max(pop_sd * 0.5) };
        sd * sd / self.n
    }
}

#[derive(Default)]
struct Player {
    f_on: Acc,
    f_ip: Acc,
    m_on: Acc,
    m_ip: Acc,
}

pub struct Row {
    pub player_id: i64,
    pub k: f64,
    pub f_gap: f64,
    pub f_on: i64,
    pub f_ip: i64,
    pub m_gap: f64,
    pub m_on: i64,
    pub m_ip: i64,
}

/// One result, as both statistics see it: who it belongs to, whether it could
/// have been played from home, and the value.
struct Marked {
    key: (i64, i64),
    online: bool,
    /// The cell the value is centred in, beyond the format itself.
    cell: u8,
    value: f64,
}

/// `online` says, for every rated tournament, whether it could be played from
/// home. Returns the scored players, best score first.
pub fn build(
    r: &Connection,
    b: &Connection,
    w: &Connection,
    online: &HashMap<i64, bool>,
) -> Vec<Row> {
    let roster = roster(w);
    let mut players: HashMap<i64, Player> = HashMap::with_capacity(60_000);
    let f_sd = attribute(&forecast(r, online), &roster, &mut players, |p, on| {
        if on { &mut p.f_on } else { &mut p.f_ip }
    });
    let m_sd = attribute(&masks(b, online), &roster, &mut players, |p, on| {
        if on { &mut p.m_on } else { &mut p.m_ip }
    });

    // How far players' true in-person means spread. A player with no in-person
    // record is compared against zero, and that unknown baseline is worth this
    // much uncertainty — without it, having never played in person would make a
    // player look more certain than one who has.
    let prior = |pick: fn(&Player) -> (&Acc, &Acc), pop: f64| -> f64 {
        let mut obs = Acc::default();
        let mut noise = 0.0;
        for p in players.values() {
            let (_, ip) = pick(p);
            if ip.n >= 10.0 {
                obs.push(ip.mean());
                noise += ip.var_mean(pop);
            }
        }
        (obs.sd().powi(2) - noise / obs.n.max(1.0)).max(1e-9).sqrt()
    };
    let prior_f = prior(|p| (&p.f_on, &p.f_ip), f_sd);
    let prior_m = prior(|p| (&p.m_on, &p.m_ip), m_sd);

    let gap = |a: &Acc, b: &Acc, pop: f64, prior: f64| -> (f64, f64) {
        let compare = b.n >= MIN_IN_PERSON;
        (
            a.mean() - if compare { b.mean() } else { 0.0 },
            a.var_mean(pop) + if compare { b.var_mean(pop) } else { prior * prior },
        )
    };
    let scored: Vec<(i64, (f64, f64), (f64, f64))> = players
        .iter()
        .filter(|(_, p)| p.f_on.n >= MIN_ONLINE || p.m_on.n >= MIN_ONLINE)
        .map(|(id, p)| {
            (
                *id,
                gap(&p.f_on, &p.f_ip, f_sd, prior_f),
                gap(&p.m_on, &p.m_ip, m_sd, prior_m),
            )
        })
        .collect();

    // The spread of true gaps across players, backed out of the observed spread
    // by removing the measurement noise. The shrinkage pulls towards zero
    // against it, and dividing by it puts the two components on one scale.
    let tau = |pick: fn(&(i64, (f64, f64), (f64, f64))) -> (f64, f64)| -> f64 {
        let mut obs = Acc::default();
        let mut noise = 0.0;
        for s in &scored {
            obs.push(pick(s).0);
            noise += pick(s).1;
        }
        (obs.sd().powi(2) - noise / obs.n.max(1.0)).max(1e-9).sqrt()
    };
    let (tau_f, tau_m) = (tau(|s| s.1), tau(|s| s.2));
    eprintln!(
        "anomaly: {} players scored, tau {tau_f:.3} / {tau_m:.3}, prior {prior_f:.3} / {prior_m:.3}",
        scored.len()
    );

    let shrink = |(g, v): (f64, f64), tau: f64| g * tau / (tau * tau + v);
    let mut out: Vec<Row> = scored
        .iter()
        .map(|(id, f, m)| {
            let p = &players[id];
            Row {
                player_id: *id,
                k: shrink(*f, tau_f) + shrink(*m, tau_m),
                f_gap: f.0,
                f_on: p.f_on.n as i64,
                f_ip: p.f_ip.n as i64,
                m_gap: m.0,
                m_on: p.m_on.n as i64,
                m_ip: p.m_ip.n as i64,
            }
        })
        .collect();
    out.sort_by(|a, b| b.k.total_cmp(&a.k));
    out
}

/// How far each team finished above its own forecast place. The cell is whether
/// the result counted for the team's rating: an unrated lineup is forecast from
/// its members alone and misses by more, in both directions.
fn forecast(r: &Connection, online: &HashMap<i64, bool>) -> Vec<Marked> {
    let field: HashMap<i64, f64> = {
        let mut st = r
            .prepare("select tournament_id, count(*) from team_events group by tournament_id")
            .unwrap();
        st.query_map([], |x| Ok((x.get(0)?, x.get::<_, i64>(1)? as f64)))
            .unwrap()
            .map(|x| x.unwrap())
            .collect()
    };
    let mut st = r
        .prepare("select tournament_id, team_id, rated, forecast_place, actual_place from team_events")
        .unwrap();
    let mut rows = st.query([]).unwrap();
    let mut out = Vec::with_capacity(500_000);
    while let Some(x) = rows.next().unwrap() {
        let tid: i64 = x.get(0).unwrap();
        let Some(&online) = online.get(&tid) else { continue };
        let (Some(f), Some(a)) = (
            x.get::<_, Option<f64>>(3).unwrap_or(None),
            x.get::<_, Option<f64>>(4).unwrap_or(None),
        ) else {
            continue;
        };
        let n = field.get(&tid).copied().unwrap_or(0.0);
        if !(f > 0.0 && a > 0.0 && n > 1.0) {
            continue;
        }
        // A team forecast last has the whole table to climb and a team forecast
        // first has none, so the cell is where in the field the forecast put it.
        // Without this the score measures playing in weak fields.
        let band = ((f / n * 8.0) as u8).min(7);
        let rated = x.get::<_, Option<i64>>(2).unwrap().unwrap_or(1) != 0;
        out.push(Marked {
            key: (tid, x.get(1).unwrap()),
            online,
            cell: band * 2 + rated as u8,
            value: f.ln() - a.ln(),
        });
    }
    out
}

/// The team's edge over the field on the hard third of the package, against the
/// edge a team with its easy-third edge normally has.
///
/// Conditioning on the easy third rather than on the overall edge matters. A
/// team whose mask is misaligned with the question order looks flat across
/// difficulty — far below the field on the easy third, level with it on the
/// hard — and against an overall-edge baseline that reads as the loudest signal
/// in the data. Against the easy third it does not.
fn masks(b: &Connection, online: &HashMap<i64, bool>) -> Vec<Marked> {
    let mut by_tour: HashMap<i64, Vec<(i64, Vec<u8>)>> = HashMap::with_capacity(online.len());
    {
        let mut st = b
            .prepare("select id, team_id, mask from tournament_results where mask is not null")
            .unwrap();
        let mut rows = st.query([]).unwrap();
        while let Some(x) = rows.next().unwrap() {
            let Ok(tid) = x.get::<_, i64>(0) else { continue };
            if !online.contains_key(&tid) {
                continue;
            }
            let m: String = x.get::<_, Option<String>>(2).unwrap().unwrap_or_default();
            if m.len() < 12 {
                continue;
            }
            by_tour.entry(tid).or_default().push((x.get(1).unwrap_or(0), m.into_bytes()));
        }
    }

    let mut pairs: Vec<((i64, i64), bool, f64, f64)> = Vec::with_capacity(500_000);
    for (tid, rows) in &by_tour {
        if rows.len() < 15 {
            continue;
        }
        let n_q = rows.iter().map(|r| r.1.len()).max().unwrap();
        let mut take = vec![(0.0f64, 0.0f64); n_q];
        for (_, m) in rows {
            for (q, c) in m.iter().enumerate() {
                match c {
                    b'1' => {
                        take[q].0 += 1.0;
                        take[q].1 += 1.0;
                    }
                    b'0' => take[q].1 += 1.0,
                    _ => {}
                }
            }
        }
        let mut order: Vec<usize> = (0..n_q).filter(|&q| take[q].1 >= 10.0).collect();
        if order.len() < 12 {
            continue;
        }
        order.sort_by(|&a, &b| (take[a].0 / take[a].1).total_cmp(&(take[b].0 / take[b].1)));
        let third = order.len() / 3;
        for (team, m) in rows {
            let edge = |qs: &[usize]| -> Option<f64> {
                let (mut me, mut field, mut n) = (0.0, 0.0, 0.0);
                for &q in qs {
                    match m.get(q) {
                        Some(b'1') => me += 1.0,
                        Some(b'0') => {}
                        _ => continue,
                    }
                    field += take[q].0 / take[q].1;
                    n += 1.0;
                }
                (n >= 6.0).then(|| (me - field) / n)
            };
            let (Some(hard), Some(easy)) =
                (edge(&order[..third]), edge(&order[order.len() - third..]))
            else {
                continue;
            };
            pairs.push(((*tid, *team), online[tid], easy, hard));
        }
    }

    let bin = |t: f64| ((t * 50.0).round() as i64).clamp(-35, 15);
    let mut curve: HashMap<i64, Acc> = HashMap::new();
    for p in &pairs {
        curve.entry(bin(p.2)).or_default().push(p.3);
    }
    pairs
        .iter()
        .filter_map(|p| {
            let c = curve.get(&bin(p.2)).filter(|a| a.n >= 30.0)?;
            Some(Marked { key: p.0, online: p.1, cell: 0, value: p.3 - c.mean() })
        })
        .collect()
}

/// Centres each result on the mean of results of the same kind, hands them to
/// the players who sat at the table, and returns the population's spread.
fn attribute(
    marked: &[Marked],
    roster: &[(i64, i64, i64)],
    players: &mut HashMap<i64, Player>,
    pick: fn(&mut Player, bool) -> &mut Acc,
) -> f64 {
    let mut cells: HashMap<(bool, u8), Acc> = HashMap::new();
    for m in marked {
        cells.entry((m.online, m.cell)).or_default().push(m.value);
    }
    let mut spread = Acc::default();
    let by_result: HashMap<(i64, i64), (bool, f64)> = marked
        .iter()
        .map(|m| {
            let v = m.value - cells[&(m.online, m.cell)].mean();
            spread.push(v);
            (m.key, (m.online, v))
        })
        .collect();
    for (pid, tid, team) in roster {
        if let Some(&(online, v)) = by_result.get(&(*tid, *team)) {
            pick(players.entry(*pid).or_default(), online).push(v);
        }
    }
    spread.sd()
}

/// Five million (player, tournament, team) rows, read from SQLite once and used
/// by both statistics.
fn roster(w: &Connection) -> Vec<(i64, i64, i64)> {
    let mut st = w
        .prepare("select player_id, tournament_id, team_id from player_game")
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

pub fn write(w: &Connection, rows: &[Row]) {
    let mut ins = w.prepare("insert into anomaly values (?1,?2,?3,?4,?5,?6,?7,?8,?9)").unwrap();
    for (i, x) in rows.iter().enumerate() {
        ins.execute(params![
            i as i64 + 1,
            x.player_id,
            x.k,
            x.f_gap,
            x.f_on,
            x.f_ip,
            x.m_gap,
            x.m_on,
            x.m_ip
        ])
        .unwrap();
    }
}
