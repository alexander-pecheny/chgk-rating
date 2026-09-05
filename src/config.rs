use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub data: Data,
    pub question: Question,
    pub player: Player,
    pub gauge: Gauge,
    pub team: Team,
    pub truedl: TrueDl,
    pub display: Display,
    pub numeric: Numeric,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub db_path: String,
    pub out_path: String,
    pub from: String,
    pub to: String,
    pub w_in_person: f64,
    pub w_sync: f64,
    pub w_strict_sync: f64,
    pub w_async: f64,
    pub rows_every: i64,
    pub passes: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub b_bar: f64,
    pub s_b: f64,
    pub s_a: f64,
    pub s_m: f64,
    pub s_aff: f64,
    pub aff_rank: usize,
    pub eta_v: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Player {
    pub mu_new: f64,
    pub sigma_new: f64,
    /// A team's answers are not independent given theta, so one tournament carries
    /// less information than its Fisher curvature claims; this is the discount.
    pub info_scale: f64,
    /// Fraction of the gap to the lineup mean each member is pulled after a tournament.
    pub teammate_shrink: f64,
    /// Volatility bounds and adaptation rate; `tau_p` is the starting volatility.
    pub v_min: f64,
    pub v_max: f64,
    pub v_eta: f64,
    /// Uncertainty each player restarts with on a second pass over the data.
    pub warm_sigma: f64,
    /// Only players who debuted before this date are warm-started on a second pass.
    pub warm_until: String,
    pub tau_p: f64,
    pub tau_aff: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gauge {
    pub n_est: u32,
    pub window_days: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Team {
    pub sigma_model: f64,
    /// Mean residual of realised strength around the raw roster estimate; added to it.
    pub roster_bias: f64,
    pub sigma_new_team: f64,
    pub beta: f64,
    pub kappa_m: f64,
    pub m_max: f64,
    pub nu: f64,
    pub e_cap: f64,
    pub v_min: f64,
    pub v_max: f64,
    pub v_eta: f64,
    pub rho: f64,
    /// Consecutive tournaments of the team a player must miss before their evidence decays.
    pub miss_streak: u32,
    pub gamma: f64,
    pub tau_t: f64,
}

/// DESIGN-adjacent: the tournament difficulty published alongside the ratings.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrueDl {
    /// Strength of each of the six reference players; fixed once, never refitted.
    pub theta_ref: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub a_team: f64,
    pub b_team: f64,
    pub a_player: f64,
    pub b_player: f64,
    /// A rating wider than this is published as provisional.
    pub provisional_sigma: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numeric {
    pub newton_iters: usize,
    pub newton_tol: f64,
    pub a_min: f64,
    pub a_max: f64,
    pub m_min: f64,
    pub m_max: f64,
    pub b_min: f64,
    pub b_max: f64,
    pub exp_lo: f64,
    pub exp_hi: f64,
    pub info_floor: f64,
    pub theta_max_step: f64,
    pub u_max: f64,
    pub crossfit_seed: u64,
}

impl Config {
    pub fn load(path: &str) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        toml::from_str(&text).map_err(|e| format!("{path}: {e}"))
    }
}
