//! One row of `team_events`, the record both team engines write of every
//! result they place in a field.

pub struct TeamEvent {
    pub team: u32,
    pub c: f64,
    pub delta_l: f64,
    pub x: f64,
    pub forecast: f64,
    pub forecast_place: f64,
    /// 1 + sum over the field of P(B beats A): the place the pairwise model expects.
    pub expected_place: f64,
    pub actual_place: f64,
    /// The old system's `predictedPosition`, carried through for DESIGN 7.2.
    pub pred_place: Option<f64>,
    pub s_r: f64,
    pub s_pair: f64,
    pub i_pair: f64,
    pub mu_before: f64,
    pub mu_after: f64,
    pub sigma_before: f64,
    pub sigma_after: f64,
    pub evidence: f64,
    pub gap_days: i32,
    pub var_diff: f64,
    pub lost_prev: f64,
    pub rated: bool,
    /// (realised strength - x) / sqrt(sigma_T^2 + sigma_model^2); should be unit variance.
    pub z: Option<f64>,
    /// The two parts of `mu_after - mu_before` under S2, on the internal scale:
    /// the placement step of DESIGN 4.3 step 3 and the pull towards the lineup.
    /// Whatever is left is the winner's floor. NaN where the engine has no such
    /// split, or the result took no update.
    pub d_place: f64,
    pub d_pull: f64,
}

