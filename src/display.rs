use crate::config::Config;

/// DESIGN 5: affine maps from the internal scale, fixed once at launch.
pub fn player(theta: f64, cfg: &Config) -> f64 {
    cfg.display.a_player + cfg.display.b_player * theta
}

pub fn team(mu: f64, cfg: &Config) -> f64 {
    cfg.display.a_team + cfg.display.b_team * mu
}
