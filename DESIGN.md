# A team-first rating for sports ChGK

This document is the specification. Code is checked against it. Constants marked *tunable* live in `config.toml`; the values that ship are in section 8.1 and were fixed offline by the calibrations of section 7.

## 1. Principles

1. A team's rating is its demonstrated strength on a stable scale. It moves only when the team plays.
2. A predicted winner that wins never loses points. The direction of every update comes from placement against the other teams in the field.
3. A lineup change never moves a team's rating on the day it is declared. Today's lineup shifts the strength the team is judged at; over the games that follow, the rating drifts towards what the lineups that turn up are worth, so a roster change is absorbed by playing rather than by decree.
4. Inactivity never decays the number. The published table lists only teams and players with a rated game in the trailing 365 days.
5. The scale is anchored to the population: each release, the mean strength of established players is zero. The team scale stands on the same footing, because both a lineup's strength and a new team's first rating are read off the players.
6. The forecast for a tournament is a separate object from the rating: it is the strength of the lineup that turned up, and uses no team state at all.
7. Individual ratings are raw estimates from the question model, published with their uncertainty, with no ladder rule.
8. Publication is online and chronological. A published number is never rewritten. Full refits run offline for tuning and the initial backfill.

## 2. Data

Source: `buff.db`, a mirror of rating.chgk.info. Tables used: `tournaments`, `tournament_results`, `players`, `team_seasons` (roster, informational only), `releases` and `ratings` (old system, for comparison).

Scope: tournaments with `date_start` in `[from, to)` and `in_rating = 1`; the shipped range is `2013-01-01` to `2026-09-05`, the upper bound cutting off tournaments that are announced but not yet played. Asynchronous tournaments are never rated and never touch team ratings. They may enter the question model at weight `w_async` (tunable, default 0, i.e. excluded) if held-out log-loss improves.

Format weight `w_t`, an evidence multiplier: in-person (`Обычный`, `Региональный`) 1, strictly synchronous 2/3, synchronous 1/2. Marathons and any unrecognised `tournament_type` are excluded.

Per result: `team_id`, `position` (fractional for shared places), `questions_total` (count of correct answers), `mask` (one char per question: `1` correct, `0` wrong, `X` removed, `?` unknown; skip `X` and `?`), `team_members` (player ids of the actual lineup), and the JSON blob `rating`, from which two fields are read:

- `inRating` — the **rated flag**. A result with `inRating` false or absent is a pickup lineup without a base roster. It earns its team no rating update, no evidence and no listing credit, and never opens a team state; the team still stands in the field as an opponent for everyone else, entering the pairwise stage at its own field strength, or at the strength of its lineup when it has no state, and its players update through the question model as usual. Of 522 131 rated-tournament results since 2013, 395 917 are flagged 1, 125 265 are flagged 0 and 949 are absent.
- `predictedPosition` — the old system's forecast, stored for the comparison of section 7.2.

Per tournament: `editors` (player ids), `tournament_type`, `date_start`, `questions_by_tour`.

Question count `n_q` of a tournament is the longest mask over its results. When no result carries a mask, `n_q` falls back to the sum of the comma-separated `questions_by_tour`; when that is absent too, `n_q = 0` and section 8.3 substitutes 36. S2 does not read `n_q` at all. About 2.4% of rated results have an empty mask; they take part in the team update by position only.

Events are processed in order of `date_start`, then tournament id. A release is a weekly snapshot (Thursday) of the state.

## 3. The question model (individual ratings)

### 3.1 Parameters

- Player `i`: strength `θ_i`, variance `σ_i²`, volatility `v_i`, affinity vector `u_i ∈ R^r`. Under the format effect of section 3.7 the affinity slot holds the format effect `φ_i` instead.
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

1. **Question fit** (`question.fit`, `take_rate` ships). The difficulty is the closed form that reproduces the observed take rate of the lineups that played the question, with `a_q = m_q = 1`:

```
b_q = mean over lineups L of ln R_L  −  ln( −ln(1 − r) ),   R_L = Σ_{i∈L} exp(θ_i + aff_i,t)
```

`r` is the share of those lineups that took the question, clamped to `[0.02, 0.98]` so a question everyone or nobody took stays on the scale. Setting `b_q` this way makes the model's predicted take rate for a lineup of average reach equal to the observed one, exactly.

`question.fit = "newton"` replaces it with the fit it succeeded: with all player parameters fixed, `(b_q, log a_q, log m_q)` per question by Newton iterations with damping and step halving on the penalised log-likelihood `prior + w_t · Σ log P`, clamped to `[a_min, a_max]`, `[m_min, m_max]`, `[b_min, b_max]`. Questions are independent given the players, so either way this is cheap and parallel. The ladder of `out/ladder/report.md` measured the difference: 0.0009 of cross-fitted log-loss and 0.018 in sample. The questions were nearly at the closed form already.

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

so a long, consistent history drives uncertainty to its floor whatever the calendar says, and only a run of results unlike the past reopens it. `v_i` starts at `τ_p`. The Kalman team engine carries the same mechanism (section 8.3); S2 has no volatility.

A player with no history enters at `θ = μ_new`, `σ = σ_new` (tunable, defaults −1.0 and 1.0). Recorded per player: count of rated games and date of the last one, for the listing rule.

### 3.5 Gauge

At each release: `shift = mean θ_i` over players with at least `n_est` (tunable, default 8) rated tournaments in the trailing 365 days. Subtract `shift` from every `θ_i`. Under `engine = "kalman"` it is subtracted from every team `μ_T` and `s̄_T` as well; S2 needs no team correction, because it never holds a team number on the `θ` scale — the shift reaches it only through the player ratings a lineup strength is read off. Questions are never reused, so `b` needs no correction. The cumulative shift is stored per release for diagnostics.

Player and team state is held on the pre-gauge scale and the cumulative shift subtracted on read, so a release costs `O(1)` and the published **change** since an entity's previous listed release (section 6) is its own movement, free of the gauge.

### 3.6 Backfill

The first season has no history, so every player starts at `μ_new` at once, the question difficulties of an elite 2013 field are underestimated and its players overshoot. The initial backfill therefore runs `passes` chronological sweeps (tunable, default 2) and writes output only on the last:

- Pass 1 runs as above. When it first reaches a tournament on or after `warm_until` (tunable, default 2016-01-01) it snapshots the published `θ` of every player who has already appeared.
- Each later pass restarts at `from` with those players at their snapshot value, `σ = warm_sigma`, `v = τ_p`, and every other player unseen, so they still enter at the newcomer prior when they first appear. Team states are rebuilt from scratch in every pass.

Because the snapshot is taken at `warm_until` and no player who debuts after it is warm-started, nothing after that date has seen its own future. Nothing snapshots the affinity or the format effect, so a later pass restarts them at their prior for the same reason. Online publication after the backfill is a single pass, per principle 8.

Passes beyond the second change nothing: the snapshot is taken on the first pass only, so every later pass starts from the same state and follows the same trajectory. `passes = 3` reproduces `passes = 2` exactly.

### 3.7 The format effect

A player is not equally strong in every format. Cheating aside (section on `anomaly`), a table that meets face to face works differently from one playing a synchronous package at home, and the gap is per player. `question.format_effect` (`in_person`, shipped) gives each player a second scalar `φ_i` and reads their strength on question `q` of tournament `t` as

```
z_i,t = θ_i + s_t · φ_i,     s_t = 1 at an in-person tournament, 0 elsewhere
```

`φ_i ~ N(0, s_φ²)` with `s_φ` tunable. It takes the same Kalman step as `θ` against the same gradient and the same information, with its own diffusion, and only where `s_t ≠ 0`:

```
σ²_φ  += τ_φ² · Δt
π'     = 1/σ²_φ + max( w_t · I_i · info_scale , info_floor )
φ_i   += clamp( s_t · w_t · info_scale · g_i / π' , ±θ_max_step ),  then clamped to ±5 s_φ
σ²_φ   = 1/π'
```

This is the affinity of section 3.1 at rank 1 against a fixed indicator instead of an editor vector, and it occupies the same slot: `aff_rank` and `format_effect` cannot both be on.

`z_i,t` is what the question fit and the player gradient of section 3.3 read, and what the lineup strength of section 4.1 is built from, so the forecast for an in-person tournament uses `θ + φ` and the forecast for a synchronous one uses `θ` alone. The **published** player rating is `θ`: the format effect is a deviation from it, not part of it, and it is what the gauge of section 3.5 centres.

`format_effect = "signed"` — `s_t = −1` off an in-person table, so `φ` is half the gap between the formats — was tried and lost by a wide margin (section 7.4).

Over the 24 970 players listed at the last release the fitted `φ` has mean 0.013 and median 0.000 — most listed players have never played in person — with the 10th and 90th percentiles at −0.059 and 0.100 and a range of −0.51 to 0.86. On the team scale a `φ` of 0.1 is about 245 points.

## 4. The team model

Model S2: Elo in a field, kept on the published scale. Six constants and no
algebra beyond a weighted average. `team.engine` selects it (`s2`, shipped)
against the Kalman filter it replaced, which stays selectable as section 8.3.

### 4.1 The team scale and the strength of a lineup

A player's strength on the team scale is the affine image of `θ`:

```
r_i = player_zero + player_scale · (θ_i + aff_i,t − σ_c · σ_i + ln 6 + roster_bias)
```

so that six equal players of rating `r` make a lineup of strength exactly `r`.
`player_scale` is the display slope of the engine this one replaced;
`player_zero` is a free shift, because nothing in S2 reads an absolute level.
`aff_i,t` is the format effect `s_t · φ_i` of section 3.7 under the shipped
switch, so a lineup at an in-person tournament is read at `θ + φ` and one at a
synchronous tournament at `θ` alone.

`σ_c` (tunable, 1.5) is the **pessimistic quantile**: a player enters at
`θ − σ_c σ` rather than at `θ`, with `σ` as of the day of the tournament,
diffusion included. A newcomer at `σ = 1` therefore counts a full 1.5 lower than
an established player with the same estimate, and a lineup with an unknown in it
— the usual reason a forecast misses at the top — is no longer credited with
that unknown's prior. `σ_c = 0` is the rule that shipped before.

The **strength of the lineup** `L` that turned up is the tempered roster
estimate — the strength at which a single player would answer as much as the
whole table, softened by a temperature `τ` (tunable, 0.7) — mapped by the same
affine map. In display units the bias and the zero cancel and only the slope
survives:

```
Q(L) = τ · player_scale · ( ln Σ_{i∈L} exp( r_i / (τ · player_scale) ) − ln 6 )
```

A short table is worth less than a full one and a strong player carries more
than his share of the average, both by construction rather than by a penalty.
`τ = 1` is the plain roster estimate, the engine's own likelihood and the rule
that shipped before; `τ < 1` sharpens it towards the strongest player at the
table, and forecasts the top of the field better.

`team.s2.lineup` selects the rule. All four agree on a full lineup of six equal
players; section 7.4 has the numbers.

- `tempered`, the rule above, shipped;
- `roster`, the same at `τ` fixed to 1;
- `mean`, the mean of `r_i` less `delta` per seat short of six;
- `best6`, chgk.gg's rule — the sorted lineup under weights 1, 5/6, 2/3, 1/2,
  1/3, 1/6, normalised, with each seat short of six filled at the weakest player
  less `delta`.

`delta` belongs to the last two and is unused under the first two.

The roster estimate also has a variance, which `calibrate` and section 8.3 use
and S2 does not.

### 4.2 Team state

- Rating `R`, on the published scale.
- Usual lineup `Q̄`: the strength of the lineups the rating was earned with.
- Count of rated games `n` and the date of the last one.

A team with no state opens one at its first rated result, at `R = Q̄ = Q(L)`.
A result flagged `inRating = 0` never opens a state.

### 4.3 Per tournament

The field is every result with a lineup or an existing state; a result with
neither is skipped. Everything below reads the player states as they stood
before the tournament.

1. **Field strength.** `F_T = R + (Q(L) − Q̄)`: the rating, shifted by how much
   stronger or weaker today's lineup is than the lineups that earned it. A team
   opening its state stands at `Q(L)`; a result with no lineup listed stands at
   `R`. Guests raise the expectation and absences lower it; neither moves `R` on
   its own, but a lineup that keeps turning up pulls `R` after it in step 3.

2. **Pairwise.** Against every other team `B` of the field,

```
P_AB = 1 / ( 1 + 10 ^ ( (F_B − F_A) / D ) )
y_AB = 1 if A placed above B, 0 if below; a shared place is skipped in both sums
S_A  = ( Σ_B y_AB  −  Σ_{B not tied with A} P_AB ) / (N − 1)
```

A shared place says nothing about which of the two teams is better, so its
expectation is left out along with its outcome. Because `y − P ≥ 0` for every
pair a first-placed team is in, `S_A ≥ 0` for a team that finished first, sole
or shared, so this step alone never lowers a winner; the floor of step 3 keeps
that true of the whole update, which is principle 2. The margin of the score
does not enter at all.

3. **Update**, on a rated result only, in this order:

```
R0  = R                                   the rating the team came with
R  += K · w_t · D · S_A                   the Elo step
R  += K_r · w_t · ( Q(L) − R )            the pull, when a lineup is listed
R   = max(R, R0)                          when A finished first, sole or shared
Q̄  += K_q · ( Q(L) − Q̄ )                 when a lineup is listed
n  += 1
```

The **pull** is what makes `R` mean something on its own. Without it the rating
is a pure placement number: it knows who beat whom and nothing about who was at
the table, so it orders a field of base-roster teams worse than either the old
system or chgk.gg (section 7.3). With it the rating takes `K_r` of the way
towards the lineup's strength at every rated game, which is the question model's
evidence entering the team number by the only door principle 1 leaves open — a
game played.

The pull can lower a team that won, when the team is rated above what its lineup
is worth, so principle 2 needs the **floor**: a team that finished first, sole or
shared, keeps at least the rating it came with. At `K_r = 0` the floor never
binds, because `S_A ≥ 0` for a first place already.

`K_q` is the memory of the usual lineup and has to be of the same order as
`K_r`: if the rating tracks the lineups and `Q̄` does not, the offset of step 1
counts today's table twice, and the innovations of section 7.2 go from white to
+0.19 autocorrelated.

An unrated result stands in the field for everyone else and takes no update, no
game and no listing credit. The date of the last rated game advances only on a
rated result.

### 4.4 Forecast

The forecast for a team at a tournament is `Q(L)`, the strength of the lineup it
declared, falling back to `F_T` when the result lists no lineup. Forecast places
are the ranking of the field by it, ties sharing the mean place.

The forecast therefore reads no team state whatsoever. That is not a
simplification but the measured result: over the in-person tournaments of the
last two seasons the lineup alone forecasts better than the team rating, better
than the rating with the lineup offset, and better than the old system's
`predictedPosition` (section 7.2). Teams are stable; the question of who turns up
is not, and the lineup answers it.

### 4.5 What is published

`R` is published through the affine map of section 5. A team's `sigma` is the provisional flag in the shape the
schema expects — `0.1` at five rated games or more, `0.5` below — so
`display.provisional_sigma` keeps a team out of the ranked tables until it has
five. S2 estimates no variance and publishes none.

### 4.6 Inactivity and lineup change

Inactivity has no effect: a rating does not decay between games, and docking an
untested rating 100 or 200 points a year was measured and makes the ranking
slightly worse (section 7.3). A lineup change has no effect on the day it is
declared — a team that changes half its roster and then plays nothing keeps the
number it had. What a lineup change does is move `Q(L)`, and from the next rated
game on both the pull of step 3 and the usual lineup of step 3 follow it, with
time constants of about three and about ten games. There is no volatility, no
departure rule and no evidence per player: `evidence` in the outputs is the count
of rated games.


### 4.7 Tournament difficulty

Published alongside the ratings, computed from the fitted question parameters against a fixed reference lineup of six identical players of strength `θ*` (`truedl.theta_ref`, fixed at launch and never recomputed):

```
p_q      = ( 1 − exp( −6 · exp( a_q (θ* − b_q) ) ) ) ^ m_q
truedl_t = 10 · ( 1 − mean over q of p_q ),   q ranging over questions with at least one answer
```

NULL when no question of the tournament was answered by anyone. `θ* = 0.2368` is the roster strength of a balanced team at places 251-500 re-expressed as six equal players, which is the band the published TrueDL uses as its reference; the number therefore means what TrueDL claims it means — ten times the share of the package such a team would miss — while being computed from fitted difficulties rather than a band-ratio table.

### 4.8 Reserved extensions

Four team-side switches are implemented, measured against section 7.3 and
switched off. All ship at the value that leaves 4.3 unchanged.

- **Margin** (`team.s2.margin_g > 0`) replaces the win/lose outcome of step 2
  with the score gap, `y_AB = clamp(½ + (s_A − s_B) / 2g, 0, 1)`, or the logistic
  `1 / (1 + 10^(−(s_A − s_B)/g))` under `margin_shape = "logistic"`. It loses
  0.04 of weighted NDCG@3 at `g = 8` and 0.08 at `g = 16`; the order of finish
  already carries what there is to carry, and the gap adds the package's length
  and hardness as noise.
- **Faster start** (`team.s2.n0 > 0`) enlarges the step of a provisional team to
  `K · max(1, n₀ / n)`. Worth +0.0017 of weighted NDCG@3 at `n₀ = 10` against a
  bootstrap SE of 0.0010, and negative by `n₀ = 20`.
- **Team-side format weights** (`team.s2.w_sync`, `w_strict_sync`) let the team
  update weigh a format differently from the player update. Counting a
  synchronous result as heavily as an in-person one is +0.0029 at an SE of
  0.0034.
- **Pull target** (`team.s2.k_r_target = "usual"`) pulls towards `Q̄` instead of
  `Q(L)`. It does nothing at any `K_r`, which is what a hundred-game average of
  a quantity the rating already tracks should do.

## 5. Display scale

Published numbers are affine maps of the internal ones, fixed once and never revised:

```
R_team   = A_T + B_T · R
R_player = A_P + B_P · θ_i
```

Both maps come from `calibrate --display`: `A_T`, `B_T` so that the 50th and 1000th of our listed non-provisional teams at the release nearest 2019-09-05 match the old system's values at those places (7028 and 2992), and `A_P`, `B_P` likewise for players at the release nearest 2019-10-03 (12865 and 9778). The top lands where the internal scale puts it.

S2 carries its rating on an internal team scale set by the player map of section 4.1: `player_scale` fixes the slope and `player_zero` the level. `player_scale` is not free — halving it halves every rating, every lineup strength and every step relative to `D`, which is a different model — and `player_zero` is a pure shift. The published map only moves that scale onto the familiar level: its slope is 1.19 because the S2 spread between the 50th and 1000th teams in 2019 is narrower than the old system's.

A rating with `σ > provisional_sigma` (tunable, default 0.3) is published as provisional and left out of the ranked tables. Under S2 that is exactly "fewer than five rated games" (section 4.5).

## 6. Outputs

`out/ratings.db` (SQLite). Per-player and per-team rows are written every `rows_every` releases and always on the last; `rows_every = 0` writes them at the last release only, which is what the research configs do. Releases and their counts are weekly regardless.

- `releases(id, date, shift, shift_cum, n_players)`
- `player_ratings(release_id, player_id, theta, sigma, games_365, display, change)`
- `team_ratings(release_id, team_id, mu, sigma, core_strength, evidence, display, listed, change)` — listed teams only, so `listed` is always 1
- `team_events(tournament_id, date, team_id, c, delta_l, x, s_r, forecast, forecast_place, actual_place, s_pair, i_pair, mu_before, mu_after, sigma_before, sigma_after, evidence, gap_days, var_diff, lost_prev, rated, z, expected_place)`
- `player_events(tournament_id, player_id, theta_before, theta_after, sigma_after)` — one row per player in a lineup, on the published scale as of that tournament, so the difference is the player's own movement including the teammate shrinkage of section 3.3
- `question_params(tournament_id, q, b, a, m)`
- `tournament_metrics(tournament_id, date, type, weight, n_teams, n_q, n_obs, logloss_insample, logloss_crossfit, spearman_forecast, spearman_old_system, truedl)`

`change` is the entity's own movement since its previous written release, excluding gauge shifts, and 0 on a first appearance or an idle week. `expected_place` is `N − Σ_B P_AB` over the field, the place the pairwise model expects, against `forecast_place`, the rank of the lineup strengths. They differ where the field is tight: a team ranked sixth on strength is beaten by more than five teams in expectation when the six are close.

Under `engine = "s2"` the columns carry the S2 quantities: `mu` and `mu_before/after` are `R`, `core_strength` is `Q̄`, `evidence` is the count of rated games, `x` is the field strength `F_T`, `s_r` and `forecast` are both the lineup strength `Q(L)`, `s_pair` is `S_T`, `i_pair` is 1, `c` is the rated flag, `delta_l` is `Q(L) − Q̄`, and `sigma` is the provisional flag of section 4.5. The columns the Kalman filter needs and S2 has no analogue for — `var_diff`, `lost_prev`, `z` — are written as 0 or NULL. Under `engine = "kalman"` they carry the meanings of section 8.3 and are what its uncertainty calibration reads.

Indexes: `team_events(team_id)`, `team_events(tournament_id)`, `team_ratings(release_id)`, `team_ratings(team_id)`, `player_ratings(player_id)`, `player_events(player_id)`.

## 7. Calibration and acceptance tests

### 7.1 Offline calibrations

`β` and `σ_model` come from `calibrate`, over in-person tournaments 2015 to 2024: `β` as the maximum-likelihood logistic slope of pairwise wins against the difference of roster estimates rescaled to a 36-question basis (812 782 pairs), `σ_model` and `roster_bias` as the standard deviation and mean of `s* − s_r` over full six-player lineups (25 971 lineups). The display constants come from `calibrate --display` against the 2019 releases of section 5.

`info_scale` is fixed by the requirement that the published player uncertainty be honest: over in-person tournaments from 2018, the standardised residuals `z_i` of section 3.4 must have unit variance. The team-side half of that test, and `ν`, belong to section 8.3.

S2's own constants are not calibrated offline. `D` is the maximum-likelihood scale of the pairwise outcomes, and the likelihood is flat from 900 to 2400 (`simple --calibrate-d`); `K` is tuned on the mean in-person forecast Spearman over 2018-09-01 to 2024-08-29, on the grid `K ∈ {0.2, 0.3, 0.4, 0.6}` against `K_q ∈ {0.01, 0.03, 0.1}`, run once per lineup rule (`out/s2_grid_kq.md`). `K_r` and `K_q` are tuned on the weighted team NDCG of section 7.3 over the same window, by `team_sweep`; the surface is a plateau and `K` is flat from 0.2 to 1.2 once the pull is in (`out/team_ndcg/report.md`).

### 7.2 Acceptance tests

Results are from `out/q_ndcg/eval_winner.md`; the numbers in brackets are the model as it stood before section 7.4 retuned it.

1. **Cross-fitted log-loss** on in-person rated tournaments, per variant. A variant (step count, affinity rank, async weight, teammate shrinkage) stays only if it lowers the number. **Result: 0.4899 over all seasons, 0.5160 in 2025/26** (0.4900 and 0.5161). Passed.
2. **Forecast rank correlation.** Spearman between forecast places and actual places per tournament, averaged over in-person tournaments of the last two seasons, against the old system's `predictedPosition`. Target: within 0.02 of the best external system. **Result: 0.8531 against 0.8177; sync 0.8186 against 0.7897** (0.8363 and 0.8194). Passed, and ahead rather than within.
3. **Individual top-200 overlap with chgk.quest of at least 70%.** **Result: 112/200 against chgk.quest and 115/200 against rating.chgk.fun** (117 and 122). **Failed**, and by five and seven players more than before: the published `θ` is now the strength off an in-person table, and the format effect it no longer carries is part of what the external systems' numbers mean. For context those two systems agree with each other on 136/200, so the 140 threshold is above what mature external systems reach between themselves; the rank correlation of `θ` over players with at least 50 games is 0.89 against chgk.quest and 0.78 against rating.chgk.fun (0.90 and 0.79).
4. **Inflation:** the display value of the 50th team at each September release from 2015 to 2026 stays within a band of ±10%. A September whose trailing year holds fewer than 200 rated teams is a hole in the data, not inflation, and is excluded from the band; only September 2021 is. **Result: largest deviation 1.7% over 11 Septembers (range 6902 to 7130 around a mean of 7020)** (6.3%, 6612 to 7416 around 6978). Passed.
5. **Transfer invariance:** swap the strongest player between two top teams in the input at a chosen date and confirm no team rating changes at that date. Forecasts must change. **Result: 0 teams moved, maximum difference exactly 0; 286 139 team events on or after the swap have a different forecast.** Passed.
6. **Winner rule:** no team that finished first, sole or shared, has `mu_after < mu_before` anywhere in `team_events`. **Result: 0, sole and shared alike.** Passed.
7. **The rating on its own.** The weighted team NDCG of section 7.3 must not be behind chgk.gg or the old system. **Result: 0.8987 against chgk.gg's 0.8893 and the old system's 0.8877 on the tune window, 0.9029 against 0.8901 and 0.8886 on the test window.** Passed. `eval --brief` prints it.
8. The user's read of the resulting tables.

**The lineup rule.** Both rules of section 4.1 were run over the same players, tuned on the same window (`out/s2_grid_kq.md`). The roster estimate wins at each rule's own optimum and on both held-out formats, and `delta` goes with the rule it belongs to:

| | tune Spearman | tune NDCG@3 | test Spearman, in person | test Spearman, sync |
|---|---:|---:|---:|---:|
| mean of `r_i` less `delta` | 0.7947 | 0.7659 | 0.8259 | 0.7796 |
| **roster estimate** | **0.7975** | **0.7716** | **0.8309** | **0.8162** |

**The forecast against the rating.** On the 284 in-person tournaments of the test window that both databases forecast (`tools/ndcg.py --common`):

| | Spearman | NDCG@3 |
|---|---:|---:|
| lineup alone (shipped forecast) | **0.8357** | 0.7765 |
| rating with the lineup offset, `F_T` | 0.8300 | 0.7711 |
| rating alone, `R` | 0.6592 | 0.7349 |
| the Kalman engine's forecast | 0.8234 | 0.7456 |
| the old system's `predictedPosition` | 0.8202 | 0.7428 |

### 7.3 The team rating on its own

Section 4.4 measures the *forecast*, which is the lineup. This measures the
*rating*: rank the teams of an in-person field by `R` as it stood that morning,
with no lineup and no player table in hand, and score the order against the
result. `tools/team_ndcg.py` and `eval --brief` compute it; `team_sweep` tunes on
it.

- In-person tournaments, results with `inRating = 1`, and **base-roster results
  only**: a lineup qualifies by МАИИ Prilozhenie A, А.4.7 — at least three
  base-roster players, more base-roster players than legionnaires, at most three
  legionnaires — with the roster read at the tournament date (А.2.2). The roster
  history comes from `data/external/chgk_gg/base_rosters.csv`, because `buff.db`'s
  `team_seasons` holds only the two current seasons. 89.7% of the tune window's
  rated in-person results qualify and 99.8% of the test window's, the latter
  because `inRating` itself encodes roster continuity from 2021 on.
- The qualifying teams are re-ranked among themselves, ties sharing the mean
  place; relevance is `1 / place`; the order under test is by `R` descending with
  ties broken by a hash of the tournament and team ids. Fewer than five
  qualifying teams and the tournament is skipped: 597 tune tournaments and 268
  test ones.
- The headline is the **mean over tournaments weighted** by
  `data/tournament_weights.csv` — the mean strength of the ten strongest lineups
  in the field, less the 10th percentile, clipped at zero, frozen at commit
  `fffb987` and never recomputed — so the fields with the best teams count for
  more than a school event. The unweighted mean is reported beside it.
- Standard errors are a paired bootstrap over tournaments, 2 000 resamples.

| weighted NDCG / NDCG@3 | tune | test |
|---|---:|---:|
| **ours** | **0.8987 / 0.8014** | **0.9029 / 0.8042** |
| chgk.gg | 0.8893 / 0.7847 | 0.8901 / 0.7713 |
| the old system | 0.8877 / 0.7845 | 0.8886 / 0.7686 |
| rating.chgk.fun | 0.9058 / 0.8081 | 0.9137 / 0.8332 |
| ours before the pull of 4.3 | 0.8792 / 0.7657 | 0.8860 / 0.7685 |

Both windows are on the tournaments all four systems cover. The chgk.fun column
is not a team state — that system composes its team number from the players who
turned up, so it is the analogue of our `Q(L)`, which scores 0.8993 and 0.9125
under the same filter.

### 7.4 Top-weighted forecast quality

The acceptance tests above weigh a school event and a championship alike, and
Spearman weighs a swap at places 27 and 30 like a swap at 1 and 2. The forecast
is judged a second way, on the metric of `tools/ndcg.py` and `eval --ndcg`: NDCG
of the `forecast` column against the actual places with relevance `1/place`,
over in-person tournaments with at least five teams, plus `NDCG@3`. Ties in the
forecast are broken by a shuffle seeded from the tournament id, so the stored
finishing order never leaks into the rank.

The mean over tournaments is **weighted** by `data/tournament_weights.csv`: the
mean lineup strength of the ten strongest lineups in the field, less the 10th
percentile of that over in-person tournaments since 2018-09-01, clipped at zero.
The weights are frozen at the model of commit `fffb987` and never move with the
model under test, so every candidate and every external system is scored on the
same footing. Two windows: **tune** 2018-09-01 to 2024-08-29, where every
decision is made, and **test** 2024-08-30 to 2026-08-27, read once per
candidate. A paired bootstrap over tournaments, 2000 resamples, gives the
standard error of a difference (`eval --paired other.db`, `ndcg.py --paired`).

On the 282 test tournaments all four databases forecast (`--common`), weighted:

| | NDCG | NDCG@3 |
|---|---:|---:|
| **ours** | **0.9069** | **0.8048** |
| rating.chgk.fun | 0.9040 | 0.7997 |
| chgk.quest | 0.9011 | 0.7914 |
| ours before this tuning | 0.8981 | 0.7803 |
| chgk.gg | 0.8710 | 0.7261 |
| the old system's `predictedPosition` | 0.8708 | 0.7258 |

The three switches that earned their place are the format effect of section 3.7,
the pessimistic quantile `σ_c` and the tempered lineup `τ` of section 4.1; each
is worth about a third of the gain and they add. `out/q_ndcg/report.md` is the
protocol, every lever tried and the tables.

The two metrics are different questions asked of the same database. Section 7.3
hides the lineup and asks what the team number is worth alone; this one hands
the lineup over and asks how well the field is ordered. The levers of section
4.3 — the pull and `K_q` — move the first and leave the second untouched, and the
levers of sections 3.7 and 4.1 — the format effect, `σ_c` and `τ` — move both,
because the lineup estimate they sharpen is what the pull reads. `out/final_report.md`
scores every system on both.

## 8. Implementation

Rust workspace, one library crate `chgk-rating` with modules `data`, `question_model`, `team_s2` (the shipped team engine), `team_model` (the legacy one), `gauge`, `display`, `output`, `util`, and binaries `fit` (online pass, writes `out/ratings.db`), `eval` (metrics report from the outputs), `calibrate` (`β`, `σ_model`, `roster_bias`, and the display constants). Both engines expose the same `step(tournament, players, config) -> Vec<TeamEvent>` and the same release rows, so `fit` differs only in which one it constructs. `src/bin/simple.rs` is the same S2 rules as a research tool, over player ratings of its own or from a finished `fit`; it stays the place tuning grids are run. Configuration in `config.toml`. Parallelism through rayon over questions within a tournament and over teams within a tournament's pairwise stage.

`fit --swap PLAYER_A:TEAM_A,PLAYER_B:TEAM_B@DATE` exchanges two players between two teams from that date on, for test 5. `fit --diag-player ID` prints that player's uncertainty decomposition at every release.

### 8.1 Shipped constants

```
[data]     w_in_person 1, w_strict_sync 2/3, w_sync 1/2, w_async 0, rows_every 1, passes 2
[question] b_bar 1.5, s_b 2.0, s_a 0.5, s_m 0, s_aff 0.3, aff_rank 0, eta_v 0.2,
           fit take_rate, gradient exact, format_effect in_person, s_phi 0.3
[player]   mu_new −1.0, sigma_new 1.0, warm_sigma 0.5, warm_until 2016-01-01,
           info_scale 0.46, teammate_shrink 0.005,
           tau_p 0.25, v_min 0.05, v_max 0.6, v_eta 0.15, tau_aff 0.05, tau_phi 0.05
[gauge]    n_est 8, window_days 365
[team]     engine s2, roster_bias −0.2427, sigma_model 0.3669
[team.s2]  D 1200, K 0.4, k_r 0.3, k_q 0.1, winner_floor true,
           lineup tempered, tau 0.7, sigma_c 1.5,
           delta 150 (unused under tempered), player_scale 2454.8828, player_zero 6000,
           k_r_target lineup, margin_g 0, margin_shape linear, n0 0,
           w_sync and w_strict_sync absent (section 4.8)
[truedl]   theta_ref 0.2368
[display]  a_team −4394.7095, b_team 1.0678, a_player 5461.6007, b_player 4832.6752,
           provisional_sigma 0.3
[numeric]  newton_iters 12, a ∈ [0.3, 3], m ∈ [0.5, 4], b ∈ [−10, 12],
           info_floor 1e−3, theta_max_step 1.5, u_max 15
```

The display map was refitted by `calibrate --display` after `σ_c` moved the level of
the team scale; the 2019 anchors of section 5 are unchanged.

The rest of `[team]` — `sigma_new_team 0.5, beta 0.3757, kappa_m 10, m_max 2, nu 2.6, e_cap 20, rho 0.8, miss_streak 3, gamma 2, tau_t 0.2, v_min 0.05, v_max 0.6, v_eta 0.15, sigma_floor 0.2` and the switches below — belongs to `engine = "kalman"` and is kept at the values that engine shipped with. `roster_bias` and `sigma_model` are read by both.

### 8.2 Reserved extensions

Two question-model extensions are implemented, tested and switched off. Both stay in the code behind their config switch; neither lowered the cross-fitted log-loss of test 1.

- **Free step count** (`question.s_m > 0`), the collaboration term `m_q` of section 3.2. Mean fitted `m` was 1.056; it bought in-sample fit and paid for it out of sample.
- **Affinity** (`question.aff_rank > 0`), the player-editor interaction `u_i · v_e` of section 3.1. Learned magnitudes reached about 0.01 on the `θ` scale: the Fisher information for `u_i` is smaller than that for `θ_i` by a factor `v̄²`, so a single online pass cannot identify it. A fair test needs a full offline refit.

The question model carries a third group, the simplification ladder of `out/ladder/report.md`. Of these only `question.fit` moved: `take_rate` is rung Q1, the last rung that held, and it ships.

- `question.fit` (`take_rate`, shipped; section 3.3 step 1) against `newton`. Retried under section 7.4 with a free step count: `newton` with `s_m = 0.5` is the best single point on the tune window (weighted NDCG 0.8961 against 0.8945) and loses on the test window (NDCG@3 0.7973 against 0.8044), which is the same in-sample-for-out-of-sample trade the free step count made before. It stays off.
- `question.format_effect = "signed"` (section 3.7), a format effect subtracted off an in-person table rather than merely absent: weighted tune NDCG 0.8780 against 0.8867 without any format effect at all. Far the worst thing tried.
- `team.s2.lineup = "best6"`, chgk.gg's weighted top six (section 4.1): +0.0003 ± 0.0021 of weighted tune NDCG, +0.0042 ± 0.0047 at 3. Indistinguishable from the roster estimate.
- `player.teammate_shrink = 0` is worth +0.0016 ± 0.0018 on its own and −0.0017 once the format effect is in: `φ` absorbs part of what the shrinkage was correcting. It stays at 0.005.
- `data.passes = 3` is a no-op by construction (section 3.6), and `data.w_async` is inert because every asynchronous tournament in the mirror carries `in_rating = 0` and never loads.
- `question.gradient` (`exact`, shipped) replaces the noisy-OR gradient of step 2 with `share`: `g_i = (y - P) * p_i / sum_j p_j`, credit in proportion to `p_i = 1 - exp(-lambda_i)`. Rung Q3; it fails on the individual table, 102/200 against chgk.quest.
- `player.update` (`kalman`, shipped) replaces the Kalman step with `schedule`: `theta_i += eta0/sqrt(c0 + games_i) * w_t * g_i`, no `sigma`, no volatility; a player under five games is provisional. Rung Q2; it costs 0.012 of forecast Spearman.

### 8.3 The Kalman team engine (`team.engine = "kalman"`)

The engine S2 replaced, kept whole and selectable. Its formulas are in the git history of this file; the summary is:

A team carried a rating `μ_T`, a variance `σ_T²`, a volatility `v_T`, per-player evidence `e_T,i` with a miss counter, a core strength `s̄_T` and the date of its last rated game. The roster estimate on the `θ` scale,

```
s_r(L) = log Σ_{i∈L} exp(θ_i + aff_i,t) + roster_bias
Var    ≈ Σ_i p_i² σ_i² + σ_model²,   p_i = softmax weights of (θ_i + aff_i,t)
```

set a new team's entry, the forecast blend and the margin channel; `roster_bias` and `σ_model` are the mean and standard deviation of `s* − s_r` over full six-player lineups, from `calibrate`. `s_r` is still what `team_model::roster_estimate` computes, still what `calibrate` fits, and still the quantity section 4.1 maps onto the display scale.

Per tournament: continuity `c` as the share of the team's evidence present today; a lineup offset `δ_L = (s_r − roster_bias) − s̄_T` and expected strength `x_T = μ_T + δ_L`; a logistic pairwise stage on `x` with slope `β_t = β √(36/n_q)` and a score-margin multiplier, normalised by `n_eff/(N−1)`; a Kalman step `μ_T += w_t c (S_A/β_t)/π'_T` with precision from the pairwise channel and a margin channel that added precision without moving the mean; then evidence bookkeeping, the departure rule `σ_T² /= (1−ℓ)^γ`, a variance cap at a new team's, a running update of `s̄_T`, a volatility step and diffusion `σ_T² += v_T² Δt` between games. The forecast was the precision-weighted blend of `(x_T, σ_T²)` and `(s_r, Var(s_r))`.

`info_scale`, `ν` and the volatility floor were fixed by the honest-uncertainty test of section 7.1 on `z_T = (s* − x_T)/√(σ_T² + σ_model²)` for teams with `E_T ≥ 6`; `γ` was never identified by the data. `eval` prints those sections only under this engine.

It also carries its own switches, tried on 2026-09-05 against the sluggishness of established teams. Of the group only `sigma_floor` ships at a live value; the rest stay at the value that leaves the engine unchanged. The comparison is `out/variants/comparison.md`.

- `team.margin_prec_scale` (1) multiplies the margin precision; 0 removes the confirmation channel, and leaves 46% of teams provisional.
- `team.sigma_floor` (0.2) is a minimum K-factor: without it an established team's precision climbs past 100 and a year of poor results moves it by a few display points.
- `team.forget` (1) divides `σ_T²` by `forget^(w_t·c)` before each rated update, a memory that fades in games rather than in calendar time.
- `team.surprise_z0` (0, off) with `surprise_max` (9) and `surprise_down_only` (false) multiplies the prior variance by `min((z_raw/z0)², surprise_max)` when `|z_raw| > z0`: an immediate response to a surprise beside the slow ramp.
- `team.k_down` (1) multiplies a negative mean step.
- `team.margin_mean` (false), with `margin_slope`, `margin_intercept` and `margin_sigma`, lets the margin channel move the mean. A team that placed first is then clamped at a non-negative step, because the channel is not placement and principle 2 has to be imposed rather than inherited.

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

What a team is called is a question neither database answers. A result carries
the name the team entered that one tournament under, which is often a one-off,
and `buff.db`'s `teams` table is no better: the mirror rebuilds it from each
team's latest result, so a team that alternates between two names alternates
there too. The authority is the rating.chgk.info team listing, swept into
`out/team_names.db` by `tools/fetch_team_names.py` and preferred by the index,
with `teams` and then the last result behind it. The played-as name is kept per
result, shown next to the team on that tournament's page when it differs, and
indexed as an alias so a search for it still finds the team.

Rating history is interactive: the chart carries its points in viewBox
coordinates, and the crosshair is a nearest-x lookup that draws a vertical rule
and a dated tooltip. Without JavaScript the line and its band still render. A
team's series has no band under S2, which publishes no uncertainty; the team page
shows its rated games and its usual lineup where the player page shows a `±`.

Styling follows the user's design system in `~/dopesuite/dopeuikit`: reuse its
tokens, type scale and table styles from `assets/core.css` and its fonts from
`assets/fonts`, rather than inventing new ones. Run the `design-review` skill
from `~/dopesuite/.claude/skills/design-review` on the built pages before
calling the site done.
