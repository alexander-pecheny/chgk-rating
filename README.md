# chgk-rating

A team-first rating for sports ChGK. `DESIGN.md` is the specification; this file is how to run it.

## Data

`buff.db` in the repo root is a mirror of rating.chgk.info (tournaments, results with masks and lineups, players, the old system's releases). It is read-only and never written to. Calibration inputs from other systems live in `data/external/`: the old system's 2019 team and player releases, and player tables exported from chgk.quest and rating.chgk.fun.

## Build

```
cargo build --release
```

## fit

Runs the chronological pass and writes `out/ratings.db` (about 1.4 GB, a few minutes).

```
./target/release/fit
./target/release/fit --config config.toml --from 2013-01-01 --to 2026-09-05 --out out/ratings.db
./target/release/fit --passes 1                       # skip the warm-start backfill sweep
./target/release/fit --diag-player 24850              # per-release uncertainty decomposition
./target/release/fit --swap 28751:49804,16329:168@2018-12-22 --out out/swap.db
```

Write large outputs to `out/`, not to `/tmp` — `/tmp` is a 3.8 GB tmpfs and a full run does not fit.

## eval

Prints the Markdown report: log-loss, the six acceptance tests, uncertainty calibration, the top tables and the external comparisons.

```
./target/release/eval > report.md
./target/release/eval --db out/ratings.db --swap-db out/swap.db --swap-date 2018-12-22
./target/release/eval --brief                         # one line, for parameter sweeps
```

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

Live at https://rating2.pecheny.me: `chgk-rating.service` runs the binary on
port 9684 and Caddy proxies to it. After a refit, `sudo systemctl restart
chgk-rating` rebuilds the index and picks up the new numbers.

## anomaly

Asks whether the rating catches cheats. Cheating is close to impossible at an
in-person tournament, awkward at a synchronous one and trivial when the
synchronous circuit is played at home, as it was in 2020 and 2021, so a cheat's
strength depends on the format and an honest player's does not.

```
./target/release/anomaly --top 200      # forecast and residual, split by format
./target/release/anomaly --masks        # the hard-question test, no ratings used
./target/release/anomaly --profile 308949   # one player, tournament by tournament
./target/release/anomaly --who 308949       # one player's mask numbers
```

## Report

```
uv run tools/fetch_fonts.py                           # once, into fonts/
typst compile --font-path fonts report/report.typ
```
