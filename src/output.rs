use rusqlite::{params, Connection};

pub struct Writer {
    pub conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE releases (id INTEGER PRIMARY KEY, date TEXT, shift REAL, shift_cum REAL, n_players INTEGER);
CREATE TABLE player_ratings (release_id INTEGER, player_id INTEGER, theta REAL, sigma REAL, games_365 INTEGER, display REAL, change REAL);
CREATE TABLE team_ratings (release_id INTEGER, team_id INTEGER, mu REAL, sigma REAL, core_strength REAL, evidence REAL, display REAL, listed INTEGER, change REAL);
CREATE TABLE team_events (tournament_id INTEGER, date TEXT, team_id INTEGER, c REAL, delta_l REAL, x REAL, s_r REAL, forecast REAL, forecast_place REAL, actual_place REAL, s_pair REAL, i_pair REAL, mu_before REAL, mu_after REAL, sigma_before REAL, sigma_after REAL, evidence REAL, gap_days INTEGER, var_diff REAL, lost_prev REAL, rated INTEGER, z REAL, expected_place REAL);
CREATE TABLE player_events (tournament_id INTEGER, player_id INTEGER, theta_before REAL, theta_after REAL, sigma_after REAL);
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

    pub fn release(
        &self,
        id: i64,
        date: &str,
        shift: f64,
        cum: f64,
        n: usize,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO releases VALUES (?1,?2,?3,?4,?5)",
            params![id, date, shift, cum, n as i64],
        )?;
        Ok(())
    }

    pub fn player_ratings(
        &self,
        release_id: i64,
        rows: &[(i64, f64, f64, u32, f64)],
        disp: impl Fn(f64) -> f64,
    ) -> rusqlite::Result<()> {
        let mut st = self
            .conn
            .prepare_cached("INSERT INTO player_ratings VALUES (?1,?2,?3,?4,?5,?6,?7)")?;
        for (pid, theta, sigma, games, change) in rows {
            st.execute(params![
                release_id,
                pid,
                theta,
                sigma,
                games,
                disp(*theta),
                change
            ])?;
        }
        Ok(())
    }

    pub fn team_ratings(
        &self,
        release_id: i64,
        rows: &[(i64, f64, f64, f64, f64, f64)],
        disp: impl Fn(f64) -> f64,
    ) -> rusqlite::Result<()> {
        let mut st = self
            .conn
            .prepare_cached("INSERT INTO team_ratings VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8)")?;
        for (tid, mu, sigma, core, ev, change) in rows {
            st.execute(params![
                release_id,
                tid,
                mu,
                sigma,
                core,
                ev,
                disp(*mu),
                change
            ])?;
        }
        Ok(())
    }

    pub fn team_events(
        &self,
        tid: i64,
        date: &str,
        team_ids: &[i64],
        evs: &[crate::team_model::TeamEvent],
    ) -> rusqlite::Result<()> {
        let mut st = self.conn.prepare_cached(
            "INSERT INTO team_events VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
        )?;
        for e in evs {
            st.execute(params![
                tid,
                date,
                team_ids[e.team as usize],
                e.c,
                e.delta_l,
                e.x,
                e.s_r.is_finite().then_some(e.s_r),
                e.forecast,
                e.forecast_place,
                e.actual_place,
                e.s_pair,
                e.i_pair,
                e.mu_before.is_finite().then_some(e.mu_before),
                e.mu_after.is_finite().then_some(e.mu_after),
                e.sigma_before,
                e.sigma_after,
                e.evidence,
                e.gap_days,
                e.var_diff,
                e.lost_prev,
                e.rated,
                e.z,
                e.expected_place
            ])?;
        }
        Ok(())
    }

    pub fn player_events(&self, tid: i64, rows: &[(i64, f64, f64, f64)]) -> rusqlite::Result<()> {
        let mut st = self
            .conn
            .prepare_cached("INSERT INTO player_events VALUES (?1,?2,?3,?4,?5)")?;
        for (pid, before, after, sigma) in rows {
            st.execute(params![tid, pid, before, after, sigma])?;
        }
        Ok(())
    }

    pub fn question_params(&self, tid: i64, qs: &[(f64, f64, f64)]) -> rusqlite::Result<()> {
        let mut st = self
            .conn
            .prepare_cached("INSERT INTO question_params VALUES (?1,?2,?3,?4,?5)")?;
        for (q, (b, a, m)) in qs.iter().enumerate() {
            st.execute(params![tid, q as i64, b, a, m])?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tournament_metrics(
        &self,
        tid: i64,
        date: &str,
        kind: &str,
        weight: f64,
        n_teams: usize,
        n_q: usize,
        n_obs: u64,
        insample: Option<f64>,
        crossfit: Option<f64>,
        sp_forecast: Option<f64>,
        sp_old: Option<f64>,
        truedl: Option<f64>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO tournament_metrics VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                tid,
                date,
                kind,
                weight,
                n_teams as i64,
                n_q as i64,
                n_obs as i64,
                insample,
                crossfit,
                sp_forecast,
                sp_old,
                truedl
            ],
        )?;
        Ok(())
    }

    pub fn checkpoint(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch("COMMIT; BEGIN")
    }

    pub fn finish(self) -> rusqlite::Result<()> {
        self.conn.execute_batch("COMMIT")?;
        self.conn.execute_batch(
            "CREATE INDEX idx_team_events_team ON team_events(team_id);
             CREATE INDEX idx_team_events_tid ON team_events(tournament_id);
             CREATE INDEX idx_team_ratings_rel ON team_ratings(release_id);
             CREATE INDEX idx_team_ratings_team ON team_ratings(team_id);
             CREATE INDEX idx_player_ratings_player ON player_ratings(player_id);
             CREATE INDEX idx_player_events_player ON player_events(player_id);",
        )?;
        Ok(())
    }
}
