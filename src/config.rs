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
    #[serde(default)]
    pub simple: Option<Simple>,
}

/// Model S, the spreadsheet-reproducible prototype of `src/bin/simple.rs`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Simple {
    #[serde(rename = "D")]
    pub d: f64,
    #[serde(rename = "K")]
    pub k: f64,
    pub k_p: f64,
    pub delta: f64,
    pub k_q: f64,
    pub r_new: f64,
    #[serde(default = "pairwise")]
    pub perf: Perf,
    #[serde(default = "surprise")]
    pub player_rule: PlayerRule,
    #[serde(default = "no_anchor")]
    pub anchor: Anchor,
    #[serde(default = "mean_lineup")]
    pub lineup: S2Lineup,
}

/// How a lineup's strength is estimated: the mean of the players' ratings less
/// a penalty per empty seat, or the roster estimate of DESIGN 4.1.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S2Lineup {
    Mean,
    Roster,
}

fn mean_lineup() -> S2Lineup {
    S2Lineup::Mean
}

/// Which team engine `fit` runs: Model S2 of DESIGN 4, or the Kalman filter
/// that DESIGN 8.3 keeps as the legacy engine.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamEngine {
    Kalman,
    S2,
}

fn kalman_engine() -> TeamEngine {
    TeamEngine::Kalman
}

/// How the score gap enters the pairwise outcome of DESIGN 4.3.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarginShape {
    Linear,
    Logistic,
}

/// What the rating is pulled towards once a rated game is scored.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullTarget {
    /// The strength of the lineup that turned up today.
    Lineup,
    /// The usual lineup, the running average of the ones before it.
    Usual,
}

/// Model S2's constants and the affine map that puts a player's `theta` on the
/// team display scale. Everything below `k_q` is a switch of DESIGN 4.8, each
/// shipped at the value that leaves the model of DESIGN 4.3 unchanged.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct S2 {
    #[serde(rename = "D")]
    pub d: f64,
    #[serde(rename = "K")]
    pub k: f64,
    pub k_q: f64,
    /// `lineup = "mean"`: points a lineup loses per seat short of six.
    pub delta: f64,
    pub lineup: S2Lineup,
    pub player_scale: f64,
    pub player_zero: f64,
    /// Questions of score gap worth a whole win; 0 is the win/lose rule.
    pub margin_g: f64,
    pub margin_shape: MarginShape,
    /// Fraction of the gap to the players' worth the rating takes per rated game.
    pub k_r: f64,
    pub k_r_target: PullTarget,
    /// Rated games below which the step is enlarged to `K n0 / n`; 0 is off.
    pub n0: f64,
    /// The team side's own format weights; absent means the shared `[data]` ones.
    pub w_sync: Option<f64>,
    pub w_strict_sync: Option<f64>,
    /// Principle 2 as a floor: a team that finished first keeps at least the
    /// rating it came with. Under the shipped constants it never binds.
    pub winner_floor: bool,
}

impl Default for S2 {
    fn default() -> S2 {
        S2 {
            d: 1200.0,
            k: 0.4,
            k_q: 0.01,
            delta: 150.0,
            lineup: S2Lineup::Mean,
            player_scale: 2454.8828,
            player_zero: 6000.0,
            margin_g: 0.0,
            margin_shape: MarginShape::Linear,
            k_r: 0.0,
            k_r_target: PullTarget::Lineup,
            n0: 0.0,
            w_sync: None,
            w_strict_sync: None,
            winner_floor: true,
        }
    }
}

/// How a result becomes a number of points: the order statistic at the team's
/// place, or the field Elo sum.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Perf {
    PlayedLike,
    Pairwise,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerRule {
    Surprise,
    Level,
}

fn pairwise() -> Perf {
    Perf::Pairwise
}

fn surprise() -> PlayerRule {
    PlayerRule::Surprise
}

/// Per-release rescaling of every rating; DESIGN-free, see out/simple/rules.md.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    None,
    Mean,
    MeanSd,
}

fn no_anchor() -> Anchor {
    Anchor::None
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    pub db_path: String,
    pub out_path: String,
    pub from: String,
    /// The last day of the window. Absent, or the literal `"today"`, means the
    /// current UTC date; `Config::load` resolves it.
    #[serde(default = "today")]
    pub to: String,
    pub w_in_person: f64,
    pub w_sync: f64,
    pub w_strict_sync: f64,
    pub w_async: f64,
    /// Tournaments the official system leaves out; chgk.quest trains on them.
    #[serde(default)]
    pub include_unrated: bool,
    /// Per-entity rows are written every Nth release and always on the last;
    /// `0` means the last release only.
    pub rows_every: i64,
    pub passes: usize,
}

fn today() -> String {
    "today".into()
}

impl Data {
    /// Whether release `release_id` gets `player_ratings` and `team_ratings`
    /// rows. `rows_every = 0` keeps rows at the last release only.
    pub fn write_rows(&self, release_id: i64, last: bool) -> bool {
        last || (self.rows_every > 0 && release_id % self.rows_every == 0)
    }
}

/// How a question's parameters are found: Newton on the penalised likelihood,
/// or the closed-form difficulty that matches the observed take rate.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QFit {
    Newton,
    TakeRate,
}

/// How a question's outcome is shared out among the lineup: the exact noisy-OR
/// gradient, or credit in proportion to each player's chance of knowing it.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QGradient {
    Exact,
    Share,
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
    #[serde(default = "newton")]
    pub fit: QFit,
    #[serde(default = "exact")]
    pub gradient: QGradient,
}

fn newton() -> QFit {
    QFit::Newton
}

fn exact() -> QGradient {
    QGradient::Exact
}

/// The Kalman step of DESIGN 3.3, or a decaying learning rate in its place.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PUpdate {
    Kalman,
    Schedule,
}

fn kalman() -> PUpdate {
    PUpdate::Kalman
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
    #[serde(default = "kalman")]
    pub update: PUpdate,
    /// `update = "schedule"`: step `eta0 / sqrt(c0 + games)`.
    #[serde(default = "eta0")]
    pub eta0: f64,
    #[serde(default = "c0")]
    pub c0: f64,
}

fn eta0() -> f64 {
    0.15
}

fn c0() -> f64 {
    0.25
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
    /// DESIGN 4 ("s2") or the legacy filter of DESIGN 8.3 ("kalman").
    #[serde(default = "kalman_engine")]
    pub engine: TeamEngine,
    #[serde(default)]
    pub s2: S2,
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
    /// The switches of DESIGN 8.2, all shipped at the value that leaves the model unchanged.
    #[serde(default = "one")]
    pub margin_prec_scale: f64,
    #[serde(default)]
    pub sigma_floor: f64,
    #[serde(default = "one")]
    pub forget: f64,
    #[serde(default)]
    pub surprise_z0: f64,
    #[serde(default = "nine")]
    pub surprise_max: f64,
    #[serde(default)]
    pub surprise_down_only: bool,
    #[serde(default = "one")]
    pub k_down: f64,
    #[serde(default)]
    pub margin_mean: bool,
    #[serde(default = "one")]
    pub margin_slope: f64,
    #[serde(default)]
    pub margin_intercept: f64,
    #[serde(default)]
    pub margin_sigma: Option<f64>,
}

fn one() -> f64 {
    1.0
}

fn nine() -> f64 {
    9.0
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
        let mut cfg: Config = toml::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        if cfg.data.to == "today" {
            cfg.data.to = crate::util::today_utc();
        }
        Ok(cfg)
    }
}
