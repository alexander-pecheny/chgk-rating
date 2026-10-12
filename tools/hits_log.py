"""The NDCG and NDCG@3 of /method/, each system on its own field: the team at
forecast position i has relevance 1/(|i - place| + 1), a shared place standing
at the centre of its range, discounted by log2(i + 1), over the same sum for
the actual order.

A spec `label=path[:column]` orders the field by `forecast` (the default), `x`,
or `mu_before`, where a team with no rating goes last. Prints the weighted and
unweighted means over the tournaments every spec covers, then the first spec's
weighted difference against each other with its paired bootstrap SE:

    uv run tools/hits_log.py 2025-01-01 2026-08-28 "A2=out/ratings.db" "elo=out/ext/elo.db:x"

`tools/rival_metrics.py` scores the same NDCG on the teams all systems share,
beside the other measures.
"""

import sys

import lab
from rival_metrics import hits

COLUMNS = ("forecast", "x", "mu_before")
UNRATED = -1e18
AT = (None, 3)


def score(db, col, frm, to):
    out = {}
    cols = f"e.tournament_id, e.team_id, e.actual_place, e.{col}"
    for tid, ts in lab.by_tournament(lab.in_person(db, frm, to, cols)).items():
        if len(ts) < lab.MIN_FIELD or (col != "mu_before" and any(t[2] is None for t in ts)):
            continue
        places = {t[0]: t[1] for t in ts}
        s = {t[0]: UNRATED if t[2] is None else t[2] for t in ts}
        order = lab.ranked(s, s.get, lab.Rng(tid))
        out[tid] = {k: hits(order, places, k) for k in AT}
    return out


def main():
    frm, to = sys.argv[1], sys.argv[2]
    specs = [a.split("=", 1) for a in sys.argv[3:]]
    w = lab.load_weights()
    runs = {}
    for label, spec in specs:
        path, _, col = spec.partition(":")
        col = col or "forecast"
        if col not in COLUMNS:
            sys.exit(f"{label}: no column {col!r}; one of {', '.join(COLUMNS)}")
        runs[label] = score(path, col, frm, to)
    tids = set.intersection(*(set(r) for r in runs.values()))
    print(len(tids), "tournaments; NDCG and NDCG@3 weighted, then unweighted")
    at = lambda r, k: {t: v[k] for t, v in r.items()}
    for label in runs:
        print(label, *(f"{lab.wmean(at(runs[label], k), tids, wt):.4f}" for wt in (w, None) for k in AT))
    first = specs[0][0]
    for label in runs:
        if label != first:
            diffs = (lab.paired(at(runs[first], k), at(runs[label], k), tids, w) for k in AT)
            print(first, "-", label, *(f"{d:+.4f} ± {se:.4f}" for d, se in diffs))


if __name__ == "__main__":
    main()
