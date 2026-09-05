# A team-first rating for sports ChGK

This document is the specification. Code is checked against it. Constants marked *tunable* live in `config.toml`; the values that ship are in section 8.1 and were fixed offline by the calibrations of section 7.

## 1. Principles

1. A team's rating is its demonstrated strength on a stable scale, with an uncertainty. It moves only when the team plays.
2. A predicted winner that wins never loses points. The direction of every update comes from placement against the other teams in the field.
3. A lineup change never moves a team's rating. Arrivals are guests until they have earned their place. Departures widen the uncertainty, and results then move the rating quickly.
4. Inactivity widens uncertainty and never decays the number. The published table lists only teams and players with a rated game in the trailing 365 days.
5. The scale is anchored to the population: each release, the mean strength of established players is zero. Inflation is impossible by construction.
6. The forecast for a tournament is a separate object from the rating. It may use the lineup, the editors and the format.
7. Individual ratings are raw estimates from the question model, published with their uncertainty, with no ladder rule.
8. Publication is online and chronological. A published number is never rewritten. Full refits run offline for tuning and the initial backfill.

## 2. Data

Source: `buff.db`, a mirror of rating.chgk.info. Tables used: `tournaments`, `tournament_results`, `players`, `team_seasons` (roster, informational only), `releases` and `ratings` (old system, for comparison).

Scope: tournaments with `date_start` in `[from, to)` and `in_rating = 1`; the shipped range is `2013-01-01` to `2026-09-05`, the upper bound cutting off tournaments that are announced but not yet played. Asynchronous tournaments are never rated and never touch team ratings. They may enter the question model at weight `w_async` (tunable, default 0, i.e. excluded) if held-out log-loss improves.

Format weight `w_t`, an evidence multiplier: in-person (`Обычный`, `Региональный`) 1, strictly synchronous 2/3, synchronous 1/2. Marathons and any unrecognised `tournament_type` are excluded.

Per result: `team_id`, `position` (fractional for shared places), `questions_total` (count of correct answers), `mask` (one char per question: `1` correct, `0` wrong, `X` removed, `?` unknown; skip `X` and `?`), `team_members` (player ids of the actual lineup), and the JSON blob `rating`, from which two fields are read:

- `inRating` — the **rated flag**. A result with `inRating` false or absent is a pickup lineup without a base roster. It earns its team no rating update, no evidence and no listing credit, and never opens a team state; the team still stands in the field as an opponent for everyone else, entering the pairwise stage at its forecast strength (the roster estimate when it has no state), and its players update through the question model as usual. Of 522 131 rated-tournament results since 2013, 395 917 are flagged 1, 125 265 are flagged 0 and 949 are absent.
- `predictedPosition` — the old system's forecast, stored for the comparison of section 7.2.

Per tournament: `editors` (player ids), `tournament_type`, `date_start`, `questions_by_tour`.

Question count `n_q` of a tournament is the longest mask over its results. When no result carries a mask, `n_q` falls back to the sum of the comma-separated `questions_by_tour`; when that is absent too, `n_q = 0` and section 4.3 substitutes 36. About 2.4% of rated results have an empty mask; they take part in the team update by position only.

Events are processed in order of `date_start`, then tournament id. A release is a weekly snapshot (Thursday) of the state.

## 3. The question model (individual ratings)

### 3.1 Parameters

- Player `i`: strength `θ_i`, variance `σ_i²`, volatility `v_i`, affinity vector `u_i ∈ R^r`.
- Editor `e`: style vector `v_e ∈ R^r`. `r` is the affinity rank (tunable; 0 disables affinity — see section 8.2).
- Question `q` of tournament `t`: difficulty `b_q`, log-discrimination `log a_q`, log-step-count `log m_q`.

Affinity of player `i` to tournament `t`: `aff_i,t = mean over e in editors(t) of u_i · v_e`, zero when the tournament lists no editors.

### 3.2 Likelihood

For team lineup `L` on question `q`:

```
λ_i,q   = exp( a_q · (θ_i + aff_i,t − b_q) )
u_q     = Σ_{i∈L} λ_i,q
P_step  = 1 − exp( −u_q )
P(answer) = P_step ^ m_q
```

With `m_q = 1` this is the noisy-OR model of chgk.quest. `m_q > 1` punishes lopsided lineups relative to balanced ones: the collaboration extension (section 8.2). The log-likelihood of a tournament is weighted by `w_t`, priors excepted.

Priors: `log a_q ~ N(0, s_a²)`, `log m_q ~ N(0, s_m²)`, `b_q ~ N(b̄, s_b²)`, `u_i, v_e ~ N(0, s_aff²)`, all tunable. Setting `s_m → 0` fixes `m = 1`, which is the nested baseline every variant is tested against.

**Saturation.** `u_q` is rescaled to the interval `[e^{−u_max}, u_max]` before entering `P_step`, so a single team-question can never cost more than `u_max` nats. Without the cap a saturated prediction against the observed outcome makes the log-loss infinite. `log(1 − e^{−u})` is evaluated as `log(−expm1(−u))` below `log 2` and `log1p(−exp(−u))` above it.

### 3.3 Online update per tournament

1. **Question fit.** With all player parameters fixed, fit `(b_q, log a_q, log m_q)` for every question by Newton iterations with damping and step halving on the penalised log-likelihood `prior + w_t · Σ log P`. Questions are independent given the players, so this is cheap and parallel. Parameters are clamped to `[a_min, a_max]`, `[m_min, m_max]`, `[b_min, b_max]`.

2. **Player update.** With question parameters fixed, compute for each player in the tournament the gradient `g_i` and observed Fisher information `I_i` of the tournament log-likelihood with respect to `θ_i`, and likewise for `u_i`. A team's answers are not independent given `θ`, so one tournament carries less information than its curvature claims; the effective precision is discounted by `info_scale` (tunable, calibrated in section 7):

```
σ_i²  += v_i² · Δt                                  (diffusion since the last game)
prior  = σ_i²
I_eff  = w_t · I_i · info_scale
π_i'   = 1/prior + max(I_eff, info_floor)
θ_i   += clamp( w_t · info_scale · g_i / π_i' , ±θ_max_step )
σ_i²   = 1/π_i'
```

The affinity vector takes the same Kalman step against `v̄_t = mean_e v_e`, with its own diffusion `σ²_{u,k} += τ_aff² · Δt`.

Editor vectors `v_e` take an SGD step with learning rate `η_v` (tunable). The `N(0, s_aff²)` prior gradient is divided by the number of tournaments that editor has run so far, so the prior is spread over an editor's own data instead of being reapplied in full at every step; applied in full it is equivalent to a prior of `s_aff²/n` and pins every editor vector at zero.

3. **Teammate shrinkage.** After the update, every member of a lineup is pulled towards the lineup's mean:

```
θ_i += f · ( mean_{j∈L} θ_j − θ_i ),    f = teammate_shrink (tunable)
```

The noisy-OR likelihood credits an answer to whoever is already strongest, which drives teammates who play the same questions thousands of places apart. The shrinkage is the blunt correction; `f = 0` disables it.

4. **Held-out scoring**, for evaluation only: before step 1, split the tournament's teams into two halves at random (seeded by `crossfit_seed` and the tournament id), fit questions on one half, score log-loss on the other, and vice versa. Report this cross-fitted log-loss as the honest metric, and the in-sample post-fit log-loss for comparison with chgk.quest's published 0.501.

### 3.4 Time, volatility and newcomers

Between a player's tournaments, `σ_i² += v_i² · Δt` with `Δt` in years. The volatility `v_i` is per player, in the Glicko-2 spirit. After each tournament with `I_eff > info_floor`, from the standardised surprise

```
z_i = ( g_i / I_i ) / sqrt( prior + 1/I_eff )
log v_i += v_eta · ( log(v_max if |z_i| > 1 else v_min) − log v_i )
```

so a long, consistent history drives uncertainty to its floor whatever the calendar says, and only a run of results unlike the past reopens it. `v_i` starts at `τ_p`. Teams carry the same mechanism in place of a constant `τ_T` (section 4.3).

A player with no history enters at `θ = μ_new`, `σ = σ_new` (tunable, defaults −1.0 and 1.0). Recorded per player: count of rated games and date of the last one, for the listing rule.

### 3.5 Gauge

At each release: `shift = mean θ_i` over players with at least `n_est` (tunable, default 8) rated tournaments in the trailing 365 days. Subtract `shift` from every `θ_i`, every team `μ_T` and every team `s̄_T` (section 4). Questions are never reused, so `b` needs no correction. The cumulative shift is stored per release for diagnostics.

Player and team state is held on the pre-gauge scale and the cumulative shift subtracted on read, so a release costs `O(1)` and the published **change** since an entity's previous listed release (section 6) is its own movement, free of the gauge.

### 3.6 Backfill

The first season has no history, so every player starts at `μ_new` at once, the question difficulties of an elite 2013 field are underestimated and its players overshoot. The initial backfill therefore runs `passes` chronological sweeps (tunable, default 2) and writes output only on the last:

- Pass 1 runs as above. When it first reaches a tournament on or after `warm_until` (tunable, default 2016-01-01) it snapshots the published `θ` of every player who has already appeared.
- Each later pass restarts at `from` with those players at their snapshot value, `σ = warm_sigma`, `v = τ_p`, and every other player unseen, so they still enter at the newcomer prior when they first appear. Team states are rebuilt from scratch in every pass.

Because the snapshot is taken at `warm_until` and no player who debuts after it is warm-started, nothing after that date has seen its own future. Online publication after the backfill is a single pass, per principle 8.

## 4. The team model

### 4.1 Team strength scale

The roster estimate of a lineup `L` at tournament `t`:

```
s_r(L) = log Σ_{i∈L} exp(θ_i + aff_i,t) + roster_bias
Var    ≈ Σ_i p_i² σ_i² + σ_model²,   p_i = softmax weights of (θ_i + aff_i,t)
```

`s_r` is on the same scale as `θ`: a single player of strength `s_r` has the same total intensity as the lineup. `roster_bias` is the mean residual of realised team strength around the uncorrected sum of parts, calibrated offline and negative — real teams score below the independent-players estimate. `σ_model²` is the residual variance around it, also calibrated offline.

`roster_bias` enters the roster estimate used for new-team entry (4.2), for the forecast blend (4.3 step 4) and for the margin channel (4.3 step 6). The core strength `s̄_T` accumulates the uncorrected estimate, so the lineup offset `δ_L` of 4.3 step 2 is a difference of like with like and is unaffected.

### 4.2 Team state

- Rating `μ_T`, variance `σ_T²`, volatility `v_T`.
- Evidence `e_T,i` per player and a miss counter `k_T,i`: the weighted count of rated games with the team, and how many of the team's rated tournaments in a row that player has missed. Total `E_T = Σ_i e_T,i`.
- Core strength `s̄_T`: the evidence-weighted mean of the uncorrected roster estimate over the lineups with which the team earned `μ_T`.
- Date of last rated game, and the share of evidence lost at the previous tournament.

A team with `E_T = 0` is new. It enters at `μ_T = s_r(L)`, `σ_T² = Var(s_r(L)) + σ_new_team²`, `s̄_T = s_r(L) − roster_bias`, `v_T = τ_T`. A result flagged `inRating = 0` never opens a state.

### 4.3 Per tournament, for each team `T` with lineup `L`

The field is every rated team of the tournament with a non-empty lineup or an existing state; a team with neither is skipped. Steps 1 to 5 read the player states as they stood before the tournament and are computed for the whole field before any state is written.

1. Continuity `c = Σ_{i∈L} e_T,i / E_T` (1 for a new team). This is the share of the team's evidence present today.
2. Lineup offset `δ_L = (s_r(L) − roster_bias) − s̄_T`: how much stronger or weaker today's lineup is than the lineups that earned the rating.
3. Expected strength for the update: `x_T = μ_T + δ_L`. The rating is judged relative to its own core; guests raise the expectation, absences lower it.
4. Forecast strength, published only: the precision-weighted blend of `(x_T, σ_T²)` and `(s_r(L), Var(s_r(L)))`. After heavy departures `σ_T²` is wide and the forecast is close to the roster estimate; the rating is not. Forecast places are the ranking of forecast strengths in the field, 1 strongest. A team whose result is flagged `inRating = 0` enters step 5 at its forecast strength instead of `x_T`.
5. Pairwise update. For every other team `B` in the field:

```
P_AB = logistic( (x_A − x_B) / β_t ),     β_t = β · sqrt(36 / n_q),  n_q = 36 when unknown
y_AB = 1 if A placed above B, 0 if below; a shared place contributes nothing
m_AB = min( 1 + |score_A − score_B| / κ_m , m_max ),  m_AB = 1 when either score is missing
S_A  = Σ_B (y_AB − P_AB) · m_AB
I_A  = Σ_B P_AB (1 − P_AB) · m_AB
```

`β` is the slope of team strength against pairwise wins on a 36-question set, calibrated offline once (tunable). A shared place is uninformative about which team is better and is skipped in both sums, equivalent to `y = P` at zero information. Because `y − P ≥ 0` for every pair a first-placed team is in, its `S_A ≥ 0`: principle 2 holds by construction, for shared first places as well as sole ones. Margin only scales magnitude.

Pairs are not independent, so normalise: multiply `S_A` and `I_A` by `n_eff / (N − 1)` with `n_eff = min(N − 1, ν)` (tunable `ν`).

6. Apply, scaled by continuity and format. Two channels contribute precision; only the placement channel moves the mean.

```
realised s*  = the strength at which one player of that strength would answer
               exactly `questions_total` of this package (bisection on the fitted
               question parameters); absent when the mask is empty
π_pair = w_t · c · I_A / β_t²
π_marg = w_t · c / σ_model²        when s* exists, 0 otherwise
π_T'   = 1/σ_T² + π_pair + π_marg
μ_T   += w_t · c · (S_A / β_t) / π_T'
σ_T²   = 1/π_T'
```

The score says how strong the team was but not whom it beat, so it sharpens the posterior without moving it and the winner rule of step 5 stays exact. Without this channel a team that wins nearly every pair has `P(1−P) ≈ 0` and gathers information an order of magnitude more slowly than an ordinary team, and never leaves the provisional list.

The standardised residual of that same observation is stored for the calibration of section 7:

```
z_T = ( s* − x_T ) / sqrt( σ_T² + σ_model² )     (σ_T² before the update)
```

7. Bookkeeping after the update:
   - Present players: `e_T,i = min(e_T,i + w_t, e_cap)`, `k_T,i = 0`.
   - Absent players: `k_T,i += 1`. Once `k_T,i ≥ miss_streak` (tunable) their evidence decays, `e_T,i *= ρ`, and joins the lost share `ℓ = Σ_decaying e_T,i (1 − ρ) / E_T`, which widens uncertainty: `σ_T² /= (1 − ℓ)^γ` with `γ ≥ 1` (tunable), `ℓ` clamped at 0.9. This is the departure rule: convex in the share lost, no threshold, and it touches only the uncertainty. The streak condition is what separates a departure from a rotating roster; without it a team that fields nine players in turn is treated as losing a member every week.
   - `σ_T²` is then capped at `Var(s_r(L)) + σ_new_team²`: after total turnover a team knows no more about itself than a new team of the same lineup.
   - `s̄_T` is updated as an evidence-weighted running mean, weight `E_T` before the update against `w_t · |L|` for the new lineup.
   - Volatility, from the pairwise innovation `S_A · β_t / I_A`:

```
z_pair = ( S_A · β_t / I_A ) / sqrt( σ_T² + 1/π_pair )      (σ_T² before the update)
log v_T += v_eta · ( log(v_max if |z_pair| > 1 else v_min) − log v_T )
```

   - Between tournaments: `σ_T² += v_T² · Δt`.

A lineup with `c = 0` (an old team id used by strangers) gets a forecast but no update. The date of the last rated game advances only on a result flagged `inRating = 1`.

Teams with an empty mask still take part in step 5 with `score` from `questions_total`; if that is missing too, `m_AB = 1`.

### 4.4 Tournament difficulty

Published alongside the ratings, computed from the fitted question parameters against a fixed reference lineup of six identical players of strength `θ*` (`truedl.theta_ref`, fixed at launch and never recomputed):

```
p_q      = ( 1 − exp( −6 · exp( a_q (θ* − b_q) ) ) ) ^ m_q
truedl_t = 10 · ( 1 − mean over q of p_q ),   q ranging over questions with at least one answer
```

NULL when no question of the tournament was answered by anyone. `θ* = 0.2368` is the roster strength of a balanced team at places 251-500 re-expressed as six equal players, which is the band the published TrueDL uses as its reference; the number therefore means what TrueDL claims it means — ten times the share of the package such a team would miss — while being computed from fitted difficulties rather than a band-ratio table.

## 5. Display scale

Internal numbers are on the `θ` scale with mean zero. Published numbers are affine maps, fixed once at launch and never revised:

```
R_team   = A_T + B_T · μ_T
R_player = A_P + B_P · θ_i
```

Constants are chosen so that the 50th and 1000th of our listed non-provisional teams at the release nearest 2019-09-05 match the old system's values at those places (7028 and 2992), and likewise for players at the release nearest 2019-10-03 (12865 and 9778). The top lands where the internal scale puts it. A rating with `σ > provisional_sigma` (tunable, default 0.3) is published as provisional and left out of the ranked tables.

## 6. Outputs

`out/ratings.db` (SQLite). Per-player and per-team rows are written every `rows_every` releases and always on the last; releases and their counts are weekly regardless.

- `releases(id, date, shift, shift_cum, n_players)`
- `player_ratings(release_id, player_id, theta, sigma, games_365, display, change)`
- `team_ratings(release_id, team_id, mu, sigma, core_strength, evidence, display, listed, change)` — listed teams only, so `listed` is always 1
- `team_events(tournament_id, date, team_id, c, delta_l, x, s_r, forecast, forecast_place, actual_place, s_pair, i_pair, mu_before, mu_after, sigma_before, sigma_after, evidence, gap_days, var_diff, lost_prev, rated, z)`
- `question_params(tournament_id, q, b, a, m)`
- `tournament_metrics(tournament_id, date, type, weight, n_teams, n_q, n_obs, logloss_insample, logloss_crossfit, spearman_forecast, spearman_old_system, truedl)`

`change` is the entity's own movement since its previous written release, excluding gauge shifts, and 0 on a first appearance or an idle week. The columns beyond section 4's formulas — `sigma_before`, `sigma_after`, `evidence`, `gap_days`, `var_diff`, `lost_prev`, `z` — are what the uncertainty calibration of section 7 reads; `s_r` and `rated` are what the site shows.

Indexes: `team_events(team_id)`, `team_events(tournament_id)`, `team_ratings(release_id)`, `team_ratings(team_id)`, `player_ratings(player_id)`.

## 7. Calibration and acceptance tests

### 7.1 Offline calibrations

`β` and `σ_model` come from `calibrate`, over in-person tournaments 2015 to 2024: `β` as the maximum-likelihood logistic slope of pairwise wins against the difference of roster estimates rescaled to a 36-question basis (812 782 pairs), `σ_model` and `roster_bias` as the standard deviation and mean of `s* − s_r` over full six-player lineups (25 971 lineups). The display constants come from `calibrate --display` against the 2019 releases of section 5.

`info_scale` and `ν` are fixed by the requirement that the published uncertainty be honest: over in-person tournaments from 2018, the standardised residuals must have unit variance — `z_i` of section 3.4 for players, `z_T` of section 4.3 step 6 for teams with `E_T ≥ 6`. The same test on teams returning after a year of inactivity fixes the volatility floor, and on teams that lost 30% of their evidence would fix `γ`; with `miss_streak` in force that group is empty, so `γ` keeps its default and is not identified by the data.

### 7.2 Acceptance tests

1. **Cross-fitted log-loss** on in-person rated tournaments, per variant. A variant (step count, affinity rank, async weight, teammate shrinkage) stays only if it lowers the number. **Result: 0.4891 over all seasons, 0.5180 in 2025/26.** Passed.
2. **Forecast rank correlation.** Spearman between forecast places and actual places per tournament, averaged over in-person tournaments of the last two seasons, against the old system's `predictedPosition`. Target: within 0.02 of the best external system. **Result: 0.8124 against 0.8177; sync 0.8073 against 0.7897.** Passed.
3. **Individual top-200 overlap with chgk.quest of at least 70%.** **Result: 126/200 against chgk.quest and 128/200 against rating.chgk.fun. Failed.** For context those two systems agree with each other on 136/200, so the 140 threshold is above what mature external systems reach between themselves; the rank correlation of `θ` over players with at least 50 games is 0.91 against chgk.quest and 0.79 against rating.chgk.fun.
4. **Inflation:** the display value of the 50th team at each September release from 2015 to 2026 stays within a band of ±10%. A September whose trailing year holds fewer than 200 rated teams is a hole in the data, not inflation, and is excluded from the band; only September 2021 is. **Result: largest deviation 5.4% over 11 Septembers (range 6380 to 7045 around a mean of 6743).** Passed.
5. **Transfer invariance:** swap the strongest player between two top teams in the input at a chosen date and confirm no team rating changes at that date. Forecasts must change. **Result: 0 teams moved, maximum difference exactly 0; the two teams' forecasts at their next tournament move by 0.38 while `mu_before` is bit-identical.** Passed.
6. **Winner rule:** no team that finished first, sole or shared, has `mu_after < mu_before` anywhere in `team_events`. **Result: 0.** Passed.
7. The user's read of the resulting tables.

## 8. Implementation

Rust workspace, one library crate `chgk-rating` with modules `data`, `question_model`, `team_model`, `gauge`, `display`, `output`, `util`, and binaries `fit` (online pass, writes `out/ratings.db`), `eval` (metrics report from the outputs), `calibrate` (`β`, `σ_model`, `roster_bias`, and the display constants). Configuration in `config.toml`. Parallelism through rayon over questions within a tournament and over teams within a tournament's pairwise stage.

`fit --swap PLAYER_A:TEAM_A,PLAYER_B:TEAM_B@DATE` exchanges two players between two teams from that date on, for test 5. `fit --diag-player ID` prints that player's uncertainty decomposition at every release.

### 8.1 Shipped constants

```
[data]     w_in_person 1, w_strict_sync 2/3, w_sync 1/2, w_async 0, rows_every 1, passes 2
[question] b_bar 1.5, s_b 2.0, s_a 0.5, s_m 0, s_aff 0.3, aff_rank 0, eta_v 0.2
[player]   mu_new −1.0, sigma_new 1.0, warm_sigma 0.5, warm_until 2016-01-01,
           info_scale 0.46, teammate_shrink 0.005,
           tau_p 0.25, v_min 0.05, v_max 0.6, v_eta 0.15, tau_aff 0.05
[gauge]    n_est 8, window_days 365
[team]     sigma_model 0.3669, roster_bias −0.2427, sigma_new_team 0.5, beta 0.3757,
           kappa_m 10, m_max 2, nu 2.6, e_cap 20, rho 0.8, miss_streak 3, gamma 2,
           tau_t 0.2, v_min 0.05, v_max 0.6, v_eta 0.15
[truedl]   theta_ref 0.2368
[display]  provisional_sigma 0.3
[numeric]  newton_iters 12, a ∈ [0.3, 3], m ∈ [0.5, 4], b ∈ [−10, 12],
           info_floor 1e−3, theta_max_step 1.5, u_max 15
```

### 8.2 Reserved extensions

Two extensions are implemented, tested and switched off. Both stay in the code behind their config switch; neither lowered the cross-fitted log-loss of test 1.

- **Free step count** (`question.s_m > 0`), the collaboration term `m_q` of section 3.2. Mean fitted `m` was 1.056; it bought in-sample fit and paid for it out of sample.
- **Affinity** (`question.aff_rank > 0`), the player-editor interaction `u_i · v_e` of section 3.1. Learned magnitudes reached about 0.01 on the `θ` scale: the Fisher information for `u_i` is smaller than that for `θ_i` by a factor `v̄²`, so a single online pass cannot identify it. A fair test needs a full offline refit.

## 9. Report

Once the acceptance tests pass, a PDF describing the system in the register of Приложение А of the current regulations, in Russian, with formulas, is produced with Typst from `report/report.typ`. Fonts: Literata Fix for text and Literata Math for formulas, kept in `fonts/`, fetched by `tools/fetch_fonts.py`. Build with `typst compile --font-path fonts report/report.typ`.

## 10. Site

A server, not a static build: the `web` crate reads `out/ratings.db` and answers
every request from SQLite, so any of the 161 194 players who have played since
2013 has a page, not only the 24 970 in the current release. Pages: team table,
player table, team page with rating history and events, player page, tournament
page with forecast against result, releases, search over players and teams,
anomalies, methodology, and the PDF report.

The anomalies page ranks players by how much their results depend on the format.
Cheating is close to impossible in person, awkward on a synchronous tournament
and trivial when a synchronous tournament is played at home, so a rating not
earned at the table shows a gap. Two independent measurements per result:
`ln(forecast place) − ln(actual place)`, centred on where in the field the
forecast put the team; and the team's edge over the field on the third of the
package the field found hardest, given its edge on the third it found easiest,
which uses no ratings at all. Each is taken as the player's online mean minus
their in-person mean, shrunk towards zero by its own noise, and the score is the
two shrunk gaps in units of their spread across players. It is a triage list,
not a verdict: both statistics are team statistics attributed to everyone in the
lineup.

Two things neither database can answer with an index it already has — one
player's games, which live behind a JSON blob in `buff.db` with no index, and
the two ranking tables, which need the last release ordered by rating out of
18.7 million rows — are precomputed once into `out/web.db` and rebuilt whenever
`out/ratings.db` is newer.

Rating history is interactive: the chart carries its points in viewBox
coordinates, and the crosshair is a nearest-x lookup that draws a vertical rule
and a dated tooltip. Without JavaScript the line and its band still render.

Styling follows the user's design system in `~/dopesuite/dopeuikit`: reuse its
tokens, type scale and table styles from `assets/core.css` and its fonts from
`assets/fonts`, rather than inventing new ones. Run the `design-review` skill
from `~/dopesuite/.claude/skills/design-review` on the built pages before
calling the site done.
