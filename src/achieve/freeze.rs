//! The player pass of `fit`, with each result's performance read off the
//! question fit of its own tournament and the fair rule's rating beside it:
//! the values that never change after the day a result was played.

use super::frozen::{Names, Res, Seasons};
use crate::config::Config;
use crate::data::{Arena, Format, Tournament, BASE_ROSTERS, CORRECT, SKIP};
use crate::pass::{self, Evening, Morning, Observer};
use crate::question_model as qm;
use crate::team_ndcg as metric;
use crate::team_s2::{lineup_strength, player_rating};
use crate::util::parse_date;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

/// The bisection for a place performance: how far past the field it looks,
/// in steps of the logistic scale, and the precision it stops at.
const PLACE_REACH: f64 = 8.0;
const PLACE_TOL: f64 = 0.5;
/// The own lineup is beaten half the time.
const HALF: f64 = 0.5;
/// A score performance is the theta of six equal players; the clean sheet is
/// kept half a question inside both ends, the exponent capped, and theta
/// bisected over a bracket in a fixed number of steps.
const SEATS: f64 = 6.0;
const CLEAN_SHEET_MARGIN: f64 = 0.5;
const EXP_CAP: f64 = 30.0;
const THETA_BRACKET: (f64, f64) = (-20.0, 20.0);
const THETA_STEPS: usize = 60;
/// A base roster is read when this many of its players are known, over its six
/// strongest; the reference strength is the season's 20 strongest rosters.
const MIN_ROSTER: usize = 4;
const LINEUP: usize = 6;
const REF_ROSTERS: usize = 20;
/// Progress on stderr every so many tournaments.
const PROGRESS_EVERY: usize = 2000;
/// The fit whose last player ratings weigh the rosters, and the town or
/// country of a result where unknown.
const RATINGS_DB: &str = "out/ratings.db";
const UNKNOWN: i64 = -1;
/// The pass reports a place against a lineup not counted twice: weight 1.
const OWN_WEIGHT: f64 = 1.0;

/// The strength at which a team's place is what the field expects: the root of
/// `sum_j P(x beats f_j) + m P(x beats own) = wins + m/2`, by bisection. The
/// same function as the `best_n` rule in `team_s2`.
fn place_perf(own: f64, others: &[f64], wins: f64, m: f64, c: f64) -> f64 {
    let g = |x: f64| {
        others.iter().map(|&f| 1.0 / (1.0 + (c * (f - x)).exp())).sum::<f64>() + m / (1.0 + (c * (own - x)).exp())
            - wins
            - m * HALF
    };
    let lo = others.iter().copied().fold(own, f64::min) - PLACE_REACH / c;
    let hi = others.iter().copied().fold(own, f64::max) + PLACE_REACH / c;
    bisect(lo, hi, |mid| g(mid) < 0.0, |lo, hi, _| hi - lo > PLACE_TOL)
}

/// The root between `lo` and `hi`, `below(x)` true left of it, halving while
/// `go(lo, hi, steps done)`.
fn bisect(mut lo: f64, mut hi: f64, below: impl Fn(f64) -> bool, go: impl Fn(f64, f64, usize) -> bool) -> f64 {
    let mut i = 0;
    while go(lo, hi, i) {
        let mid = lo.midpoint(hi);
        if below(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
        i += 1;
    }
    lo.midpoint(hi)
}

/// The theta at which six equal players expect to take `k` of the package's
/// questions, `k` kept half a question inside both ends so a clean sheet stays
/// finite.
fn score_perf(mask: &[u8], params: &[qm::QParams]) -> (f64, f64, f64) {
    let qs: Vec<usize> = mask.iter().enumerate().filter(|(q, &y)| y != SKIP && *q < params.len()).map(|(q, _)| q).collect();
    let n = qs.len() as f64;
    let k = qs.iter().filter(|&&q| mask[q] == CORRECT).count() as f64;
    if n < 1.0 {
        return (f64::NAN, k, n);
    }
    let target = k.clamp(CLEAN_SHEET_MARGIN, n - CLEAN_SHEET_MARGIN);
    let taken = |th: f64| {
        let p = |q: &usize| &params[*q];
        qs.iter().map(|q| (-(-SEATS * (p(q).a() * (th - p(q).b)).min(EXP_CAP).exp()).exp_m1()).powf(p(q).m())).sum::<f64>()
    };
    let th = bisect(THETA_BRACKET.0, THETA_BRACKET.1, |mid| taken(mid) < target, |_, _, i| i < THETA_STEPS);
    (th, k, n)
}

fn fmt_char(f: Format) -> char {
    match f {
        Format::InPerson => 'i',
        Format::Sync => 's',
        Format::StrictSync => 'S',
        Format::Async => 'a',
    }
}

/// The combined strength of the 20 strongest base rosters of the season on
/// `day`, on the published scale, each roster read over its six strongest
/// known players the way a lineup is: in-person strength when `ip`, the plain
/// rating otherwise.
fn ref20(rosters: &[Vec<u32>], players: &qm::Players, day: i32, ip: bool, cfg: &Config) -> f64 {
    let p = &cfg.team.s2;
    let rating = |i: usize| {
        let th = players.theta(i) + if ip { players.phi(i) } else { 0.0 };
        player_rating(th - p.sigma_c * players.sigma_at(i, day, cfg), cfg)
    };
    let strength = |r: &Vec<u32>| {
        let mut rs: Vec<f64> = r.iter().map(|&i| i as usize).filter(|&i| players.state[i].seen).map(rating).collect();
        if rs.len() < MIN_ROSTER {
            return None;
        }
        rs.sort_by(|a, b| b.total_cmp(a));
        rs.truncate(LINEUP);
        Some((cfg.display.a_team + cfg.display.b_team * lineup_strength(&rs, p)).max(0.0))
    };
    let mut qs: Vec<f64> = rosters.iter().filter_map(strength).collect();
    qs.sort_by(|a, b| b.total_cmp(a));
    qs.iter().take(REF_ROSTERS).sum()
}

/// Every result of the arena with its frozen values.
fn build(arena: &Arena, cfg: &Config, seasons: &Seasons, geo: &HashMap<String, (i64, i64)>) -> pass::Result<Vec<Res>> {
    let mut b = Build {
        arena,
        cfg,
        seasons,
        by_season: rosters_by_season(arena, seasons),
        // Each town of the arena as (town id, country id), -1 where unknown.
        town_geo: arena.towns.iter().map(|n| geo.get(n).copied().unwrap_or((UNKNOWN, UNKNOWN))).collect(),
        refs: None,
        qual: metric::qualify(metric::QUALIFY),
        tw: metric::weights(metric::WEIGHTS),
        out: Vec::new(),
        start: std::time::Instant::now(),
    };
    pass::run(arena, cfg, pass::Options::default(), &mut b)?;
    Ok(b.out)
}

/// Every season's base rosters as player indices of this arena.
fn rosters_by_season(arena: &Arena, seasons: &Seasons) -> HashMap<i64, Vec<Vec<u32>>> {
    let pidx: HashMap<i64, u32> = arena.player_ids.iter().enumerate().map(|(i, &p)| (p, i as u32)).collect();
    let mut by_season: HashMap<i64, Vec<Vec<u32>>> = HashMap::new();
    for ((_, s), v) in &seasons.players {
        let r: Vec<u32> = v.iter().filter_map(|p| pidx.get(p).copied()).collect();
        if r.len() >= MIN_ROSTER {
            by_season.entry(*s).or_default().push(r);
        }
    }
    by_season
}

struct Build<'a> {
    arena: &'a Arena,
    cfg: &'a Config,
    seasons: &'a Seasons,
    by_season: HashMap<i64, Vec<Vec<u32>>>,
    town_geo: Vec<(i64, i64)>,
    /// The reference strengths of the week, in person and not: recomputed once
    /// a week, like a release.
    refs: Option<(i32, f64, f64)>,
    qual: HashSet<(i64, i64)>,
    tw: HashMap<i64, f64>,
    out: Vec<Res>,
    start: std::time::Instant,
}

impl Build<'_> {
    fn refs_for(&mut self, m: &Morning) -> (f64, f64) {
        let week = m.next_release;
        if self.refs.map_or(true, |r| r.0 != week) {
            let season = self.seasons.season_of(m.t.day).unwrap_or(UNKNOWN);
            let rs = self.by_season.get(&season).map_or(&[][..], |v| &v[..]);
            let ip = ref20(rs, m.players, m.t.day, true, self.cfg);
            let sync = ref20(rs, m.players, m.t.day, false, self.cfg);
            self.refs = Some((week, ip, sync));
        }
        let (_, ip, sync) = self.refs.unwrap_or_default();
        (ip, sync)
    }

    fn disp(&self, x: f64) -> f64 {
        self.cfg.display.a_team + self.cfg.display.b_team * x
    }

    /// The score performance on the lineup scale, and the questions behind it.
    fn perf_s(&self, r: &crate::data::TeamResult, m: &Morning) -> (f64, f64, f64) {
        let s2 = &self.cfg.team.s2;
        let (th, taken, n_q) = score_perf(&r.mask, m.params);
        if !th.is_finite() {
            return (f64::NAN, taken, n_q);
        }
        (s2.player_zero + s2.player_scale * (th + SEATS.ln() + self.cfg.team.roster_bias), taken, n_q)
    }
}

/// What one morning knows about the field: each lineup's strength and each
/// team's fair rating, the placed field, the reference strength of the
/// format, and the logistic scale.
struct Day<'a> {
    t: &'a Tournament,
    mu: HashMap<u32, f64>,
    q: HashMap<u32, f64>,
    field: PlaceField,
    ref20: f64,
    c: f64,
}

impl Observer for Build<'_> {
    fn morning(&mut self, m: &Morning) -> pass::Result<()> {
        if m.t.train_only {
            return Ok(());
        }
        let (ref_ip, ref_sync) = self.refs_for(m);
        let q: HashMap<u32, f64> = m.rows.iter().map(|r| (r.team, r.q)).collect();
        let day = Day {
            t: m.t,
            mu: m.events.iter().map(|e| (e.team, e.mu_before)).collect(),
            field: PlaceField::new(m.t, &q),
            q,
            ref20: if m.t.format == Format::InPerson { ref_ip } else { ref_sync },
            c: std::f64::consts::LN_10 / self.cfg.team.s2.d,
        };
        for r in &m.t.teams {
            let res = self.result(&day, r, m);
            self.out.push(res);
        }
        Ok(())
    }

    fn evening(&mut self, e: &Evening) -> pass::Result<()> {
        if e.index % PROGRESS_EVERY == 0 {
            eprintln!("{}/{} {:.0}s", e.index, self.arena.tournaments.len(), self.start.elapsed().as_secs_f64());
        }
        Ok(())
    }
}

impl Build<'_> {
    fn result(&self, d: &Day, r: &crate::data::TeamResult, m: &Morning) -> Res {
        let t = d.t;
        let qi = d.q.get(&r.team).copied().unwrap_or(f64::NAN);
        let (perf_s, taken, n_q) = self.perf_s(r, m);
        let team = self.arena.team_ids[r.team as usize];
        let geo = self.town_geo.get(r.town as usize);
        Res {
            tid: t.id,
            day: t.day,
            format: fmt_char(t.format),
            team,
            place: r.position,
            rated: r.rated,
            n: r.members.len() as u32,
            n_base: r.base.iter().filter(|&&b| b).count() as u32,
            q: self.disp(qi),
            perf_s: self.disp(perf_s),
            perf_p: self.disp(d.field.perf(r.position, qi, d.c)),
            taken,
            n_q,
            fair_mu: d.mu.get(&r.team).map_or(f64::NAN, |&m| self.disp(m)),
            venue: r.venue,
            vplace: r.venue_position,
            qual: self.qual.contains(&(t.id, team)),
            tw: self.tw.get(&t.id).copied().unwrap_or(0.0),
            ref20: d.ref20,
            town: geo.map_or(UNKNOWN, |g| g.0),
            country: geo.map_or(UNKNOWN, |g| g.1),
        }
    }
}

/// The placed results of one field as (place, lineup strength), and the median
/// known strength, which stands in for a lineup nobody listed.
struct PlaceField {
    placed: Vec<(f64, f64)>,
    mid: f64,
}

impl PlaceField {
    fn new(t: &Tournament, q: &HashMap<u32, f64>) -> PlaceField {
        let placed: Vec<(f64, f64)> = t
            .teams
            .iter()
            .filter(|r| r.position > 0.0)
            .map(|r| (r.position, q.get(&r.team).copied().unwrap_or(f64::NAN)))
            .collect();
        let mut known: Vec<f64> = placed.iter().map(|x| x.1).filter(|x| x.is_finite()).collect();
        known.sort_by(|a, b| a.total_cmp(b));
        let mid = known.get(known.len() / 2).copied().unwrap_or(0.0);
        PlaceField { placed, mid }
    }

    /// The place performance of the result at `place` with lineup strength
    /// `qi`: it beats every known lineup placed below it and half of those
    /// level with it, and does not count itself.
    fn perf(&self, place: f64, qi: f64, c: f64) -> f64 {
        if place <= 0.0 {
            return f64::NAN;
        }
        let (others, wins) = self.against(place, qi);
        if others.is_empty() {
            return f64::NAN;
        }
        let own = if qi.is_finite() { qi } else { self.mid };
        place_perf(own, &others, wins, OWN_WEIGHT, c)
    }

    /// The known lineups of the field but this result's own, and the wins over them.
    fn against(&self, place: f64, qi: f64) -> (Vec<f64>, f64) {
        let mut others = Vec::with_capacity(self.placed.len());
        let mut wins = 0.0;
        let mut seen_self = false;
        for &(pos, fq) in &self.placed {
            if !seen_self && pos == place && (fq == qi || (fq.is_nan() && qi.is_nan())) {
                seen_self = true;
                continue;
            }
            if !fq.is_finite() {
                continue;
            }
            others.push(fq);
            wins += match pos.total_cmp(&place) {
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Equal => 0.5,
                std::cmp::Ordering::Less => 0.0,
            };
        }
        (others, wins)
    }
}

// ------------------------------------------------------------------ the lists

fn names(conn: &Connection, sql: &str) -> HashMap<i64, String> {
    let mut st = conn.prepare(sql).unwrap();
    let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).unwrap();
    rows.filter_map(|x| x.ok()).collect()
}

/// Seasons and every base roster, from the chgk.gg history and `buff.db`.
fn seasons(conn: &Connection) -> Seasons {
    let mut st = conn.prepare("SELECT id, substr(date_start,1,10), substr(date_end,1,10) FROM seasons").unwrap();
    let list = st
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
        .unwrap()
        .filter_map(|x| x.ok())
        .filter_map(|(i, a, b)| Some((i, parse_date(&a)?, parse_date(&b)?)))
        .collect();
    let mut s = Seasons { list, has: HashSet::new(), players: base_rosters(conn), strength: HashMap::new() };
    s.dedup();
    s.has = s.players.keys().copied().collect();
    s.strength = player_strengths();
    eprintln!("player strengths: {}", s.strength.len());
    s
}

/// Every (team, season) base roster: the chgk.gg file, then `buff.db`.
fn base_rosters(conn: &Connection) -> HashMap<(i64, i64), Vec<i64>> {
    let mut players: HashMap<(i64, i64), Vec<i64>> = HashMap::new();
    for l in std::fs::read_to_string(BASE_ROSTERS).unwrap_or_default().lines().skip(1) {
        let f: Vec<i64> = l.split(',').map_while(|x| x.parse().ok()).collect();
        if let [t, pl, s, ..] = f[..] {
            players.entry((t, s)).or_default().push(pl);
        }
    }
    let mut st = conn.prepare("SELECT team_id, season_id, player_id FROM team_seasons WHERE player_id > 0").unwrap();
    let rows: Vec<(i64, i64, i64)> =
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().filter_map(|x| x.ok()).collect();
    for (t, s, pl) in rows {
        players.entry((t, s)).or_default().push(pl);
    }
    players
}

/// Each player's last published rating, from the fit the site serves.
fn player_strengths() -> HashMap<i64, f64> {
    Connection::open(RATINGS_DB)
        .and_then(|c| {
            c.prepare("SELECT player_id, display, max(release_id) FROM player_ratings GROUP BY player_id")?
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))?
                .collect::<Result<_, _>>()
        })
        .unwrap_or_default()
}

/// Run the player pass over `cfg`'s arena and write `dir/results.tsv`.
pub fn freeze(cfg: &Config, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let arena = crate::data::load(cfg, &cfg.data.from, &cfg.data.to)?;
    eprintln!("loaded {} tournaments, {} teams", arena.tournaments.len(), arena.n_teams());
    let conn = Connection::open(&cfg.data.db_path)?;
    // Town name to (town id, country id): results carry the name only.
    let geo: HashMap<String, (i64, i64)> = conn
        .prepare("SELECT name, min(id), coalesce(max(country_id), -1) FROM towns GROUP BY name")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))))?
        .collect::<Result<_, _>>()?;
    let rs = build(&arena, cfg, &seasons(&conn), &geo)?;
    let path = format!("{dir}/results.tsv");
    super::frozen::save_results(&rs, &path)?;
    eprintln!("wrote {path}: {} results", rs.len());
    Ok(())
}

/// Write `dir/names.tsv` and `dir/rosters.tsv` from `buff.db`.
pub fn save_lists(cfg: &Config, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::open(&cfg.data.db_path)?;
    let n = Names { teams: names(&conn, "SELECT id, name FROM teams"), tours: names(&conn, "SELECT id, name FROM tournaments") };
    n.save(&format!("{dir}/names.tsv"))?;
    seasons(&conn).save(&format!("{dir}/rosters.tsv"))?;
    Ok(())
}
