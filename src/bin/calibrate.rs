use chgk_rating::config::Config;
use chgk_rating::data::{Format, TeamResult, Tournament, CORRECT};
use chgk_rating::pass::{Morning, Observer};
use chgk_rating::question_model as qm;
use chgk_rating::team_model::roster_estimate;
use chgk_rating::util::{csv_rows, parse_date};
use rusqlite::{Connection, OpenFlags};

const BINS: usize = 1000;
const HALF: f64 = 10.0;
/// The gap axis runs from -HALF to HALF.
const SPAN: f64 = 2.0 * HALF;
/// Gaps are put on the basis of a 36-question package.
const BASIS_QUESTIONS: f64 = 36.0;
/// A probability is kept this far from 0 and 1 inside a log.
const P_FLOOR: f64 = 1e-12;
/// The ternary search for beta: its bracket, its steps, and the thirds it cuts.
const BETA_BRACKET: (f64, f64) = (0.05, 5.0);
const SEARCH_STEPS: usize = 200;
const THIRDS: f64 = 3.0;
/// The residual is read over full six-player lineups only.
const LINEUP: usize = 6;
/// The default window of the beta fit.
const FROM: &str = "2015-01-01";
const TO: &str = "2025-01-01";

/// DESIGN 5: the affine display map is pinned to two places of the old 2019
/// releases and the display value each had there.
const PLACE_HI: i64 = 50;
const PLACE_LO: i64 = 1000;
const TEAM_AT: (f64, f64) = (7028.0, 2992.0);
const PLAYER_AT: (f64, f64) = (12865.0, 9778.0);
const TEAM_RELEASE: &str = "data/external/team_release_2019.csv";
const PLAYER_RELEASE: &str = "data/external/player_release_1417_old_export.csv";
/// What `--display` checks: label, file, place column, value column, place,
/// the value the map is pinned to.
const DISPLAY_CHECKS: [(&str, &str, usize, usize, i64, f64); 4] = [
    ("team 50", TEAM_RELEASE, 2, 1, PLACE_HI, TEAM_AT.0),
    ("team 1000", TEAM_RELEASE, 2, 1, PLACE_LO, TEAM_AT.1),
    ("player 50", PLAYER_RELEASE, 6, 7, PLACE_HI, PLAYER_AT.0),
    ("player 1000", PLAYER_RELEASE, 6, 7, PLACE_LO, PLAYER_AT.1),
];

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

/// Pairwise wins bucketed by strength gap on a 36-question basis.
struct Hist {
    n: Vec<f64>,
    y: Vec<f64>,
}

impl Hist {
    fn new() -> Hist {
        Hist { n: vec![0.0; BINS], y: vec![0.0; BINS] }
    }
    fn add(&mut self, d: f64, y: f64) {
        let k = (((d + HALF) / SPAN) * BINS as f64) as isize;
        let k = k.clamp(0, BINS as isize - 1) as usize;
        self.n[k] += 1.0;
        self.y[k] += y;
    }
    fn centre(k: usize) -> f64 {
        (k as f64).midpoint(k as f64 + 1.0) / BINS as f64 * SPAN - HALF
    }
    fn loglik(&self, beta: f64) -> f64 {
        let mut v = 0.0;
        for k in (0..BINS).filter(|&k| self.n[k] != 0.0) {
            let p = 1.0 / (1.0 + (-Hist::centre(k) / beta).exp());
            v += self.y[k] * p.max(P_FLOOR).ln() + (self.n[k] - self.y[k]) * (1.0 - p).max(P_FLOOR).ln();
        }
        v
    }
    /// Ternary search on log beta; the likelihood is unimodal in it.
    fn fit(&self) -> f64 {
        let (mut lo, mut hi) = (BETA_BRACKET.0.ln(), BETA_BRACKET.1.ln());
        for _ in 0..SEARCH_STEPS {
            let (a, b) = (lo + (hi - lo) / THIRDS, hi - (hi - lo) / THIRDS);
            if self.loglik(a.exp()) < self.loglik(b.exp()) {
                lo = a;
            } else {
                hi = b;
            }
        }
        lo.midpoint(hi).exp()
    }
}

/// Position of the nth entry of an external release, by its own place column.
fn target(path: &str, place_col: usize, value_col: usize, place: i64) -> Option<f64> {
    let place_of = |r: &Vec<String>| r.get(place_col).and_then(|p| p.trim().parse::<f64>().ok()).map(|p| p as i64);
    csv_rows(path)
        .ok()?
        .iter()
        .find(|r| place_of(r) == Some(place))
        .and_then(|r| r.get(value_col)?.trim().parse().ok())
}

/// The value of the nth listed row of a rating table at a release.
fn nth(conn: &Connection, table: &str, col: &str, rid: i64, prov: f64, n: i64) -> rusqlite::Result<f64> {
    conn.query_row(
        &format!(
            "SELECT {col} FROM {table} WHERE release_id = ?1 AND sigma <= ?2
                     ORDER BY {col} DESC LIMIT 1 OFFSET ?3"
        ),
        rusqlite::params![rid, prov, n - 1],
        |r| r.get(0),
    )
}

/// The affine map that sends the 50th and the 1000th of the release nearest
/// `date` to the values `at`.
fn fit_map(conn: &Connection, table: &str, col: &str, date: &str, at: (f64, f64), prov: f64) -> rusqlite::Result<(f64, f64)> {
    let rid: i64 = conn.query_row(
        "SELECT id FROM releases ORDER BY ABS(julianday(date) - julianday(?1)) LIMIT 1",
        [date],
        |r| r.get(0),
    )?;
    let (v50, v1000) = (nth(conn, table, col, rid, prov, PLACE_HI)?, nth(conn, table, col, rid, prov, PLACE_LO)?);
    let b = (at.0 - at.1) / (v50 - v1000);
    println!("  {table} release {rid} ({date}): 50th {v50:.4}, 1000th {v1000:.4}");
    Ok((at.0 - b * v50, b))
}

/// DESIGN 5: the affine map is fixed by two places of the old 2019 release.
fn display_constants(cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
    for (what, path, pc, vc, place, want) in DISPLAY_CHECKS {
        let got = target(path, pc, vc, place);
        println!("{what:12} expected {want}, file gives {got:?}");
    }
    let db = arg("--db").unwrap_or(cfg.data.out_path.clone());
    let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let prov = cfg.display.provisional_sigma;
    let (at, bt) = fit_map(&conn, "team_ratings", "mu", "2019-09-05", TEAM_AT, prov)?;
    let (ap, bp) = fit_map(&conn, "player_ratings", "theta", "2019-10-03", PLAYER_AT, prov)?;
    println!("\n[display]\na_team = {at:.4}\nb_team = {bt:.4}\na_player = {ap:.4}\nb_player = {bp:.4}");
    Ok(())
}

/// The pairwise outcomes of the in-person fields in `[lo, hi)` against the
/// roster estimates of the morning, and the residuals of six-player lineups as
/// `(n, sum, sum of squares, summed player variance)`.
struct Calibration<'a> {
    cfg: &'a Config,
    lo: i32,
    hi: i32,
    hist: Hist,
    res: [f64; 4],
}

/// 1 when `a` placed above `b`, a half for a tie, 0 below.
fn win(a: &TeamResult, b: &TeamResult) -> f64 {
    match a.position.total_cmp(&b.position) {
        std::cmp::Ordering::Less => 1.0,
        std::cmp::Ordering::Equal => 0.5,
        std::cmp::Ordering::Greater => 0.0,
    }
}

impl Calibration<'_> {
    /// Each lineup's roster estimate and its variance, from the morning's players.
    fn estimates(&self, m: &Morning) -> Vec<Option<(f64, f64)>> {
        let (t, players) = (m.t, m.players);
        let vbar = players.editor_mean(t);
        let estimate = |r: &TeamResult| {
            (!r.members.is_empty()).then(|| {
                let at = |i: &u32| players.theta(*i as usize) + players.affinity(*i as usize, &vbar);
                let z: Vec<f64> = r.members.iter().map(at).collect();
                let v: Vec<f64> = r.members.iter().map(|&i| players.state[i as usize].var).collect();
                roster_estimate(&z, &v, self.cfg)
            })
        };
        t.teams.iter().map(estimate).collect()
    }

    fn pairs(&mut self, t: &Tournament, est: &[Option<(f64, f64)>]) {
        let basis = (t.n_q.max(1) as f64 / BASIS_QUESTIONS).sqrt();
        for (a, ea) in est.iter().enumerate() {
            let Some((sa, _)) = *ea else { continue };
            for (b, eb) in est.iter().enumerate().skip(a + 1) {
                let Some((sb, _)) = *eb else { continue };
                self.hist.add((sa - sb) * basis, win(&t.teams[a], &t.teams[b]));
            }
        }
    }

    fn residuals(&mut self, m: &Morning, est: &[Option<(f64, f64)>]) {
        let cfg = self.cfg;
        for (r, e) in m.t.teams.iter().zip(est) {
            let Some((sr, var)) = *e else { continue };
            if r.members.len() != LINEUP || r.mask.is_empty() {
                continue;
            }
            let hits = r.mask.iter().filter(|&&y| y == CORRECT).count() as f64;
            let Some(s) = qm::realised_strength(m.params, &r.mask, hits, cfg) else { continue };
            let d = s - sr;
            let add = [1.0, d, d * d, var - cfg.team.sigma_model * cfg.team.sigma_model];
            self.res.iter_mut().zip(add).for_each(|(a, x)| *a += x);
        }
    }
}

impl Observer for Calibration<'_> {
    fn morning(&mut self, m: &Morning) -> chgk_rating::pass::Result<()> {
        let t = m.t;
        if t.format != Format::InPerson || t.day < self.lo || t.day >= self.hi {
            return Ok(());
        }
        let est = self.estimates(m);
        self.pairs(t, &est);
        self.residuals(m, &est);
        Ok(())
    }
}

fn report(from: &str, to: &str, hist: &Hist, [rn, rs, rss, vsum]: [f64; 4]) {
    let pairs: f64 = hist.n.iter().sum();
    println!("beta window {from} to {to}, in-person only");
    println!("  pairs            {pairs:.0}");
    println!("  beta             {:.4}", hist.fit());
    let mean = rs / rn;
    let var = rss / rn - mean * mean;
    println!("six-player lineups {rn:.0}");
    println!("  residual mean    {mean:.4}");
    println!("  residual sd      {:.4}", var.sqrt());
    println!("  mean player term {:.4}", vsum / rn);
    println!("  sigma_model      {:.4}  (sd of the residual, DESIGN 4.1)", var.sqrt());
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::load(&arg("--config").unwrap_or_else(|| "config.toml".into()))?;
    if flag("--display") {
        return display_constants(&cfg);
    }
    let from = arg("--from").unwrap_or_else(|| FROM.into());
    let to = arg("--to").unwrap_or_else(|| TO.into());
    let (lo, hi) = (parse_date(&from).unwrap(), parse_date(&to).unwrap());
    let arena = chgk_rating::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
    let mut c = Calibration { cfg: &cfg, lo, hi, hist: Hist::new(), res: Default::default() };
    chgk_rating::pass::run(&arena, &cfg, Default::default(), &mut c)?;
    report(&from, &to, &c.hist, c.res);
    Ok(())
}
