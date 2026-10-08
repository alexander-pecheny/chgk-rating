use chgk_rating::config::Config;
use chgk_rating::data::{Format, CORRECT};
use chgk_rating::question_model as qm;
use chgk_rating::team_model::roster_estimate;
use chgk_rating::pass::{Morning, Observer};
use chgk_rating::util::{csv_rows, parse_date};

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

const BINS: usize = 1000;
const HALF: f64 = 10.0;

/// Pairwise wins bucketed by strength gap on a 36-question basis.
struct Hist {
    n: Vec<f64>,
    y: Vec<f64>,
}

impl Hist {
    fn add(&mut self, d: f64, y: f64) {
        let k = (((d + HALF) / (2.0 * HALF)) * BINS as f64) as isize;
        let k = k.clamp(0, BINS as isize - 1) as usize;
        self.n[k] += 1.0;
        self.y[k] += y;
    }
    fn centre(k: usize) -> f64 {
        (k as f64 + 0.5) / BINS as f64 * 2.0 * HALF - HALF
    }
    fn loglik(&self, beta: f64) -> f64 {
        let mut v = 0.0;
        for k in 0..BINS {
            if self.n[k] == 0.0 {
                continue;
            }
            let p = 1.0 / (1.0 + (-Hist::centre(k) / beta).exp());
            v +=
                self.y[k] * p.max(1e-12).ln() + (self.n[k] - self.y[k]) * (1.0 - p).max(1e-12).ln();
        }
        v
    }
    /// Ternary search on log beta; the likelihood is unimodal in it.
    fn fit(&self) -> f64 {
        let (mut lo, mut hi) = (0.05f64.ln(), 5.0f64.ln());
        for _ in 0..200 {
            let (a, b) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
            if self.loglik(a.exp()) < self.loglik(b.exp()) {
                lo = a;
            } else {
                hi = b;
            }
        }
        (0.5 * (lo + hi)).exp()
    }
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

/// Position of the nth entry of an external release, by its own place column.
fn target(path: &str, place_col: usize, value_col: usize, place: i64) -> Option<f64> {
    csv_rows(path)
        .ok()?
        .iter()
        .find(|r| {
            r.get(place_col)
                .and_then(|p| p.trim().parse::<f64>().ok())
                .map(|p| p as i64)
                == Some(place)
        })
        .and_then(|r| r.get(value_col)?.trim().parse().ok())
}

/// DESIGN 5: the affine map is fixed by two places of the old 2019 release.
fn display_constants(cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let db = arg("--db").unwrap_or(cfg.data.out_path.clone());
    let conn =
        rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let prov = cfg.display.provisional_sigma;
    let fit =
        |table: &str, col: &str, date: &str, hi: f64, lo: f64| -> rusqlite::Result<(f64, f64)> {
            let rid: i64 = conn.query_row(
                "SELECT id FROM releases ORDER BY ABS(julianday(date) - julianday(?1)) LIMIT 1",
                [date],
                |r| r.get(0),
            )?;
            let nth = |n: i64| -> rusqlite::Result<f64> {
                conn.query_row(
                    &format!(
                        "SELECT {col} FROM {table} WHERE release_id = ?1 AND sigma <= ?2
                     ORDER BY {col} DESC LIMIT 1 OFFSET ?3"
                    ),
                    rusqlite::params![rid, prov, n - 1],
                    |r| r.get(0),
                )
            };
            let (v50, v1000) = (nth(50)?, nth(1000)?);
            let b = (hi - lo) / (v50 - v1000);
            println!("  {table} release {rid} ({date}): 50th {v50:.4}, 1000th {v1000:.4}");
            Ok((hi - b * v50, b))
        };
    let (at, bt) = fit("team_ratings", "mu", "2019-09-05", 7028.0, 2992.0)?;
    let (ap, bp) = fit("player_ratings", "theta", "2019-10-03", 12865.0, 9778.0)?;
    println!(
        "\n[display]\na_team = {at:.4}\nb_team = {bt:.4}\na_player = {ap:.4}\nb_player = {bp:.4}"
    );
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

impl Observer for Calibration<'_> {
    fn morning(&mut self, m: &Morning) -> chgk_rating::pass::Result<()> {
        let (t, players, cfg) = (m.t, m.players, self.cfg);
        if t.format != Format::InPerson || t.day < self.lo || t.day >= self.hi {
            return Ok(());
        }
        let vbar = players.editor_mean(t);
        let estimate = |r: &chgk_rating::data::TeamResult| {
            (!r.members.is_empty()).then(|| {
                let at = |i: &u32| players.theta(*i as usize) + players.affinity(*i as usize, &vbar);
                let z: Vec<f64> = r.members.iter().map(at).collect();
                let v: Vec<f64> = r.members.iter().map(|&i| players.state[i as usize].var).collect();
                roster_estimate(&z, &v, cfg)
            })
        };
        let est: Vec<Option<(f64, f64)>> = t.teams.iter().map(estimate).collect();
        let basis = (t.n_q.max(1) as f64 / 36.0).sqrt();
        for (a, ea) in est.iter().enumerate() {
            let Some((sa, _)) = *ea else { continue };
            for (b, eb) in est.iter().enumerate().skip(a + 1) {
                let Some((sb, _)) = *eb else { continue };
                let y = match t.teams[a].position.total_cmp(&t.teams[b].position) {
                    std::cmp::Ordering::Less => 1.0,
                    std::cmp::Ordering::Equal => 0.5,
                    std::cmp::Ordering::Greater => 0.0,
                };
                self.hist.add((sa - sb) * basis, y);
            }
        }
        for (r, e) in t.teams.iter().zip(&est) {
            let Some((sr, var)) = *e else { continue };
            if r.members.len() != 6 || r.mask.is_empty() {
                continue;
            }
            let hits = r.mask.iter().filter(|&&y| y == CORRECT).count() as f64;
            let Some(s) = qm::realised_strength(m.params, &r.mask, hits, cfg) else { continue };
            let d = s - sr;
            self.res[0] += 1.0;
            self.res[1] += d;
            self.res[2] += d * d;
            self.res[3] += var - cfg.team.sigma_model * cfg.team.sigma_model;
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg_path = arg("--config").unwrap_or_else(|| "config.toml".into());
    let cfg = Config::load(&cfg_path)?;
    if flag("--display") {
        for (what, path, pc, vc, place, want) in [
            (
                "team 50",
                "data/external/team_release_2019.csv",
                2,
                1,
                50,
                7028.0,
            ),
            (
                "team 1000",
                "data/external/team_release_2019.csv",
                2,
                1,
                1000,
                2992.0,
            ),
            (
                "player 50",
                "data/external/player_release_1417_old_export.csv",
                6,
                7,
                50,
                12865.0,
            ),
            (
                "player 1000",
                "data/external/player_release_1417_old_export.csv",
                6,
                7,
                1000,
                9778.0,
            ),
        ] {
            let got = target(path, pc, vc, place);
            println!("{what:12} expected {want}, file gives {got:?}");
        }
        return display_constants(&cfg);
    }
    let from = arg("--from").unwrap_or_else(|| "2015-01-01".into());
    let to = arg("--to").unwrap_or_else(|| "2025-01-01".into());
    let (lo, hi) = (parse_date(&from).unwrap(), parse_date(&to).unwrap());

    let arena = chgk_rating::data::load(&cfg, &cfg.data.from, &cfg.data.to)?;
    let mut c = Calibration { cfg: &cfg, lo, hi, hist: Hist { n: vec![0.0; BINS], y: vec![0.0; BINS] }, res: [0.0; 4] };
    chgk_rating::pass::run(&arena, &cfg, Default::default(), &mut c)?;
    let (hist, [rn, rs, rss, vsum]) = (c.hist, c.res);

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
    println!(
        "  sigma_model      {:.4}  (sd of the residual, DESIGN 4.1)",
        var.sqrt()
    );
    Ok(())
}
