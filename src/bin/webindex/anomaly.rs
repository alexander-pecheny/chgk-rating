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
use std::collections::{BTreeMap, HashMap};

/// Minimum online results before a player is scored at all.
const MIN_ONLINE: f64 = 8.0;
/// Fewer in-person results than this and the comparison is against zero.
const MIN_IN_PERSON: f64 = 5.0;
/// Effective sample size ceiling. A player's results are not independent draws:
/// they are the same handful of teammates against the same circuit, week after
/// week, so the four hundredth result adds almost nothing the fiftieth did not.
/// DESIGN §4.3 caps the pairwise stage the same way and for the same reason.
const NU: f64 = 30.0;
/// The least spread a player with four results or more is credited with.
const SD_FLOOR: f64 = 0.5;

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
    /// Squared standard error of the mean. Results are standardised before they
    /// reach here, so a single one has unit variance and the spread is floored
    /// at half of that: a player with three results cannot look certain.
    fn var_mean(&self) -> f64 {
        if self.n < 1.0 {
            return 1.0;
        }
        let sd = if self.n < 4.0 { 1.0 } else { self.sd().max(SD_FLOOR) };
        sd * sd / self.n.min(NU)
    }
}

/// Per player, each statistic twice: in its own units, which is what the table
/// shows, and standardised, which is what the score is computed from.
#[derive(Default)]
struct Player {
    f_on: Acc,
    f_ip: Acc,
    m_on: Acc,
    m_ip: Acc,
    zf_on: Acc,
    zf_ip: Acc,
    zm_on: Acc,
    zm_ip: Acc,
}

pub struct Row {
    pub player_id: i64,
    pub k: f64,
    /// The two halves of `k`, kept so the table can say which way a row leans.
    pub k_f: f64,
    pub k_m: f64,
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

/// Fewer results in person than this, and a player adds nothing to the spread
/// of in-person means.
const BASELINE_MIN: f64 = 10.0;
/// Floors that keep a variance or a spread off zero.
const TINY: f64 = 1e-9;
/// Room for the players and results of the whole history, to spare rehashing.
const PLAYERS_CAP: usize = 60_000;
const RESULTS_CAP: usize = 500_000;

/// A player's two gaps, each as (gap, variance), standardised.
type Scored = (i64, (f64, f64), (f64, f64));

/// `online` says, for every rated tournament, whether it could be played from
/// home. Returns the scored players, best score first, a tie by id.
pub fn build(
    r: &Connection,
    b: &Connection,
    w: &Connection,
    online: &HashMap<i64, bool>,
) -> Vec<Row> {
    let players = gather(r, b, w, online);
    let scored = scored(&players);
    let prior_f = Prior::fit(&scored.iter().map(|s| s.1).collect::<Vec<_>>());
    let prior_m = Prior::fit(&scored.iter().map(|s| s.2).collect::<Vec<_>>());
    eprintln!("anomaly: {} players scored, {prior_f:?} / {prior_m:?}", scored.len());
    let mut out: Vec<Row> = scored.iter().map(|s| row(&players[&s.0], s, &prior_f, &prior_m)).collect();
    out.sort_by(|a, b| b.k.total_cmp(&a.k).then(a.player_id.cmp(&b.player_id)));
    out
}

/// Both statistics of every result, attributed to the players at the table,
/// in id order so every sum over them comes out the same on every run.
fn gather(r: &Connection, b: &Connection, w: &Connection, online: &HashMap<i64, bool>) -> BTreeMap<i64, Player> {
    let roster = roster(w);
    let mut players: HashMap<i64, Player> = HashMap::with_capacity(PLAYERS_CAP);
    attribute(&forecast(r, online), &roster, &mut players, |p, on| {
        if on { (&mut p.f_on, &mut p.zf_on) } else { (&mut p.f_ip, &mut p.zf_ip) }
    });
    attribute(&masks(b, online), &roster, &mut players, |p, on| {
        if on { (&mut p.m_on, &mut p.zm_on) } else { (&mut p.m_ip, &mut p.zm_ip) }
    });
    players.into_iter().collect()
}

/// How far players' true in-person means spread. A player with no in-person
/// record is compared against zero, and that unknown baseline is worth this
/// much uncertainty — without it, having never played in person would make a
/// player look more certain than one who has.
fn baseline(players: &BTreeMap<i64, Player>, pick: fn(&Player) -> &Acc) -> f64 {
    let mut obs = Acc::default();
    let mut noise = 0.0;
    for ip in players.values().map(pick).filter(|ip| ip.n >= BASELINE_MIN) {
        obs.push(ip.mean());
        noise += ip.var_mean();
    }
    (obs.sd().powi(2) - noise / obs.n.max(1.0)).max(TINY).sqrt()
}

/// Online against in person, or against zero with the baseline's uncertainty
/// when there is too little in person to compare with.
fn gap(a: &Acc, b: &Acc, prior: f64) -> (f64, f64) {
    let compare = b.n >= MIN_IN_PERSON;
    (
        a.mean() - if compare { b.mean() } else { 0.0 },
        a.var_mean() + if compare { b.var_mean() } else { prior * prior },
    )
}

/// The players with enough online results to score, with both gaps.
fn scored(players: &BTreeMap<i64, Player>) -> Vec<Scored> {
    let baseline_f = baseline(players, |p| &p.zf_ip);
    let baseline_m = baseline(players, |p| &p.zm_ip);
    players
        .iter()
        .filter(|(_, p)| p.f_on.n >= MIN_ONLINE || p.m_on.n >= MIN_ONLINE)
        .map(|(id, p)| (*id, gap(&p.zf_on, &p.zf_ip, baseline_f), gap(&p.zm_on, &p.zm_ip, baseline_m)))
        .collect()
}

/// The gap in the statistic's own units, as the table shows it.
fn shown_gap(on: &Acc, ip: &Acc) -> f64 {
    on.mean() - if ip.n >= MIN_IN_PERSON { ip.mean() } else { 0.0 }
}

fn row(p: &Player, &(id, f, m): &Scored, prior_f: &Prior, prior_m: &Prior) -> Row {
    Row {
        player_id: id,
        k: prior_f.score(f) + prior_m.score(m),
        k_f: prior_f.score(f),
        k_m: prior_m.score(m),
        f_gap: shown_gap(&p.f_on, &p.f_ip),
        f_on: p.f_on.n as i64,
        f_ip: p.f_ip.n as i64,
        m_gap: shown_gap(&p.m_on, &p.m_ip),
        m_on: p.m_on.n as i64,
        m_ip: p.m_ip.n as i64,
    }
}

/// Forecast cells: eight bands of where in the field the forecast put a team.
const BANDS: f64 = 8.0;
const LAST_BAND: u8 = 7;

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
    let mut out = Vec::with_capacity(RESULTS_CAP);
    crate::data::each(
        r,
        "select tournament_id, team_id, rated, forecast_place, actual_place from team_events",
        |x| out.extend(forecast_miss(x, &field, online)),
    );
    out
}

fn forecast_miss(x: &rusqlite::Row, field: &HashMap<i64, f64>, online: &HashMap<i64, bool>) -> Option<Marked> {
    let tid: i64 = x.get("tournament_id").unwrap();
    let online = *online.get(&tid)?;
    let f: f64 = x.get::<_, Option<f64>>("forecast_place").unwrap_or(None)?;
    let a: f64 = x.get::<_, Option<f64>>("actual_place").unwrap_or(None)?;
    let n = field.get(&tid).copied().unwrap_or(0.0);
    if !(f > 0.0 && a > 0.0 && n > 1.0) {
        return None;
    }
    // A team forecast last has the whole table to climb and a team forecast
    // first has none, so the cell is where in the field the forecast put it.
    // Without this the score measures playing in weak fields.
    let band = ((f / n * BANDS) as u8).min(LAST_BAND);
    let rated = x.get::<_, Option<i64>>("rated").unwrap().unwrap_or(1) != 0;
    let key = (tid, x.get("team_id").unwrap());
    Some(Marked { key, online, cell: band * 2 + rated as u8, value: f.ln() - a.ln() })
}

/// A mask shorter than this is no package.
const MIN_MASK: usize = 12;
/// Fewer teams than this and the field's take rate is no difficulty scale.
const MIN_TEAMS: usize = 15;
/// A question answered by fewer teams than this has no take rate.
const MIN_ANSWERED: f64 = 10.0;
/// Fewer usable questions than this and the package has no thirds.
const MIN_QUESTIONS: usize = 12;
const THIRDS: usize = 3;
/// A team that saw fewer questions of a third than this has no edge on it.
const MIN_SEEN: f64 = 6.0;
/// Mask cells: the easy-third edge in fiftieths, from −0.70 to +0.30.
const EDGE_STEPS: f64 = 50.0;
const EDGE_LO: i64 = -35;
const EDGE_HI: i64 = 15;

/// Each team's answers, by tournament.
type Masks = BTreeMap<i64, Vec<(i64, Vec<u8>)>>;
/// Per question: teams that took it, teams that answered at all.
type Take = Vec<(f64, f64)>;

/// The team's edge over the field on the hard third of the package, against the
/// edge a team with its easy-third edge normally has.
///
/// Conditioning on the easy third rather than on the overall edge matters. A
/// team whose mask is misaligned with the question order looks flat across
/// difficulty — far below the field on the easy third, level with it on the
/// hard — and against an overall-edge baseline that reads as the loudest signal
/// in the data. Against the easy third it does not.
fn masks(b: &Connection, online: &HashMap<i64, bool>) -> Vec<Marked> {
    let mut out = Vec::with_capacity(RESULTS_CAP);
    for (tid, rows) in &read_masks(b, online) {
        if rows.len() < MIN_TEAMS {
            continue;
        }
        let take = take_rates(rows);
        let Some(order) = by_difficulty(&take) else { continue };
        let third = order.len() / THIRDS;
        for (team, m) in rows {
            let hard = edge(m, &order[..third], &take);
            let easy = edge(m, &order[order.len() - third..], &take);
            if let (Some(hard), Some(easy)) = (hard, easy) {
                out.push(Marked { key: (*tid, *team), online: online[tid], cell: edge_cell(easy), value: hard });
            }
        }
    }
    out
}

fn edge_cell(easy: f64) -> u8 {
    (((easy * EDGE_STEPS).round() as i64).clamp(EDGE_LO, EDGE_HI) - EDGE_LO) as u8
}

fn read_masks(b: &Connection, online: &HashMap<i64, bool>) -> Masks {
    let mut by_tour = Masks::new();
    crate::data::each(b, "select id, team_id, mask from tournament_results where mask is not null", |x| {
        let Ok(tid) = x.get::<_, i64>("id") else { return };
        if !online.contains_key(&tid) {
            return;
        }
        let m: String = x.get::<_, Option<String>>("mask").unwrap().unwrap_or_default();
        if m.len() >= MIN_MASK {
            by_tour.entry(tid).or_default().push((x.get("team_id").unwrap_or(0), m.into_bytes()));
        }
    });
    by_tour
}

fn take_rates(rows: &[(i64, Vec<u8>)]) -> Take {
    let n_q = rows.iter().map(|r| r.1.len()).max().unwrap();
    let mut take = vec![(0.0f64, 0.0f64); n_q];
    for (q, c) in rows.iter().flat_map(|(_, m)| m.iter().enumerate()) {
        match c {
            b'1' => {
                take[q].0 += 1.0;
                take[q].1 += 1.0;
            }
            b'0' => take[q].1 += 1.0,
            _ => {}
        }
    }
    take
}

/// The usable questions, hardest first; nothing when too few are usable.
fn by_difficulty(take: &Take) -> Option<Vec<usize>> {
    let mut order: Vec<usize> = (0..take.len()).filter(|&q| take[q].1 >= MIN_ANSWERED).collect();
    if order.len() < MIN_QUESTIONS {
        return None;
    }
    order.sort_by(|&a, &b| (take[a].0 / take[a].1).total_cmp(&(take[b].0 / take[b].1)));
    Some(order)
}

/// The team's take rate on `qs` less the field's, per question it answered.
fn edge(m: &[u8], qs: &[usize], take: &Take) -> Option<f64> {
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
    (n >= MIN_SEEN).then(|| (me - field) / n)
}

/// A cell with fewer results than this cannot estimate its own spread.
const THIN_CELL: f64 = 50.0;
/// A cell's spread is never taken below this share of everyone's.
const CELL_SD_FLOOR: f64 = 0.2;

/// Centres each result on the mean of results of its own kind and divides by
/// their spread, then hands both the centred and the standardised value to the
/// players who sat at the table.
///
/// The scale matters as much as the centre. Beating a forecast by a factor of
/// two means one thing in a field of thirty and another in a field of two
/// hundred, and without dividing it out the noisy cells drown the rest.
fn attribute(
    marked: &[Marked],
    roster: &[(i64, i64, i64)],
    players: &mut HashMap<i64, Player>,
    pick: fn(&mut Player, bool) -> (&mut Acc, &mut Acc),
) {
    let by_result = standardised(marked);
    for (pid, tid, team) in roster {
        if let Some(&(online, v, z)) = by_result.get(&(*tid, *team)) {
            let (raw, std) = pick(players.entry(*pid).or_default(), online);
            raw.push(v);
            std.push(z);
        }
    }
}

/// Each result as (online, centred, standardised).
fn standardised(marked: &[Marked]) -> HashMap<(i64, i64), (bool, f64, f64)> {
    let mut cells: HashMap<(bool, u8), Acc> = HashMap::new();
    for m in marked {
        cells.entry((m.online, m.cell)).or_default().push(m.value);
    }
    let mut all = Acc::default();
    for m in marked {
        all.push(m.value - cells[&(m.online, m.cell)].mean());
    }
    let overall = all.sd();
    let one = |m: &Marked| {
        let c = &cells[&(m.online, m.cell)];
        // A thin cell cannot estimate its own spread; use everyone's.
        let sd = if c.n >= THIN_CELL { c.sd().max(overall * CELL_SD_FLOOR) } else { overall };
        let v = m.value - c.mean();
        (m.key, (m.online, v, v / sd))
    };
    marked.iter().map(one).collect()
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
    let mut ins = w.prepare("insert into anomaly values (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)").unwrap();
    for (i, x) in rows.iter().enumerate() {
        let rank = i as i64 + 1;
        ins.execute(params![rank, x.player_id, x.k, x.k_f, x.k_m, x.f_gap, x.f_on, x.f_ip, x.m_gap, x.m_on, x.m_ip])
            .unwrap();
    }
}

/// What the population of true gaps looks like: a bulk sitting close to zero
/// and a small share genuinely far from it.
///
/// One normal will not do. Fitted to everybody it takes a spread of about 0.07
/// on the log-place scale, under which a gap of 1.03 is a fourteen-sigma event —
/// so a player with fifteen tournaments and an enormous gap is written off as
/// noise, while a player with seven hundred tournaments and a fifth of the gap
/// is believed. Two components fix that: a result far outside the bulk is
/// evidence of belonging to the small component, not of bad luck, and is barely
/// shrunk at all.
#[derive(Debug)]
struct Prior {
    /// Spread of the bulk.
    spike: f64,
    /// Spread of the rest.
    slab: f64,
    /// Share of players in the second component.
    share: f64,
}

/// Where EM starts: the bulk at half the excess spread, the rest at three
/// times it, holding a twentieth of the players.
const START_SPIKE: f64 = 0.25;
const START_SLAB: f64 = 9.0;
const START_SHARE: f64 = 0.05;
const EM_ROUNDS: usize = 200;
/// The second component's share stays within these.
const SHARE_MIN: f64 = 1e-4;
const SHARE_MAX: f64 = 0.5;
/// Floor of a variance, so a spread never reaches zero.
const VAR_FLOOR: f64 = 1e-12;

/// One EM round's sums: the weight on each component, and the expected
/// squared true gap under each, weighted.
#[derive(Default)]
struct Sums {
    w: f64,
    w0: f64,
    t2: f64,
    s2: f64,
}

impl Prior {
    /// EM over the observed gaps and their variances.
    fn fit(obs: &[(f64, f64)]) -> Prior {
        let n = obs.len() as f64;
        let g2: f64 = obs.iter().map(|(g, _)| g * g).sum::<f64>() / n;
        let v: f64 = obs.iter().map(|(_, v)| v).sum::<f64>() / n;
        let excess = (g2 - v).max(VAR_FLOOR);
        let mut p = Prior { spike: (excess * START_SPIKE).sqrt(), slab: (excess * START_SLAB).sqrt(), share: START_SHARE };
        for _ in 0..EM_ROUNDS {
            p.step(obs, n);
        }
        p
    }

    fn step(&mut self, obs: &[(f64, f64)], n: f64) {
        let s = self.sums(obs);
        self.share = (s.w / n).clamp(SHARE_MIN, SHARE_MAX);
        self.slab = (s.t2 / s.w.max(TINY)).max(VAR_FLOOR).sqrt();
        self.spike = (s.s2 / s.w0.max(TINY)).max(VAR_FLOOR).sqrt();
        if self.slab < self.spike {
            std::mem::swap(&mut self.slab, &mut self.spike);
            self.share = 1.0 - self.share;
        }
    }

    fn sums(&self, obs: &[(f64, f64)]) -> Sums {
        let (s2, t2) = (self.spike * self.spike, self.slab * self.slab);
        let mut s = Sums::default();
        for &(g, v) in obs {
            let w = self.weight(g, v);
            // E[theta^2] under a component of prior variance p2.
            let m = |p2: f64| p2 * v / (p2 + v) + (g * p2 / (p2 + v)).powi(2);
            s.w += w;
            s.w0 += 1.0 - w;
            s.t2 += w * m(t2);
            s.s2 += (1.0 - w) * m(s2);
        }
        s
    }

    /// Posterior probability that this gap belongs to the second component.
    fn weight(&self, g: f64, v: f64) -> f64 {
        const HALF: f64 = 0.5;
        let log_dens = |p2: f64, w: f64| {
            let var = p2 + v;
            w.ln() - HALF * var.ln() - HALF * g * g / var
        };
        let a = log_dens(self.slab * self.slab, self.share);
        let b = log_dens(self.spike * self.spike, 1.0 - self.share);
        1.0 / (1.0 + (b - a).exp())
    }

    /// The posterior mean of the true gap, in units of the second component's
    /// spread so the two statistics can be added.
    fn score(&self, (g, v): (f64, f64)) -> f64 {
        let (s2, t2) = (self.spike * self.spike, self.slab * self.slab);
        let w = self.weight(g, v);
        (w * g * t2 / (t2 + v) + (1.0 - w) * g * s2 / (s2 + v)) / self.slab
    }
}
