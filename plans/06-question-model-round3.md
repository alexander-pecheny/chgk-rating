# Plan 6: the question model, third round — new kinds of signal

## Where round 2 left it

`out/q_ndcg/round2.md`: weighted test NDCG 0.9079 / @3 0.8076, ahead of
rating.chgk.fun (0.9040) by 0.7–2.2 SE. Its diagnosis: 90% of the weighted
loss is on fully measured lineups in strong fields (World Championship
groups, Minsk A-league), where an established team forecast 5th–10th wins;
no per-player feature (σ, in-person games, φ, recency) correlates with the
residual above 0.10; a field's NDCG is driven by how far apart its top four
forecasts are. Levers of the round-1 kind are exhausted. This round tries
signals that are not properties of a single player.

Same protocol: tune 2018-09-01 to 2024-08-29 decides, test 2024-08-30 to
2026-08-27 reported once per row, weights from `data/tournament_weights.csv`,
paired bootstrap SE against the round-2 winner, fun and quest; every change a
switch with the shipped default; `to = "2026-09-05"`, `rows_every = 0`.

**Use the frozen mirror** `/mnt/HC_Volume_106816389/build/herdr-worktrees/r2out/buff_frozen.db`
(symlink it as `buff.db`), not the live one, which the daily update replaces;
refit the round-2 baseline from `config.toml` at `ded58dd` on it first and
check it reproduces round 2's numbers to the fourth decimal.

## Levers, in the order to try

1. **Chemistry: games played together.** A lineup whose core has played many
   tournaments together should outperform the sum of its parts. Keep, per
   team, a count of rated games for each *set of players*, or more simply
   per pair of players; define the lineup's togetherness `T(L)` as the mean
   over pairs in `L` of `log(1 + games together)` (or the count of past
   games in which at least four of today's lineup were on the table). Add
   `γ · T(L)` to the forecast only (`team.s2.chemistry`, `γ` on the display
   scale), then also try it inside the question model as a lineup-level
   bonus to `u` (`u · exp(γ' T)`), where it also changes the player updates.
   Check first, on the round-2 residuals, whether the winning 5th–10th
   forecasts are stable lineups and the losing favourites are not.

2. **Collaboration as a global constant, and the difficulty-aware forecast.**
   The Newton fit's per-question `m_q` helped the forecast and hurt log-loss.
   Try a single global `m` (`question.m_fixed`, default 1) with the take-rate
   closed form adjusted for it (`P = (1 − e^{−u})^m`, so `b` is solved from
   `r^{1/m}`), `m` in {1.1, 1.25, 1.5, 2}. With `m ≠ 1` the ranking of a
   star-plus-passengers lineup against a balanced one depends on the
   package's difficulty, so the forecast should become the **expected number
   of answers on a package of the difficulty this field usually gets**:
   `Σ_q P(L, b_q)` over a difficulty sample drawn from the fitted `b`s of
   in-person tournaments with a similar top-10 strength (bucket by the
   field's weight from the CSV, or use buff's `tournaments.difficulty_forecast`
   if it is populated). Rank by that.

3. **Train on unrated tournaments, score on rated ones.** chgk.quest trains
   on `in_rating = 0` events and is level with us exactly where players have
   the most data. Add `data.train_unrated = true`: load them, let players
   update from them (with their own weight `w_unrated`, try 0.5 and 1.0),
   but never let them open or move a team state and never let them enter the
   metric's tournament set or `tournament_metrics` scoring. Check the
   tournament counts of `eval --ndcg` are unchanged before trusting a number.

4. **Lineup-level quantile.** Round 1 subtracts `σ_c·σ` per player. Instead
   subtract `c · sd(Q(L))`, the standard deviation of the roster estimate
   propagated from the players' σ (DESIGN 4.1 mentions it; `calibrate` has
   the code), which treats a lineup of six moderately uncertain players
   differently from one with a single unknown. `c` in {0.5, 1, 1.5}, with
   `sigma_c` reduced to 0 and to 0.75.

5. **Discrimination by format.** `a_q` is pinned at 1. Try a global
   `question.a_in_person` in {0.8, 1.2, 1.5} (λ = exp(a(θ − b)), take-rate
   `b` solved with that `a`), which changes how far strong players separate
   at in-person tables without touching the Newton machinery.

6. **Series memory.** Teams that peak at a championship do so every year.
   Per team, the mean in-person surprise over its last k in-person results
   (k = 5) as a forecast bonus `κ · S̄`: not the team rating, only its recent
   over- or under-performance against its own lineup forecast. Round 2 ruled
   out blending with `R`; this is a different quantity. Try κ small.

Run 1, 2 and 3 first; they are the ones with a mechanism behind the loss
the diagnosis describes. Budget ~120 fits. Stop when the paired lead over fun
on the tune window reaches three SE and holds on test, or when the list is
exhausted.

## Deliverables

- `out/q_ndcg/round3.md` (`git add -f`): for each lever the mechanism
  checked on the round-2 residuals before the sweep, the lever table (tune,
  test, weighted and unweighted, paired SE vs round 2, fun, quest), the
  regenerated eight-row `tools/compare_ndcg.py` table, the acceptance row
  (tests 1–7) beside round 2, top 20 teams and players beside round 2.
- Code and docs as before: switches with shipped defaults, `config.toml`
  flipped to the winner only if it beats round 2 on tune beyond one SE and
  does not lose on test; DESIGN/README/method page/report updated where a
  rule changed. Commits on your branch ending with the attribution lines in
  your instructions. Do not merge.
