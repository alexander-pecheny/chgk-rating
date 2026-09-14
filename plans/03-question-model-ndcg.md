# Plan 3: the question model, tuned for the in-person forecast (NDCG)

## Goal

Beat rating.chgk.fun and chgk.quest clearly on top-weighted forecast quality at
in-person tournaments. The forecast is the roster estimate of the declared
lineup (DESIGN 4.4), so this is entirely about the player strengths `θ` the
question model produces and about the glue that turns a lineup's `θ`s into a
number. Synchronous and asynchronous tournaments may feed the ratings with any
weights you like; only in-person tournaments are scored.

## The metric

`tools/ndcg.py`: NDCG with relevance `1/place` of the `forecast` column of
`team_events`, in-person tournaments with at least five teams, plus `--k 3`.
Two windows:

- **tune**: 2018-09-01 to 2024-08-29 — every decision is made here;
- **test**: 2024-08-30 to 2026-08-27 — reported once per candidate, never
  used to choose.

Baseline on the test window, `--common` over the 282 tournaments all four
databases forecast (`out/baseline_ndcg.txt`):

| db | NDCG | NDCG@3 |
|---|---:|---:|
| ours (`out/shipped.db`, S2 with the question model's players) | 0.8944 | 0.7772 |
| rating.chgk.fun (`out/ext/fun.db`) | 0.8982 | 0.7932 |
| chgk.quest (`out/ext/quest.db`) | 0.8973 | 0.7880 |
| chgk.gg, the official one (`out/ext/gg.db`) | 0.8749 | 0.7421 |
| old system's `predictedPosition` | 0.8748 | 0.7414 |

We trail both targets today. "Handily" means something like 0.905 / 0.800 or
better on the test window; report whatever you reach honestly.

**Weighting (the user's addition on 2026-09-14).** Prediction quality at the
high-profile tournaments with the best teams matters far more than at a
school event where nearly every player is unrated and no forecast can work.
So the headline number is the **weighted** mean over tournaments, with the
weight of each tournament frozen in `data/tournament_weights.csv` (committed;
columns `tournament_id, date, top10_mean_strength, weight`): the mean lineup
strength of the ten strongest lineups in the field under the shipped model at
commit fffb987, minus the 10th percentile of that over in-person tournaments
since 2018-09-01, clipped at zero. The weakest decile weighs nothing, the
strongest fields about twice a mid-level one. The weights never change with
the model under test, so every candidate and every external system is scored
on the same footing. Report the unweighted mean beside it, but decide on the
weighted one. Add `--weights data/tournament_weights.csv` to `tools/ndcg.py`
(on by default; `--unweighted` turns it off) and the same to `eval --brief`;
recompute the baseline table above in weighted form first and put it in the
report.

First add the same metric to `eval --brief` in Rust (both windows, NDCG and
NDCG@3, in-person, `forecast` column, same tie rule: shuffle by a seed from
the tournament id, then stable sort) so a sweep does not pay for Python; check
it matches `tools/ndcg.py` to four decimals on `out/shipped.db`. Also compute
a paired bootstrap standard error over tournaments for the difference between
two databases (2000 resamples, in Rust or in `tools/ndcg.py --paired a.db
b.db`), so you know what is noise: with 282 test tournaments the SE of a
difference is probably around 0.002–0.003.

## Ground rules

- The pipeline is `fit` with `team.engine = "s2"` and `question.fit` as
  configured; `config.toml` must reproduce `out/shipped.db`'s numbers before
  you touch anything (it does at commit fffb987; check `eval --brief`).
- Every change is a config switch whose default is the shipped behaviour; flip
  defaults in `config.toml` only at the end, for the winner.
- Set `data.rows_every = 0` in your experiment configs (plan 2 adds it:
  rows only at the last release) — if that flag is not merged yet when you
  start, run with `rows_every = 50` and accept the bigger files.
- Keep the other acceptance tests in view and report them for the final
  candidate: in-person Spearman (test 2), top-200 overlap with chgk.quest and
  rating.chgk.fun (test 3), inflation (test 4), winner rule (test 6), and the
  top 20 teams and top 20 players by eye. A gain in NDCG bought with a broken
  player table is not a gain; say so if it happens.
- One `fit` takes about a minute. Budget up to ~120 runs. Coordinate sweeps,
  not grids; one lever at a time, then combine the winners and re-check
  interactions of the top two or three.

## Levers, in the order to try them

1. **Format weights** (`data.w_sync`, `w_strict_sync`, `w_async`; today
   0.5, 2/3, 0). In-person is the target, so maybe sync results should count
   less for the players, or maybe more data wins. Sweep `w_sync` in {0.25,
   0.5, 0.75, 1.0} and `w_async` in {0, 0.1, 0.25} (check `src/data.rs`: async
   tournaments load only when their weight is positive, so this also changes
   the data).
2. **A per-player format effect.** A player's strength at an in-person table
   is `θ_i + φ_i`; at a synchronous one it is `θ_i` (or `θ_i − φ_i`, choose
   by the tune score). `φ_i` is a Kalman-tracked scalar with prior
   `N(0, s_φ²)` and its own diffusion, updated from the same gradient as `θ`
   but only on in-person results, and used in the roster estimate whenever the
   tournament is in-person. The `/anomaly/` page shows format dependence is
   real for some players; this is the likeliest single win. It is also close
   to affinity with rank 1 and a fixed "editor" vector — if the affinity code
   (`aff_rank`, `s_aff`, `eta_v`) can express it with a format indicator
   instead of editors, reuse that rather than writing a new state.
3. **Pace and uncertainty**: `player.info_scale`, `tau_p`, `v_min`, `v_max`,
   `v_eta`, `teammate_shrink` — re-tuned on NDCG rather than on log-loss.
4. **Newcomers**: `mu_new`, `sigma_new`. A lineup with an unknown player is
   the usual reason a forecast misses at the top; try a lower `mu_new`, and
   try a forecast that uses `θ − c·σ` (a pessimistic quantile) for the roster
   estimate with `c` in {0, 0.25, 0.5}.
5. **Question fit**: `question.fit = "newton"` with `s_m > 0` (the ladder
   report shows the Newton fit was 0.002 better on NDCG@3 than the take-rate
   closed form). Check whether that survives the tune window.
6. **Lineup glue**: a tempered roster estimate `τ · ln Σ exp(θ_i/τ)` with `τ`
   in {0.7, 1, 1.5, 2}, and a best-six-with-weights variant like chgk.gg's
   (1, 5/6, 2/3, 1/2, 1/3, 1/6 on the sorted `θ`s). This changes the forecast
   and the S2 lineup strength together (`team_s2::roster`); keep the change
   behind `team.s2.lineup`.
7. **Editor affinity** `aff_rank` 1 or 2: in-person tournaments name their
   editors.
8. **Backfill**: `passes = 3`, `warm_until` 2017.

## Deliverables

- `out/q_ndcg/report.md`, in English: the protocol, one table with baseline,
  every lever tried (tune and test NDCG, NDCG@3, the bootstrap SE), the
  combined winner, and the acceptance-test row for the winner next to the
  shipped model; the `--common` table against the three external systems and
  the old system; the top 20 teams and top 20 players of the winner beside the
  shipped ones.
- Code: the switches, with the shipped defaults flipped to the winner in
  `config.toml`; DESIGN.md sections 3 and 4 updated where the model changed;
  README where the flags changed; `web/src/method.rs` (the method page, in
  English) and `report/report.typ` updated in a paragraph each if the rules
  changed.
- Commits on your branch in plain English, ending with the attribution lines
  given in your instructions. Do not merge; the orchestrator merges.

## Environment

- You run in your own git worktree of this repository. Before anything:
  symlink the data in, without copying:
  `ln -s /home/pecheny/new_chgk_rating/buff.db buff.db`,
  `ln -s /home/pecheny/new_chgk_rating/data/external data/external`
  (`data/` is tracked and holds the weights CSV), `mkdir -p out`,
  `ln -s /home/pecheny/new_chgk_rating/out/ext out/ext`,
  `ln -s /home/pecheny/new_chgk_rating/out/ratings.db out/shipped.db`.
  Treat everything under `/home/pecheny/new_chgk_rating` as read-only.
- Never write large files to `/tmp` or the scratchpad (a 3.8 GB tmpfs);
  experiment databases go under `out/` in your worktree and are deleted when
  they have been scored. Keep the winner's database.
- Use `cargo build --release`; the target directory is local to your worktree.
- Prefer Write/Edit or `uv run python -c` for file creation over bash heredocs.
