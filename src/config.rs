use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub model: Model,
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
    #[serde(default)]
    pub blend: Option<Blend>,
}

/// The published forecast: A2's own, or mixed with an `ext_elo` replay's.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blend {
    /// The replay's database, whose `team_events.x` is Elo's forecast.
    pub elo_db: String,
    /// Elo's share of the forecast, in units of each field's spread.
    pub elo: f64,
    /// Elo's share of each published player rating, in units of the spread of
    /// the experienced players' ratings at the release; 0 leaves them A2's.
    #[serde(default)]
    pub players: f64,
    /// A venue where more than this share of the players have fewer than
    /// `NEWCOMER_GAMES` in-person games keeps A2's forecast: Elo starts every
    /// newcomer alike, A2 does not. 0 blends every venue.
    #[serde(default)]
    pub newcomer_gate: f64,
}

/// What the fit writes into `meta` to say which rules made its numbers.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    #[serde(default)]
    pub version: String,
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
    /// The pull of rule 8: the team takes this much of the gap to the strength
    /// of the lineup that turned up. 0 is the rule S2 shipped with.
    #[serde(default)]
    pub k_r: f64,
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
/// a penalty per empty seat, the roster estimate of DESIGN 4.1, the roster
/// estimate tempered by `tau`, or chgk.gg's weighted best six. All four agree on
/// a full lineup of six equal players.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S2Lineup {
    Mean,
    Roster,
    Tempered,
    Best6,
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

/// What the editor memory of DESIGN 4.8 is keyed on.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdKey {
    /// Each `(player, editor)` pair; round 3's rule.
    PlayerEditor,
    /// The `(team, editor)` pair: a property of the team rather than of whoever
    /// happens to sit at the table.
    TeamEditor,
    /// The player against the whole set of a tournament's editors, so a package
    /// written by two people is its own author.
    PlayerSet,
}

/// How the forecast turns the lineups of a field into an order.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankRule {
    /// By each lineup's mean strength; the rule of DESIGN 4.1.
    Mean,
    /// By `E[1/place]`, the relevance the metric pays, under draws of every
    /// lineup's strength from its own posterior.
    Erel,
    /// By `P(place = 1)` under the same draws.
    PFirst,
    /// `Erel` with the pairwise noise of the Elo scale `D` added to each draw,
    /// which makes one draw a Plackett-Luce ranking of the field.
    ErelD,
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
/// team display scale. Everything below `k_q` is a switch of DESIGN 4.9, each
/// shipped at the value that leaves the model of DESIGN 4.3 unchanged.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct S2 {
    #[serde(rename = "D")]
    pub d: f64,
    #[serde(rename = "K")]
    pub k: f64,
    pub k_q: f64,
    /// `lineup = "mean"` and `"best6"`: points a lineup loses per seat short of six.
    pub delta: f64,
    pub lineup: S2Lineup,
    /// `lineup = "tempered"`: the temperature of the soft maximum. 1 is the
    /// roster estimate, larger flattens towards the mean.
    pub tau: f64,
    /// A pessimistic lineup: each player enters at `theta - sigma_c * sigma`, so
    /// a newcomer or a player long unseen counts for less. 0 is the shipped rule.
    pub sigma_c: f64,
    /// Widens that discount for a player whose evidence is mostly not in person:
    /// the factor is `1 + sigma_ip / (1 + n_in_person)`. 0 leaves it flat.
    pub sigma_ip: f64,
    /// How much of the fast estimate the forecast reads: a player enters at
    /// `theta + k_fast (theta_fast - theta)`. 0 is the long-run estimate alone.
    pub k_fast: f64,
    /// A lineup's own spread enters the forecast with this coefficient, on top
    /// of the pessimistic mean shift of `sigma_c`. In a field whose top is
    /// bunched, the table that might be very good wins more often than its
    /// expected strength says, and `sigma_c` can only push it down. 0 is off.
    pub sigma_v: f64,
    /// Sharpens the temperature when the field itself is bunched:
    /// `tau_eff = tau * (spread / 1000)^tau_spread`, clamped to [0.2, 2].
    /// 0 leaves `tau` alone.
    pub tau_spread: f64,
    /// A quantile of the lineup instead of one of each player: the strength is
    /// lowered by `q_sigma_c · sd(Q(L))`, the sd of the roster estimate
    /// propagated from the players' sigma. 0 leaves it alone.
    pub q_sigma_c: f64,
    /// Display points the forecast gains per unit of `T(L)`, the mean over the
    /// pairs of a lineup of `log(1 + rated games together)`. 0 is off.
    pub chemistry: f64,
    /// Rank the field by the expected number of answers on a package of the
    /// difficulty a field of this strength usually gets, instead of by `Q(L)`.
    /// The difficulties come from the in-person tournaments already fitted, so
    /// nothing today's field carries enters its own forecast.
    pub difficulty_forecast: bool,
    /// Display points the forecast gains per unit of a team's mean surprise over
    /// its last `series_k` in-person results. 0 is off.
    pub kappa_s: f64,
    pub series_k: usize,
    /// Shares of the gap from the lineup's strength to the team's usual lineup
    /// and to the team's rating that the forecast takes. 0 is the lineup alone.
    pub k_usual: f64,
    pub k_team: f64,
    /// Display points the forecast gains per unit of the lineup's mean past
    /// residual under today's editors. 0 is off. This is the editor effect where
    /// the residual check finds it -- in the outcomes -- rather than where
    /// `question.aff_rank` looks for it, in the question gradients.
    pub editor_memory: f64,
    /// Past results a `(player, editor)` pair needs before it counts.
    pub ed_min_n: f64,
    /// Display points the forecast gains per unit of the **smallest** format
    /// effect `phi` in the lineup. The shipped lineup strength reads every
    /// player at `theta + phi`, so the lineup's mean `phi` is already in; this
    /// is the weakest member's, which is not. The ranker of plan 8 puts it
    /// seventh of 132 and worth +0.002. 0 is off.
    pub phi_min: f64,
    /// Display points per unit of the spread of `theta + phi` inside the lineup:
    /// a term for a lopsided table against a balanced one, which `tau` and
    /// `question.m_fixed` reach only indirectly. 0 is off.
    pub phi_spread: f64,
    /// Share of the established panel whose members carry an editor memory at
    /// all; 0 is every player. The residual check says the effect is a property
    /// of a strong lineup, and this is the switch that says so.
    pub ed_gate: f64,
    /// Years after which an entry of the editor memory counts for half. 0 never
    /// forgets, which is round 3's rule.
    pub ed_half_life: f64,
    /// The editor memory learns only from fields on or after this date, so
    /// the forecasts of a burn-in that starts from unplaced players do not
    /// stay in it for good. Empty is every field. Measured with a 2005 start
    /// and changes nothing: -0.0035 against -0.0036 without it.
    pub ed_from: String,
    /// What the memory is keyed on: each `(player, editor)` pair, the
    /// `(team, editor)` pair, or the player against the whole set of a
    /// tournament's editors.
    pub ed_key: EdKey,
    /// How much of the memory the in-person format effect takes per field, as a
    /// share of the residual. 0 leaves `phi` alone, which is round 3's rule and
    /// the only setting whose blast radius is the forecast alone.
    pub ed_phi: f64,
    /// How the forecast orders a field: by the mean strength of each lineup, or
    /// by the relevance the metric pays, `E[1/place]` or `P(place = 1)` under
    /// the lineups' own uncertainty. `erel_d` adds the pairwise noise of `D` on
    /// top, which makes the draw a Plackett-Luce ranking at the Elo scale.
    pub rank_rule: RankRule,
    /// Draws per field when `rank_rule` samples. 0 falls back to the mean.
    pub rank_draws: usize,
    /// Sample only where the residual says the spread of a lineup buys
    /// anything: fields whose 1st-to-4th forecast gap is under this many
    /// display points. 0 samples every field.
    pub rank_gap: f64,
    /// Multiplies every lineup's sd before the draws. The offline re-ranking of
    /// `tools/q_diag4.py` puts the tune window's optimum near 0.5, which says
    /// the propagated sd is about twice what the ordering can use; 1 is the sd
    /// at face value.
    pub rank_sd: f64,
    /// Display points a lineup's forecast gains when at least `home_min` of its
    /// players have played out of the tournament's own town before. The residual
    /// says the sign is negative: the visitors are the ones who beat the
    /// forecast. 0 is off.
    pub home_bonus: f64,
    pub home_min: f64,
    /// The same for the town the team itself is registered in. 0 is off.
    pub home_team_bonus: f64,
    /// Display points per in-person tournament each player has already played in
    /// the trailing 7 and 30 days, averaged over the lineup. 0 is off.
    pub load_7: f64,
    pub load_30: f64,
    /// A quantile that depends on how bunched the field is. When `tight_gap` is
    /// positive the forecast -- and the forecast alone -- reads each player at
    /// `sigma_c_tight` in a field whose 1st-to-4th forecast gap is below
    /// `tight_gap` display points and at `sigma_c_loose` above it. 0 is off and
    /// leaves `sigma_c` in force everywhere.
    pub tight_gap: f64,
    pub sigma_c_tight: f64,
    pub sigma_c_loose: f64,
    /// The player display map of `[display]`, copied here on load: lineups and
    /// every point below are on the scale players are published on.
    #[serde(skip)]
    pub player_scale: f64,
    #[serde(skip)]
    pub player_zero: f64,
    /// Questions of score gap worth a whole win; 0 is the win/lose rule.
    pub margin_g: f64,
    pub margin_shape: MarginShape,
    /// Fraction of the gap to the players' worth the rating takes per rated game.
    pub k_r: f64,
    pub k_r_target: PullTarget,
    /// Multiplier on the pull at synchronous and strictly synchronous
    /// tournaments; 1 is the shipped rule, 0 confines the pull to in-person games.
    pub k_r_sync: f64,
    /// Rated games below which the step is enlarged to `K n0 / n`; 0 is off.
    pub n0: f64,
    /// The team side's own format weights; absent means the shared `[data]` ones.
    pub w_sync: Option<f64>,
    pub w_strict_sync: Option<f64>,
    /// Principle 2 as a floor: a team that finished first keeps at least the
    /// rating it came with. Under the shipped constants it never binds.
    pub winner_floor: bool,
    /// The two-part rating: `R = Q(core) + b`, where the core is the team's own
    /// players weighted by how often they turn up and `b` is the team effect,
    /// which only the placement step moves. Replaces the pull and the usual
    /// lineup in the field strength, which becomes `Q(L) + b`.
    pub two_part: bool,
    /// Attendance memory of the core: per rated game every weight takes this
    /// much of the gap to 1 if the player turned up and to 0 if not.
    pub k_c: f64,
    /// Per rated game `b` takes this much of the way back to zero, which is a
    /// pull towards the team's own players rather than today's guests.
    pub k_b: f64,
    /// Read the core at every tournament from the players' latest ratings, not
    /// as it stood at the team's last game. Breaks principle 1; for measurement.
    pub core_live: bool,
    /// Read the core as its `round(seats)` most frequent players under the
    /// lineup rule, instead of every player at their attendance weight.
    pub core_top: bool,
    /// Attendance weight below which a player is not yet, or no longer, part
    /// of the core; at `k_c = 0.1`, 0.3 is four games in a row. 0 counts all.
    pub core_min_w: f64,
    /// Count only players on the team's base roster on the day they last
    /// played for it; legionnaires never enter the core.
    pub core_base: bool,
    /// Field-strength weighting of the team step: each tournament's step is
    /// multiplied by `exp(field_gamma (S - field_ref) / 1000)`, `S` the mean
    /// lineup strength of the field's `field_k` strongest, clamped to
    /// [0.2, 5]. 0 is off. `field_ref` defaults to the median in-person `S`.
    pub field_gamma: f64,
    pub field_k: usize,
    pub field_ref: f64,
    /// Weight synchronous fields by their strength too; otherwise only
    /// in-person steps are scaled.
    pub field_sync: bool,
    /// Scale the player update of a tournament by the same factor, as if its
    /// format weight were that much larger.
    pub field_players: bool,
    /// The sign of the placement step follows the published forecast: a team
    /// at or above its forecast place cannot lose, one below it cannot gain.
    pub place_sign: bool,
    /// How much of the lineup's difference from the team's reference the
    /// field strength reads: `R + lineup_adj (Q(L) - ref)`. 1 is the shipped rule.
    pub lineup_adj: f64,
    /// The reference is the base roster, weighted by attendance, at today's
    /// player ratings; otherwise it is the usual lineup of `k_q`.
    pub ref_core: bool,
    /// Divide a team's wins minus expected by the field's information about it,
    /// 4 sum p(1 - p) over its pairs, instead of by n - 1: the two agree in a
    /// field of equals, and a big field of far weaker teams stops diluting the
    /// step of the few teams that were close. Floored at `info_min`.
    pub info_norm: bool,
    pub info_min: f64,
    /// Points below its first lineup's strength a new team starts at; 0 is
    /// the shipped rule. The first game's forecast still reads the lineup.
    pub start_drop: f64,
    /// chgk.gg's start at a share of the roster: a new team starts this share
    /// of the way from its lineup's strength down to `start_zero`, the
    /// internal rating that publishes as 0 (-a_team / b_team). 0 is off.
    pub start_frac: f64,
    pub start_zero: f64,
    /// Judge every team against the published forecast: the pairwise
    /// expectation reads the forecast strengths of the field (the lineups that
    /// turned up, plus the editor memory) instead of the ratings. The rating
    /// then never enters its own expectation.
    pub expect_forecast: bool,
    /// chgk.gg's forecast: publish the field strength the update is judged at,
    /// the rating corrected for the lineup, instead of the lineup alone.
    pub forecast_rating: bool,
    /// chgk.gg's correction, as a ratio on the published scale:
    /// `Z + (R - Z) (Q(L) - Z) / (ref - Z)`, `Z = start_zero`, the ratio clipped
    /// to [0.5, 2]; otherwise `R + lineup_adj (Q(L) - ref)`.
    pub adj_mult: bool,
    /// The best-results rating: a team's rating is the mean of its `best_n`
    /// best counted performances, empty slots and anything below the published
    /// zero counting as zero. A performance is the strength at which the
    /// lineup's place is what the field's forecasts expect, less how far the
    /// lineup stood above the team's base roster. A new result expires at most
    /// one result older than `best_window` days, so an idle team keeps its
    /// number. 0 is off.
    pub best_n: usize,
    pub best_window: i32,
    /// Pseudo-games against the team's own forecast that keep a performance
    /// finite for a first or last place.
    pub perf_prior: f64,
    /// How far the forecast moves from `Q(L)` to the London Elo's pack score
    /// of the lineup, its players at `pack_a + pack_b · theta`. 0 is off.
    #[serde(default)]
    pub pack_w: f64,
    #[serde(default)]
    pub pack_a: f64,
    #[serde(default)]
    pub pack_b: f64,
    /// How far the forecast moves to the lineup read at the players' score
    /// ratings (`player.score_k`), put on the field's scale. 0 is off.
    #[serde(default)]
    pub k_score: f64,
    /// The forecast reads a member below their teammates' mean `k_mates ·
    /// min(1, sigma^2 / sigma_new^2)` of the way up to it: an unsure player
    /// at a strong table is read nearer the table. 0 is off.
    #[serde(default)]
    pub k_mates: f64,
    /// Display points per unit of the lineup's past-year share of games in
    /// school, student or novice fields (`data.classes`), on the forecast
    /// alone. 0 is off. Measured and off (2026-10-13): -150 to -450 moved
    /// test Pearson by under 0.0005, as such lineups rarely reach a weighted field.
    #[serde(default)]
    pub k_youth: f64,
    /// Display points per unit of the lineup's mean `1 / (1 + games_ip)`, at an
    /// in-person table and on the forecast alone: a player new to the table
    /// underperforms the strength their synchronous games earned. 0 is off.
    /// At -600 on top of `player.mu_young` (2026-10-13) young in-person fields
    /// gain +0.025 per-field Pearson on tune and +0.016 on test, open and
    /// synchronous fields move by under 0.001 and the weighted NDCG by
    /// -0.0006 +/- 0.0011 and -0.0010 +/- 0.0015.
    #[serde(default)]
    pub k_ip_new: f64,
    /// Under `k_ip_new`, each game not played in person counts as this much of
    /// one that was, so a player who only lacks in-person tables is less fresh.
    #[serde(default)]
    pub ip_new_sync: f64,
    /// Under `k_ip_new`, a player's freshness is scaled by `g0 / (g0 + games)`,
    /// so the penalty falls on players new to the game. 0 is off.
    #[serde(default)]
    pub ip_new_g0: f64,
    /// A field whose ten strongest lineups average at least this many display
    /// points is strong: there `k_ip_new` is multiplied by `ip_new_strong` and
    /// `editor_memory` by `ed_strong`. 0 is off.
    #[serde(default)]
    pub strong_top: f64,
    #[serde(default = "one")]
    pub ip_new_strong: f64,
    #[serde(default = "one")]
    pub ed_strong: f64,
    /// Under "best6", a seat short of six is filled at no more than this many
    /// display points, so a strong lineup loses what a strong sixth player
    /// adds. 0 is off.
    #[serde(default)]
    pub fill_max: f64,
    /// The forecast reads the lineup's seats under weights `fc_seat_decay^k +
    /// fc_seat_floor` instead of 6, 5 ... 1, with a seat short of six filled at
    /// no more than `fc_fill_max`; ratings see it only through the forecast
    /// place the fair rule reads. 0 keeps the rating's lineup.
    #[serde(default)]
    pub fc_seat_decay: f64,
    #[serde(default)]
    pub fc_seat_floor: f64,
    #[serde(default)]
    pub fc_fill_max: f64,
    /// An empty seat at `fc_fill_top` times the strongest member plus
    /// `fc_fill_const` instead of the weakest less `delta`. Both 0 is off.
    #[serde(default)]
    pub fc_fill_top: f64,
    #[serde(default)]
    pub fc_fill_const: f64,
    /// A member weaker than an empty seat counts as one, so a lineup never
    /// loses strength by adding a player.
    #[serde(default)]
    pub fc_fill_floor: bool,
    /// The seat rule at synchronous tables only.
    #[serde(default)]
    pub fc_seat_sync_only: bool,
}

/// What a config that leaves a key of `[team.s2]` out gets: the rule as it
/// stood before that key was measured. The shipped values are in config.toml,
/// which sets every key, so these defaults only reach the older research
/// configs.
const S2_DEFAULT: S2 = S2 {
    pack_w: 0.0,
    pack_a: 0.0,
    pack_b: 0.0,
    k_score: 0.0,
    k_mates: 0.0,
    k_youth: 0.0,
    k_ip_new: 0.0,
    ip_new_sync: 0.0,
    ip_new_g0: 0.0,
    strong_top: 0.0,
    ip_new_strong: 1.0,
    ed_strong: 1.0,
    fill_max: 0.0,
    fc_seat_decay: 0.0,
    fc_seat_floor: 0.0,
    fc_fill_max: 0.0,
    fc_fill_top: 0.0,
    fc_fill_const: 0.0,
    fc_fill_floor: false,
    fc_seat_sync_only: false,
    d: 1200.0,
    k: 0.4,
    k_q: 0.01,
    delta: 150.0,
    lineup: S2Lineup::Mean,
    tau: 1.0,
    sigma_c: 0.0,
    sigma_ip: 0.0,
    k_fast: 0.0,
    sigma_v: 0.0,
    tau_spread: 0.0,
    q_sigma_c: 0.0,
    chemistry: 0.0,
    phi_min: 0.0,
    phi_spread: 0.0,
    difficulty_forecast: false,
    kappa_s: 0.0,
    k_usual: 0.0,
    k_team: 0.0,
    series_k: 5,
    editor_memory: 0.0,
    ed_min_n: 2.0,
    ed_gate: 0.0,
    ed_half_life: 0.0,
    ed_from: String::new(),
    ed_key: EdKey::PlayerEditor,
    ed_phi: 0.0,
    rank_rule: RankRule::Mean,
    rank_draws: 200,
    rank_gap: 0.0,
    rank_sd: 1.0,
    home_bonus: 0.0,
    home_min: 0.5,
    home_team_bonus: 0.0,
    load_7: 0.0,
    load_30: 0.0,
    tight_gap: 0.0,
    sigma_c_tight: 1.5,
    sigma_c_loose: 1.5,
    player_scale: 4269.2612,
    player_zero: 5738.6039,
    margin_g: 0.0,
    margin_shape: MarginShape::Linear,
    k_r: 0.0,
    k_r_target: PullTarget::Lineup,
    k_r_sync: 1.0,
    n0: 0.0,
    w_sync: None,
    w_strict_sync: None,
    winner_floor: true,
    two_part: false,
    k_c: 0.1,
    k_b: 0.0,
    core_live: false,
    core_top: false,
    core_min_w: 0.0,
    core_base: false,
    field_gamma: 0.0,
    field_k: 10,
    field_ref: 10366.0,
    field_sync: false,
    field_players: false,
    place_sign: false,
    lineup_adj: 1.0,
    ref_core: false,
    info_norm: false,
    info_min: 4.0,
    start_drop: 0.0,
    start_frac: 0.0,
    start_zero: 5944.6,
    expect_forecast: false,
    forecast_rating: false,
    adj_mult: false,
    best_n: 0,
    best_window: 365,
    perf_prior: 1.0,
};

impl Default for S2 {
    fn default() -> S2 {
        S2_DEFAULT
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
    /// The day the window ends before. Absent, or the literal `"today"`, means
    /// the day after `buff.db`'s last complete update, not the clock's day, so
    /// that a run is a function of its data; `Config::load` resolves it.
    #[serde(default = "today")]
    pub to: String,
    pub w_in_person: f64,
    pub w_sync: f64,
    pub w_strict_sync: f64,
    pub w_async: f64,
    /// Tournaments the official system leaves out; chgk.quest trains on them.
    #[serde(default)]
    pub include_unrated: bool,
    /// The same tournaments, but for the player side only: they move `theta` and
    /// `phi` and never open or move a team state, never produce a team event and
    /// never enter the metric's tournament set.
    #[serde(default)]
    pub train_unrated: bool,
    /// From this date on, every tournament the settings above leave out or keep
    /// for the players alone still gets its team events and its metrics row,
    /// forecast as an ordinary one would be, so every system can be scored on
    /// the same fields. A tournament left out moves nothing; one kept for the
    /// players trains them as before. Empty is off.
    #[serde(default)]
    pub forecast_from: String,
    /// `include_unrated` for in-person tournaments alone: an unrated in-person
    /// tournament counts as an ordinary one, and its results flagged
    /// `inRating` move their teams.
    #[serde(default)]
    pub unrated_in_person: bool,
    /// One field for an in-person tournament played in several cities: the
    /// fields of one day that share the editors and the question count, and
    /// that rating.chgk.info links as sharing questions, are merged under the
    /// lowest id and re-ranked by questions taken.
    #[serde(default)]
    pub merge_cities: bool,
    /// Days apart two such fields may be and still merge: a package played on
    /// Saturday in Europe is Friday in California. 0 is the same day only.
    #[serde(default)]
    pub merge_days: i32,
    /// A question is dropped from a merged field only when a whole venue did
    /// not play it; one team's missing answer stays that team's.
    #[serde(default)]
    pub merge_venue_drop: bool,
    /// A CSV of `alias_id,player_id`: one person entered under two ids, whose
    /// games under `alias_id` count as `player_id`'s. Empty reads none.
    #[serde(default)]
    pub player_aliases: String,
    /// Weight of one of those tournaments, on top of its format weight.
    #[serde(default = "one")]
    pub w_unrated: f64,
    /// `tools/tournament_class.py`'s table of tournament classes, read by the
    /// class weights below. Empty reads none.
    #[serde(default)]
    pub classes: String,
    /// Weights, on top of the format weight, of a school, student or novice
    /// field, of an easy pack and of a major tournament. 1 is off. Measured
    /// and off (2026-10-13): 0.5 for the young fields, 0.5 for easy packs and
    /// 1.5 for majors each moved test Pearson by under 0.0005.
    #[serde(default = "one")]
    pub w_school: f64,
    #[serde(default = "one")]
    pub w_student: f64,
    #[serde(default = "one")]
    pub w_novice: f64,
    #[serde(default = "one")]
    pub w_easy: f64,
    #[serde(default = "one")]
    pub w_major: f64,
    /// Per-entity rows are written every Nth release and always on the last;
    /// `0` means the last release only.
    pub rows_every: i64,
    pub passes: usize,
    /// Nothing before this date is published: the tournaments before it only
    /// teach the model, and the first release on or after it is release 1.
    /// Empty publishes from `from`.
    #[serde(default)]
    pub publish_from: String,
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

/// The per-player format effect `phi_i` of DESIGN 3.7. `none` ships. `in_person`
/// gives a player the strength `theta_i + phi_i` at an in-person table and
/// `theta_i` anywhere else; `signed` makes the synchronous strength
/// `theta_i - phi_i`, so `phi` is half the gap between the two formats.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatEffect {
    None,
    InPerson,
    Signed,
}

fn no_format_effect() -> FormatEffect {
    FormatEffect::None
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
    #[serde(default = "no_format_effect")]
    pub format_effect: FormatEffect,
    /// Prior width of `phi`, the per-player format effect.
    #[serde(default = "s_phi")]
    pub s_phi: f64,
    /// A second per-player scalar, read at a synchronous table only, so that
    /// synchronous evidence stops moving the in-person strength without forcing
    /// the symmetry `signed` imposes. Off is the shipped model.
    #[serde(default)]
    pub sync_effect: bool,
    /// Prior width of that scalar.
    #[serde(default = "s_phi")]
    pub s_psi: f64,
    /// A single global collaboration exponent: `P = (1 − e^{−u})^m` for every
    /// question. 1 is the noisy-OR every round so far has fitted, and the
    /// take-rate closed form solves `b` from `r^{1/m}` when it is not.
    #[serde(default = "one")]
    pub m_fixed: f64,
    /// Relative player updates: each lineup is judged against the lineups of
    /// similar expected score in its own field rather than against its raw
    /// expectation. The neighbours' mean surplus, under a Gaussian kernel of
    /// this width in expected answers, shifts the difficulty the lineup is read
    /// at before its gradient is taken. 0 is off, the plain likelihood gradient.
    #[serde(default)]
    pub rel_h: f64,
    /// With `rel_h`: the players of a lineup that finished first, sole or
    /// shared, never take a negative gradient from that tournament.
    #[serde(default)]
    pub rel_winner: bool,
    /// The general form of `rel_winner`: a lineup that finished at or above
    /// its forecast place never takes a negative player step from that
    /// tournament, and one that finished below it never a positive one. The
    /// questions set the size, the place the direction.
    #[serde(default)]
    pub rel_place: bool,
    /// A single global discrimination, read at an in-person table only; 1 is the
    /// pinned value of every round so far. The take-rate closed form solves `b`
    /// with this `a`, so the difficulties stay on the take rate.
    #[serde(default = "one")]
    pub a_in_person: f64,
    /// Lineup chemistry inside the likelihood: the team's intensity is
    /// `u · exp(chem_u · T(L))`, so a lineup that has played together often
    /// reaches further and its players are credited less for what it answers.
    /// 0 leaves the likelihood alone.
    #[serde(default)]
    pub chem_u: f64,
    /// A lineup of `n` below six reads at `(6 / n)^size_k` times the summed
    /// intensity, so a short lineup is not expected to fall as far short as
    /// the plain noisy-OR says, and its members do not carry the difference in
    /// their own ratings. 0 is the plain sum.
    #[serde(default)]
    pub size_k: f64,
    /// The same for a lineup of more than six, where the members beyond the
    /// six at the table add less than the sum says.
    #[serde(default)]
    pub size_k_long: f64,
    /// `size_k` at an in-person table, for every lineup size; absent reads the
    /// two above.
    #[serde(default)]
    pub size_k_ip: Option<f64>,
    /// Each question's share of a player's step is weighed by
    /// `(4 P (1 − P))^ls_gamma`, `P` the lineup's chance of taking it: 1 counts
    /// every question by its surplus alone, as the London Elo does, where the
    /// likelihood (0) makes the most of a take nobody expected.
    #[serde(default)]
    pub ls_gamma: f64,
    /// A lineup's intensity is the power mean `(Σ λ_i^norm_p)^(1/norm_p)` of
    /// its members' rather than their sum: above 1 its strongest members carry
    /// more of it and each further member adds less, as the London Elo's
    /// `rho = 1 / norm_p` has it. 0 or 1 is the plain sum of the noisy-OR.
    #[serde(default)]
    pub norm_p: f64,
    /// Each member enters the likelihood at `theta + var_boost * var`, `var`
    /// being their uncertainty diffused to the day. Intensity is convex in
    /// `theta`, so a player known only roughly answers, on average, like one
    /// `a var / 2` stronger than their point estimate: 0.5 is that mean. A
    /// lineup of one established player and several unsettled ones then
    /// credits the unsettled ones with more of what it answers. 0 is off.
    #[serde(default)]
    pub var_boost: f64,
    /// How a lineup's credit for a question is shared among its members: each
    /// takes `λ^(1/share_temp)` of it, normalised, instead of their own share
    /// `λ / u` of the intensity. The team's total gradient and the likelihood
    /// are untouched. 1 (or 0) is the plain noisy-OR share, below 1 sharpens it; larger spreads the
    /// credit towards an even split, as rating.chgk.fun does, so a constant
    /// member of a lineup of unsettled players does not take all of it.
    #[serde(default)]
    pub share_temp: f64,
    /// Blend every member's gradient towards the lineup's mean gradient, as the
    /// London Elo gives each member the same surplus. 0 is off, 1 an even split.
    #[serde(default)]
    pub equal_credit: f64,
    /// The same blend for each member's information.
    #[serde(default)]
    pub equal_info: f64,
    /// The London Elo's slope: a player of strength `z` reads every question
    /// at discrimination `a / (1 + slope_c · max(z − slope_z0, 0))`, never
    /// below `a · slope_min`, so a strong player is less sure of an easy
    /// question and less lost on a hard one. 0 is off.
    #[serde(default)]
    pub slope_c: f64,
    #[serde(default)]
    pub slope_z0: f64,
    #[serde(default)]
    pub slope_min: f64,
    /// A strength per team id, added to every member of that team's lineup in
    /// the likelihood and in the forecast, with this prior sd; it takes a
    /// Kalman step on the team's whole gradient, so a stable team's surplus
    /// over its members goes to the team rather than to its constant member.
    /// 0 is off.
    #[serde(default)]
    pub team_effect_sd: f64,
    /// The team strength's random walk per year.
    #[serde(default)]
    pub team_effect_tau: f64,
    /// Whether the editor affinity of section 3.6 is a property of every player
    /// or of a strong one; `none` is the shipped model.
    #[serde(default = "flat_affinity")]
    pub aff_strength: AffStrength,
    /// `aff_strength = "scale"`: the affinity of a player at `theta` is
    /// multiplied by `exp(aff_kappa · theta)`, clipped to `[0, aff_s_max]`.
    /// The gauge centres the established panel at `theta = 0`, so that is the
    /// reference the multiplier is one at.
    #[serde(default = "one")]
    pub aff_kappa: f64,
    #[serde(default = "five")]
    pub aff_s_max: f64,
    /// `aff_strength = "gate"`: the share of the established panel that carries
    /// an affinity vector at all. 0.2 is the top fifth.
    #[serde(default)]
    pub aff_gate: f64,
}

/// Whether every player's editor affinity counts the same, or a strong player's
/// counts for more. A weak player misses a question whoever wrote it; the
/// hypothesis is that reading an editor well is something only a strong player
/// has room to do.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AffStrength {
    /// Every player's vector counts the same: the model of DESIGN 3.6.
    None,
    /// Multiply the affinity, and its gradient, by `exp(aff_kappa · theta)`.
    Scale,
    /// Only the top `aff_gate` of the established panel carries a vector.
    Gate,
}

fn flat_affinity() -> AffStrength {
    AffStrength::None
}

fn five() -> f64 {
    5.0
}

fn s_phi() -> f64 {
    0.3
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
    /// Under `question.rel_place`, a lineup at or above its forecast place
    /// cannot lose from the tournament; when set, the pull towards the
    /// lineup's mean that follows is not allowed to undo that.
    #[serde(default)]
    pub shrink_keeps_floor: bool,
    /// The pull towards the lineup mean is shared by uncertainty: each member
    /// moves `teammate_shrink * var / mean var` of their gap, so the unsettled
    /// members of a lineup come up to an established one rather than the
    /// established one going down to them. Off is the same pull for everyone.
    #[serde(default)]
    pub shrink_by_var: bool,
    /// A newcomer starts this share of the way from `mu_new` to the mean of
    /// the lineup's players who already have a rating; 0 starts every newcomer
    /// at `mu_new`, however strong the table they join.
    #[serde(default)]
    pub seed_w: f64,
    /// Added to the start of a player whose first game is in a school,
    /// student or novice field (`crate::age`): such a field cannot place its
    /// players against adults, so they arrive in open fields overrated. 0 is
    /// off. At -0.25 (2026-10-13) young fields gain +0.035 per-field Pearson on
    /// tune and +0.036 on test (tools/class_metrics.py), the weighted metric
    /// +0.0009 / +0.0006, and open fields lose nothing.
    #[serde(default)]
    pub mu_young: f64,
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
    /// Diffusion of `phi` between a player's tournaments.
    #[serde(default = "tau_phi")]
    pub tau_phi: f64,
    /// Measure that diffusion from the player's last tournament of the pinned
    /// column's own format instead of their last tournament of any format, so a
    /// season of synchronous play does not age their in-person strength.
    #[serde(default)]
    pub tau_phi_own: bool,
    /// One joint Kalman step for `theta` and the pinned format columns, which
    /// enter the likelihood as one sum, instead of a full step for each against
    /// the same gradient. The shares follow the prior variances, so a newcomer's
    /// surprise goes to the uncertain `theta` and not to `phi`, which never
    /// forgets it. Off is the separate steps. Measured against the one-pass
    /// burn-in and off: +0.0001 +/- 0.0007 on tune (+0.0018 +/- 0.0009 on
    /// test), and it moves every player's `phi`, not only the burn-in's.
    #[serde(default)]
    pub phi_joint: bool,
    /// At `data.publish_from`, put every pinned format column back to its
    /// prior, so `phi` is learned only once the burn-in has put `theta` where
    /// it belongs. Off keeps what the burn-in learned. Measured and off:
    /// -0.0026 +/- 0.0012 on tune; three years of `phi` evidence are worth
    /// more than the cold-start bias they carry.
    #[serde(default)]
    pub burn_reset_phi: bool,
    /// `phi` learns only from tournaments after which the player's `theta`
    /// variance is at most this, so the surprises of a newcomer's climb go to
    /// `theta` alone. 0 is off. Measured and off: -0.0008 +/- 0.0010 on tune
    /// at 0.25 and -0.0007 +/- 0.0011 at 0.1.
    #[serde(default)]
    pub phi_gate_var: f64,
    /// A tournament with no answer mask, which the question model cannot
    /// read, moves the players by its places instead: each lineup's strength
    /// by the tempered roster rule of `team.s2`, its surprise as wins over the
    /// field less the expected ones at the pairwise scale of `team.s2.D`, and
    /// one joint Kalman step for its members, shared by their uncertainty.
    /// Only `theta` moves. With `data.from` before the masks begin, this
    /// places the players before the question model starts. Off ignores them.
    /// Measured from 2005 against the one-pass burn-in from 2013 and off:
    /// every variant loses on tune, -0.0012 +/- 0.0012 at best (all fields
    /// read by places until 2013, hand-off at variance 1), although it gains
    /// +0.0017 on test. A 2005 start without it loses -0.0036.
    #[serde(default)]
    pub places: bool,
    /// One joint step for a lineup's members: each takes `var_i g_i` over
    /// `1 + Σ_j var_j I_j` summed over the whole lineup instead of over
    /// themselves alone, so a lineup of unplaced players shares one surprise
    /// rather than each taking all of it, and a placed teammate of a newcomer
    /// moves little. Off is the separate steps. Measured and off: -0.0033 +/-
    /// 0.0013 on tune.
    #[serde(default)]
    pub team_joint: bool,
    /// Extra diffusion of `theta` per year of a player's absence beyond the
    /// first. 0 is off. Measured and off: -0.0018 +/- 0.0007 on tune at 0.2.
    #[serde(default)]
    pub tau_gap: f64,
    /// Newcomer boost: a player with fewer than `new_boost_games` games enters
    /// each Kalman step with their prior variance multiplied by
    /// `1 + new_boost * (1 - games / new_boost_games)`, so they move further on
    /// what their lineups do, as rating.chgk.fun's newcomer multiplier does.
    /// 0 is off.
    #[serde(default)]
    pub new_boost: f64,
    #[serde(default)]
    pub new_boost_games: u32,
    /// Theta a player gains per weighted tournament played, whatever the
    /// result: the learning that comes with experience, which the London Elo
    /// gets from its inflation. `learn_games` makes it fade as
    /// `learn_games / (learn_games + games)`; 0 keeps it constant.
    #[serde(default)]
    pub learn: f64,
    #[serde(default)]
    pub learn_games: f64,
    /// Newcomer cap: while a player has fewer than `new_cap_games` games, one
    /// tournament moves their theta by at most `new_cap`, so one result far
    /// above or below the lineup's forecast cannot set a career. 0 is off.
    #[serde(default)]
    pub new_cap: f64,
    #[serde(default)]
    pub new_cap_games: u32,
    /// The joint step of `team_joint` for newcomers only: members with fewer
    /// than `joint_new_games` games share one surprise among themselves, so a
    /// lineup of six newcomers does not hand each of them the whole of its
    /// result. Established players keep their separate steps. 0 is off.
    #[serde(default)]
    pub joint_new_games: u32,
    /// Held start: a newcomer's first `hold_games` tournaments do not move
    /// them. Their evidence, scored at the newcomer prior, is summed and taken
    /// in one step after the last of them, so no single early result sets
    /// where the player stands, and two newcomers who share a lineup are read
    /// at one strength until then. 0 is off.
    #[serde(default)]
    pub hold_games: u32,
    /// Pair covariance, a prototype. Empty is off. "diag" steps each member on
    /// the expected information of the lineup's questions without the cross
    /// terms, the control. "full" takes one joint step per lineup on the whole
    /// information matrix and keeps the covariance of every two players who
    /// shared a lineup, so partners who always play together stay uncertain
    /// about which of them is stronger, and a game one of them plays apart
    /// settles it.
    #[serde(default)]
    pub pair_cov: String,
    /// Pair covariances below this correlation are forgotten.
    #[serde(default)]
    pub pair_prune: f64,
    /// Partner pull: after each tournament, two lineup members who have
    /// shared at least `pair_pull_games` lineups, each at least
    /// `pair_pull_share` of their own games, move `pair_pull` of the way to
    /// their mean; their sum stays. 1 gives such partners one number. 0 is off.
    #[serde(default)]
    pub pair_pull: f64,
    #[serde(default)]
    pub pair_pull_games: u32,
    #[serde(default)]
    pub pair_pull_share: f64,
    /// Only the lower partner moves, towards the higher; the higher keeps its score.
    #[serde(default)]
    pub pair_pull_up: bool,
    /// Star cap: a player whose strength led their lineup by at least
    /// `star_margin` gains at most `star_cap` of theta per year from such
    /// tournaments (a ledger decaying with a one-year time constant). 0 is off.
    #[serde(default)]
    pub star_cap: f64,
    #[serde(default)]
    pub star_margin: f64,
    /// The star cap applies from this date; empty is from the start.
    #[serde(default)]
    pub star_from: String,
    /// The format columns learn from `v / (v + var)` of each tournament's
    /// evidence, `var` being `theta`'s: little while `theta` is unsettled.
    /// 0 is off. On the one-pass burn-in, tune against the burn-in is -0.0011
    /// at 0.2, -0.0001 at 0.05, +0.0015 +/- 0.0011 at 0.02, +0.0006 at 0.01
    /// and +0.0014 at 0.005; 0.02 is +0.0017 +/- 0.0014 on test. Under the
    /// two-pass warm start it costs -0.0009 on tune: it repairs the cold start,
    /// not the model.
    #[serde(default)]
    pub phi_share_v: f64,
    /// How many independent pairwise results one field is worth, before the
    /// format weight.
    #[serde(default = "places_eff")]
    pub places_eff: f64,
    /// Only fields before this date move players by places; empty is all.
    #[serde(default)]
    pub places_until: String,
    /// Before `places_until`, read every field by its places, masked or not,
    /// and leave the question model's player step out.
    #[serde(default)]
    pub places_masked: bool,
    /// At `places_until`, every player already seen restarts at this variance,
    /// the starting volatility and the prior format columns, keeping `theta`:
    /// the warm start's restart, made from the past alone. 0 is no hand-off.
    #[serde(default)]
    pub places_handoff_var: f64,
    /// Volatility floor of the second, fast estimate of the same strength. It
    /// takes the same Kalman step against the same gradient and forgets far
    /// faster; only the forecast reads it, through `team.s2.k_fast`. 0 is off.
    #[serde(default)]
    pub v_fast: f64,
    /// The fast estimate's share of equal credit: each member steps on the
    /// lineup's mean gradient and information, as the London Elo credits a
    /// team's surprise. 0 is the member's own gradient.
    #[serde(default)]
    pub fast_credit: f64,
    /// Equal credit for the unsure: a player steps on `c` of the lineup's mean
    /// gradient and information instead of their own, `c = credit_unsure ·
    /// min(1, var / sigma_new^2)`, so a newcomer at a strong table shares the
    /// table's surprise. 0 is off.
    #[serde(default)]
    pub credit_unsure: f64,
    /// Equal credit for everyone: `credit_all` of the lineup's mean gradient
    /// and information, on top of `credit_unsure`'s share, as the London Elo
    /// credits a team's surprise to all its members. 0 is off.
    #[serde(default)]
    pub credit_all: f64,
    /// `credit_all` shares only the gradient; each member keeps their own
    /// information, so the step's size stays the member's own, as in the London
    /// Elo, where everyone moves by the same surplus at their own volatility.
    #[serde(default)]
    pub credit_own_info: bool,
    /// A player's variance is raised to at least `sigma_floor`² before each
    /// step, so an established player keeps a minimum gain per tournament, as
    /// the London Elo's `alpha` does, rather than one that fades with the
    /// games. 0 is off.
    #[serde(default)]
    pub sigma_floor: f64,
    /// The score rating, a second strength only the forecast reads: once per
    /// tournament every member moves by `w (score_k + (v / score_kv)^2)` times
    /// the lineup's answers minus the answers the fitted questions expected,
    /// capped at the lineup's uniform shift and at `score_cap`, plus
    /// `score_infl`; `1/v^2` grows by `w h^2 / score_sv^2`, `h` the member's
    /// derivative of the expected answers, from `score_v0`. The London Elo's
    /// update on our scale. 0 is off.
    #[serde(default)]
    pub score_k: f64,
    #[serde(default)]
    pub score_kv: f64,
    #[serde(default)]
    pub score_cap: f64,
    #[serde(default)]
    pub score_infl: f64,
    #[serde(default)]
    pub score_sv: f64,
    #[serde(default)]
    pub score_v0: f64,
    /// Credit by each member's derivative instead of equally.
    #[serde(default)]
    pub score_grad: bool,
    /// Judge the score rating against questions fitted at the score ratings,
    /// not at `theta`.
    #[serde(default)]
    pub score_own_fit: bool,
    /// The fast estimate's gain per game, faded by `learn_games` like `learn`.
    #[serde(default)]
    pub fast_learn: f64,
    #[serde(default = "kalman")]
    pub update: PUpdate,
    /// `update = "schedule"`: step `eta0 / sqrt(c0 + games)`.
    #[serde(default = "eta0")]
    pub eta0: f64,
    #[serde(default = "c0")]
    pub c0: f64,
    /// The weight of an unrated or async result fades as `extra_games / (extra_games + games)`,
    /// so it teaches a player with little history and leaves an established one alone. 0 is off.
    #[serde(default)]
    pub extra_games: f64,
}

fn tau_phi() -> f64 {
    0.05
}

fn places_eff() -> f64 {
    5.0
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
    /// The week's shift is taken only from a reference group of at least this
    /// many established players; a smaller one leaves the scale where it was.
    /// 0 takes it from any group, which is how 2021's lockdown moved every
    /// rating by 1722 points when the group shrank to seven players.
    #[serde(default)]
    pub min_panel: u32,
    /// The largest shift one release may make, in theta; 0 is no limit.
    #[serde(default)]
    pub max_shift: f64,
    /// The published scale follows the gauge but moves at most this far, in
    /// theta, per release; the model reads the gauge itself either way. 0
    /// publishes the gauge as it is.
    #[serde(default)]
    pub publish_step: f64,
    /// The published scale stands still from `publish_hold_from` until
    /// `publish_hold_until` (dates; empty is no hold), then rejoins the gauge
    /// under `publish_step`. The model reads the gauge itself either way.
    #[serde(default)]
    pub publish_hold_from: String,
    #[serde(default)]
    pub publish_hold_until: String,
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
    /// How much of the format effect the published player number carries:
    /// 0 is `theta`, the strength away from an in-person table, 1 is
    /// `theta + phi`, the strength at one. The forecast never reads this.
    #[serde(default)]
    pub player_phi: f64,
    /// The player table ranks everyone, provisional or not, by the rating less
    /// this many standard deviations, so a rating resting on little evidence
    /// stands where its lower bound puts it; the number shown is the rating
    /// itself. 0 lists the settled players first, by rating.
    #[serde(default)]
    pub rank_sigmas: f64,
    /// The first day of the clean test window: no rule was chosen on a
    /// tournament after it. Tournament pages from this day show every
    /// system's forecast quality. Empty shows none.
    #[serde(default)]
    pub clean_from: String,
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
        cfg.team.s2.player_zero = cfg.display.a_player;
        cfg.team.s2.player_scale = cfg.display.b_player;
        if cfg.data.to == "today" {
            cfg.data.to = crate::util::data_day(&cfg.data.db_path).unwrap_or_else(|| {
                eprintln!("note: {} has no db_updates stamp; the window ends today", cfg.data.db_path);
                crate::util::today_utc()
            });
        }
        Ok(cfg)
    }
}
