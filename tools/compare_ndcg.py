"""One table, every system, both ways of ordering a field.

A *player-based* row ranks the field by the `forecast` column, the strength of
the lineup that turned up.  A *team-based* row ranks it by `mu_before`, the team
number as it stood that morning, with a team that has no rating last.  Both are
scored with the NDCG of `tools/ndcg.py` -- relevance `1/place`, the same
tie-break -- so the two orderings are directly comparable, and every row is
averaged over the tournaments all of them cover.

    uv run tools/compare_ndcg.py "label=path[:mu]" ... [--paired LABEL]
    uv run tools/compare_ndcg.py --list          # the rows of out/final_report.md
    uv run tools/compare_ndcg.py ... --window tune

`--paired LABEL` adds the paired bootstrap standard error of that row's weighted
difference against every other row, over the same tournaments.  `--window tune`
scores the tune window of plan 3 instead of the test window, which is the window
a round's own decisions were made on; `--window a:b` takes any two dates.
"""

import random
import sqlite3
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ndcg as N  # noqa: E402

FROM, TO = "2024-08-30", "2026-08-27"
BOOT = 2000
SEED = 20260914
# A team with no rating cannot be placed by its rating, so it goes last.
UNRATED = -1e18

QUERY = """
select e.tournament_id, e.team_id, e.actual_place, e.forecast, e.mu_before
from team_events e
join tournament_metrics m on m.tournament_id = e.tournament_id
where m.type = 'in-person' and m.date >= ? and m.date < ? and e.actual_place > 0
order by e.tournament_id
"""


def score(db, col, frm=FROM, to=TO):
    """`{tournament: {k: ndcg}}` for k in (None, 3), ranking the field by `col`.

    A `forecast` row drops a tournament where any team has no forecast; a `mu`
    row keeps it, because an unrated team still has a place in the order.
    """
    rows = sqlite3.connect(db).execute(QUERY, (frm, to)).fetchall()
    by_t = {}
    for tid, team, place, forecast, mu in rows:
        by_t.setdefault(tid, []).append((team, place, forecast, mu))
    out = {}
    for tid, ts in by_t.items():
        if len(ts) < 5:
            continue
        if col == "forecast" and any(t[2] is None for t in ts):
            continue
        places = {t[0]: t[1] for t in ts}
        strength = {t[0]: (t[2] if col == "forecast" else (UNRATED if t[3] is None else t[3])) for t in ts}
        order = list(strength)
        # The tie-break of tools/ndcg.py and src/ndcg.rs: a shuffle seeded from
        # the tournament id, so the stored finishing order never leaks in.
        N.Rng(tid).shuffle(order)
        order.sort(key=lambda t: -strength[t])
        out[tid] = {k: N.ndcg(order, places, k, strength) for k in (None, 3)}
    return out


def wmean(runs, tids, k, w):
    num = sum(w.get(t, 0.0) * runs[t][k] for t in tids)
    den = sum(w.get(t, 0.0) for t in tids)
    return num / den if den > 0 else float("nan")


def umean(runs, tids, k):
    return sum(runs[t][k] for t in tids) / len(tids)


def paired(a, b, tids, k, w):
    """Difference of the weighted means, a - b, and its bootstrap standard error."""
    keys = sorted(tids)
    rng = random.Random(SEED)

    def diff(sample):
        num = sum(w.get(t, 0.0) * (a[t][k] - b[t][k]) for t in sample)
        den = sum(w.get(t, 0.0) for t in sample)
        return num / den if den > 0 else float("nan")

    point = diff(keys)
    draws = [diff([keys[rng.randrange(len(keys))] for _ in keys]) for _ in range(BOOT)]
    m = sum(draws) / len(draws)
    se = (sum((d - m) ** 2 for d in draws) / len(draws)) ** 0.5
    return point, se


TEAM_WINNER = (
    "/home/pecheny/new_chgk_rating/.claude/worktrees/agent-a5dfc6cbe2f6c30ce/out/team_ndcg/winner.db"
)

ROWS = [
    ("rating.chgk.fun (player based)", "out/ext/fun.db", "forecast"),
    ("chgk.quest (player based)", "out/ext/quest.db", "forecast"),
    ("shipped model (player based)", "out/shipped.db", "forecast"),
    ("proposed model (player based, new players)", "out/final.db", "forecast"),
    ("official chgk.gg (player based)", "out/ext/gg.db", "forecast"),
    ("proposed model (team based, current players)", TEAM_WINNER, "mu"),
    ("proposed model (team based, new players)", "out/final.db", "mu"),
    ("official chgk.gg (team based)", "out/ext/gg.db", "mu"),
]
DEFAULT_PAIRED = "proposed model (player based, new players)"


WINDOWS = {"tune": ("2018-09-01", "2024-08-29"), "test": (FROM, TO)}


def main():
    global FROM, TO
    args = sys.argv[1:]
    specs, against = [], None
    i = 0
    while i < len(args):
        if args[i] == "--window":
            w = args[i + 1]
            FROM, TO = WINDOWS.get(w) or tuple(w.split(":"))
            i += 2
        elif args[i] == "--list":
            specs, against = list(ROWS), DEFAULT_PAIRED
            i += 1
        elif args[i] == "--paired":
            against = args[i + 1]
            i += 2
        else:
            label, rest = args[i].split("=", 1)
            path, _, col = rest.partition(":")
            specs.append((label, path, col or "forecast"))
            i += 1
    if not specs:
        specs, against = list(ROWS), DEFAULT_PAIRED

    w = N.load_weights(N.WEIGHTS)
    runs = {label: score(path, col, FROM, TO) for label, path, col in specs}
    tids = set.intersection(*(set(r) for r in runs.values()))

    table = [
        (
            label,
            wmean(runs[label], tids, None, w),
            wmean(runs[label], tids, 3, w),
            umean(runs[label], tids, None),
            umean(runs[label], tids, 3),
        )
        for label, _, _ in specs
    ]
    table.sort(key=lambda r: -r[1])

    print(
        f"NDCG, relevance 1/place, in-person, {FROM} to {TO}, "
        f"{len(tids)} common tournaments, weights from {N.WEIGHTS}\n"
    )
    print("| system | NDCG (weighted) | NDCG@3 (weighted) | NDCG | NDCG@3 |")
    print("|---|---:|---:|---:|---:|")
    for r in table:
        print(f"| {r[0]} | {r[1]:.4f} | {r[2]:.4f} | {r[3]:.4f} | {r[4]:.4f} |")

    if against is None:
        return
    print(f"\nPaired bootstrap over the same tournaments, {BOOT} resamples, weighted; "
          f"{against} minus the other row.\n")
    print("| against | NDCG | SE | NDCG@3 | SE |")
    print("|---|---:|---:|---:|---:|")
    for label, _, _ in specs:
        if label == against:
            continue
        d, se = paired(runs[against], runs[label], tids, None, w)
        d3, se3 = paired(runs[against], runs[label], tids, 3, w)
        print(f"| {label} | {d:+.4f} | {se:.4f} | {d3:+.4f} | {se3:.4f} |")


if __name__ == "__main__":
    main()
