# Plan 4: the team rating, judged by ranking from the team number alone

## Goal

The published team rating `R` (DESIGN 4, Model S2) should, on its own, order
an in-person field as well as possible — no lineup, no player numbers at
forecast time — while staying a model a reader can run in a spreadsheet. The
forecast the site publishes stays the lineup strength (DESIGN 4.4); this plan
is about what `R` means and how it moves.

## The metric

A new tool, `tools/team_ndcg.py` (and the same in `eval --brief`, in Rust):

- In-person tournaments (`tournament_metrics.type = 'in-person'`), results
  with `inRating = 1` (`team_events.rated = 1`).
- **Base-roster results only.** `buff.db` has `team_seasons(team_id,
  season_id, player_id, date_added, date_removed, player_number)` (30 091
  rows, 6 454 teams) and `seasons(id, date_start, date_end)`. A result
  qualifies when the lineup (`tournament_results.team_members`) contains at
  least `max(4, size − 2)` players who were on the team's base roster for
  that season on the tournament date (added on or before, not removed before):
  4 of 6, 5 of 7, 6 of 8, 4 of 5, 4 of 4. Teams with no `team_seasons` rows
  never qualify. Write the rule down in the report; if the official МАИИ
  definition of a base lineup is at hand in `data/external/chgk_gg/
  prilozhenie_A.txt`, use that instead and say so.
- Rank the qualifying teams of a tournament by `mu_before` (the rating as it
  stood that morning), ties shuffled by a seed from the tournament id, then
  NDCG with relevance `1/place` against `actual_place` **among the qualifying
  teams re-ranked** (a team's place is its rank among the qualifying subset,
  ties shared). Tournaments with fewer than five qualifying teams are skipped.
  Report NDCG and NDCG@3, mean over tournaments.
- Compare on the same tournaments (`--common`): ours; chgk.gg (`out/ext/gg.db`,
  `mu_before` is the official team rating); rating.chgk.fun
  (`out/ext/fun.db`); the old system's rating from `buff.db`'s `ratings`
  (release_id, team_id, rating) joined to the last `releases.date` before the
  tournament — that table starts in September 2021, so the old column covers
  the test window only. chgk.quest has no team rating; leave it out.
- Windows as in plan 3: **tune** 2018-09-01 to 2024-08-29 for decisions,
  **test** 2024-08-30 to 2026-08-27 reported once per candidate. Add a paired
  bootstrap SE over tournaments (2000 resamples) for differences.

- **Weighting (the user's addition on 2026-09-14).** The high-profile
  tournaments with the best teams matter far more than a school event where
  nearly every player is unrated. The headline number is therefore the
  **weighted** mean over tournaments, with weights frozen in
  `data/tournament_weights.csv` (committed; `tournament_id, date,
  top10_mean_strength, weight`): the mean lineup strength of the ten strongest
  lineups in the field under the shipped model at commit fffb987, minus the
  10th percentile over in-person tournaments since 2018-09-01, clipped at
  zero. The weights never change with the model under test. Report the
  unweighted mean beside it; decide on the weighted one. `tools/team_ndcg.py`
  reads the CSV by default (`--unweighted` to switch off), and so does the
  `eval --brief` version.

Run the baseline first and put the table at the top of the report.

## Ground rules

- The engine is `src/team_s2.rs`, run through `fit` (a minute per run;
  `data.rows_every = 0` from plan 2 keeps the databases small — if the flag is
  not there yet, use `rows_every = 50`). `src/bin/simple.rs` is the research
  twin of the same rules; use it if it is faster for sweeps, but the shipped
  change lands in `team_s2.rs` and the two must agree.
- Player strengths are the shipped question model's; plan 3 is changing them
  in parallel. Design every change as a config switch with the shipped
  default, and expect the constants to be re-tuned once plan 3's players land
  (the orchestrator will run that sweep; keep your sweep scripts in
  `tools/` or `out/team_ndcg/` so it is a one-liner).
- Conceptual simplicity is a hard constraint: after the change, the
  regulation of DESIGN 4.3 must still fit in a dozen numbered sentences, and
  the spreadsheet description ("one row per opponent, expected win from the
  rating gap, actual minus expected, average, times K·D") must still be
  roughly true. No new per-team state beyond one number if you can help it.
- Keep every existing acceptance test in view for the final candidate: the
  lineup forecast NDCG (must not fall), in-person Spearman, winner rule (a
  first-placed team never loses points — sole or shared), inflation of the
  50th team, innovation autocorrelation, the top 20 by eye.

## Candidates, in order

1. **Margin.** Today the pairwise outcome is win/lose/skip. Let the score gap
   in: `y_AB = clamp(½ + (s_A − s_B) / (2g), 0, 1)` with `g` in questions
   (try 2, 4, 8, 16; `∞` is the shipped rule), or the logistic
   `1/(1+10^(−(s_A−s_B)/g))`. A team that leads by ten questions then earns
   more than one that leads by one. Check that the winner rule still holds by
   construction (a sole first still has `y ≥ P` against everyone; a shared
   first with equal scores has `y = ½`, so the skip rule for ties must stay).
2. **Pull towards the lineup.** `R += K_r · w · (Q̄ − R)` per rated game, or the
   equivalent `R += K_r · w · (Q(L) − R)`: the rating drifts toward what the
   players are worth, which is the question-model signal the user asked to
   admit. Try `K_r` in {0, 0.02, 0.05, 0.1, 0.2}.
3. **Faster start**: `K_n = K · max(1, n₀ / n)` with `n₀` in {0, 3, 6, 10}
   rated games — provisional teams move faster, established ones as now.
4. **Team-side format weights** separate from the players': a sync result
   moves `R` by `w_team_sync` instead of `w_sync`. Sweep {0.25, 0.5, 0.75}.
5. **`K` and `K_q`** re-tuned on the new metric (today 0.4 and 0.01).
6. **Inactivity** (DESIGN 4.6 says none): a rating that has not been tested
   for a year counts as `R − d` per idle year in the ranking, `d` in {0, 100,
   200}. Only if 1–5 leave something on the table.

Score each lever alone on the tune window, keep the ones that beat the
bootstrap noise, combine, re-check interactions of the top two, and report
the test window once for each row.

## Deliverables

- `out/team_ndcg/report.md`, in English: the qualifying rule, the baseline
  table (ours, chgk.gg, chgk.fun, old system, both windows, NDCG and NDCG@3,
  tournament counts), one row per lever and one for the combination, the
  acceptance-test row of the winner next to the shipped model, the top 20
  teams side by side, and the regulation rewritten as numbered sentences the
  way `out/simple/rules.md` does it.
- Code: `tools/team_ndcg.py`, the `eval --brief` metric, the switches in
  `team_s2.rs` and `simple.rs` with the shipped defaults, flipped in
  `config.toml` and `config_simple.toml` to the winner; DESIGN.md section 4
  rewritten for the winner; README; `web/src/method.rs` and
  `report/report.typ` in a paragraph each.
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
  scored. Keep the winner's database.
- `cargo build --release`; the target directory is local to your worktree.
- Prefer Write/Edit or `uv run python -c` for file creation over bash heredocs.
