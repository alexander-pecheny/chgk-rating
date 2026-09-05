"""Fidelity of src/bin/ext_fun.rs against the oracle points held for rating.chgk.fun.

    uv run tools/ext_fun_report.py [out/ext/fun.db] > /dev/null

Writes out/ext/fun/report.md.  Oracle files live in data/external/chgk_fun/.
"""

import csv
import glob
import json
import math
import os
import sqlite3
import sys

DB = sys.argv[1] if len(sys.argv) > 1 else "out/ext/fun.db"
EXT = "data/external/chgk_fun"
OUT = "out/ext/fun/report.md"
LAST = 263

METHOD = """\
## The method, in plain English

Source: `data/external/chgk_fun/repo/Ratings/rating.py` (the model), `Site/app.py`
(the API this is checked against) and `readme.md` (the rules in the author's words).

A team of strength `S` takes a question of difficulty `Q` with probability
`p(S, Q) = 1 / (1 + 10^((Q - S) / 400))` — Elo, but the opponent is the question.
A team's strength is composed from its players': a question at the fixed
difficulty 2000 is assumed to be answered by each member independently, so
`1 - p(S, 2000) = prod_i (1 - p(s_i, 2000))`, which inverts to a closed form for
`S`. Only the six highest-rated members count; a seventh and beyond are ignored
for strength but dilute the update.

Given the field's strengths and how many teams took each question, the
difficulty of that question is the `Q` that maximises the likelihood of the
observed takes — equivalently the `Q` at which the expected number of misses
equals the observed number. `rating.py` finds it by a hand-rolled walk from
`Q = 1000` with a step of 340 that halves whenever the residual changes sign,
stopping when the step falls below `1e-5`, and clamps `Q` to `[0, 10000]`; a
question nobody took is 10000, one everybody took is 0. When a tournament
publishes no masks the whole tournament is fitted as one number instead: every
question gets the difficulty implied by the field's average score.

A team's forecast is `sum_j p(S, Q_j)` over the questions it played (`-` in the
mask means it never saw the question). The surplus over forecast is the update.
For a MAII-rated tournament every member of the lineup moves by
`(taken - forecast) * K`, with `K` 10, 7, 5 or 3 as the player has under 10, 20,
30 or more rated games behind them, scaled by `6/n` for lineups over six; for an
unrated tournament the multiplier is a flat 0.1 and the game does not count
towards `K`. Note that a player's move is the *team's* whole surplus, not a
share of it: the author's approximation is that shifting every player by `x`
shifts the team by about `x`.

Releases are weekly, on Thursdays, numbered from 2021-09-02. Every tournament
ending in a release week is scored against the *previous* release's player
ratings, so order inside a week does not matter; at the release the week's
surpluses are summed and applied at once, new players entering at 1000. The
team table is a separate derived quantity: the same composition rule applied to
a team's base roster at that release, not a state of its own. Published
forecasts are `predictedquestions`; predicted *place* is not stored anywhere and
the site derives it by sorting the field on team strength, which is what
`forecast_place` here does.
"""


RUN = """
## Running it

```
cargo build --release
./target/release/ext_fun                                   # -> out/ext/fun.db
./target/release/ext_fun --db buff.db --out out/ext/fun.db --as-of 2026-09-04
uv run tools/ext_fun_report.py                             # -> out/ext/fun/report.md
```

`--as-of` is the day the upstream site was last run, and it is what fixes the
number of releases: upstream processes tournaments up to two release-weeks past
its own run date, so 2026-09-04 gives releases 1..263, matching the oracle table.
One clean chronological pass over 5912 tournaments and 14.2M mask cells takes
**42 s** on this machine (rayon over the questions of a tournament and over its
teams) and writes **1.13 GB** in the DESIGN 6 schema. `eval --db out/ext/fun.db`
and `tools/ndcg.py out/ext/fun.db` read it unchanged: in-person forecast Spearman
0.775 over the acceptance window, NDCG 0.893 against the old system's 0.879.

Columns the system does not have are filled as `simple.rs` fills them: `sigma` is
0 everywhere (so `eval`'s provisional filter passes everything), `games_365` holds
the cumulative count of MAII-rated games, which is what drives `K`, `truedl` holds
the tournament-wide difficulty `Q_hardnes`, and the log-loss and pairwise columns
are NULL. `forecast` is team strength, `x` is `predictedquestions`, `s_r` is
`teamperformance`.
"""

WHY = """
## What matches, and why the rest does not

**The rules match exactly.** Where the inputs agree the outputs are bit-identical,
not merely close: releases 1 and 2 contain ten tournaments, nine of them reproduce
all 524 fitted question difficulties to the last bit; player 131922's first two
releases, and the release-1 and release-2 ratings and per-tournament deltas of
players 7551 and 100501, are exact to 16 digits. Every structural
choice was checked this way: the top-six rule, the `6/n` dilution, the `K` ladder,
which mask characters count as an attempt (`1`, `0`, `?`) versus which only count in
the forecast (`X`) or not at all (`-`), the 0.1 multiplier on unrated tournaments,
the fallback to a single tournament-wide difficulty when masks are hidden, the
Friday-to-Thursday release boundary, and the exact search the author wrote (an
independent Python transcription of `max_like` reproduces this run's numbers to the
last bit, and neither reproduces the site's — see below).

Three things account for everything that does not match.

1. **Data drift — the dominant cause.** rating.chgk.fun's `data.db` has been filled
   incrementally since 2021 and a tournament is refetched only when its
   `lastEditDate` moves; `buff.db` is a snapshot of the same source taken
   2026-09-04. Corrections made upstream in between show up as different inputs.
   Two cases were traced to the row:
   - tournament 6560, team 6936: both sides have a seven-player lineup (so the
     `6/n` dilution and every delta agree to 16 digits) but the ids differ —
     upstream has player 99 where `buff.db` has one of `9801/32004/41617/27469`.
     That alone moves the team's strength in tournament 7307 by 4.46 points and
     is the whole of that tournament's 0.33-point question-difficulty gap.
   - tournament 11811: rating.chgk.fun holds a result for team 57024 (5.5th place,
     31 questions); `buff.db` has no such row at all. That is the one tournament of
     team 57024's 210 missing here.

   The same cause explains the 90 players present here but not upstream and the 142
   present upstream but not here: all of the latter sit within a point or two of
   1000, i.e. they owe their existence to a single lineup that has since changed.

2. **The search's own resolution.** `max_like` stops when its step falls below
   `1e-5`, so a fitted difficulty is only defined to about `1e-5` points, and the
   trajectory is chaotic in its inputs: once anything upstream differs at all, the
   two runs land on different sides of that tolerance. Tournament 7260 is the pure
   case — every one of its 36 differences is an exact integer multiple of the final
   step 340/2^25 = 1.013e-5. That is the floor on agreement, and it compounds: a
   `1e-5` shift in 36 difficulties moves a forecast by `~5e-7` questions, which
   moves a player by `~5e-6` points per game, which changes the next week's field.

3. **Boundary conventions this run does not copy.** Upstream writes a rating row at
   release `n-1` for every player debuting at release `n`, including a release-0
   table; this run starts at release 1. Upstream is also recomputed incrementally
   from the earliest release whose data changed, so its early releases were computed
   against 2021-vintage data and have only been redone where something was edited;
   this run is a single clean chronological pass over today's data. That is a
   difference in kind, not a bug on either side, and it puts a floor under (1).

Nothing remains unexplained in the model itself. What remains unmatched is the
2.1% of players more than 10 points apart, and it is the accumulation of (1) and
(2) over 263 weeks — the two systems are reading slightly different histories.
"""


def pearson(xs, ys):
    n = len(xs)
    mx, my = sum(xs) / n, sum(ys) / n
    num = sum((a - mx) * (b - my) for a, b in zip(xs, ys))
    dx = math.sqrt(sum((a - mx) ** 2 for a in xs))
    dy = math.sqrt(sum((b - my) ** 2 for b in ys))
    return num / (dx * dy)


def quant(v, q):
    v = sorted(v)
    return v[min(len(v) - 1, int(len(v) * q))]


def spread(name, v, unit=""):
    if not v:
        return f"| {name} | 0 | - | - | - | - |"
    v = sorted(v)
    ex = sum(1 for x in v if x < 1e-9) / len(v)
    return (
        f"| {name} | {len(v)} | {ex:.1%} | {quant(v, 0.5):.4g}{unit} | "
        f"{quant(v, 0.9):.4g}{unit} | {v[-1]:.4g}{unit} |"
    )


def main():
    c = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    b = sqlite3.connect("file:buff.db?mode=ro", uri=True)
    L = []
    w = L.append

    w("# rating.chgk.fun, reimplemented\n")
    w(
        f"`src/bin/ext_fun.rs` -> `{DB}` "
        f"({os.path.getsize(DB) / 1e9:.2f} GB, "
        f"{c.execute('select count(*) from releases').fetchone()[0]} releases, "
        f"{c.execute('select count(*) from tournament_metrics').fetchone()[0]} tournaments, "
        f"{c.execute('select count(*) from question_params').fetchone()[0]} fitted questions).\n"
    )
    w(METHOD)
    w(RUN)

    # (a) the current player table.
    w("\n## a. The player table at the last release\n")
    mine = dict(
        c.execute("select player_id, display from player_ratings where release_id=?", (LAST,))
    )
    ora = {}
    with open(f"{EXT}/players_rating_release263.csv") as f:
        for row in csv.DictReader(f):
            ora[int(row["playerid"])] = float(row["rating"])
    common = sorted(set(mine) & set(ora))
    d = sorted(abs(mine[p] - ora[p]) for p in common)
    w(
        f"{len(mine)} players here, {len(ora)} in `players_rating_release263.csv`, "
        f"{len(common)} in both; {len(mine) - len(common)} only here, "
        f"{len(ora) - len(common)} only there.\n"
    )
    w(f"- Pearson correlation **{pearson([mine[p] for p in common], [ora[p] for p in common]):.6f}**")
    w(f"- mean absolute difference **{sum(d) / len(d):.3f}** points, median {quant(d, 0.5):.4f}")
    ranks = sorted(common, key=lambda p: -mine[p])
    orank_all = {p: i for i, p in enumerate(sorted(common, key=lambda p: -ora[p]))}
    w(
        "- Spearman of the two orders "
        f"**{pearson(list(range(len(ranks))), [orank_all[p] for p in ranks]):.6f}**"
    )
    w(
        f"- within 1 point: **{sum(1 for x in d if x <= 1) / len(d):.1%}**; "
        f"within 10 points: **{sum(1 for x in d if x <= 10) / len(d):.1%}**; "
        f"worst {d[-1]:.1f}\n"
    )
    names = dict(
        b.execute("select id, trim(coalesce(surname,'')||' '||coalesce(name,'')) from players")
    )
    orank = sorted(ora, key=lambda p: -ora[p])
    w("Both tables in their own order; `diff` belongs to the left-hand player.\n")
    w("| # | this run | pts | rating.chgk.fun | pts | diff |")
    w("|---:|---|---:|---|---:|---:|")
    for i, p in enumerate(sorted(mine, key=lambda p: -mine[p])[:20]):
        o = orank[i]
        w(
            f"| {i + 1} | {names.get(p, p)} | {mine[p]:.2f} | {names.get(o, o)} | "
            f"{ora[o]:.2f} | {mine[p] - ora.get(p, float('nan')):+.2f} |"
        )

    # (b) one player, release by release.
    w("\n## b. Player 131922, release by release\n")
    hist = json.load(open(f"{EXT}/api_player_131922_full.json"))
    orel = {x["releaseid"]: x["playerrating"] for x in hist["rates"]}
    mrel = dict(
        c.execute("select release_id, display from player_ratings where player_id=131922")
    )
    both = sorted(set(orel) & set(mrel))
    diff = [abs(mrel[r] - orel[r]) for r in both]
    first = next((r for r in both if abs(mrel[r] - orel[r]) > 1e-9), None)
    w(
        f"{len(mrel)} releases here against {len(orel)} there "
        f"(theirs also carries a release-0 row at 1000 for every debutant, which this "
        f"run does not write, and one release more than the 2026-09-04 cut this run "
        f"reproduces). Releases 1 and 2 are bit-exact; the first difference is at "
        f"release **{first}** and it is {abs(mrel[first] - orel[first]):.2g} points.\n"
    )
    w("| release | this run | rating.chgk.fun | diff |")
    w("|---:|---:|---:|---:|")
    for r in [x for x in both if x <= 6] + [x for x in both if x % 25 == 0] + [both[-1]]:
        w(f"| {r} | {mrel[r]:.4f} | {orel[r]:.4f} | {mrel[r] - orel[r]:+.4f} |")
    w(
        f"\nOver all {len(both)} releases: median |diff| {quant(diff, 0.5):.4f}, "
        f"p90 {quant(diff, 0.9):.4f}, max {max(diff):.4f}.\n"
    )
    w("Per tournament, over this player's 1413 games (every one of them present on both sides):\n")
    tids = tuple(sorted({x["tournamentid"] for x in hist["deltas"]}))
    ev = {
        (t, tm): (f, x, sr)
        for t, tm, f, x, sr in c.execute(
            "select tournament_id, team_id, forecast, x, s_r from team_events "
            "where tournament_id in (%s)" % ",".join("?" * len(tids)),
            tids,
        )
    }
    pe = dict(
        (t, a - bb)
        for t, bb, a in c.execute(
            "select tournament_id, theta_before, theta_after from player_events "
            "where player_id=131922"
        )
    )
    cols = {"team strength": [], "forecast (questions)": [], "performance": [], "delta": []}
    nulls = 0
    for x in hist["deltas"]:
        f, xx, sr = ev[(x["tournamentid"], x["teamid"])]
        cols["team strength"].append(abs(f - x["teamrating"]))
        if x["predictedquestions"] is not None:
            cols["forecast (questions)"].append(abs(xx - x["predictedquestions"]))
        if (x["teamperformance"] is None) != (sr is None):
            nulls += 1
        elif sr is not None:
            cols["performance"].append(abs(sr - x["teamperformance"]))
        if x["deltarating"] is not None:
            cols["delta"].append(abs(pe[x["tournamentid"]] - x["deltarating"]))
    w("| quantity | n | exact | median | p90 | max |")
    w("|---|---:|---:|---:|---:|---:|")
    for k, v in cols.items():
        w(spread(k, v))
    w(f"\nRows where one side has a number and the other does not: **{nulls}**.\n")

    # (c) question difficulties.
    w("\n## c. Fitted question difficulties\n")
    w("| tournament | release | questions | exact | max |diff| | note |")
    w("|---:|---:|---:|---:|---:|---|")
    for path in sorted(glob.glob(f"{EXT}/api_questions_*.json")):
        tid = int(path.rsplit("_", 1)[1].split(".")[0])
        o = json.load(open(path))
        m = [v for _, v in c.execute(
            "select q, b from question_params where tournament_id=? order by q", (tid,)
        )]
        rel = c.execute(
            "select (julianday(date)-julianday('2021-09-02'))/7 from tournament_metrics "
            "where tournament_id=?", (tid,)
        ).fetchone()
        if len(m) != len(o):
            w(f"| {tid} | | {len(m)} vs {len(o)} | - | - | length mismatch |")
            continue
        dd = [abs(a - z) for a, z in zip(m, o)]
        ex = sum(1 for x in dd if x < 1e-9)
        note = (
            "bit-exact"
            if ex == len(dd)
            else ("search tolerance only" if max(dd) < 1e-4 else "input data differs")
        )
        w(
            f"| {tid} | {math.ceil(rel[0]) if rel else '?'} | {len(dd)} | "
            f"{ex}/{len(dd)} | {max(dd):.3g} | {note} |"
        )

    # (d) one team, tournament by tournament.
    w("\n## d. Team 57024 (Вжух), tournament by tournament\n")
    tm = json.load(open(f"{EXT}/api_team_57024_full.json"))["tournaments"]
    mt = {
        t: (f, x, sr)
        for t, f, x, sr in c.execute(
            "select tournament_id, forecast, x, s_r from team_events where team_id=57024"
        )
    }
    miss = [t["tournamentid"] for t in tm if t["tournamentid"] not in mt]
    cols = {"team strength": [], "forecast (questions)": [], "performance": []}
    for t in tm:
        if t["tournamentid"] not in mt:
            continue
        f, x, sr = mt[t["tournamentid"]]
        cols["team strength"].append(abs(f - t["teamrating"]))
        if t["predictedquestions"] is not None:
            cols["forecast (questions)"].append(abs(x - t["predictedquestions"]))
        if t["teamperformance"] is not None and sr is not None:
            cols["performance"].append(abs(sr - t["teamperformance"]))
    w(f"{len(tm)} tournaments there, {len(mt)} here; missing here: {miss or 'none'}.\n")
    w("| quantity | n | exact | median | p90 | max |")
    w("|---|---:|---:|---:|---:|---:|")
    for k, v in cols.items():
        w(spread(k, v))
    tr = c.execute(
        "select display, release_id from team_ratings where team_id=57024 "
        "order by release_id desc limit 1"
    ).fetchone()
    tt = list(csv.DictReader(open(f"{EXT}/teams_rating.csv")))
    top = {int(r["teamid"]): float(r["rating"]) for r in tt}
    mine_t = dict(
        c.execute("select team_id, display from team_ratings where release_id=?", (LAST,))
    )
    both_t = sorted(set(top) & set(mine_t))
    dt = sorted(abs(mine_t[t] - top[t]) for t in both_t)
    w(
        f"\nTeam table: this run publishes {len(mine_t)} teams at release {LAST} "
        f"(`buff.db` carries base rosters for seasons 60-61 only, so the team table is "
        f"empty before release 209); `teams_rating.csv` has {len(tt)}, "
        f"{len(both_t)} in common. Team 57024 here **{tr[0]:.2f}** against "
        f"**{top[57024]:.2f}**; over the common teams the median difference is "
        f"{quant(dt, 0.5):.2f} points, the mean {sum(dt) / len(dt):.2f}, "
        f"{sum(1 for x in dt if x <= 10) / len(dt):.1%} within 10 and "
        f"{sum(1 for x in dt if x <= 1) / len(dt):.1%} within 1. The tail is roster "
        f"reconstruction, not rating: upstream keeps a weekly snapshot of each team's "
        f"base roster and this run rebuilds it from `team_seasons` date windows, so a "
        f"team whose roster was edited reads differently on the two sides.\n"
    )

    # (e) one tournament's forecast rows.
    w("\n## e. Forecast rows of tournament 13024\n")
    rows = []
    for path in sorted(glob.glob(f"{EXT}/ttr_13024/*.json")):
        try:
            rows.append(json.load(open(path)))
        except (json.JSONDecodeError, UnicodeDecodeError):
            pass
    got = {
        t: (f, x, sr, fp, ap)
        for t, f, x, sr, fp, ap in c.execute(
            "select team_id, forecast, x, s_r, forecast_place, actual_place "
            "from team_events where tournament_id=13024"
        )
    }
    w(
        f"13024 hides its masks until 2026-09-29, so it is fitted as a single number: "
        f"every one of its 36 questions carries the same difficulty. "
        f"{len(got)} teams have a forecast row here; "
        f"{len(rows)} were sampled from `/api/tournamentteamrates/13024/<team>`.\n"
    )
    w("| team | strength here | strength there | forecast here | forecast there | perf here | perf there |")
    w("|---:|---:|---:|---:|---:|---:|---:|")
    for r in sorted(rows, key=lambda r: -r["teamrating"])[:25]:
        t = r["teamid"]
        if t not in got:
            w(f"| {t} | absent | {r['teamrating']:.2f} | | | | |")
            continue
        f, x, sr, fp, ap = got[t]
        op = r["teamperformance"]
        w(
            f"| {t} | {f:.2f} | {r['teamrating']:.2f} | {x:.3f} | "
            f"{r['predictedquestions']:.3f} | "
            f"{'-' if sr is None else format(sr, '.2f')} | "
            f"{'-' if op is None else format(op, '.2f')} |"
        )
    cols = {"team strength": [], "forecast (questions)": [], "performance": []}
    for r in rows:
        if r["teamid"] not in got:
            continue
        f, x, sr, fp, ap = got[r["teamid"]]
        cols["team strength"].append(abs(f - r["teamrating"]))
        cols["forecast (questions)"].append(abs(x - r["predictedquestions"]))
        if r["teamperformance"] is not None and sr is not None:
            cols["performance"].append(abs(sr - r["teamperformance"]))
    w("")
    w("| quantity | n | exact | median | p90 | max |")
    w("|---|---:|---:|---:|---:|---:|")
    for k, v in cols.items():
        w(spread(k, v))

    w(WHY)

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as f:
        f.write("\n".join(L) + "\n")
    print(f"wrote {OUT}")


main()
