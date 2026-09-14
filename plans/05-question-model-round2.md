# Plan 5: the question model, second round

## Where round 1 left it

Plan 3's winner (per-player in-person effect `φ`, pessimistic quantile
`sigma_c = 1.5`, tempered roster estimate `tau = 0.7`) scores, weighted, on
the test window: NDCG 0.9069 and NDCG@3 0.8048, against rating.chgk.fun's
0.9040 / 0.7997 and chgk.quest's 0.9011 / 0.7914. The lead over fun is
+0.0023 ± 0.0035 and +0.0043 ± 0.0076: ahead, not measurably. The user asked
for a clear margin. Read `out/q_ndcg/report.md` first; the levers it tried and
their sizes are the starting point, and its `tools/q_ndcg_sweep.py` and
`eval --ndcg --paired` are the harness. Same protocol as plan 3: tune window
decides, test reported once per row, weights from
`data/tournament_weights.csv`, every change a switch with the shipped default.

## Where the remaining error is

Before any lever, spend a few runs on diagnosis and put it in the report:

1. Per-tournament residual: which of the top-weighted tournaments lose the
   most NDCG, and what do their misses have in common (a debutant lineup, a
   team of sync-only players, a strong team on a bad day, a lineup with a
   missing member)? Twenty worst tournaments by `weight × (1 − NDCG)`.
2. Per-team residual on those: forecast rank against actual for the teams
   that cost the most, with each player's `θ`, `φ`, `σ`, in-person game count
   and days since the last in-person game.

Let that decide the order below; do not sweep blindly.

## Levers

1. **Uncertainty that knows the format.** The pessimistic quantile used a
   single `σ`. Make the penalty depend on how much of a player's evidence is
   in-person: `θ − c · σ · (1 + a / (1 + n_inperson))`, or a separate `σ_φ`
   on the format effect entering the quantile. Sync-only players are the
   likeliest source of over-forecasts at in-person events.
2. **Rank-2 affinity: format and editors together.** Round 1 could not
   combine `aff_rank = 1` (+0.0019) with the format effect because the
   implementation has one slot. Add a second slot with one column pinned to
   the format indicator.
3. **Newton fit with `s_m = 0.5`** was the best tune point (0.8961) and gave
   the NDCG@3 back on test; the round-1 agent did not ship it. Re-run it on
   top of the round-1 winner with a finer `s_m` grid {0.25, 0.5, 1.0} and
   `s_a` {0.25, 0.5}; ship it only if tune and test agree in sign.
4. **A sync effect of its own.** The signed variant (`θ ± φ`) lost; try a
   second per-player scalar for synchronous play, with the in-person one
   untouched, so the sync data stops polluting the in-person strength without
   forcing symmetry.
5. **Weight of sync data in the `φ` update** separately from `w_sync` for `θ`.
6. **Recency**: a shorter effective memory for in-person strength (a higher
   `v_min` only for `φ`, or diffusion in in-person games rather than calendar
   time).
7. **Tempered glue per lineup size**: `tau` for full tables and for short
   ones; and interaction of `tau` with `sigma_c` on a finer grid
   ({0.6, 0.7, 0.8} × {1.0, 1.5, 2.0}).
8. **Publishing**: the player table should list `θ + φ` (or a documented
   blend) so test 3 recovers from 112/115 toward the shipped 117/122; this
   does not change the forecast but is part of shipping the model. Do it
   last and report the overlap.

Budget up to ~100 fits. Stop early if the paired lead over fun on the tune
window reaches three standard errors and holds on test.

## Deliverables

- `out/q_ndcg/round2.md` (add with `git add -f`): the diagnosis, the lever
  table with tune and test, weighted and unweighted, paired SE against
  shipped, fun and quest; the final `tools/compare_ndcg.py` table with the
  eight rows of `out/final_report.md` regenerated; acceptance row; top 20
  teams and players beside the round-1 winner.
- Code, config defaults flipped to the winner, DESIGN/README/method
  page/report updated where rules changed. Commits on your branch, ending
  with the attribution lines in your instructions. Do not merge.
