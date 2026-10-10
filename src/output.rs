use crate::team_event::TeamEvent;
use rusqlite::{params, Connection};

/// Columns of `team_events`.
const TEAM_EVENT_COLS: usize = 25;

/// One row of `player_ratings`, as `Writer::player_ratings` documents it.
pub type PlayerRow = (i64, f64, f64, u32, f64, f64);

/// Rows of a rating published on its own scale, where the change is already
/// the move of the published number.
pub fn published(rows: &[(i64, f64, f64, u32, f64)]) -> Vec<PlayerRow> {
    rows.iter().map(|&(p, t, s, g, c)| (p, t, s, g, c, c)).collect()
}

pub struct Writer {
    pub conn: Connection,
}

/// One player's state around one tournament: what the update moved, and what the
/// forecast read before it.
pub struct PlayerEvent {
    pub player: i64,
    pub theta_before: f64,
    pub theta_after: f64,
    pub sigma_after: f64,
    /// The format effect, the uncertainty the forecast used, and how much of the
    /// player's evidence was in person on the morning of the tournament.
    pub phi_before: f64,
    pub sigma_before: f64,
    pub games_ip: i64,
    pub days_ip: i64,
}

/// One row of `tournament_metrics`: the field, how well the question model fits
/// it in sample and cross-fitted, and how well the forecasts ordered it.
pub struct TournamentMetrics<'a> {
    pub tid: i64,
    pub date: &'a str,
    pub kind: &'a str,
    pub weight: f64,
    pub n_teams: usize,
    pub n_q: usize,
    pub n_obs: u64,
    pub insample: Option<f64>,
    pub crossfit: Option<f64>,
    pub sp_forecast: Option<f64>,
    pub sp_old: Option<f64>,
    pub truedl: Option<f64>,
}

impl<'a> TournamentMetrics<'a> {
    /// A row with the field filled in and no fit or forecast measured yet.
    pub fn field(tid: i64, date: &'a str, kind: &'a str, weight: f64, n_teams: usize, n_q: usize) -> Self {
        let (insample, crossfit, sp_forecast, sp_old, truedl) = (None, None, None, None, None);
        TournamentMetrics { tid, date, kind, weight, n_teams, n_q, n_obs: 0, insample, crossfit, sp_forecast, sp_old, truedl }
    }
}

const SCHEMA: &str = "
CREATE TABLE releases (id INTEGER PRIMARY KEY, date TEXT, shift REAL, shift_cum REAL, n_players INTEGER);
CREATE TABLE player_ratings (release_id INTEGER, player_id INTEGER, theta REAL, sigma REAL, games_365 INTEGER, display REAL, change REAL, delta REAL);
CREATE TABLE team_ratings (release_id INTEGER, team_id INTEGER, mu REAL, sigma REAL, core_strength REAL, evidence REAL, display REAL, listed INTEGER, change REAL);
CREATE TABLE team_events (tournament_id INTEGER, date TEXT, team_id INTEGER, c REAL, delta_l REAL, x REAL, s_r REAL, forecast REAL, forecast_place REAL, actual_place REAL, s_pair REAL, i_pair REAL, mu_before REAL, mu_after REAL, sigma_before REAL, sigma_after REAL, evidence REAL, gap_days INTEGER, var_diff REAL, lost_prev REAL, rated INTEGER, z REAL, expected_place REAL, d_place REAL, d_pull REAL);
CREATE TABLE player_events (tournament_id INTEGER, player_id INTEGER, theta_before REAL, theta_after REAL, sigma_after REAL, phi_before REAL, sigma_before REAL, games_ip INTEGER, days_ip INTEGER);
CREATE TABLE question_params (tournament_id INTEGER, q INTEGER, b REAL, a REAL, m REAL);
CREATE TABLE tournament_metrics (tournament_id INTEGER PRIMARY KEY, date TEXT, type TEXT, weight REAL, n_teams INTEGER, n_q INTEGER, n_obs INTEGER, logloss_insample REAL, logloss_crossfit REAL, spearman_forecast REAL, spearman_old_system REAL, truedl REAL);
";

impl Writer {
    pub fn create(path: &str) -> rusqlite::Result<Writer> {
        if let Some(dir) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::remove_file(path);
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;")?;
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch("BEGIN")?;
        Ok(Writer { conn })
    }

    pub fn release(&self, id: i64, date: &str, shift: f64, cum: f64, n: usize) -> rusqlite::Result<()> {
        self.conn.execute("INSERT INTO releases VALUES (?1,?2,?3,?4,?5)", params![id, date, shift, cum, n as i64])?;
        Ok(())
    }

    /// `(player, theta, sigma, games in the window, change, delta)` of every
    /// listed player. `change` is the move of the raw strength alone, without
    /// the gauge; `delta` the move of the published number since its last row.
    pub fn player_ratings(&self, release_id: i64, rows: &[PlayerRow], disp: impl Fn(f64) -> f64) -> rusqlite::Result<()> {
        let mut st = self.conn.prepare_cached("INSERT INTO player_ratings VALUES (?1,?2,?3,?4,?5,?6,?7,?8)")?;
        for (pid, theta, sigma, games, change, delta) in rows {
            st.execute(params![release_id, pid, theta, sigma, games, disp(*theta), change, delta])?;
        }
        Ok(())
    }

    /// `(team, mu, sigma, core strength, evidence, change)` of every listed team.
    pub fn team_ratings(&self, release_id: i64, rows: &[(i64, f64, f64, f64, f64, f64)], disp: impl Fn(f64) -> f64) -> rusqlite::Result<()> {
        let mut st = self.conn.prepare_cached("INSERT INTO team_ratings VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8)")?;
        for (tid, mu, sigma, core, ev, change) in rows {
            st.execute(params![release_id, tid, mu, sigma, core, ev, disp(*mu), change])?;
        }
        Ok(())
    }

    /// NaN marks a value the engine does not have, and is stored as NULL.
    pub fn team_events(&self, tid: i64, date: &str, team_ids: &[i64], evs: &[TeamEvent]) -> rusqlite::Result<()> {
        let holes: Vec<String> = (1..=TEAM_EVENT_COLS).map(|i| format!("?{i}")).collect();
        let mut st = self.conn.prepare_cached(&format!("INSERT INTO team_events VALUES ({})", holes.join(",")))?;
        let known = |x: f64| x.is_finite().then_some(x);
        for e in evs {
            st.execute(params![
                tid, date, team_ids[e.team as usize], e.c, e.delta_l, e.x, known(e.s_r), e.forecast, e.forecast_place,
                e.actual_place, e.s_pair, e.i_pair, known(e.mu_before), known(e.mu_after), e.sigma_before, e.sigma_after,
                e.evidence, e.gap_days, e.var_diff, e.lost_prev, e.rated, e.z, e.expected_place, known(e.d_place), known(e.d_pull),
            ])?;
        }
        Ok(())
    }

    /// One row per player per tournament. The first three numbers are the state
    /// the update moved; the last four are the state the forecast read, which is
    /// what a residual diagnosis needs.
    pub fn player_events(&self, tid: i64, rows: &[PlayerEvent]) -> rusqlite::Result<()> {
        let mut st = self.conn.prepare_cached("INSERT INTO player_events VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)")?;
        for e in rows {
            let (before, after) = ((e.theta_before, e.phi_before, e.sigma_before), (e.theta_after, e.sigma_after));
            st.execute(params![tid, e.player, before.0, after.0, after.1, before.1, before.2, e.games_ip, e.days_ip])?;
        }
        Ok(())
    }

    pub fn question_params(&self, tid: i64, qs: &[(f64, f64, f64)]) -> rusqlite::Result<()> {
        let mut st = self.conn.prepare_cached("INSERT INTO question_params VALUES (?1,?2,?3,?4,?5)")?;
        for (q, (b, a, m)) in qs.iter().enumerate() {
            st.execute(params![tid, q as i64, b, a, m])?;
        }
        Ok(())
    }

    pub fn tournament_metrics(&self, m: &TournamentMetrics) -> rusqlite::Result<()> {
        let sql = "INSERT INTO tournament_metrics VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)";
        let counts = (m.n_teams as i64, m.n_q as i64, m.n_obs as i64);
        let fit = (m.insample, m.crossfit, m.sp_forecast, m.sp_old, m.truedl);
        self.conn.execute(sql, params![m.tid, m.date, m.kind, m.weight, counts.0, counts.1, counts.2, fit.0, fit.1, fit.2, fit.3, fit.4])?;
        Ok(())
    }

    pub fn checkpoint(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch("COMMIT; BEGIN")
    }

    pub fn finish(self) -> rusqlite::Result<()> {
        self.conn.execute_batch("COMMIT")?;
        // Keyed the way the site looks rows up: one entity's rows by release or
        // tournament, one tournament's row for a team. Each also serves a lookup
        // by its first column alone. Single-column versions of the same made a
        // player page scan every event of the player once per game it lists.
        self.conn.execute_batch(
            "CREATE INDEX idx_team_events_team ON team_events(team_id);
             CREATE INDEX idx_team_events_tt ON team_events(tournament_id, team_id);
             CREATE INDEX idx_team_ratings_rel ON team_ratings(release_id);
             CREATE INDEX idx_team_ratings_tr ON team_ratings(team_id, release_id);
             CREATE INDEX idx_player_ratings_pr ON player_ratings(player_id, release_id);
             CREATE INDEX idx_player_ratings_rel ON player_ratings(release_id);
             CREATE INDEX idx_player_events_pt ON player_events(player_id, tournament_id);",
        )?;
        Ok(())
    }
}
