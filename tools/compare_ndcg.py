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
`--team-winner PATH` is the plan-4 team-rule winner of the `--list` table.
"""

import lab

# A team with no rating cannot be placed by its rating, so it goes last.
UNRATED = -1e18
AT = (None, lab.TOP)
FIELD_COLS = "e.tournament_id, e.team_id, e.actual_place, e.forecast, e.mu_before"

TEAM_WINNER = "out/team_ndcg/winner.db"
DEFAULT_PAIRED = "proposed model (player based, new players)"
OPTIONS = [
    ("specs", dict(nargs="*", metavar="label=path[:mu]")),
    ("--window", dict(default="test", help="tune, test or FROM:TO")),
    ("--list", dict(action="store_true", help="the rows of out/final_report.md")),
    ("--paired", dict(metavar="LABEL")),
    ("--team-winner", dict(default=TEAM_WINNER, help="the plan-4 team-rule winner of --list")),
]


def rows(team_winner):
    """The rows of out/final_report.md."""
    return [
        ("rating.chgk.fun (player based)", "out/ext/fun.db", "forecast"),
        ("chgk.quest (player based)", "out/ext/quest.db", "forecast"),
        ("shipped model (player based)", "out/shipped.db", "forecast"),
        (DEFAULT_PAIRED, "out/final.db", "forecast"),
        ("official chgk.gg (player based)", "out/ext/gg.db", "forecast"),
        ("proposed model (team based, current players)", team_winner, "mu"),
        ("proposed model (team based, new players)", "out/final.db", "mu"),
        ("official chgk.gg (team based)", "out/ext/gg.db", "mu"),
    ]


def score(db, col, frm, to):
    """`{tournament: {k: ndcg}}` for k in (None, 3), ranking the field by `col`.

    A `forecast` row drops a tournament where any team has no forecast; a `mu`
    row keeps it, because an unrated team still has a place in the order.
    """
    out = {}
    for tid, ts in lab.by_tournament(lab.in_person(db, frm, to, FIELD_COLS)).items():
        if len(ts) >= lab.MIN_FIELD and (col != "forecast" or all(t[2] is not None for t in ts)):
            places = {t[0]: t[1] for t in ts}
            strength = {t[0]: (t[2] if col == "forecast" else (UNRATED if t[3] is None else t[3])) for t in ts}
            order = lab.ranked(strength, strength.get, lab.Rng(tid))
            out[tid] = {k: lab.ndcg(order, places, k) for k in AT}
    return out


def at(run, k):
    return {t: v[k] for t, v in run.items()}


def parse():
    a = lab.cli(__doc__, OPTIONS, intermixed=True)
    specs = [spec(s) for s in a.specs]
    if a.list or not specs:
        specs, a.paired = rows(a.team_winner), DEFAULT_PAIRED
    return a, specs


def spec(s):
    """`label=path[:mu]` as `(label, path, column)`."""
    label, rest = s.split("=", 1)
    path, _, col = rest.partition(":")
    return label, path, col or "forecast"


def print_means(specs, runs, tids, w):
    table = [(label, *(lab.wmean(at(runs[label], k), tids, wt) for wt in (w, None) for k in AT))
             for label, _, _ in specs]
    table.sort(key=lambda r: -r[1])
    lab.header("| system | NDCG (weighted) | NDCG@3 (weighted) | NDCG | NDCG@3 |", "|---|---:|---:|---:|---:|")
    for r in table:
        print(f"| {r[0]} | {r[1]:.4f} | {r[2]:.4f} | {r[3]:.4f} | {r[4]:.4f} |")


def print_paired(specs, runs, tids, w, against):
    print(f"\nPaired bootstrap over the same tournaments, {lab.BOOT} resamples, weighted; "
          f"{against} minus the other row.\n")
    lab.header("| against | NDCG | SE | NDCG@3 | SE |", "|---|---:|---:|---:|---:|")
    for label, _, _ in specs:
        if label != against:
            (d, se), (d3, se3) = (lab.paired(at(runs[against], k), at(runs[label], k), tids, w) for k in AT)
            print(f"| {label} | {d:+.4f} | {se:.4f} | {d3:+.4f} | {se3:.4f} |")


def main():
    a, specs = parse()
    frm, to = lab.window(a.window)
    w = lab.load_weights()
    runs = {label: score(path, col, frm, to) for label, path, col in specs}
    tids = set.intersection(*(set(r) for r in runs.values()))
    print(f"NDCG, relevance 1/place, in-person, {frm} to {to}, "
          f"{len(tids)} common tournaments, weights from {lab.WEIGHTS}\n")
    print_means(specs, runs, tids, w)
    if a.paired is not None:
        print_paired(specs, runs, tids, w, a.paired)


if __name__ == "__main__":
    main()
