//! The acceptance report of DESIGN 7.3 over a fit's database, one section per
//! test; `--brief` is the same numbers on a few lines, `--ndcg` the forecast
//! metric alone.

use chgk_rating::config::{Config, TeamEngine};
use chgk_rating::ndcg;
use chgk_rating::team_ndcg as tn;
use chgk_rating::util::{csv_rows, spearman};
use rusqlite::{params, Connection, OpenFlags, Params, Row};
use std::collections::HashMap;

type Loss = (String, i64, i64, f64, f64);
/// Format, tournaments, mean Spearman of `mu` alone and of `x`.
type Naive = (String, f64, f64, i64);

const FORMATS: [&str; 3] = ["in-person", "sync", "strict-sync"];
/// Test 2 and the tuning grid both look at the last two seasons.
const WIN: (&str, &str) = ("2024-08-30", "2026-08-27");
/// The log-loss tables: the last season, and everything.
const SEASON: (&str, &str) = ("2025-08-29", "2026-08-27");
const ALL_TIME: (&str, &str) = ("0000", "9999");
const NO_HISTORY: &str = "n/a (rows only at the last release)";
/// NDCG@3: the podium.
const PODIUM: usize = 3;
/// Bootstrap draws of the paired standard error.
const BOOT: usize = 2000;

/// The falling-too-slowly case: two teams, the releases they are read at, and
/// the three tournaments whose steps are shown.
const CASE_TEAMS: [i64; 2] = [45556, 86405];
const CASE_RELEASES: [&str; 2] = ["2025-09-05", "2026-03-05"];
const CASE_TOURNAMENTS: [i64; 3] = [13917, 12826, 13444];
/// Test 4 reads each September in these years; one with fewer rated teams is
/// a hole in the data, and the band is the drift allowed.
const SEPTEMBERS: (i32, i32) = (2015, 2026);
const SEPTEMBER_MIN_TEAMS: i64 = 200;
const INFLATION_BAND: f64 = 0.10;
const ZVAR_GROUPS: [(&str, &str); 3] = [
    ("all (drives nu)", ""),
    ("returning after >= 1 year (tau_T)", "AND e.gap_days >= 365"),
    ("after losing >= 30% of evidence (gamma)", "AND e.lost_prev > 0.3"),
];
const ACF_GROUPS: [(&str, &str); 2] = [("in-person", "AND m.type='in-person'"), ("all", "")];
const QUARTILES: [f64; 3] = [0.25, 0.5, 0.75];
/// Test 3: the top of each table compared, and the career a player needs to
/// enter the rank correlation.
const TOP_OVERLAP: usize = 200;
const MIN_GAMES: f64 = 50.0;
/// DESIGN 5: the places the display scale is read at, and the 2019 releases.
const DISPLAY_RANKS: [i64; 6] = [1, 10, 50, 100, 500, 1000];
const DISPLAY_2019: (&str, &str) = ("2019-09-05", "2019-10-03");
/// The release listing shows about this many rows.
const RELEASE_ROWS: i64 = 12;

/// An external player table: file, id column, place column, rating column and
/// games column, if any.
struct Source {
    name: &'static str,
    path: &'static str,
    id: usize,
    place: usize,
    val: usize,
    games: Option<usize>,
}

const QUEST_GAMES: usize = 6;
const QUEST: Source = Source {
    name: "chgk.quest",
    path: "data/external/chgk_quest/players_all.csv",
    id: 1,
    place: 0,
    val: 4,
    games: Some(QUEST_GAMES),
};
const FUN: Source = Source {
    name: "rating.chgk.fun",
    path: "data/external/chgk_fun/players_rating_release263.csv",
    id: 1,
    place: 0,
    val: 3,
    games: None,
};

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

/// Prints a Markdown table: the header cells joined by `|`, then `align`, one
/// letter per column (`l` or `r`), then the rows.
fn print_table(head: &str, align: &str, rows: &[Vec<String>]) {
    println!("| {head} |");
    let marks: Vec<&str> = align.chars().map(|c| if c == 'r' { "---:" } else { "---" }).collect();
    println!("|{}|", marks.join("|"));
    for r in rows {
        println!("| {} |", r.join(" | "));
    }
}

/// Every row of a query, each read as a tuple.
fn all<T>(conn: &Connection, sql: &str, p: impl Params) -> rusqlite::Result<Vec<T>>
where
    T: for<'a> TryFrom<&'a Row<'a>, Error = rusqlite::Error>,
{
    let mut st = conn.prepare(sql)?;
    let v = st.query_map(p, |r| T::try_from(r))?.collect();
    v
}

fn one<T>(conn: &Connection, sql: &str, p: impl Params) -> rusqlite::Result<T>
where
    T: for<'a> TryFrom<&'a Row<'a>, Error = rusqlite::Error>,
{
    conn.query_row(sql, p, |r| T::try_from(r))
}

fn f3(v: f64) -> String {
    format!("{v:.3}")
}

fn f4(v: f64) -> String {
    format!("{v:.4}")
}

fn fmt_key(f: &str) -> usize {
    FORMATS.iter().position(|x| *x == f).unwrap_or(FORMATS.len())
}

// ------------------------------------------------------------------ queries

/// Test 7 of DESIGN 7.3: does the team rating alone order an in-person field of
/// base-roster teams? `(weighted NDCG, weighted NDCG@3, tournaments)` over one
/// window. Needs `out/team_ndcg/qualify.csv`, which `tools/team_ndcg.py` writes.
fn team_ndcg(conn: &Connection, from: &str, to: &str) -> rusqlite::Result<(f64, f64, usize)> {
    let qual = tn::qualify(tn::QUALIFY);
    let w = tn::weights(tn::WEIGHTS);
    let mut by_t: HashMap<i64, (Vec<i64>, Vec<f64>, Vec<f64>)> = HashMap::new();
    for (tid, team, mu, place) in team_ndcg_rows(conn, from, to)? {
        if qual.contains(&(tid, team)) {
            let e = by_t.entry(tid).or_default();
            e.0.push(team);
            e.1.push(mu);
            e.2.push(place);
        }
    }
    let score = |k: usize| -> HashMap<i64, f64> {
        by_t.iter().filter_map(|(tid, (team, mu, place))| Some((*tid, tn::score(*tid, team, mu, place, k)?))).collect()
    };
    let (full, at3) = (score(usize::MAX), score(PODIUM));
    Ok((tn::wmean(&full, &w), tn::wmean(&at3, &w), full.len()))
}

fn team_ndcg_rows(conn: &Connection, from: &str, to: &str) -> rusqlite::Result<Vec<(i64, i64, f64, f64)>> {
    all(
        conn,
        "SELECT e.tournament_id, e.team_id, e.mu_before, e.actual_place
         FROM team_events e JOIN tournament_metrics m ON m.tournament_id = e.tournament_id
         WHERE m.type = 'in-person' AND m.date >= ?1 AND m.date < ?2
           AND e.actual_place > 0 AND e.rated = 1 AND e.mu_before IS NOT NULL
         ORDER BY e.tournament_id",
        [from, to],
    )
}

fn losses(conn: &Connection, from: &str, to: &str) -> rusqlite::Result<Vec<Loss>> {
    let sums: Vec<(String, i64, i64, f64, f64)> = all(
        conn,
        "SELECT type, COUNT(*), SUM(n_obs),
                SUM(logloss_insample*n_obs), SUM(logloss_crossfit*n_obs)
         FROM tournament_metrics
         WHERE date >= ?1 AND date < ?2 AND n_obs > 0
         GROUP BY type",
        [from, to],
    )?;
    let mut v: Vec<Loss> = sums.into_iter().map(|(f, n, obs, i, c)| (f, n, obs, i / obs.max(1) as f64, c / obs.max(1) as f64)).collect();
    v.sort_by_key(|r| fmt_key(&r.0));
    Ok(v)
}

/// Mean per-tournament Spearman of two naive rankings, by format: `mu_before`
/// alone, and `x = mu_before + delta_l`, the rating with the lineup offset.
fn naive_spearman(conn: &Connection) -> rusqlite::Result<Vec<Naive>> {
    let rows: Vec<(String, i64, f64, f64, f64)> = all(
        conn,
        "SELECT m.type, e.tournament_id, e.mu_before, e.mu_before + e.delta_l, e.actual_place
         FROM team_events e JOIN tournament_metrics m ON m.tournament_id = e.tournament_id
         WHERE m.date >= ?1 AND m.date < ?2 AND e.mu_before IS NOT NULL
         ORDER BY m.type, e.tournament_id",
        [WIN.0, WIN.1],
    )?;
    let mut acc: Vec<Naive> = Vec::new();
    for g in rows.chunk_by(|a, b| a.1 == b.1) {
        add_naive(&mut acc, g);
    }
    for e in acc.iter_mut() {
        e.1 /= e.3 as f64;
        e.2 /= e.3 as f64;
    }
    acc.sort_by_key(|r| fmt_key(&r.0));
    Ok(acc)
}

/// One tournament's two correlations, added to its format's sums.
fn add_naive(acc: &mut Vec<Naive>, g: &[(String, i64, f64, f64, f64)]) {
    let mu: Vec<f64> = g.iter().map(|r| -r.2).collect();
    let x: Vec<f64> = g.iter().map(|r| -r.3).collect();
    let pl: Vec<f64> = g.iter().map(|r| r.4).collect();
    let (Some(a), Some(b)) = (spearman(&mu, &pl), spearman(&x, &pl)) else { return };
    let kind = &g[0].0;
    match acc.iter_mut().find(|e| &e.0 == kind) {
        Some(e) => {
            e.1 += a;
            e.2 += b;
            e.3 += 1;
        }
        None => acc.push((kind.clone(), a, b, 1)),
    }
}

/// Lag-1 correlation of a team's successive pairwise innovations. A well-tuned
/// filter has white innovations; a sluggish one leaves them positively correlated.
fn innovation_acf(conn: &Connection, extra: &str) -> rusqlite::Result<(usize, f64)> {
    let rows: Vec<(i64, f64)> = all(
        conn,
        &format!(
            "SELECT e.team_id, e.s_pair / MAX(e.i_pair, 1e-9)
         FROM team_events e JOIN tournament_metrics m ON m.tournament_id = e.tournament_id
         WHERE m.date >= ?1 AND m.date < ?2 AND e.rated = 1 AND e.evidence >= 20 {extra}
         ORDER BY e.team_id, m.date, e.tournament_id"
        ),
        [WIN.0, WIN.1],
    )?;
    let (a, b): (Vec<f64>, Vec<f64>) = rows.windows(2).filter(|w| w[0].0 == w[1].0).map(|w| (w[0].1, w[1].1)).unzip();
    if a.len() < 2 {
        return Ok((0, f64::NAN));
    }
    Ok((a.len(), correlation(&a, &b)))
}

fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len();
    let mean = |v: &[f64]| v.iter().sum::<f64>() / n as f64;
    let (ma, mb) = (mean(a), mean(b));
    let cov: f64 = a.iter().zip(b).map(|(u, v)| (u - ma) * (v - mb)).sum();
    let va: f64 = a.iter().map(|u| (u - ma) * (u - ma)).sum();
    let vb: f64 = b.iter().map(|v| (v - mb) * (v - mb)).sum();
    cov / (va * vb).sqrt()
}

/// Whether the database carries a row per release or only the last one.
/// `rows_every = 0` keeps the last release only, and every section that reads an
/// earlier release has nothing to read.
fn has_history(conn: &Connection) -> rusqlite::Result<bool> {
    let (n,): (i64,) = one(conn, "SELECT COUNT(DISTINCT release_id) FROM team_ratings", [])?;
    Ok(n > 1)
}

fn nearest_release(conn: &Connection, date: &str) -> rusqlite::Result<i64> {
    let (id,) = one(conn, "SELECT id FROM releases ORDER BY ABS(julianday(date)-julianday(?1)) LIMIT 1", [date])?;
    Ok(id)
}

fn violations(conn: &Connection) -> rusqlite::Result<(i64, i64)> {
    let (sole, shared): (Option<i64>, Option<i64>) = one(
        conn,
        "SELECT SUM(e.actual_place = 1.0), SUM(e.actual_place > 1.0)
         FROM team_events e
         JOIN (SELECT tournament_id, MIN(actual_place) mp FROM team_events
               GROUP BY tournament_id) g
           ON g.tournament_id = e.tournament_id AND e.actual_place = g.mp
         WHERE e.mu_after < e.mu_before - 1e-12",
        [],
    )?;
    Ok((sole.unwrap_or(0), shared.unwrap_or(0)))
}

/// Variance of the standardised residual; 1 means the published sigma is honest.
fn zvar(conn: &Connection, extra: &str) -> rusqlite::Result<(i64, f64, f64)> {
    let (n, m, m2): (i64, Option<f64>, Option<f64>) = one(
        conn,
        &format!(
            "SELECT COUNT(*), AVG(z), AVG(z*z) FROM team_events e
             JOIN tournament_metrics m ON m.tournament_id = e.tournament_id
             WHERE e.z IS NOT NULL AND m.type='in-person'
               AND m.date >= '2018-01-01' AND e.evidence >= 6 {extra}"
        ),
        [],
    )?;
    match (m, m2) {
        (Some(m), Some(m2)) if n > 0 => Ok((n, m, m2 - m * m)),
        _ => Ok((0, 0.0, 0.0)),
    }
}

fn mean_forecast(conn: &Connection) -> rusqlite::Result<f64> {
    let (v,): (Option<f64>,) = one(
        conn,
        "SELECT AVG(spearman_forecast) FROM tournament_metrics
         WHERE type='in-person' AND date >= ?1 AND date < ?2 AND spearman_forecast IS NOT NULL",
        [WIN.0, WIN.1],
    )?;
    Ok(v.unwrap_or(0.0))
}

/// Listed entities at a release, and how many of them are provisional.
fn listed(conn: &Connection, table: &str, rid: i64, prov: f64) -> rusqlite::Result<(i64, i64)> {
    let sql = format!("SELECT COUNT(*), SUM(sigma > ?2) FROM {table} WHERE release_id = ?1");
    let (n, p): (i64, Option<i64>) = one(conn, &sql, params![rid, prov])?;
    Ok((n, p.unwrap_or(0)))
}

/// One external player table: id -> (place, rating, games).
fn external(s: &Source) -> Vec<(i64, f64, f64, f64)> {
    let num = |r: &Vec<String>, i: usize| r.get(i).and_then(|x| x.trim().parse::<f64>().ok());
    let games = |r: &Vec<String>| s.games.map_or(f64::INFINITY, |g| num(r, g).unwrap_or(0.0));
    let row = |r: &Vec<String>| Some((r.get(s.id)?.trim().parse().ok()?, num(r, s.place)?, num(r, s.val)?, games(r)));
    let mut v: Vec<(i64, f64, f64, f64)> = csv_rows(s.path).unwrap_or_default().iter().filter_map(row).collect();
    v.sort_by(|a, b| a.1.total_cmp(&b.1));
    v
}

// ------------------------------------------------------------------ the report

/// The latest release, which every table of the current standings reads.
struct Release {
    id: i64,
    date: String,
}

/// What every section reads: the database, the provisional cut, whether the
/// database keeps every release, and whether the team engine keeps a variance.
struct Report<'a> {
    conn: &'a Connection,
    prov: f64,
    b_team: f64,
    history: bool,
    kalman: bool,
}

impl Report<'_> {
    fn new<'a>(conn: &'a Connection, cfg: &Config) -> rusqlite::Result<Report<'a>> {
        Ok(Report {
            conn,
            prov: cfg.display.provisional_sigma,
            b_team: cfg.display.b_team,
            history: has_history(conn)?,
            // S2 keeps no variance, so everything the uncertainty calibration
            // reads is a placeholder under it.
            kalman: cfg.team.engine == TeamEngine::Kalman,
        })
    }

    fn latest(&self) -> rusqlite::Result<Release> {
        let (id, date) = one(self.conn, "SELECT id, date FROM releases ORDER BY id DESC LIMIT 1", [])?;
        Ok(Release { id, date })
    }

    /// The tests of DESIGN 7.3 in order, then the current standings.
    fn print(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.loss_table("Log-loss, season 2025/26", SEASON)?;
        self.loss_table("Log-loss, all seasons", ALL_TIME)?;
        self.rank_correlation()?;
        self.innovation()?;
        self.case_teams()?;
        self.winner_rule()?;
        self.calibration()?;
        self.inflation()?;
        self.transfer_invariance()?;
        self.standings(&self.latest()?)
    }

    fn standings(&self, rel: &Release) -> Result<(), Box<dyn std::error::Error>> {
        self.top_teams(rel)?;
        self.uncertainty_budget(rel)?;
        self.synergy(rel)?;
        self.external_agreement(rel)?;
        self.display_scale(rel)?;
        self.top_players(rel)?;
        self.release_list(rel)?;
        Ok(())
    }

    fn loss_table(&self, title: &str, (from, to): (&str, &str)) -> rusqlite::Result<()> {
        println!("\n### {title} ({from} to {to})\n");
        let rows = losses(self.conn, from, to)?;
        let mut cells: Vec<Vec<String>> =
            rows.iter().map(|(f, nt, no, i, c)| vec![f.clone(), nt.to_string(), no.to_string(), f4(*c), f4(*i)]).collect();
        cells.extend(loss_total(&rows));
        print_table("format | tournaments | observations | cross-fitted | in-sample", "lrrrr", &cells);
        Ok(())
    }

    /// Test 2.
    fn rank_correlation(&self) -> rusqlite::Result<()> {
        println!("\n### Forecast rank correlation, {} to {} (test 2)\n", WIN.0, WIN.1);
        let naive = naive_spearman(self.conn)?;
        let mut rows: Vec<(String, i64, f64, f64)> = all(
            self.conn,
            "SELECT type, COUNT(spearman_forecast), AVG(spearman_forecast), AVG(spearman_old_system)
             FROM tournament_metrics WHERE date >= ?1 AND date < ?2 GROUP BY type",
            [WIN.0, WIN.1],
        )?;
        rows.sort_by_key(|r| fmt_key(&r.0));
        let cells: Vec<Vec<String>> = rows.iter().map(|r| correlation_row(r, &naive)).collect();
        print_table("format | tournaments | ours | old system | naive (mu only) | naive (x)", "lrrrrr", &cells);
        Ok(())
    }

    fn innovation(&self) -> rusqlite::Result<()> {
        println!("\n### Innovation autocorrelation\n");
        println!("Lag 1 over teams with evidence >= 20, {} to {}. Zero is a white innovation.\n", WIN.0, WIN.1);
        let mut cells = Vec::new();
        for (name, extra) in ACF_GROUPS {
            let (n, c) = innovation_acf(self.conn, extra)?;
            cells.push(vec![name.to_string(), n.to_string(), f3(c)]);
        }
        print_table("formats | consecutive pairs | correlation", "lrr", &cells);
        Ok(())
    }

    fn case_teams(&self) -> rusqlite::Result<()> {
        println!("\n### Case teams\n");
        println!("Display value at three releases, then the display step at three tournaments.\n");
        let cells: Vec<Vec<String>> =
            self.case_rows()?.into_iter().map(|(team, c)| [vec![team.to_string()], c].concat()).collect();
        print_table("team | 2025-09 | 2026-03 | last | 13917 | 12826 | 13444", "lrrrrrr", &cells);
        if !self.history {
            println!("\nThe 2025-09 and 2026-03 columns are {NO_HISTORY}.");
        }
        Ok(())
    }

    /// The two teams of the falling-too-slowly case: where they stand and how far
    /// three tournaments moved them, in display points.
    fn case_rows(&self) -> rusqlite::Result<Vec<(i64, Vec<String>)>> {
        let rel = self.case_releases()?;
        let mut out = Vec::new();
        for team in CASE_TEAMS {
            let mut cells = self.case_displays(team, &rel);
            cells.extend(CASE_TOURNAMENTS.iter().map(|&t| self.case_step(team, t)));
            out.push((team, cells));
        }
        Ok(out)
    }

    /// The releases nearest the case dates, then the last; the last only when
    /// the database keeps no history.
    fn case_releases(&self) -> rusqlite::Result<Vec<i64>> {
        let (last,): (i64,) = one(self.conn, "SELECT MAX(id) FROM releases", [])?;
        let mut rel: Vec<i64> = vec![last; CASE_RELEASES.len()];
        if self.history {
            rel = CASE_RELEASES.iter().map(|d| nearest_release(self.conn, d)).collect::<rusqlite::Result<_>>()?;
        }
        rel.push(last);
        Ok(rel)
    }

    fn case_displays(&self, team: i64, rel: &[i64]) -> Vec<String> {
        let sql = "SELECT display FROM team_ratings WHERE release_id = ?1 AND team_id = ?2";
        let cell = |(i, r): (usize, &i64)| -> String {
            if !self.history && i < CASE_RELEASES.len() {
                return "n/a".into();
            }
            let v: Option<(f64,)> = one(self.conn, sql, params![r, team]).ok();
            v.map_or("-".into(), |v| format!("{:.0}", v.0))
        };
        rel.iter().enumerate().map(cell).collect()
    }

    fn case_step(&self, team: i64, tid: i64) -> String {
        let sql = "SELECT mu_after - mu_before FROM team_events WHERE team_id = ?1 AND tournament_id = ?2";
        let v: Option<f64> = one::<(Option<f64>,)>(self.conn, sql, params![team, tid]).ok().and_then(|v| v.0);
        v.map_or("-".into(), |v| format!("{:+.0}", v * self.b_team))
    }

    /// Test 6.
    fn winner_rule(&self) -> rusqlite::Result<()> {
        let (sole, shared) = violations(self.conn)?;
        println!("\n### Winner rule (test 6)\n");
        println!("Teams finishing first, sole or shared, with `mu_after < mu_before`: **{}**.", sole + shared);
        println!("(sole first {sole}, shared first {shared})");
        Ok(())
    }

    fn calibration(&self) -> rusqlite::Result<()> {
        println!("\n### Uncertainty calibration\n");
        if !self.kalman {
            println!("n/a: `team.engine = \"s2\"` publishes no variance to calibrate.");
            return Ok(());
        }
        println!("In-person from 2018, evidence >= 6. Variance of 1 means sigma is honest.\n");
        let mut cells = Vec::new();
        for (name, extra) in ZVAR_GROUPS {
            let (n, m, v) = zvar(self.conn, extra)?;
            cells.push(vec![name.to_string(), n.to_string(), f3(m), f3(v)]);
        }
        print_table("group | n | mean z | var z", "lrrr", &cells);
        Ok(())
    }

    /// Test 4: the display value of the 50th team each September, which needs a
    /// row per release.
    fn inflation(&self) -> rusqlite::Result<()> {
        println!("\n### Inflation of the 50th team, September releases (test 4)\n");
        if !self.history {
            println!("{NO_HISTORY}.");
            return Ok(());
        }
        let rows = self.septembers()?;
        let cells: Vec<Vec<String>> =
            rows.iter().map(|(id, date, n, m)| vec![id.to_string(), date.clone(), n.to_string(), format!("{m:.0}")]).collect();
        print_table("release | date | rated teams | display of 50th", "rlrr", &cells);
        let band: Vec<f64> = rows.iter().filter(|r| r.2 >= SEPTEMBER_MIN_TEAMS).map(|r| r.3).collect();
        print_band(&band);
        Ok(())
    }

    /// The first release of each September, its rated teams, and the display of
    /// its 50th; a September without a 50th is left out.
    fn septembers(&self) -> rusqlite::Result<Vec<(i64, String, i64, f64)>> {
        let mut out = Vec::new();
        for y in SEPTEMBERS.0..=SEPTEMBERS.1 {
            let first = "SELECT id, date FROM releases WHERE date LIKE ?1 ORDER BY date LIMIT 1";
            let Ok((id, date)) = one::<(i64, String)>(self.conn, first, [format!("{y}-09%")]) else { continue };
            let (n,): (i64,) =
                one(self.conn, "SELECT COUNT(*) FROM team_ratings WHERE release_id = ?1 AND sigma <= ?2", params![id, self.prov])?;
            let nth = "SELECT display FROM team_ratings WHERE release_id = ?1 AND sigma <= ?2
                 ORDER BY mu DESC LIMIT 1 OFFSET 49";
            if let Ok((m,)) = one::<(f64,)>(self.conn, nth, params![id, self.prov]) {
                out.push((id, date, n, m));
            }
        }
        Ok(out)
    }

    /// Test 5.
    fn transfer_invariance(&self) -> rusqlite::Result<()> {
        let (Some(other), Some(date)) = (arg("--swap-db"), arg("--swap-date")) else { return Ok(()) };
        println!("\n### Transfer invariance (test 5)\n");
        self.conn.execute_batch(&format!("ATTACH DATABASE '{other}' AS sw"))?;
        let (rid, rdate): (i64, String) =
            one(self.conn, "SELECT id, date FROM releases WHERE date >= ?1 ORDER BY date LIMIT 1", [&date])?;
        let moved = self.history.then(|| self.swap_moved(rid)).transpose()?;
        let (fc,): (i64,) = one(
            self.conn,
            "SELECT COUNT(*) FROM team_events a
             JOIN sw.team_events b ON b.tournament_id=a.tournament_id AND b.team_id=a.team_id
             JOIN tournament_metrics m ON m.tournament_id=a.tournament_id
             WHERE m.date >= ?1 AND ABS(a.forecast-b.forecast) > 1e-9",
            [&date],
        )?;
        println!("Swap date {date}, first release after it: {rid} of {rdate}.");
        match moved {
            Some((n, mx)) => println!("Teams whose `mu` moved: **{n}** (max |delta| {mx:.2e})."),
            None => println!("Teams whose `mu` moved: {NO_HISTORY}."),
        }
        println!("Team events on or after the swap whose forecast moved: {fc}.");
        Ok(())
    }

    fn swap_moved(&self, rid: i64) -> rusqlite::Result<(i64, f64)> {
        one(
            self.conn,
            "SELECT COUNT(*), COALESCE(MAX(ABS(a.mu-b.mu)),0) FROM team_ratings a
             JOIN sw.team_ratings b ON b.release_id=a.release_id AND b.team_id=a.team_id
             WHERE a.release_id=?1 AND ABS(a.mu-b.mu) > 1e-9",
            [rid],
        )
    }

    fn top_teams(&self, rel: &Release) -> rusqlite::Result<()> {
        let (nteams, nprov) = listed(self.conn, "team_ratings", rel.id, self.prov)?;
        println!("\n### Top 10 teams, release {} of {}\n", rel.id, rel.date);
        println!("{nteams} teams listed, {nprov} of them provisional (sigma > {}) and left out.\n", self.prov);
        let rows: Vec<(f64, f64, f64, f64, Option<f64>, Option<String>)> = all(
            self.conn,
            "SELECT r.mu, r.sigma, r.evidence, r.core_strength,
                (SELECT e.s_r FROM team_events e WHERE e.team_id=r.team_id AND e.s_r IS NOT NULL
                 ORDER BY e.rowid DESC LIMIT 1),
                COALESCE((SELECT NULLIF(TRIM(t.name), '') FROM buff.teams t WHERE t.id=r.team_id),
                         (SELECT x.team_current_name FROM buff.tournament_results x
                          WHERE x.team_id=r.team_id ORDER BY x.id DESC LIMIT 1))
         FROM team_ratings r WHERE r.release_id = ?1 AND r.sigma <= ?2
         ORDER BY r.mu DESC LIMIT 10",
            params![rel.id, self.prov],
        )?;
        let cells: Vec<Vec<String>> = rows.into_iter().enumerate().map(|(i, r)| top_team_row(i, r)).collect();
        print_table("# | team | mu | sigma | evidence | core | s_r of last lineup", "rlrrrrr", &cells);
        Ok(())
    }

    /// Where a top team's uncertainty comes from.
    fn uncertainty_budget(&self, rel: &Release) -> rusqlite::Result<()> {
        println!("\n### Uncertainty budget of the top 20 teams, trailing year\n");
        if !self.kalman {
            println!("n/a under `team.engine = \"s2\"`.");
            return Ok(());
        }
        println!("Variance per rated event: removed by results, re-added by diffusion and by departures.\n");
        let rows: Vec<(f64, i64, f64, f64, f64, Option<String>)> = all(self.conn, BUDGET, params![rel.id, self.prov, rel.date])?;
        let cells: Vec<Vec<String>> = rows.into_iter().enumerate().map(|(i, r)| budget_row(i, r)).collect();
        print_table("# | team | sigma | events | -var results | +var diffusion | +var departures", "rlrrrrr", &cells);
        Ok(())
    }

    /// The spread of `mu` over the roster estimate of the last lineup.
    fn synergy(&self, rel: &Release) -> rusqlite::Result<()> {
        let rows: Vec<(Option<f64>,)> = all(
            self.conn,
            "SELECT r.mu - (SELECT e.s_r FROM team_events e WHERE e.team_id=r.team_id
                        AND e.s_r IS NOT NULL ORDER BY e.rowid DESC LIMIT 1)
         FROM team_ratings r WHERE r.release_id = ?1 AND r.sigma <= ?2",
            params![rel.id, self.prov],
        )?;
        let mut d: Vec<f64> = rows.into_iter().filter_map(|r| r.0).collect();
        d.sort_by(f64::total_cmp);
        if d.is_empty() {
            return Ok(());
        }
        println!("\n### Synergy: mu minus the roster estimate of the last lineup\n");
        println!("Listed non-provisional teams at release {}: {}.\n", rel.id, d.len());
        print_table("min | q1 | median | q3 | max | mean", "rrrrrr", &[spread_row(&d)]);
        Ok(())
    }

    /// Test 3.
    fn external_agreement(&self, rel: &Release) -> rusqlite::Result<()> {
        let ours: Vec<(i64, f64)> = all(
            self.conn,
            "SELECT player_id, theta FROM player_ratings
         WHERE release_id = ?1 AND sigma <= ?2 ORDER BY theta DESC",
            params![rel.id, self.prov],
        )?;
        let any: Vec<(i64,)> =
            all(self.conn, "SELECT player_id FROM player_ratings WHERE release_id = ?1 ORDER BY theta DESC LIMIT 200", [rel.id])?;
        let ours = Ours::new(ours, any.into_iter().map(|x| x.0).collect());
        let quest = external(&QUEST);
        let qgames: HashMap<i64, f64> = quest.iter().map(|(i, _, _, g)| (*i, *g)).collect();
        println!("\n### Agreement with external systems (test 3)\n");
        println!("Our {} non-provisional players at release {}.\n", ours.theta.len(), rel.id);
        let cells = [(&QUEST, &quest), (&FUN, &external(&FUN))].map(|(s, ext)| ours.agreement(s.name, ext, &qgames));
        print_table(
            "source | entries | top-200 overlap | same, provisional kept | common (>= 50 games) | spearman",
            "lrrrrr",
            &cells,
        );
        println!("\nThe games filter uses chgk.quest's career count, the only one available.");
        Ok(())
    }

    /// DESIGN 5: the display value at fixed places, now and in 2019.
    fn display_scale(&self, rel: &Release) -> rusqlite::Result<()> {
        println!("\n### Display scale (DESIGN 5)\n");
        let mut rels = Vec::new();
        if self.history {
            rels.push(("2019", nearest_release(self.conn, DISPLAY_2019.0)?, nearest_release(self.conn, DISPLAY_2019.1)?));
        }
        rels.push(("latest", rel.id, rel.id));
        let mut cells = Vec::new();
        for (label, rt, rp) in rels {
            cells.push(self.display_row(label, "teams", "team_ratings", "mu", rt));
            cells.push(self.display_row(label, "players", "player_ratings", "theta", rp));
        }
        print_table("release | table | 1 | 10 | 50 | 100 | 500 | 1000", "llrrrrrr", &cells);
        if !self.history {
            println!("\nThe 2019 rows are {NO_HISTORY}.");
        }
        Ok(())
    }

    fn display_row(&self, label: &str, what: &str, table: &str, col: &str, rid: i64) -> Vec<String> {
        let sql = format!(
            "SELECT display FROM {table} WHERE release_id=?1 AND sigma<=?2
         ORDER BY {col} DESC LIMIT 1 OFFSET ?3"
        );
        let at = |n: i64| one::<(f64,)>(self.conn, &sql, params![rid, self.prov, n - 1]).ok();
        let values = DISPLAY_RANKS.iter().map(|&n| at(n).map_or("-".into(), |v| format!("{:.0}", v.0)));
        [label.to_string(), what.to_string()].into_iter().chain(values).collect()
    }

    fn top_players(&self, rel: &Release) -> rusqlite::Result<()> {
        let (npl, npprov) = listed(self.conn, "player_ratings", rel.id, self.prov)?;
        println!("\n### Top 50 players, release {} of {}\n", rel.id, rel.date);
        println!("{npl} players listed, {npprov} of them provisional (sigma > {}) and left out.\n", self.prov);
        let rows: Vec<(Option<String>, f64, f64, i64)> = all(
            self.conn,
            "SELECT trim(p.surname || ' ' || coalesce(p.name,'')), r.theta, r.sigma, r.games_365
         FROM player_ratings r LEFT JOIN buff.players p ON p.id = r.player_id
         WHERE r.release_id = ?1 AND r.sigma <= ?2 ORDER BY r.theta DESC LIMIT 50",
            params![rel.id, self.prov],
        )?;
        let cells: Vec<Vec<String>> = rows
            .into_iter()
            .enumerate()
            .map(|(i, (name, th, s, g))| vec![(i + 1).to_string(), name.unwrap_or_else(|| "?".into()), f3(th), f3(s), g.to_string()])
            .collect();
        print_table("# | player | theta | sigma | games 365d", "rlrrr", &cells);
        Ok(())
    }

    fn release_list(&self, rel: &Release) -> rusqlite::Result<()> {
        println!("\n### Listed players and teams per release\n");
        let rows: Vec<(i64, String, i64, f64, f64)> = all(
            self.conn,
            "SELECT id, date, n_players, shift, shift_cum FROM releases
         WHERE id % ?1 = 0 OR id = ?2 ORDER BY id",
            [(rel.id / RELEASE_ROWS).max(1), rel.id],
        )?;
        let cells: Vec<Vec<String>> =
            rows.into_iter().map(|(id, d, n, s, c)| vec![id.to_string(), d, n.to_string(), f4(s), f4(c)]).collect();
        print_table("release | date | players | shift | cumulative shift", "rlrrr", &cells);
        Ok(())
    }

    /// The same numbers on a few lines, for a sweep's log.
    fn brief(&self) -> rusqlite::Result<()> {
        let (sole, _shared) = violations(self.conn)?;
        let (n, prov_n): (i64, Option<i64>) = one(
            self.conn,
            "SELECT COUNT(*), SUM(sigma > ?1) FROM team_ratings
             WHERE release_id = (SELECT MAX(release_id) FROM team_ratings)",
            [self.prov],
        )?;
        let z = |extra: &str| zvar(self.conn, extra).map(|x| x.2);
        let (v, vr, vd) = (z(ZVAR_GROUPS[0].1)?, z(ZVAR_GROUPS[1].1)?, z(ZVAR_GROUPS[2].1)?);
        println!(
            "spearman {:.4}  rated {:.0}%  zvar {v:.3} return {vr:.3} depart {vd:.3}  violations {sole}",
            mean_forecast(self.conn)?,
            100.0 * (n - prov_n.unwrap_or(0)) as f64 / n.max(1) as f64
        );
        self.brief_naive()?;
        self.brief_teams()
    }

    fn brief_naive(&self) -> rusqlite::Result<()> {
        let naive = naive_spearman(self.conn)?;
        let col = |f: &str| naive.iter().find(|e| e.0 == f).map_or((f64::NAN, f64::NAN), |e| (e.1, e.2));
        let ((ip_mu, ip_x), (sync_mu, sync_x)) = (col(FORMATS[0]), col(FORMATS[1]));
        let (_, ip) = innovation_acf(self.conn, ACF_GROUPS[0].1)?;
        let (_, af) = innovation_acf(self.conn, ACF_GROUPS[1].1)?;
        println!(
            "naive mu {ip_mu:.4}/{sync_mu:.4}  x {ip_x:.4}/{sync_x:.4} (in-person/sync)  innovation acf {ip:.3} in-person, {af:.3} all"
        );
        Ok(())
    }

    fn brief_teams(&self) -> rusqlite::Result<()> {
        let (ft, t3, nt) = team_ndcg(self.conn, tn::TUNE.0, tn::TUNE.1)?;
        let (fs, s3, ns) = team_ndcg(self.conn, tn::TEST.0, tn::TEST.1)?;
        println!("team ndcg (weighted, mu only) tune {ft:.4}/@3 {t3:.4} over {nt}  test {fs:.4}/@3 {s3:.4} over {ns}");
        for (team, cells) in self.case_rows()? {
            println!("team {team}: {}", cells.join(" "));
        }
        Ok(())
    }
}

/// The trailing year of each top team's rated events, split by where the
/// variance went.
const BUDGET: &str = "WITH e AS (
               SELECT te.team_id AS tid, te.sigma_before AS sb, te.sigma_after AS sa,
                      te.var_diff AS vd,
                      LAG(te.sigma_after) OVER (PARTITION BY te.team_id ORDER BY te.rowid) AS pa
               FROM team_events te JOIN tournament_metrics m ON m.tournament_id = te.tournament_id
               WHERE m.date >= date(?3, '-365 day') AND te.rated = 1
             )
             SELECT r.sigma, COUNT(*),
                    AVG(sb*sb - sa*sa), AVG(vd),
                    AVG(sb*sb - pa*pa - vd),
                    COALESCE((SELECT NULLIF(TRIM(t.name), '') FROM buff.teams t WHERE t.id = r.team_id),
                             (SELECT x.team_current_name FROM buff.tournament_results x
                              WHERE x.team_id = r.team_id ORDER BY x.id DESC LIMIT 1))
             FROM team_ratings r JOIN e ON e.tid = r.team_id AND e.pa IS NOT NULL
             WHERE r.release_id = ?1 AND r.sigma <= ?2
             GROUP BY r.team_id ORDER BY r.mu DESC LIMIT 20";

/// The weighted total of a log-loss table, when it has observations.
fn loss_total(rows: &[Loss]) -> Option<Vec<String>> {
    let (mut n, mut obs, mut ins, mut cf) = (0, 0, 0.0, 0.0);
    for (_, nt, no, i, c) in rows {
        n += nt;
        obs += no;
        ins += i * *no as f64;
        cf += c * *no as f64;
    }
    let o = obs as f64;
    (obs > 0).then(|| vec!["**all**".into(), n.to_string(), obs.to_string(), f4(cf / o), f4(ins / o)])
}

fn correlation_row((f, n, ours, old): &(String, i64, f64, f64), naive: &[Naive]) -> Vec<String> {
    let (nv, nx) = naive.iter().find(|e| &e.0 == f).map_or((f64::NAN, f64::NAN), |e| (e.1, e.2));
    vec![f.clone(), n.to_string(), f4(*ours), f4(*old), f4(nv), f4(nx)]
}

fn print_band(band: &[f64]) {
    if band.is_empty() {
        return;
    }
    let mean = band.iter().sum::<f64>() / band.len() as f64;
    let dev = band.iter().map(|v| ((v - mean) / mean).abs()).fold(0.0, f64::max);
    let verdict = if dev <= INFLATION_BAND { "within" } else { "outside" };
    println!(
        "\nOver the {} Septembers with at least 200 rated teams: mean {mean:.0}, largest deviation {:.1}% -- **{verdict} the +/-10% band**.",
        band.len(),
        100.0 * dev
    );
}

fn top_team_row(i: usize, (mu, sigma, ev, core, sr, name): (f64, f64, f64, f64, Option<f64>, Option<String>)) -> Vec<String> {
    let sr = sr.map_or("-".into(), f3);
    vec![(i + 1).to_string(), name.unwrap_or_else(|| "?".into()), f3(mu), f3(sigma), format!("{ev:.1}"), f3(core), sr]
}

fn budget_row(i: usize, (sigma, n, res, diff, dep, name): (f64, i64, f64, f64, f64, Option<String>)) -> Vec<String> {
    let f5 = |v: f64| format!("{v:.5}");
    vec![(i + 1).to_string(), name.unwrap_or_else(|| "?".into()), f3(sigma), n.to_string(), f5(res), f5(diff), f5(dep)]
}

/// Min, quartiles, max and mean of a sorted sample.
fn spread_row(d: &[f64]) -> Vec<String> {
    let q = |f: f64| d[((d.len() - 1) as f64 * f) as usize];
    let mean = d.iter().sum::<f64>() / d.len() as f64;
    let mut v = vec![d[0]];
    v.extend(QUARTILES.map(q));
    v.extend([d[d.len() - 1], mean]);
    v.into_iter().map(f3).collect()
}

/// Our non-provisional players by theta, and our top 200 with provisional ones kept.
struct Ours {
    top: Vec<i64>,
    top_all: Vec<i64>,
    theta: HashMap<i64, f64>,
}

impl Ours {
    fn new(ours: Vec<(i64, f64)>, top_all: Vec<i64>) -> Ours {
        let top = ours.iter().take(TOP_OVERLAP).map(|(i, _)| *i).collect();
        Ours { top, top_all, theta: ours.into_iter().collect() }
    }

    /// One row of test 3: the top overlaps, and the rank correlation over the
    /// players chgk.quest counts with enough games.
    fn agreement(&self, name: &str, ext: &[(i64, f64, f64, f64)], qgames: &HashMap<i64, f64>) -> Vec<String> {
        let top: Vec<i64> = ext.iter().take(TOP_OVERLAP).map(|(i, _, _, _)| *i).collect();
        let overlap = top.iter().filter(|i| self.top.contains(i)).count();
        let overlap_all = top.iter().filter(|i| self.top_all.contains(i)).count();
        let played = |id: &i64| qgames.get(id).copied().unwrap_or(0.0) >= MIN_GAMES;
        let (a, b): (Vec<f64>, Vec<f64>) =
            ext.iter().filter(|e| played(&e.0)).filter_map(|(id, _, val, _)| Some((*self.theta.get(id)?, *val))).unzip();
        let sp = spearman(&a, &b).unwrap_or(f64::NAN);
        let n = TOP_OVERLAP;
        vec![name.into(), ext.len().to_string(), format!("{overlap}/{n}"), format!("{overlap_all}/{n}"), a.len().to_string(), f4(sp)]
    }
}

/// The plan-3 metric on both windows: NDCG and NDCG@3 of the `forecast` column
/// over in-person tournaments, weighted by the frozen tournament weights, with
/// the unweighted mean beside it. `--paired other.db` adds the paired bootstrap
/// standard error of the difference against that database, over the tournaments
/// both forecast.
fn ndcg_lines(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    let table = ndcg::weights(&arg("--weights").unwrap_or_else(|| ndcg::WEIGHTS.into()));
    let w = (!flag("--unweighted")).then_some(&table);
    // `--window FROM:TO` scores that one window in place of tune and test.
    let own: Option<(String, String)> =
        arg("--window").and_then(|w| w.split_once(':').map(|(a, b)| (a.to_string(), b.to_string())));
    let windows: Vec<(&str, (&str, &str))> = match &own {
        Some((a, b)) => vec![("window", (a.as_str(), b.as_str()))],
        None => vec![("tune", ndcg::TUNE), ("test", ndcg::TEST)],
    };
    let mut scored = Vec::new();
    for &(name, (from, to)) in &windows {
        let v = ndcg::per_tournament(conn, from, to)?;
        print_window(name, &v, w);
        scored.push(v);
    }
    if let Some(other) = arg("--paired") {
        let o = Connection::open_with_flags(&other, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        for (v, &(name, (from, to))) in scored.iter().zip(&windows) {
            let ov = ndcg::per_tournament(&o, from, to)?;
            let (se, d) = ndcg::paired_se(v, &ov, w, BOOT, |s| s.ndcg);
            let (se3, d3) = ndcg::paired_se(v, &ov, w, BOOT, |s| s.ndcg3);
            println!("paired {name} minus {other}: {d:+.4} se {se:.4}  @3 {d3:+.4} se {se3:.4}");
        }
    }
    Ok(())
}

fn print_window(name: &str, v: &[ndcg::Scored], w: Option<&HashMap<i64, f64>>) {
    // `--dump` is the four-decimal check against tools/ndcg_check.py.
    if flag("--dump") {
        for s in v {
            println!("{name}\t{}\t{:.6}\t{:.6}", s.tid, s.ndcg, s.ndcg3);
        }
    }
    let (a, b) = ndcg::summary(v, w);
    let (ua, ub) = ndcg::summary(v, None);
    println!("ndcg {name} {a:.4} @3 {b:.4}  unweighted {ua:.4} @3 {ub:.4}  ({} tournaments)", v.len());
}

fn print_title(db: &str, cfg_path: &str, cfg: &Config) {
    println!("# Rating report\n\n`{db}` with `{cfg_path}`");
    println!(
        "\naffinity rank {}, s_m {}, w_async {}, beta {}, sigma_model {}",
        cfg.question.aff_rank, cfg.question.s_m, cfg.data.w_async, cfg.team.beta, cfg.team.sigma_model
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg_path = arg("--config").unwrap_or_else(|| "config.toml".into());
    let cfg = Config::load(&cfg_path)?;
    let db = arg("--db").unwrap_or(cfg.data.out_path.clone());
    let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.execute_batch(&format!("ATTACH DATABASE '{}' AS buff", cfg.data.db_path))?;
    let report = Report::new(&conn, &cfg)?;
    // The forecast metric alone: what a parameter sweep pays for.
    if flag("--ndcg") {
        return ndcg_lines(&conn);
    }
    if flag("--brief") {
        report.brief()?;
        return ndcg_lines(&conn);
    }
    print_title(&db, &cfg_path, &cfg);
    report.print()
}
