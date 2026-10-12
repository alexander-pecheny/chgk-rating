"""Gaps between near-inseparable partners under several fits.

A pair qualifies when the two shared at least 100 lineups from 2013 on and
each played at least 85% of their games with the other. For each fit, prints
the median and 90th percentile of the absolute theta gap at the latest
release, and how many pairs are more than 0.4 apart.

    uv run tools/partner_gaps.py buff.db out/krapil/base.db out/krapil/st2_cap02.db ...
"""

import itertools
import json
import sqlite3
import statistics
import sys
from collections import Counter

MIN_SHARED = 100
MIN_SHARE = 0.85
WIDE = 0.4


def pairs(buff):
    c = sqlite3.connect(buff)
    games, shared = Counter(), Counter()
    rows = c.execute("""select tr.team_members from tournament_results tr join tournaments t on t.id = tr.id
                        where t.date_start >= '2013' and tr.mask is not null and tr.team_members != '[]'""")
    for (members,) in rows:
        ids = sorted(set(json.loads(members)))
        games.update(ids)
        shared.update(itertools.combinations(ids, 2))
    return [(a, b) for (a, b), n in shared.items()
            if n >= MIN_SHARED and n >= MIN_SHARE * games[a] and n >= MIN_SHARE * games[b]]


def gaps(db, ps):
    c = sqlite3.connect(db)
    rel = c.execute("select max(id) from releases").fetchone()[0]
    theta = dict(c.execute("select player_id, theta from player_ratings where release_id = ?", (rel,)))
    return [abs(theta[a] - theta[b]) for a, b in ps if a in theta and b in theta]


def main():
    global MIN_SHARE
    args = sys.argv[1:]
    if args[0].startswith("--share="):
        MIN_SHARE = float(args.pop(0).split("=")[1])
    buff, dbs = args[0], args[1:]
    ps = pairs(buff)
    print(f"{len(ps)} pairs at share {MIN_SHARE}")
    for db in dbs:
        g = sorted(gaps(db, ps))
        p90 = g[int(0.9 * (len(g) - 1))]
        print(f"{db}: n {len(g)}  median {statistics.median(g):.3f}  p90 {p90:.3f}  >{WIDE} {sum(x > WIDE for x in g)}")


if __name__ == "__main__":
    main()
