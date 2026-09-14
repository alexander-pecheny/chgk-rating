use chgk_rating::config::{Config, TeamEngine};
use chgk_rating::ndcg;
use chgk_rating::team_ndcg as tn;
use chgk_rating::util::{csv_rows, spearman};
use rusqlite::Connection;
use std::collections::HashMap;

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

type Loss = (String, i64, i64, f64, f64);

const FORMATS: [&str; 3] = ["in-person", "sync", "strict-sync"];
/// Test 2 and the tuning grid both look at the last two seasons.
const WIN: (&str, &str) = ("2024-08-30", "2026-08-27");

/// Test 7 of DESIGN 7.3: does the team rating alone order an in-person field of
/// base-roster teams? `(weighted NDCG, weighted NDCG@3, tournaments)` over one
/// window. Needs `out/team_ndcg/qualify.csv`, which `tools/team_ndcg.py` writes.
fn team_ndcg(conn: &Connection, from: &str, to: &str) -> rusqlite::Result<(f64, f64, usize)> {
    let qual = tn::qualify(tn::QUALIFY);
    let w = tn::weights(tn::WEIGHTS);
    let mut st = conn.prepare(
        "SELECT e.tournament_id, e.team_id, e.mu_before, e.actual_place
         FROM team_events e JOIN tournament_metrics m ON m.tournament_id = e.tournament_id
         WHERE m.type = 'in-person' AND m.date >= ?1 AND m.date < ?2
           AND e.actual_place > 0 AND e.rated = 1 AND e.mu_before IS NOT NULL
         ORDER BY e.tournament_id",
    )?;
    let mut rows = st.query([from, to])?;
    let mut by_t: HashMap<i64, (Vec<i64>, Vec<f64>, Vec<f64>)> = HashMap::new();
    while let Some(r) = rows.next()? {
        let (tid, team): (i64, i64) = (r.get(0)?, r.get(1)?);
        if !qual.contains(&(tid, team)) {
            continue;
        }
        let e = by_t.entry(tid).or_default();
        e.0.push(team);
        e.1.push(r.get(2)?);
        e.2.push(r.get(3)?);
    }
    let (mut full, mut at3) = (HashMap::new(), HashMap::new());
    for (tid, (team, mu, place)) in &by_t {
        if let Some(v) = tn::score(*tid, team, mu, place, usize::MAX) {
            full.insert(*tid, v);
        }
        if let Some(v) = tn::score(*tid, team, mu, place, 3) {
            at3.insert(*tid, v);
        }
    }
    let n = full.len();
    Ok((tn::wmean(&full, &w), tn::wmean(&at3, &w), n))
}

fn fmt_key(f: &str) -> usize {
    FORMATS.iter().position(|x| *x == f).unwrap_or(9)
}

fn losses(conn: &Connection, from: &str, to: &str) -> rusqlite::Result<Vec<Loss>> {
    let mut st = conn.prepare(
        "SELECT type, COUNT(*), SUM(n_obs),
                SUM(logloss_insample*n_obs), SUM(logloss_crossfit*n_obs)
         FROM tournament_metrics
         WHERE date >= ?1 AND date < ?2 AND n_obs > 0
         GROUP BY type",
    )?;
    let rows = st.query_map([from, to], |r| {
        let obs: i64 = r.get(2)?;
        let d = obs.max(1) as f64;
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            obs,
            r.get::<_, f64>(3)? / d,
            r.get::<_, f64>(4)? / d,
        ))
    })?;
    let mut v: Vec<_> = rows.collect::<Result<_, _>>()?;
    v.sort_by_key(|r| fmt_key(&r.0));
    Ok(v)
}

fn loss_table(conn: &Connection, title: &str, from: &str, to: &str) -> rusqlite::Result<()> {
    println!("\n### {title} ({from} to {to})\n");
    println!("| format | tournaments | observations | cross-fitted | in-sample |");
    println!("|---|---:|---:|---:|---:|");
    let rows = losses(conn, from, to)?;
    let (mut n, mut obs, mut ins, mut cf) = (0, 0i64, 0.0, 0.0);
    for (f, nt, no, i, c) in &rows {
        println!("| {f} | {nt} | {no} | {c:.4} | {i:.4} |");
        n += nt;
        obs += no;
        ins += i * *no as f64;
        cf += c * *no as f64;
    }
    if obs > 0 {
        println!(
            "| **all** | {n} | {obs} | {:.4} | {:.4} |",
            cf / obs as f64,
            ins / obs as f64
        );
    }
    Ok(())
}

/// Mean per-tournament Spearman of two naive rankings, by format: `mu_before`
/// alone, and `x = mu_before + delta_l`, the rating with the lineup offset.
fn naive_spearman(conn: &Connection) -> rusqlite::Result<Vec<(String, f64, f64, i64)>> {
    let mut st = conn.prepare(
        "SELECT m.type, e.tournament_id, e.mu_before, e.mu_before + e.delta_l, e.actual_place
         FROM team_events e JOIN tournament_metrics m ON m.tournament_id = e.tournament_id
         WHERE m.date >= ?1 AND m.date < ?2 AND e.mu_before IS NOT NULL
         ORDER BY m.type, e.tournament_id",
    )?;
    let mut rows = st.query([WIN.0, WIN.1])?;
    let mut acc: Vec<(String, f64, f64, i64)> = Vec::new();
    let (mut cur, mut kind) = (-1i64, String::new());
    let (mut mu, mut x, mut pl) = (Vec::new(), Vec::new(), Vec::new());
    let flush = |kind: &str,
                 mu: &mut Vec<f64>,
                 x: &mut Vec<f64>,
                 pl: &mut Vec<f64>,
                 acc: &mut Vec<(String, f64, f64, i64)>| {
        if let (Some(a), Some(b)) = (spearman(mu, pl), spearman(x, pl)) {
            match acc.iter_mut().find(|e| e.0 == kind) {
                Some(e) => {
                    e.1 += a;
                    e.2 += b;
                    e.3 += 1;
                }
                None => acc.push((kind.to_string(), a, b, 1)),
            }
        }
        mu.clear();
        x.clear();
        pl.clear();
    };
    while let Some(r) = rows.next()? {
        let tid: i64 = r.get(1)?;
        if tid != cur {
            flush(&kind, &mut mu, &mut x, &mut pl, &mut acc);
            cur = tid;
            kind = r.get(0)?;
        }
        mu.push(-r.get::<_, f64>(2)?);
        x.push(-r.get::<_, f64>(3)?);
        pl.push(r.get::<_, f64>(4)?);
    }
    flush(&kind, &mut mu, &mut x, &mut pl, &mut acc);
    for e in acc.iter_mut() {
        e.1 /= e.3 as f64;
        e.2 /= e.3 as f64;
    }
    acc.sort_by_key(|r| fmt_key(&r.0));
    Ok(acc)
}

/// Lag-1 correlation of a team's successive pairwise innovations. A well-tuned
/// filter has white innovations; a sluggish one leaves them positively correlated.
fn innovation_acf(conn: &Connection, extra: &str) -> rusqlite::Result<(usize, f64)> {
    let mut st = conn.prepare(&format!(
        "SELECT e.team_id, e.s_pair / MAX(e.i_pair, 1e-9)
         FROM team_events e JOIN tournament_metrics m ON m.tournament_id = e.tournament_id
         WHERE m.date >= ?1 AND m.date < ?2 AND e.rated = 1 AND e.evidence >= 20 {extra}
         ORDER BY e.team_id, m.date, e.tournament_id"
    ))?;
    let mut rows = st.query([WIN.0, WIN.1])?;
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let (mut cur, mut prev) = (-1i64, 0.0);
    while let Some(r) = rows.next()? {
        let (team, v): (i64, f64) = (r.get(0)?, r.get(1)?);
        if team == cur {
            a.push(prev);
            b.push(v);
        }
        cur = team;
        prev = v;
    }
    let n = a.len();
    if n < 2 {
        return Ok((0, f64::NAN));
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / n as f64;
    let (ma, mb) = (mean(&a), mean(&b));
    let cov: f64 = a.iter().zip(&b).map(|(u, v)| (u - ma) * (v - mb)).sum();
    let va: f64 = a.iter().map(|u| (u - ma) * (u - ma)).sum();
    let vb: f64 = b.iter().map(|v| (v - mb) * (v - mb)).sum();
    Ok((n, cov / (va * vb).sqrt()))
}

/// Whether the database carries a row per release or only the last one.
/// `rows_every = 0` keeps the last release only, and every section that reads an
/// earlier release has nothing to read.
fn has_history(conn: &Connection) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT release_id) FROM team_ratings",
        [],
        |r| r.get(0),
    )?;
    Ok(n > 1)
}

const NO_HISTORY: &str = "n/a (rows only at the last release)";

/// Test 4: the display value of the 50th team each September, which needs a row
/// per release.
fn inflation(conn: &Connection, prov: f64) -> rusqlite::Result<()> {
    println!("| release | date | rated teams | display of 50th |");
    println!("|---:|---|---:|---:|");
    let mut find =
        conn.prepare("SELECT id, date FROM releases WHERE date LIKE ?1 ORDER BY date LIMIT 1")?;
    let mut nth = conn.prepare(
        "SELECT display FROM team_ratings WHERE release_id = ?1 AND sigma <= ?2
         ORDER BY mu DESC LIMIT 1 OFFSET 49",
    )?;
    let mut cnt =
        conn.prepare("SELECT COUNT(*) FROM team_ratings WHERE release_id = ?1 AND sigma <= ?2")?;
    let mut band: Vec<f64> = Vec::new();
    for y in 2015..=2026 {
        let Ok((id, date)) = find.query_row([format!("{y}-09%")], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        }) else {
            continue;
        };
        let n: i64 = cnt.query_row(rusqlite::params![id, prov], |r| r.get(0))?;
        let mu: Option<f64> = nth
            .query_row(rusqlite::params![id, prov], |r| r.get(0))
            .ok();
        if let Some(m) = mu {
            // A September with almost no rated teams is a hole in the data, not inflation.
            if n >= 200 {
                band.push(m);
            }
            println!("| {id} | {date} | {n} | {m:.0} |");
        }
    }
    if !band.is_empty() {
        let mean = band.iter().sum::<f64>() / band.len() as f64;
        let dev = band
            .iter()
            .map(|v| ((v - mean) / mean).abs())
            .fold(0.0, f64::max);
        println!(
            "\nOver the {} Septembers with at least 200 rated teams: mean {mean:.0}, largest deviation {:.1}% -- {}.",
            band.len(),
            100.0 * dev,
            if dev <= 0.10 {
                "**within the +/-10% band**"
            } else {
                "**outside the +/-10% band**"
            }
        );
    }
    Ok(())
}

/// The two teams of the falling-too-slowly case: where they stand and how far
/// three tournaments moved them, in display points.
fn case_rows(
    conn: &Connection,
    b_team: f64,
    history: bool,
) -> rusqlite::Result<Vec<(i64, Vec<String>)>> {
    let mut near = conn
        .prepare("SELECT id FROM releases ORDER BY ABS(julianday(date)-julianday(?1)) LIMIT 1")?;
    let last: i64 = conn.query_row("SELECT MAX(id) FROM releases", [], |r| r.get(0))?;
    let mut rel = vec![last, last, last];
    if history {
        for (i, d) in ["2025-09-05", "2026-03-05"].iter().enumerate() {
            rel[i] = near.query_row([d], |r| r.get(0))?;
        }
    }
    let mut disp =
        conn.prepare("SELECT display FROM team_ratings WHERE release_id = ?1 AND team_id = ?2")?;
    let mut step = conn.prepare(
        "SELECT mu_after - mu_before FROM team_events WHERE team_id = ?1 AND tournament_id = ?2",
    )?;
    let mut out = Vec::new();
    for team in [45556i64, 86405] {
        let mut cells = Vec::new();
        for (i, r) in rel.iter().enumerate() {
            if !history && i < 2 {
                cells.push("n/a".into());
                continue;
            }
            let v: Option<f64> = disp
                .query_row(rusqlite::params![r, team], |x| x.get(0))
                .ok();
            cells.push(v.map_or("-".into(), |v| format!("{v:.0}")));
        }
        for t in [13917i64, 12826, 13444] {
            let v: Option<f64> = step
                .query_row(rusqlite::params![team, t], |x| x.get::<_, Option<f64>>(0))
                .ok()
                .flatten();
            cells.push(v.map_or("-".into(), |v| format!("{:+.0}", v * b_team)));
        }
        out.push((team, cells));
    }
    Ok(out)
}

fn violations(conn: &Connection) -> rusqlite::Result<(i64, i64)> {
    conn.query_row(
        "SELECT SUM(e.actual_place = 1.0), SUM(e.actual_place > 1.0)
         FROM team_events e
         JOIN (SELECT tournament_id, MIN(actual_place) mp FROM team_events
               GROUP BY tournament_id) g
           ON g.tournament_id = e.tournament_id AND e.actual_place = g.mp
         WHERE e.mu_after < e.mu_before - 1e-12",
        [],
        |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                r.get::<_, Option<i64>>(1)?.unwrap_or(0),
            ))
        },
    )
}

/// Variance of the standardised residual; 1 means the published sigma is honest.
fn zvar(conn: &Connection, extra: &str) -> rusqlite::Result<(i64, f64, f64)> {
    conn.query_row(
        &format!(
            "SELECT COUNT(*), AVG(z), AVG(z*z) FROM team_events e
             JOIN tournament_metrics m ON m.tournament_id = e.tournament_id
             WHERE e.z IS NOT NULL AND m.type='in-person'
               AND m.date >= '2018-01-01' AND e.evidence >= 6 {extra}"
        ),
        [],
        |r| {
            let n: i64 = r.get(0)?;
            if n == 0 {
                return Ok((0, 0.0, 0.0));
            }
            let (m, m2): (f64, f64) = (r.get(1)?, r.get(2)?);
            Ok((n, m, m2 - m * m))
        },
    )
}

fn mean_forecast(conn: &Connection) -> rusqlite::Result<f64> {
    conn.query_row(
        "SELECT AVG(spearman_forecast) FROM tournament_metrics
         WHERE type='in-person' AND date >= ?1 AND date < ?2 AND spearman_forecast IS NOT NULL",
        [WIN.0, WIN.1],
        |r| Ok(r.get::<_, Option<f64>>(0)?.unwrap_or(0.0)),
    )
}

/// One external player table: id -> (place, rating, games).
fn external(
    path: &str,
    id: usize,
    place: usize,
    val: usize,
    games: Option<usize>,
) -> Vec<(i64, f64, f64, f64)> {
    let mut v: Vec<(i64, f64, f64, f64)> = csv_rows(path)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| {
            Some((
                r.get(id)?.trim().parse().ok()?,
                r.get(place)?.trim().parse().ok()?,
                r.get(val)?.trim().parse().ok()?,
                games.map_or(f64::INFINITY, |g| {
                    r.get(g).and_then(|x| x.trim().parse().ok()).unwrap_or(0.0)
                }),
            ))
        })
        .collect();
    v.sort_by(|a, b| a.1.total_cmp(&b.1));
    v
}

/// The plan-3 metric on both windows: NDCG and NDCG@3 of the `forecast` column
/// over in-person tournaments, weighted by the frozen tournament weights, with
/// the unweighted mean beside it. `--paired other.db` adds the paired bootstrap
/// standard error of the difference against that database, over the tournaments
/// both forecast.
fn ndcg_lines(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    let table = ndcg::weights(&arg("--weights").unwrap_or_else(|| ndcg::WEIGHTS.into()));
    let w = (!flag("--unweighted")).then_some(&table);
    let windows = [("tune", ndcg::TUNE), ("test", ndcg::TEST)];
    let mut scored = Vec::new();
    for (name, (from, to)) in windows {
        let v = ndcg::per_tournament(conn, from, to)?;
        // `--dump` is the four-decimal check against tools/ndcg_check.py.
        if flag("--dump") {
            for s in &v {
                println!("{name}\t{}\t{:.6}\t{:.6}", s.tid, s.ndcg, s.ndcg3);
            }
        }
        let (a, b) = ndcg::summary(&v, w);
        let (ua, ub) = ndcg::summary(&v, None);
        println!(
            "ndcg {name} {a:.4} @3 {b:.4}  unweighted {ua:.4} @3 {ub:.4}  ({} tournaments)",
            v.len()
        );
        scored.push((name, v));
    }
    if let Some(other) = arg("--paired") {
        let o = Connection::open_with_flags(&other, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        for ((name, v), (_, (from, to))) in scored.iter().zip(windows) {
            let ov = ndcg::per_tournament(&o, from, to)?;
            let (se, d) = ndcg::paired_se(v, &ov, w, 2000, |s| s.ndcg);
            let (se3, d3) = ndcg::paired_se(v, &ov, w, 2000, |s| s.ndcg3);
            println!("paired {name} minus {other}: {d:+.4} se {se:.4}  @3 {d3:+.4} se {se3:.4}");
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg_path = arg("--config").unwrap_or_else(|| "config.toml".into());
    let cfg = Config::load(&cfg_path)?;
    let db = arg("--db").unwrap_or(cfg.data.out_path.clone());
    let conn = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.execute_batch(&format!("ATTACH DATABASE '{}' AS buff", cfg.data.db_path))?;
    let prov = cfg.display.provisional_sigma;
    // S2 keeps no variance, so everything the uncertainty calibration reads is a
    // placeholder under it.
    let kalman = cfg.team.engine == TeamEngine::Kalman;
    // `rows_every = 0` keeps per-entity rows at the last release only.
    let history = has_history(&conn)?;

    // The forecast metric alone: what a parameter sweep pays for.
    if flag("--ndcg") {
        return ndcg_lines(&conn);
    }

    if flag("--brief") {
        let (sole, _shared) = violations(&conn)?;
        let (n, prov_n): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), SUM(sigma > ?1) FROM team_ratings
             WHERE release_id = (SELECT MAX(release_id) FROM team_ratings)",
            [prov],
            |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
        )?;
        let (_, _, v) = zvar(&conn, "")?;
        let (_, _, vr) = zvar(&conn, "AND e.gap_days >= 365")?;
        let (_, _, vd) = zvar(&conn, "AND e.lost_prev > 0.3")?;
        println!(
            "spearman {:.4}  rated {:.0}%  zvar {v:.3} return {vr:.3} depart {vd:.3}  violations {sole}",
            mean_forecast(&conn)?,
            100.0 * (n - prov_n) as f64 / n.max(1) as f64
        );
        let naive = naive_spearman(&conn)?;
        let col = |f: &str, i: usize| {
            naive
                .iter()
                .find(|e| e.0 == f)
                .map_or(f64::NAN, |e| if i == 0 { e.1 } else { e.2 })
        };
        let (_, ip) = innovation_acf(&conn, "AND m.type='in-person'")?;
        let (_, af) = innovation_acf(&conn, "")?;
        println!(
            "naive mu {:.4}/{:.4}  x {:.4}/{:.4} (in-person/sync)  innovation acf {ip:.3} in-person, {af:.3} all",
            col("in-person", 0),
            col("sync", 0),
            col("in-person", 1),
            col("sync", 1)
        );
        let (ft, t3, nt) = team_ndcg(&conn, tn::TUNE.0, tn::TUNE.1)?;
        let (fs, s3, ns) = team_ndcg(&conn, tn::TEST.0, tn::TEST.1)?;
        println!(
            "team ndcg (weighted, mu only) tune {ft:.4}/@3 {t3:.4} over {nt}  test {fs:.4}/@3 {s3:.4} over {ns}"
        );
        for (team, cells) in case_rows(&conn, cfg.display.b_team, history)? {
            println!("team {team}: {}", cells.join(" "));
        }
        return ndcg_lines(&conn);
    }

    println!("# Rating report\n\n`{db}` with `{cfg_path}`");
    println!(
        "\naffinity rank {}, s_m {}, w_async {}, beta {}, sigma_model {}",
        cfg.question.aff_rank,
        cfg.question.s_m,
        cfg.data.w_async,
        cfg.team.beta,
        cfg.team.sigma_model
    );

    loss_table(
        &conn,
        "Log-loss, season 2025/26",
        "2025-08-29",
        "2026-08-27",
    )?;
    loss_table(&conn, "Log-loss, all seasons", "0000", "9999")?;

    // Test 2.
    println!(
        "\n### Forecast rank correlation, {} to {} (test 2)\n",
        WIN.0, WIN.1
    );
    println!("| format | tournaments | ours | old system | naive (mu only) | naive (x) |");
    println!("|---|---:|---:|---:|---:|---:|");
    let naive = naive_spearman(&conn)?;
    let mut st = conn.prepare(
        "SELECT type, COUNT(spearman_forecast), AVG(spearman_forecast), AVG(spearman_old_system)
         FROM tournament_metrics WHERE date >= ?1 AND date < ?2 GROUP BY type",
    )?;
    let mut rows = st.query([WIN.0, WIN.1])?;
    let mut all: Vec<(String, i64, f64, f64)> = Vec::new();
    while let Some(r) = rows.next()? {
        all.push((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?));
    }
    all.sort_by_key(|r| fmt_key(&r.0));
    for (f, n, ours, old) in &all {
        let (nv, nx) = naive
            .iter()
            .find(|e| &e.0 == f)
            .map_or((f64::NAN, f64::NAN), |e| (e.1, e.2));
        println!("| {f} | {n} | {ours:.4} | {old:.4} | {nv:.4} | {nx:.4} |");
    }

    println!("\n### Innovation autocorrelation\n");
    println!(
        "Lag 1 over teams with evidence >= 20, {} to {}. Zero is a white innovation.\n",
        WIN.0, WIN.1
    );
    println!("| formats | consecutive pairs | correlation |");
    println!("|---|---:|---:|");
    for (name, extra) in [("in-person", "AND m.type='in-person'"), ("all", "")] {
        let (n, c) = innovation_acf(&conn, extra)?;
        println!("| {name} | {n} | {c:.3} |");
    }

    println!("\n### Case teams\n");
    println!("Display value at three releases, then the display step at three tournaments.\n");
    println!("| team | 2025-09 | 2026-03 | last | 13917 | 12826 | 13444 |");
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for (team, cells) in case_rows(&conn, cfg.display.b_team, history)? {
        println!("| {team} | {} |", cells.join(" | "));
    }
    if !history {
        println!("\nThe 2025-09 and 2026-03 columns are {NO_HISTORY}.");
    }

    // Test 6.
    let (sole, shared) = violations(&conn)?;
    println!("\n### Winner rule (test 6)\n");
    println!(
        "Teams finishing first, sole or shared, with `mu_after < mu_before`: **{}**.",
        sole + shared
    );
    println!("(sole first {sole}, shared first {shared})");

    println!("\n### Uncertainty calibration\n");
    if !kalman {
        println!("n/a: `team.engine = \"s2\"` publishes no variance to calibrate.");
    } else {
        println!("In-person from 2018, evidence >= 6. Variance of 1 means sigma is honest.");
        println!("\n| group | n | mean z | var z |");
        println!("|---|---:|---:|---:|");
        for (name, extra) in [
            ("all (drives nu)", ""),
            ("returning after >= 1 year (tau_T)", "AND e.gap_days >= 365"),
            (
                "after losing >= 30% of evidence (gamma)",
                "AND e.lost_prev > 0.3",
            ),
        ] {
            let (n, m, v) = zvar(&conn, extra)?;
            println!("| {name} | {n} | {m:.3} | {v:.3} |");
        }
    }

    // Test 4.
    println!("\n### Inflation of the 50th team, September releases (test 4)\n");
    if history {
        inflation(&conn, prov)?;
    } else {
        println!("{NO_HISTORY}.");
    }

    // Test 5.
    if let (Some(other), Some(date)) = (arg("--swap-db"), arg("--swap-date")) {
        println!("\n### Transfer invariance (test 5)\n");
        conn.execute_batch(&format!("ATTACH DATABASE '{other}' AS sw"))?;
        let (rid, rdate): (i64, String) = conn.query_row(
            "SELECT id, date FROM releases WHERE date >= ?1 ORDER BY date LIMIT 1",
            [&date],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let moved = history
            .then(|| {
                conn.query_row(
                    "SELECT COUNT(*), COALESCE(MAX(ABS(a.mu-b.mu)),0) FROM team_ratings a
                     JOIN sw.team_ratings b ON b.release_id=a.release_id AND b.team_id=a.team_id
                     WHERE a.release_id=?1 AND ABS(a.mu-b.mu) > 1e-9",
                    [rid],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)),
                )
            })
            .transpose()?;
        let fc: i64 = conn.query_row(
            "SELECT COUNT(*) FROM team_events a
             JOIN sw.team_events b ON b.tournament_id=a.tournament_id AND b.team_id=a.team_id
             JOIN tournament_metrics m ON m.tournament_id=a.tournament_id
             WHERE m.date >= ?1 AND ABS(a.forecast-b.forecast) > 1e-9",
            [&date],
            |r| r.get(0),
        )?;
        println!("Swap date {date}, first release after it: {rid} of {rdate}.");
        match moved {
            Some((n, mx)) => {
                println!("Teams whose `mu` moved: **{n}** (max |delta| {mx:.2e}).")
            }
            None => println!("Teams whose `mu` moved: {NO_HISTORY}."),
        }
        println!("Team events on or after the swap whose forecast moved: {fc}.");
    }

    let (rid, rdate): (i64, String) = conn.query_row(
        "SELECT id, date FROM releases ORDER BY id DESC LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    // Top teams.
    let (nteams, nprov): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), SUM(sigma > ?2) FROM team_ratings WHERE release_id = ?1",
        rusqlite::params![rid, prov],
        |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
    )?;
    println!("\n### Top 10 teams, release {rid} of {rdate}\n");
    println!("{nteams} teams listed, {nprov} of them provisional (sigma > {prov}) and left out.");
    println!("\n| # | team | mu | sigma | evidence | core | s_r of last lineup |");
    println!("|---:|---|---:|---:|---:|---:|---:|");
    let mut st = conn.prepare(
        "SELECT r.team_id, r.mu, r.sigma, r.evidence, r.core_strength,
                (SELECT e.s_r FROM team_events e WHERE e.team_id=r.team_id AND e.s_r IS NOT NULL
                 ORDER BY e.rowid DESC LIMIT 1),
                COALESCE((SELECT NULLIF(TRIM(t.name), '') FROM buff.teams t WHERE t.id=r.team_id),
                         (SELECT x.team_current_name FROM buff.tournament_results x
                          WHERE x.team_id=r.team_id ORDER BY x.id DESC LIMIT 1))
         FROM team_ratings r WHERE r.release_id = ?1 AND r.sigma <= ?2
         ORDER BY r.mu DESC LIMIT 10",
    )?;
    let mut rows = st.query(rusqlite::params![rid, prov])?;
    let mut i = 0;
    while let Some(r) = rows.next()? {
        i += 1;
        println!(
            "| {i} | {} | {:.3} | {:.3} | {:.1} | {:.3} | {} |",
            r.get::<_, Option<String>>(6)?.unwrap_or_else(|| "?".into()),
            r.get::<_, f64>(1)?,
            r.get::<_, f64>(2)?,
            r.get::<_, f64>(3)?,
            r.get::<_, f64>(4)?,
            r.get::<_, Option<f64>>(5)?
                .map_or("-".into(), |v| format!("{v:.3}"))
        );
    }

    // Where a top team's uncertainty comes from.
    if !kalman {
        println!("\n### Uncertainty budget of the top 20 teams, trailing year\n");
        println!("n/a under `team.engine = \"s2\"`.");
    } else {
            println!("\n### Uncertainty budget of the top 20 teams, trailing year\n");
        println!(
            "Variance per rated event: removed by results, re-added by diffusion and by departures.\n"
        );
        println!("| # | team | sigma | events | -var results | +var diffusion | +var departures |");
        println!("|---:|---|---:|---:|---:|---:|---:|");
        let mut st = conn.prepare(
            "WITH e AS (
               SELECT te.team_id AS tid, te.sigma_before AS sb, te.sigma_after AS sa,
                      te.var_diff AS vd,
                      LAG(te.sigma_after) OVER (PARTITION BY te.team_id ORDER BY te.rowid) AS pa
               FROM team_events te JOIN tournament_metrics m ON m.tournament_id = te.tournament_id
               WHERE m.date >= date(?3, '-365 day') AND te.rated = 1
             )
             SELECT r.team_id, r.sigma, COUNT(*),
                    AVG(sb*sb - sa*sa), AVG(vd),
                    AVG(sb*sb - pa*pa - vd),
                    COALESCE((SELECT NULLIF(TRIM(t.name), '') FROM buff.teams t WHERE t.id = r.team_id),
                             (SELECT x.team_current_name FROM buff.tournament_results x
                              WHERE x.team_id = r.team_id ORDER BY x.id DESC LIMIT 1))
             FROM team_ratings r JOIN e ON e.tid = r.team_id AND e.pa IS NOT NULL
             WHERE r.release_id = ?1 AND r.sigma <= ?2
             GROUP BY r.team_id ORDER BY r.mu DESC LIMIT 20",
        )?;
        let mut rows = st.query(rusqlite::params![rid, prov, rdate])?;
        let mut i = 0;
        while let Some(r) = rows.next()? {
            i += 1;
            println!(
                "| {i} | {} | {:.3} | {} | {:.5} | {:.5} | {:.5} |",
                r.get::<_, Option<String>>(6)?.unwrap_or_else(|| "?".into()),
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, f64>(4)?,
                r.get::<_, f64>(5)?
            );
        }

    }

    // Synergy.
    let mut st = conn.prepare(
        "SELECT r.mu - (SELECT e.s_r FROM team_events e WHERE e.team_id=r.team_id
                        AND e.s_r IS NOT NULL ORDER BY e.rowid DESC LIMIT 1)
         FROM team_ratings r WHERE r.release_id = ?1 AND r.sigma <= ?2",
    )?;
    let mut d: Vec<f64> = st
        .query_map(rusqlite::params![rid, prov], |r| r.get::<_, Option<f64>>(0))?
        .filter_map(|v| v.ok().flatten())
        .collect();
    d.sort_by(f64::total_cmp);
    if !d.is_empty() {
        let q = |f: f64| d[((d.len() - 1) as f64 * f) as usize];
        println!("\n### Synergy: mu minus the roster estimate of the last lineup\n");
        println!(
            "Listed non-provisional teams at release {rid}: {}.\n",
            d.len()
        );
        println!("| min | q1 | median | q3 | max | mean |");
        println!("|---:|---:|---:|---:|---:|---:|");
        println!(
            "| {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
            d[0],
            q(0.25),
            q(0.5),
            q(0.75),
            d[d.len() - 1],
            d.iter().sum::<f64>() / d.len() as f64
        );
    }

    // Test 3.
    let mut st = conn.prepare(
        "SELECT player_id, theta FROM player_ratings
         WHERE release_id = ?1 AND sigma <= ?2 ORDER BY theta DESC",
    )?;
    let ours: Vec<(i64, f64)> = st
        .query_map(rusqlite::params![rid, prov], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let ours_top: Vec<i64> = ours.iter().take(200).map(|(i, _)| *i).collect();
    let mut all_top = conn.prepare(
        "SELECT player_id FROM player_ratings WHERE release_id = ?1 ORDER BY theta DESC LIMIT 200",
    )?;
    let ours_top_all: Vec<i64> = all_top
        .query_map([rid], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let theta: HashMap<i64, f64> = ours.iter().cloned().collect();
    let quest = external("data/external/chgk_quest/players_all.csv", 1, 0, 4, Some(6));
    let fun = external(
        "data/external/chgk_fun/players_rating_release263.csv",
        1,
        0,
        3,
        None,
    );
    let qgames: HashMap<i64, f64> = quest.iter().map(|(i, _, _, g)| (*i, *g)).collect();
    println!("\n### Agreement with external systems (test 3)\n");
    println!(
        "Our {} non-provisional players at release {rid}.\n",
        ours.len()
    );
    println!(
        "| source | entries | top-200 overlap | same, provisional kept | common (>= 50 games) | spearman |"
    );
    println!("|---|---:|---:|---:|---:|---:|");
    for (name, ext) in [("chgk.quest", &quest), ("rating.chgk.fun", &fun)] {
        let top: Vec<i64> = ext.iter().take(200).map(|(i, _, _, _)| *i).collect();
        let overlap = top.iter().filter(|i| ours_top.contains(i)).count();
        let overlap_all = top.iter().filter(|i| ours_top_all.contains(i)).count();
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for (id, _, val, _) in ext.iter() {
            if qgames.get(id).copied().unwrap_or(0.0) < 50.0 {
                continue;
            }
            if let Some(t) = theta.get(id) {
                a.push(*t);
                b.push(*val);
            }
        }
        let sp = spearman(&a, &b).unwrap_or(f64::NAN);
        println!(
            "| {name} | {} | {overlap}/200 | {overlap_all}/200 | {} | {sp:.4} |",
            ext.len(),
            a.len()
        );
    }
    println!("\nThe games filter uses chgk.quest's career count, the only one available.");

    // Display scale.
    println!("\n### Display scale (DESIGN 5)\n");
    println!("| release | table | 1 | 10 | 50 | 100 | 500 | 1000 |");
    println!("|---|---|---:|---:|---:|---:|---:|---:|");
    let mut at = conn.prepare(
        "SELECT display FROM team_ratings WHERE release_id=?1 AND sigma<=?2
         ORDER BY mu DESC LIMIT 1 OFFSET ?3",
    )?;
    let mut ap = conn.prepare(
        "SELECT display FROM player_ratings WHERE release_id=?1 AND sigma<=?2
         ORDER BY theta DESC LIMIT 1 OFFSET ?3",
    )?;
    let near = |d: &str| -> rusqlite::Result<i64> {
        conn.query_row(
            "SELECT id FROM releases ORDER BY ABS(julianday(date)-julianday(?1)) LIMIT 1",
            [d],
            |r| r.get(0),
        )
    };
    let mut rels = Vec::new();
    if history {
        rels.push(("2019", near("2019-09-05")?, near("2019-10-03")?));
    }
    rels.push(("latest", rid, rid));
    for (label, rt, rp) in rels {
        for (what, st, r) in [("teams", &mut at, rt), ("players", &mut ap, rp)] {
            let mut line = format!("| {label} | {what} |");
            for n in [1i64, 10, 50, 100, 500, 1000] {
                let v: Option<f64> = st
                    .query_row(rusqlite::params![r, prov, n - 1], |x| x.get(0))
                    .ok();
                line += &v.map_or(" - |".into(), |v| format!(" {v:.0} |"));
            }
            println!("{line}");
        }
    }
    if !history {
        println!("\nThe 2019 rows are {NO_HISTORY}.");
    }

    // Top players.
    let (npl, npprov): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), SUM(sigma > ?2) FROM player_ratings WHERE release_id = ?1",
        rusqlite::params![rid, prov],
        |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
    )?;
    println!("\n### Top 50 players, release {rid} of {rdate}\n");
    println!("{npl} players listed, {npprov} of them provisional (sigma > {prov}) and left out.");
    println!("\n| # | player | theta | sigma | games 365d |");
    println!("|---:|---|---:|---:|---:|");
    let mut st = conn.prepare(
        "SELECT trim(p.surname || ' ' || coalesce(p.name,'')), r.theta, r.sigma, r.games_365
         FROM player_ratings r LEFT JOIN buff.players p ON p.id = r.player_id
         WHERE r.release_id = ?1 AND r.sigma <= ?2 ORDER BY r.theta DESC LIMIT 50",
    )?;
    let mut rows = st.query(rusqlite::params![rid, prov])?;
    let mut i = 0;
    while let Some(r) = rows.next()? {
        i += 1;
        println!(
            "| {i} | {} | {:.3} | {:.3} | {} |",
            r.get::<_, Option<String>>(0)?.unwrap_or_else(|| "?".into()),
            r.get::<_, f64>(1)?,
            r.get::<_, f64>(2)?,
            r.get::<_, i64>(3)?
        );
    }

    println!("\n### Listed players and teams per release\n");
    println!("| release | date | players | shift | cumulative shift |");
    println!("|---:|---|---:|---:|---:|");
    let mut st = conn.prepare(
        "SELECT id, date, n_players, shift, shift_cum FROM releases
         WHERE id % ?1 = 0 OR id = ?2 ORDER BY id",
    )?;
    let stride = (rid / 12).max(1);
    let mut rows = st.query([stride, rid])?;
    while let Some(r) = rows.next()? {
        println!(
            "| {} | {} | {} | {:.4} | {:.4} |",
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, f64>(3)?,
            r.get::<_, f64>(4)?
        );
    }
    Ok(())
}
