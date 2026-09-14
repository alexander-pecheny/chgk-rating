# Deployment

`chgk-rating.service` runs `web/target/release/serve` on port 9684 and Caddy
proxies rating2.pecheny.me to it. `deploy/chgk-rating.service` is the copy of
the unit file.

## The daily update

Cron runs `deploy/update.sh` at 06:30 UTC every day. `deploy/crontab` holds the
line. The script copies the new `buff.db` from `vps2day-ee` (it is rebuilt there
at 01:05 UTC and the graph at 04:50 UTC), refreshes `out/team_names.db` from the
rating API with `tools/fetch_team_names.py`, runs `fit` and `eval`, restarts the
service and waits for the site to answer. `fit` now ends its window today, so
each run picks up the tournaments added since yesterday. The whole thing takes
about five minutes.

The team-name sweep is the one step that may fail without stopping the run: the
serving index falls back on the `out/team_names.db` of the previous day. The
script skips it altogether when `tools/fetch_team_names.py` is not in the
checkout. It needs `uv`, which is why the script sets its own `PATH` — cron
gives it almost none.

The log is `out/update.log`, one line per step with a UTC timestamp, and the
last line of a run gives the size of `buff.db`, the date of the last release and
how long the run took. `out/fit.log` and `out/eval.md` hold the full output of
the two binaries. A failed step leaves the service running on the old numbers.

## By hand

```
/home/pecheny/new_chgk_rating/deploy/update.sh
SKIP_SYNC=1 SKIP_RESTART=1 REPO=$PWD deploy/update.sh   # a dry run elsewhere
```

`SKIP_SYNC=1` keeps the current `buff.db`, `SKIP_RESTART=1` leaves the service
alone, and `REPO` points the run at another checkout. Runs cannot overlap:
`out/update.lock` is held for the length of one.
