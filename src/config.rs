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
    pub player_scale: f64,
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
}

impl Default for S2 {
    fn default() -> S2 {
        S2 {
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
            series_k: 5,
            editor_memory: 0.0,
            ed_min_n: 2.0,
            ed_gate: 0.0,
            ed_half_life: 0.0,
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
            player_scale: 2454.8828,
            player_zero: 6000.0,
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
    /// The same tournaments, but for the player side only: they move `theta` and
    /// `phi` and never open or move a team state, never produce a team event and
    /// never enter the metric's tournament set.
    #[serde(default)]
    pub train_unrated: bool,
    /// `include_unrated` for in-person tournaments alone: an unrated in-person
    /// tournament counts as an ordinary one, and its results flagged
    /// `inRating` move their teams.
    #[serde(default)]
    pub unrated_in_person: bool,
    /// One field for an in-person tournament played in several cities: the
    /// fields of one day that share the editors and the question count are
    /// merged under the lowest id and re-ranked by questions taken.
    #[serde(default)]
    pub merge_cities: bool,
    /// Weight of one of those tournaments, on top of its format weight.
    #[serde(default = "one")]
    pub w_unrated: f64,
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
    /// Volatility floor of the second, fast estimate of the same strength. It
    /// takes the same Kalman step against the same gradient and forgets far
    /// faster; only the forecast reads it, through `team.s2.k_fast`. 0 is off.
    #[serde(default)]
    pub v_fast: f64,
    #[serde(default = "kalman")]
    pub update: PUpdate,
    /// `update = "schedule"`: step `eta0 / sqrt(c0 + games)`.
    #[serde(default = "eta0")]
    pub eta0: f64,
    #[serde(default = "c0")]
    pub c0: f64,
}

fn tau_phi() -> f64 {
    0.05
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
    /// How much of the format effect the published player number carries:
    /// 0 is `theta`, the strength away from an in-person table, 1 is
    /// `theta + phi`, the strength at one. The forecast never reads this.
    #[serde(default)]
    pub player_phi: f64,
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
