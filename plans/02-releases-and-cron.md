# Plan 2: test runs keep only the last release; the site refits itself daily

Two maintenance changes in one agent because both touch `src/config.rs`,
`src/bin/fit.rs` and `README.md`.

## Part A: research runs write rows only for the last release

`out/ratings.db` is 1.4 GB because `player_ratings` and `team_ratings` carry a
row per entity per weekly release (`data.rows_every = 1` in `config.toml`;
`fit.rs` ~line 437 writes rows when `last || release_id % rows_every == 0`).
Research fits (`config_simple*.toml`, the ladder tomls, every sweep) need none
of that history: their scores come from `team_events`, `player_events` and the
last release. Only the production setup keeps every release.

1. **Semantics.** `rows_every = 0` means "rows at the last release only".
   `fit` gets a `--rows-every N` CLI override (same style as `--to`,
   `--passes`). `simple.rs` has the same release loop (`st.release` at ~244 and
   ~499); give it the same rule and flag. The `releases` table itself stays
   weekly in both — it is a few thousand rows.
2. **Configs.** `config_simple.toml`, `config_simple_level.toml`,
   `config_simple_s.toml`, `config_gg.toml`, `config_quest.toml` set
   `rows_every = 0` (where the section exists; add it if the binary reads
   `data.rows_every`). `config.toml`, the production config, keeps `1` with a
   comment saying it is the only config that keeps history.
3. **`eval` degrades gracefully.** Run `eval` on a last-release-only database
   and make every section that needs a historical release print `n/a (rows
   only at the last release)` instead of panicking or printing zeros: the
   inflation test (September releases of the 50th team), the swap test's
   nearest-release lookup, the case-team columns at 2025-09 and 2026-03, the
   2019 display-map checks, anything that does `ORDER BY ABS(julianday(date)-
   julianday(?))`. Detect the situation by `SELECT COUNT(DISTINCT release_id)
   FROM team_ratings` = 1. `eval --brief` must keep working. `tools/ndcg.py`
   reads only `team_events` and needs no change; confirm.
4. **Verify.** Build; `./target/release/fit --config config_simple.toml
   --rows-every 0 --out out/last_only.db` is not a thing (`simple` has its own
   binary) — so run both:
   `./target/release/fit --rows-every 0 --out out/t_last.db` and
   `./target/release/simple --config config_simple.toml --out out/s_last.db`.
   Check sizes against the 1.4 GB full run, run `eval --db` on each, and
   confirm `team_events` row counts equal the full run's (compare with
   `out/shipped.db`). Delete the test databases when done.
5. **Docs.** README (the `fit` section and the `simple` section), DESIGN.md
   section 6 (outputs) get the rule in one sentence each.

## Part B: `to` defaults to today

`config.toml` pins `to = "2026-09-05"`, so a daily run would stop there.
Make `data.to` optional: absent (or the literal `"today"`) means the current
UTC date. `--to` on the CLI still overrides. Remove the pinned date from
`config.toml`; leave it in the research configs if they rely on a fixed window
(check; the ladder report says its runs were "2013-01-01 to 2026-09-05", so a
fixed `to` in research configs is the right thing and stays). `fit` prints the
resolved window at start.

## Part C: the daily update script and its cron entry

Facts:

- `buff.db` is refreshed on `vps2day-ee` by `/home/ap/buff/update_db.py` at
  01:05 UTC daily and a graph rebuild at 04:50 UTC (both machines run UTC).
  It was last copied with `rsync` over ssh (`~/.ssh/config` has the host);
  1 GB at about 11 MB/s takes 80 seconds.
- The production checkout is `/home/pecheny/new_chgk_rating`
  (`chgk-rating.service`, `deploy/chgk-rating.service`). `serve` holds
  `buff.db` and `out/ratings.db` open and rebuilds `out/web.db` on restart when
  `ratings.db` is newer (about 90 seconds). Replacing an open SQLite file by
  `mv` is safe: the old inode lives until restart. `fit`'s `Writer::create`
  unlinks and recreates `out/ratings.db`, which is the same story.
- `sudo -n systemctl restart chgk-rating` works for this user without a
  password (checked: `sudo -n true` succeeds).
- The user's crontab already has one line (`sess index` every 15 minutes).
  Preserve it.

Write `deploy/update.sh` (POSIX sh, `set -eu`, executable):

1. `REPO=${REPO:-/home/pecheny/new_chgk_rating}`; `cd "$REPO"`. Take an
   exclusive lock (`flock -n "$REPO/out/update.lock"`) so two runs never
   overlap; log lines with a UTC timestamp to stdout (cron appends to
   `out/update.log`).
2. Unless `SKIP_SYNC=1`: `rsync -a --partial vps2day-ee:/home/ap/buff/buff.db
   "$REPO/buff.db.tmp"` then `mv -f buff.db.tmp buff.db`. Abort the run if
   rsync fails, keeping the old `buff.db`.
3. `./target/release/fit > out/fit.log 2>&1` (the resolved window ends today
   after Part B). Then `./target/release/eval > out/eval.md 2>&1`. On failure,
   print which step failed and exit non-zero, leaving the service running on
   the old numbers.
4. Unless `SKIP_RESTART=1`: `sudo -n systemctl restart chgk-rating`, then
   poll `curl -sf http://127.0.0.1:9684/` for up to three minutes and log
   whether the site answered.
5. Print one summary line: date, buff.db size, last release date from
   `out/ratings.db` (`select max(date) from releases`), elapsed seconds.

Cron line (keep a copy at `deploy/crontab`):

```
30 6 * * * /home/pecheny/new_chgk_rating/deploy/update.sh >> /home/pecheny/new_chgk_rating/out/update.log 2>&1
```

Install it with a short `uv run python -c` snippet that reads `crontab -l`,
appends the line only if it is not already present, and writes the result
back with `crontab -`. Verify with `crontab -l`.

The script path in the cron line is the production checkout, where the script
does not exist until this branch is merged; the orchestrator merges it. Test
the script here with `REPO=$PWD SKIP_SYNC=1 SKIP_RESTART=1 deploy/update.sh`
(it will refit into this worktree's `out/`, which is fine; `buff.db` here is a
symlink to the production copy and is read only). Delete the test outputs it
leaves except the log.

Also add `deploy/README.md` (or a section in `README.md`) with the three
sentences a future reader needs: what runs when, where the log is, and how to
run it by hand.

## Constraints

- Never write under `/home/pecheny/new_chgk_rating` or `/tmp`; large outputs
  go to `out/` in this worktree. Do not restart the production service.
- Another agent edits `web/` and the two team-name subqueries in
  `src/bin/eval.rs` concurrently. Use the Edit tool, not Write, on
  `src/bin/eval.rs`, `README.md` and `DESIGN.md`.
- Prefer the Write/Edit tools or `uv run python -c` for file creation; do not
  build files with bash heredocs.
- Commit on the current branch in plain English, one commit per part is fine;
  end each message with the attribution lines given in your instructions.
