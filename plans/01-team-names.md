# Plan 1: the site shows a team's base name, not its last one-off name

## Problem

https://rating2.pecheny.me/teams/308/ is titled "Maroon 6". The team is
"Номер 2" (Донецк); "Maroon 6" is the name it entered one tournament under.
`web/src/index.rs` (the "one pass over the results" block, ~line 163) takes the
team's name and town from `tournament_results.team_current_name` /
`team_current_town` of the team's latest result. `buff.db` carries the base name
in `teams(id, name, name_fold, town, last_tournament_id)`:

```
sqlite3 buff.db "select * from teams where id=308"
308|Номер 2|номер 2|Донецк|13013
```

Every team with a one-off name on its latest result is affected, not only 308.

## Change

1. **Base name from `teams`.** In the index build, load `teams(id, name, town)`
   from `buff.db`. The `team` row takes `name`/`town` from `teams` when the row
   exists and `name` is non-empty; otherwise it keeps the current fallback (the
   name on the latest result). `games` and `last_date` stay as computed now.
2. **Keep the played-as name per result.** Add a `name TEXT` column to `result_`
   (or a small `result_name(tournament_id, team_id, name)` table) holding
   `team_current_name`. On the tournament page (`web/src/pages.rs`, the query
   near line 1022) show the base name as the link and, when the played-as name
   differs after trimming and case-folding, append it in muted style:
   `Номер 2 <span class="muted">(как Maroon 6)</span>`. Add the `.muted` rule
   to the stylesheet if there is none. Do not change the player page's game
   history (it already joins `x.team`).
3. **Search still finds aliases.** Add an `aliases` column to `team_fts`
   holding every distinct played-as name of the team other than the base name,
   space-separated, so a search for "Maroon 6" still returns team 308. Search
   result rows display the base name. Keep the FTS column count in sync with
   every `insert into team_fts` and the search query in `pages.rs`.
4. **`eval` top tables.** `src/bin/eval.rs` lines ~509 and ~552 label teams by
   a `team_current_name` subquery; switch them to `buff.teams.name` with the
   same fallback. Grep the whole repo (`src`, `web`, `tools`) for
   `team_current_name` and treat every display use the same way; the data
   loader in `src/data.rs` is not a display use.
5. **Index version.** `serve` rebuilds `out/web.db` only when `out/ratings.db`
   is newer. The schema changes, so bump whatever schema/version marker the
   index has (or add one: a `meta(key, value)` table with `schema = N`) so an
   old `web.db` is rebuilt on the next start without `--rebuild`.

## Verify

- `cd web && cargo build --release`, then from the repo root
  `./web/target/release/serve out/shipped.db --addr 127.0.0.1:8089 --rebuild`
  (it writes `out/web.db` relative to the cwd, so this does not touch the
  production index). `curl -s localhost:8089/teams/308/ | grep -o '<h1>[^<]*'`
  must say Номер 2; `curl -s 'localhost:8089/search/?q=Maroon+6'` must list
  team 308; a tournament page where 308 played as Maroon 6 must show the
  muted alias. Kill the server afterwards.
- Count how many teams changed name: print it from the index build
  (`eprintln!`), and put the number in the commit message.
- `cargo build --release` for the root crate and `./target/release/eval --db
  out/shipped.db --brief` still run.

## Constraints

- Work only in this worktree; never write under `/home/pecheny/new_chgk_rating`
  or `/tmp`. Large outputs go to `out/` here.
- Use the Edit tool for shared files (`web/src/pages.rs`, `src/bin/eval.rs`);
  another agent is editing `src/config.rs`, `src/bin/fit.rs`, `src/bin/eval.rs`
  and `README.md` at the same time — keep your `eval.rs` change to the two
  name subqueries.
- Update `README.md`'s Site section and `DESIGN.md` section 10 in one sentence
  each if they describe where names come from.
- Commit on the current branch with a message in plain English; end it with the
  attribution lines given in your instructions.
