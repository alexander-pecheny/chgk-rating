# Deployment

`chgk-rating.service` runs `web/serve`, the Go server, on port 9684 and Caddy
proxies a2.pecheny.me to it. `deploy/chgk-rating.service` is the copy of
the unit file. At start-up the server runs `typst` to draw the formulas on
/method/, and finds it through `/usr/local/bin/typst`, a link to
`~/.local/bin/typst`, since a unit's PATH leaves out `~/.local/bin`.

## The daily update

Cron runs `deploy/update.sh` at 06:30 UTC every day. `deploy/crontab` holds the
line. The script brings `buff.db` over from `vps2day-ee` (it is rebuilt there
at 01:05 UTC and the graph at 04:50 UTC), runs `fit`, `eval`, `achieve` and `webindex`, restarts the
service and waits for the site to answer. `fit` now ends its window today, so
each run picks up the tournaments added since yesterday. The whole thing takes
about three minutes.

The copy is an rsync onto a duplicate of yesterday's file, so only the pages
that changed cross the network: about 10 MB of the 1 GB, in five seconds.
Everything the site needs is in `buff.db`, team names included — the mirror
takes them from the rating site's own team listing every night.

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

## Previews

`just dev <name>` in a worktree serves it at `https://<name>.a2-dev.pecheny.me`.
It links production's `buff.db` and `data/external` into the worktree, builds,
fits into the worktree's own `out/` if it has no index yet (`just dev <name>
refit` fits again), and starts the instance with `~/.local/bin/a2-dev`. Each
instance is an `a2-dev@<name>` unit running the worktree's `web/serve` on its
own port, behind a route in `/etc/caddy/a2-dev.d/`; the wildcard DNS record and
certificate cover every name. `a2-dev ls` lists the instances and `a2-dev rm
<name>` removes one. A rebuild of the server alone needs `just web` and
`a2-dev restart <name>`.
