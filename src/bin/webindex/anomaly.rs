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
//! statistics attributed to everyone at the table, and both are noisy. The
//! statistics themselves live in `chgk_rating::anomaly`, shared with the report.

use chgk_rating::anomaly::{self as stat, Acc, Prior};
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, HashMap};

/// Minimum online results before a player is scored at all.
const MIN_ONLINE: f64 = 8.0;
/// Room for the players and results of the whole history, to spare rehashing.
const PLAYERS_CAP: usize = 60_000;
const RESULTS_CAP: usize = 500_000;
/// Forecast cells: eight bands of where in the field the forecast put a team.
const BANDS: f64 = 8.0;
const LAST_BAND: u8 = 7;
/// A cell with fewer results than this cannot estimate its own spread.
const THIN_CELL: f64 = 50.0;
/// A cell's spread is never taken below this share of everyone's.
const CELL_SD_FLOOR: f64 = 0.2;

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

/// A player's two gaps, each as (gap, variance), standardised.
type Scored = (i64, (f64, f64), (f64, f64));

/// `online` says, for every rated tournament, whether it could be played from
/// home. Returns the scored players, best score first, a tie by id.
pub fn build(r: &Connection, b: &Connection, w: &Connection, online: &HashMap<i64, bool>) -> Vec<Row> {
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
    let roster = stat::roster(w, "select player_id, tournament_id, team_id from player_game");
    let mut players: HashMap<i64, Player> = HashMap::with_capacity(PLAYERS_CAP);
    attribute(&forecast(r, online), &roster, &mut players, |p, on| {
        if on { (&mut p.f_on, &mut p.zf_on) } else { (&mut p.f_ip, &mut p.zf_ip) }
    });
    attribute(&masks(b, online), &roster, &mut players, |p, on| {
        if on { (&mut p.m_on, &mut p.zm_on) } else { (&mut p.m_ip, &mut p.zm_ip) }
    });
    players.into_iter().collect()
}

/// The players with enough online results to score, with both gaps.
fn scored(players: &BTreeMap<i64, Player>) -> Vec<Scored> {
    let baseline_f = stat::baseline(players.values().map(|p| &p.zf_ip));
    let baseline_m = stat::baseline(players.values().map(|p| &p.zm_ip));
    players
        .iter()
        .filter(|(_, p)| p.f_on.n >= MIN_ONLINE || p.m_on.n >= MIN_ONLINE)
        .map(|(id, p)| (*id, stat::gap(&p.zf_on, &p.zf_ip, baseline_f), stat::gap(&p.zm_on, &p.zm_ip, baseline_m)))
        .collect()
}

fn row(p: &Player, &(id, f, m): &Scored, prior_f: &Prior, prior_m: &Prior) -> Row {
    Row {
        player_id: id,
        k: prior_f.score(f) + prior_m.score(m),
        k_f: prior_f.score(f),
        k_m: prior_m.score(m),
        f_gap: stat::shown_gap(&p.f_on, &p.f_ip),
        f_on: p.f_on.n as i64,
        f_ip: p.f_ip.n as i64,
        m_gap: stat::shown_gap(&p.m_on, &p.m_ip),
        m_on: p.m_on.n as i64,
        m_ip: p.m_ip.n as i64,
    }
}

/// How far each team finished above its own forecast place. The cell is whether
/// the result counted for the team's rating: an unrated lineup is forecast from
/// its members alone and misses by more, in both directions.
fn forecast(r: &Connection, online: &HashMap<i64, bool>) -> Vec<Marked> {
    let mut field: HashMap<i64, f64> = HashMap::new();
    crate::data::each(r, "select tournament_id, count(*) as n from team_events group by tournament_id", |x| {
        field.insert(x.get("tournament_id").unwrap(), x.get::<_, i64>("n").unwrap() as f64);
    });
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
    if n <= 1.0 {
        return None;
    }
    let value = stat::surprise(f, a)?;
    // A team forecast last has the whole table to climb and a team forecast
    // first has none, so the cell is where in the field the forecast put it.
    // Without this the score measures playing in weak fields.
    let band = ((f / n * BANDS) as u8).min(LAST_BAND);
    let rated = x.get::<_, Option<i64>>("rated").unwrap().unwrap_or(1) != 0;
    let key = (tid, x.get("team_id").unwrap());
    Some(Marked { key, online, cell: band * 2 + rated as u8, value })
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
    let edges = stat::mask_edges(&stat::read_masks(b, online));
    let cell = |easy: f64| (stat::edge_bin(easy) - stat::EDGE_LO) as u8;
    let mark = |e: stat::MaskEdge| Marked { key: (e.tid, e.team), online: online[&e.tid], cell: cell(e.easy), value: e.hard };
    edges.into_iter().map(mark).collect()
}

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
    stat::attribute(roster, &standardised(marked), |pid, &(online, v, z)| {
        let (raw, std) = pick(players.entry(pid).or_default(), online);
        raw.push(v);
        std.push(z);
    });
}

/// Each result as (online, centred, standardised).
fn standardised(marked: &[Marked]) -> HashMap<(i64, i64), (bool, f64, f64)> {
    let items = || marked.iter().map(|m| ((m.online, m.cell), m.value));
    let cells = stat::cell_means(items());
    let overall = stat::centred_sd(items(), &cells);
    let one = |m: &Marked| {
        let c = &cells[&(m.online, m.cell)];
        // A thin cell cannot estimate its own spread; use everyone's.
        let sd = if c.n >= THIN_CELL { c.sd().max(overall * CELL_SD_FLOOR) } else { overall };
        let v = m.value - c.mean();
        (m.key, (m.online, v, v / sd))
    };
    marked.iter().map(one).collect()
}

pub fn write(w: &Connection, rows: &[Row]) {
    let mut ins = w.prepare("insert into anomaly values (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)").unwrap();
    for (i, x) in rows.iter().enumerate() {
        let rank = i as i64 + 1;
        ins.execute(params![rank, x.player_id, x.k, x.k_f, x.k_m, x.f_gap, x.f_on, x.f_ip, x.m_gap, x.m_on, x.m_ip])
            .unwrap();
    }
}
