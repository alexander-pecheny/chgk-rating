# Plan 8: feature discovery with a boosted ranker, then distil

## Why

Rounds 1–3 (`out/q_ndcg/report.md`, `round2.md`, `round3.md`) tested
seventeen hypotheses one at a time, 400 fits, and kept four. The user's
proposal: go wide first. Emit every feature the pipeline can know on the
morning of a tournament, train a gradient-boosted ranker on the tune window,
score it on the test window with the same weighted NDCG, read the feature
importances, and only then decide what to port into the simple model. The
ranker is a research instrument, not a shipping model: nothing about the
published rating changes in this plan.

## Step 1: the feature table, from `fit`

Add `fit --features out/features.parquet` (or a SQLite table
`features(tournament_id, team_id, ...)` in the output database; SQLite is
fine and needs no new crate) written for every result of every in-person
tournament, from the state **before** the tournament's updates — leakage is
the one way this plan fails, so every feature must be computable at 09:00
that morning. Include the identifiers and the target (`actual_place`,
`n_teams`, the `rated` flag, the field weight from
`data/tournament_weights.csv`).

Feature families (aim for 80–150 columns; cheap ones are fine):

- **Lineup strength**: `Q(L)` under each rule (roster, tempered at τ 0.5/0.7/1.0,
  mean, best-6 weighted), with and without `σ_c` at 0.75/1.5/3; `roster_sd`.
- **Per-player aggregates over the lineup** (mean, min, max, sd, top-3 sum,
  bottom-2 mean): `θ`, `φ`, `θ+φ`, `σ`, rated games, in-person games,
  in-person games in the last 365 days, days since last game, days since
  last in-person game, volatility `v`, age of the player's first record,
  share of the player's games that were in-person, last-5-games mean
  surprise (player-level gradient sign), the player's `θ` change over the
  last 90 and 365 days.
- **Lineup shape**: size, number of newcomers (σ > provisional), number of
  players with < 5 in-person games, share of the team's usual lineup present
  (`Q(L) − Q̄`, and the count of players who played the team's last rated
  game), chemistry `T(L)` (pairs' games together, mean and min), how many of
  the six played together at the team's last in-person event.
- **Team state**: `R`, `Q̄`, `R − Q̄`, rated games `n`, days since the last
  rated game, last-5 surprises mean and sd (series memory), the team's
  mean surprise at in-person events over 365 days, best and worst place in
  the last year, the team's rating rank in the last release.
- **Editor**: editor memory value (player×editor, team×editor, gated and
  ungated), count of past tournaments under these editors, mean past
  residual under them.
- **Field context**: field size, tournament weight from the CSV, the
  lineup's rank in the field by `Q(L)`, gap to first and to fourth in
  display points, z-score of `Q(L)` within the field, number of teams
  within 100 points, the field's mean and top-10 `Q(L)`.
- **Calendar and place**: month of season, day of week, number of in-person
  tournaments on the same date (multi-stage events), home/away flags
  (team town = tournament town; share of lineup from the tournament town).
- **External systems as features, clearly labelled**: the old system's
  `predictedPosition`, chgk.gg `mu_before` and its published forecast,
  fun's forecast, quest's forecast (joined from `out/ext/*.db`). These are
  for *diagnosis only* — if the ranker leans on them, that says what
  information we lack, and they must never be in the distilled model.

## Step 2: the ranker

`tools/rank_boost.py`, a uv script with `lightgbm` (lambdarank, groups =
tournaments, relevance = graded from place: e.g. 31 − place clipped at 0,
or use `1/place` scaled to integers) and, as a cross-check, `catboost`
(YetiRank). Train on tune 2018-09-01 to 2024-08-29, early-stop on a
held-out fifth of tune tournaments, score the test window 2024-08-30 to
2026-08-27 with **exactly** `tools/ndcg.py`'s weighted NDCG and NDCG@3 on
the same 282 common tournaments, paired bootstrap against round 3 and fun.
Sample weights = the tournament weights from the CSV. Report:

1. Ranker with all features, with and without the external-system
   features: the ceiling.
2. Feature importance three ways: gain, permutation (NDCG drop when the
   column is shuffled within tournament), and SHAP if cheap.
3. Ablations by family: drop each family, rerun; and forward selection:
   the best 1, 2, 3, 5, 8, 12 features by permutation importance, retrained.
4. A linear ranker (logistic pairwise or a linear LambdaRank) on the top 8
   features, standardised, with coefficients: the first distillation.

## Step 3: distil

For every feature in the top 8 that is not already in the shipped forecast,
say how it would enter the simple model (one sentence, one constant), and
run that as a switch through the round-3 harness (`tools/q_ndcg_sweep.py`)
on the tune window. Ship nothing in this plan; hand the list to the next
round with the ranker's number as the ceiling and the linear model's as the
target.

## Deliverables

`out/discovery/report.md` (`git add -f`): the feature list with one line
each, the ceilings table, the importance table (top 25), the ablations, the
linear model, the distillation list. Code: the `--features` writer in Rust,
`tools/rank_boost.py`, `tools/features_check.py` (a leakage check: every
feature's correlation with `actual_place` computed on tournaments *after*
the feature date must be reproducible from data dated before it — at
minimum, assert no feature uses the tournament's own masks or places).
Commits on your branch ending with the attribution lines in your
instructions; do not merge.

## Environment

Own worktree from the current branch; `ln -s
/mnt/HC_Volume_106816389/build/herdr-worktrees/r2out/buff_frozen.db buff.db`,
`ln -s /home/pecheny/new_chgk_rating/data/external data/external`,
`mkdir -p out`, `ln -s /home/pecheny/new_chgk_rating/out/ext out/ext`.
Never write large files to /tmp or the scratchpad; `/home/pecheny/new_chgk_rating`
is read-only. Python is allowed here because the ranker is a research
instrument; the feature writer stays in Rust.
