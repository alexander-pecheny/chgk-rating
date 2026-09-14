# chgk-rating

A team-first rating for sports ChGK. `DESIGN.md` is the specification; this file is how to run it.

## The pipeline

One chronological pass over the tournaments, in `fit`. Per tournament, in order:

1. **Questions.** Each question's difficulty is set from the take rate of the lineups that played it, given their strengths as they stood this morning.
2. **Teams.** Model S2 — Elo in a field, on the published scale. A team stands at its rating shifted by how much stronger today's lineup is than its usual one, gains or loses `K · w · D ·` (actual minus expected wins over the field), then takes `K_r` of the way towards what the lineup that turned up is worth, and never ends a tournament it won below where it started. Lineup strength is the tempered roster estimate, with every player read at `θ − σ_c · σ`. The forecast published beside the result is the strength of the lineup alone.
3. **Players.** A Kalman step on `θ` from the question model's gradient and Fisher information, then a small pull towards the lineup mean. A second Kalman step tracks `φ`, how much stronger the player is at an in-person table than elsewhere.

Every Thursday is a release: the player gauge is re-centred on the established players and the two tables are written out. The team engine is `[team] engine` — `s2` ships, `kalman` is the filter it replaced and still runs from the same config. `DESIGN.md` sections 3, 4 and 8.3.

## Data

`buff.db` in the repo root is a mirror of rating.chgk.info (tournaments, results with masks and lineups, players, the old system's releases). It is read-only and never written to. Calibration inputs from other systems live in `data/external/`: the old system's 2019 team and player releases, and player tables exported from chgk.quest and rating.chgk.fun.

## Build

```
cargo build --release
```

## fit

Runs the chronological pass and writes `out/ratings.db` (about 1.4 GB, under a minute).

```
./target/release/fit
./target/release/fit --config config.toml --from 2013-01-01 --to 2026-09-05 --out out/ratings.db
./target/release/fit --passes 1                       # skip the warm-start backfill sweep
./target/release/fit --rows-every 0                   # rows at the last release only
./target/release/fit --diag-player 24850              # per-release uncertainty decomposition
./target/release/fit --swap 28751:49804,16329:168@2018-12-22 --out out/swap.db
```

`data.to` is optional: absent, or the literal `"today"`, means the current UTC
date, which is what the daily run uses. `--to` still overrides it, and `fit`
prints the resolved window on its first line.

`data.rows_every` says how often a row per player and per team is written: 1,
every release, is the production setting and the reason `out/ratings.db` is 1.4
GB; 0 writes rows at the last release only and leaves the file near 220 MB. The
research configs use 0, because their scores come from `team_events`,
`player_events` and the last release. The `releases` table stays weekly either
way. `--rows-every N` overrides the config.

`--diag-player` and the uncertainty machinery it prints belong to `engine = "kalman"`.

Write large outputs to `out/`, not to `/tmp` — `/tmp` is a 3.8 GB tmpfs and a full run does not fit.

## eval

Prints the Markdown report: log-loss, the six acceptance tests, uncertainty calibration, the top tables and the external comparisons.

```
./target/release/eval > report.md
./target/release/eval --db out/ratings.db --swap-db out/swap.db --swap-date 2018-12-22
./target/release/eval --brief                         # one line, for parameter sweeps
./target/release/eval --ndcg                          # the forecast metric alone
./target/release/eval --ndcg --paired out/other.db    # difference and bootstrap SE
```

`--ndcg` is the top-weighted forecast metric of DESIGN 7.4 on both windows:
NDCG and NDCG@3 of the forecast, in person, averaged with the frozen weights of
`data/tournament_weights.csv` and unweighted beside it. `--unweighted` drops the
weights, `--weights PATH` points elsewhere. `tools/ndcg.py` is the same metric in
Python, to the last digit, and prints the column-by-column table against other
databases:

```
uv run tools/ndcg.py out/ratings.db out/ext/fun.db --common [--k 3] [--unweighted]
uv run tools/ndcg.py out/ratings.db out/ext/fun.db --common --paired
uv run tools/q_ndcg_sweep.py out/q_ndcg/batch/glue.txt --jobs 2   # a config sweep
uv run tools/q_ndcg_sweep.py --dir out/q2 out/q_ndcg/batch2/glue.txt   # elsewhere
uv run tools/compare_ndcg.py --list          # every system in one table
uv run tools/compare_ndcg.py "ours=out/final.db" "ours, rating=out/final.db:mu"
```

Three tools read the residual rather than the score, and all three want a
database fitted by this code, because they read the `player_events` columns that
record what the forecast saw:

```
uv run tools/q_diag.py out/ratings.db --teams 6 --cohorts   # the worst fields
uv run tools/q_feat.py out/ratings.db [--top 3]             # features vs residual
uv run tools/q_vs.py out/ratings.db out/ext/fun.db          # where they beat us
```

`q_diag` ranks fields by `weight x (1 - NDCG)`, then prints the teams inside the
worst ones with every player's theta, phi, sigma and in-person record, the loss
split by cohort of lineup, and the loss against how far apart the top of the
field is. `q_feat` regresses the residual on what the lineup looked like, after
removing the part that is a function of where the forecast put the team.
`q_vs` is the difference against another system, field by field and by cohort.
`out/q_ndcg/round2.md` is what they found.

`tools/compare_ndcg.py` scores each database twice over the same tournaments: a
*player based* row ranks the field by the forecast, a *team based* row (`:mu`) by
`mu_before` with an unrated team last. `--list` is the table of
`out/final_report.md`; `--paired LABEL` adds the paired bootstrap SE of that row
against the others.

## simple

Model S2, a prototype: the whole rating on the display scale with five
constants, written to the `ratings.db` schema so `eval` and `tools/ndcg.py` read
it unchanged. `out/simple/rules.md` is the regulation, `out/simple/comparison.md`
the numbers against the shipped model.

```
./target/release/simple --config config_simple.toml            # Model S2 -> out/s2.db
./target/release/simple --config config_simple_level.toml      # the level player rule
./target/release/simple --config config_simple_s.toml          # Model S, the order statistic
./target/release/simple --config config_simple.toml --out out/s2_hybrid.db \
    --players-from out/ratings.db          # player strengths from the question model
./target/release/simple --config config_simple.toml --anchor mean_sd \
    --out out/s2_anchor_sd.db                                  # rule 11, no inflation
./target/release/simple --config config_simple.toml --lineup roster  # the shipped lineup rule
./target/release/simple --config config_simple.toml --grid     # the tuning sweep
./target/release/simple --config config_simple.toml --players-from out/ratings.db \
    --grid-kq                                     # K x k_q, per lineup rule -> out/s2_grid_kq.md
./target/release/simple --config config_simple.toml --calibrate-d   # D by log-loss
./target/release/simple --config config_simple.toml --dry      # scores only, nothing written
```

`simple` takes `--rows-every` too, and the `config_simple*.toml` files set
`rows_every = 0`, so a run writes rows at the last release only.

`[simple].lineup` and `[team.s2].lineup` pick the lineup rule of DESIGN 4.1 —
`tempered`, the shipped one, the roster estimate at temperature `tau`; `roster`,
the same at `tau = 1`; `mean`, the average less `delta` per empty seat; or
`best6`, chgk.gg's weighted top six — and `--lineup` overrides it. `[simple].perf` picks the rule that turns a result into points — `pairwise`, the
field Elo sum, or `played_like`, the order statistic at the finishing place —
`[simple].player_rule` picks `surprise` or `level`, and `[simple].anchor` picks
`none`, `mean` or `mean_sd`, the per-release rescaling that keeps the scale
still. `--perf`, `--player-rule` and `--anchor` override them.

## team_ndcg and team_sweep

Whether the published team number, on its own, orders an in-person field of
base-roster teams — DESIGN 7.3. `tools/team_ndcg.py` scores a finished database
and writes `out/team_ndcg/qualify.csv`, the base-roster rule of МАИИ
Prilozhenie A; `eval --brief` prints the same number; `team_sweep` runs the
question model once and replays the team engine over its output, so a grid point
costs a second instead of a fit.

```
uv run tools/team_ndcg.py out/ratings.db --window tune
uv run tools/team_ndcg.py out/ratings.db out/ext/gg.db out/ext/fun.db --old --common
uv run tools/team_top20.py out/a.db out/b.db          # the top teams side by side
./target/release/team_sweep --boot 2000               # the whole lever scan, 63 s
./target/release/team_sweep --axis "k_r=0.1,0.2,0.3,0.5" --axis "k_q=0.03,0.1,0.3"
./target/release/team_sweep --base "k_r=0.3,k_q=0.1" --point "K=0.8"
```

`out/team_ndcg/report.md` is the study: the baseline against chgk.gg,
rating.chgk.fun and the old system, a row per lever, and the regulation as
numbered sentences.

## calibrate

Recomputes the constants that `config.toml` holds. Both modes print a block to paste into the config.

```
./target/release/calibrate                            # beta, sigma_model, roster_bias
./target/release/calibrate --display --db out/ratings.db   # the affine display maps
```

`--display` reads an existing `out/ratings.db`, so the order is: fit, calibrate, paste, fit again.

## Site

`web/` is a separate crate: an HTTP server that reads `out/ratings.db` and
`buff.db` directly, so every player, team and tournament has a page, not only
the ones in the current release.

```
cd web && cargo build --release
./web/target/release/serve out/ratings.db --addr 127.0.0.1:8080
```

On start it builds `out/web.db`, the serving index — rosters, names, the two
ranking tables and a full-text index — and rebuilds it whenever `out/ratings.db`
is newer. `--rebuild` forces it; the build takes about ten seconds.

A team is named by `out/team_names.db`, which `tools/fetch_team_names.py` sweeps
out of the rating.chgk.info API; `buff.db`'s `teams` table and then the name on
the team's last result are the fallbacks. Neither fallback is authoritative —
the mirror builds `teams` from each team's latest result too, so a team that
alternates between two names alternates in both. The name a team entered a
tournament under is kept per result and shown on that tournament's page, and
every name it has used is searchable.

Live at https://rating2.pecheny.me: `chgk-rating.service` runs the binary on
port 9684 and Caddy proxies to it. After a refit, `sudo systemctl restart
chgk-rating` rebuilds the index and picks up the new numbers. Cron does all of
that at 06:30 UTC every day; `deploy/README.md` says how.

## anomaly

Asks whether the rating catches cheats. Cheating is close to impossible at an
in-person tournament, awkward at a synchronous one and trivial when the
synchronous circuit is played at home, as it was in 2020 and 2021, so a cheat's
strength depends on the format and an honest player's does not.

The site's `/anomaly/` page is a separate, leaner implementation of the same
idea in `web/src/anomaly.rs`, computed at index time: one score per player,
sorted descending.

```
./target/release/anomaly --top 200      # forecast and residual, split by format
./target/release/anomaly --masks        # the hard-question test, no ratings used
./target/release/anomaly --profile 308949   # one player, tournament by tournament
./target/release/anomaly --who 308949       # one player's mask numbers
```

## ext_fun

rating.chgk.fun (Sirotkin's alternative rating) reimplemented from its published
rules, over its own scope — no `in_rating` filter, asynchronous tournaments
included — and written to the `ratings.db` schema. `--as-of` is the day the
upstream site was last run and is what fixes the number of releases.

```
./target/release/ext_fun                                   # -> out/ext/fun.db, 1.1 GB, 40 s
./target/release/ext_fun --db buff.db --out out/ext/fun.db --as-of 2026-09-04
uv run tools/ext_fun_report.py                             # -> out/ext/fun/report.md
```

`out/ext/fun/report.md` is the method in prose and the fidelity check against the
site's own API: nine of the ten tournaments of releases 1 and 2 reproduce every
fitted question difficulty bit-exactly, and the current player table correlates
0.9990 with `data/external/chgk_fun/players_rating_release263.csv`.

## Report

```
uv run tools/fetch_fonts.py                           # once, into fonts/
typst compile --font-path fonts report/report.typ
```
