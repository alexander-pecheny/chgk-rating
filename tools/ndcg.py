"""Top-weighted forecast quality: NDCG of the forecast order against actual places.

Relevance of a team is 1/actual_place, so the first three places carry most of
the weight and a swap at places 27 and 30 is nearly free.

Tournaments are averaged with the frozen weights of `data/tournament_weights.csv`
-- how strong the ten best lineups of the field were under the shipped model,
less the 10th percentile of that, clipped at zero -- so the high-profile events
count and a school event where no forecast can work does not.  The weights never
move with the model under test.  `--unweighted` is the plain mean.  Usage:

    uv run tools/ndcg.py out/ratings.db [more.db ...] [--from 2024-08-30 --to 2026-08-27] [--k 10]
    uv run tools/ndcg.py --paired a.db b.db          # difference and bootstrap SE

The metric itself lives in `tools/lab.py`, which the other tools share.
"""

import json
import math
import sqlite3

import lab
from lab import Rng, ndcg, ranked, ranks

# The columns of `team_events` a field can be ordered by, after the team id.
COLUMNS = {"forecast": 1, "x": 3, "mu": 4, "s_r": 5}
KEYS = ("forecast", "x", "mu", "s_r", "old")
FIELD_COLS = "e.tournament_id, e.team_id, e.forecast, e.actual_place, e.x, e.mu_before, e.s_r"
OPTIONS = [
    ("dbs", dict(nargs="+")),
    ("--common", dict(action="store_true", help="average over the tournaments all databases forecast")),
    ("--unweighted", dict(action="store_true")),
    ("--paired", dict(action="store_true", help="each db against the first, with bootstrap SE")),
    ("--weights", dict(default=lab.WEIGHTS)),
    ("--spearman", dict(action="store_true")),
    ("--from", dict(dest="frm", default=lab.TEST[0])),
    ("--to", dict(default=lab.TEST[1])),
    ("--k", dict(type=int)),
]


def spearman(order, places, _k=None, values=None):
    """Spearman of the forecast values (ties averaged) against actual place."""
    fr = ranks([-values[t] for t in order])
    ar = ranks([places[t] for t in order])
    n = len(order)
    if n < 2:
        return None
    mf, ma = sum(fr) / n, sum(ar) / n
    cov = sum((a - mf) * (b - ma) for a, b in zip(fr, ar))
    vf = math.sqrt(sum((a - mf) ** 2 for a in fr))
    va = math.sqrt(sum((b - ma) ** 2 for b in ar))
    return cov / (vf * va) if vf and va else None


def old_forecasts():
    """The old system's `predictedPosition` per (tournament, team), from the mirror."""
    q = "select id, team_id, rating from tournament_results where rating like '%predictedPosition%'"
    pred = {}
    for tid, team, js in sqlite3.connect(lab.BUFF).execute(q):
        p = json.loads(js).get("predictedPosition")
        if isinstance(p, (int, float)):
            pred[(tid, team)] = float(p)
    return pred


def run(db, pred, frm, to, k, metric=ndcg):
    """`{column: {tournament: metric}}` for every column a field can be ordered by."""
    acc = {key: {} for key in KEYS}
    for tid, ts in lab.by_tournament(lab.in_person(db, frm, to, FIELD_COLS)).items():
        if len(ts) >= lab.MIN_FIELD:
            score_field(acc, tid, ts, pred, lambda order, places, s: metric(order, places, k, s))
    return acc


def score_field(acc, tid, ts, pred, metric):
    """Every column's score of one field. One generator per field, advanced
    column by column, as `eval` does."""
    rng = Rng(tid)
    places = {t[0]: t[2] for t in ts}
    for key, col in COLUMNS.items():
        if all(t[col] is not None for t in ts):
            s = {t[0]: t[col] for t in ts}
            acc[key][tid] = metric(ranked(s, s.get, rng), places, s)
    old = {t[0]: pred.get((tid, t[0])) for t in ts}
    if all(v is not None for v in old.values()):
        s = {t: -v for t, v in old.items()}
        acc["old"][tid] = metric(ranked(s, s.get, rng), places, s)


def mean_over(d, tids, w=None):
    """The mean and how many tournaments it is over."""
    return lab.wmean(d, tids, w), sum(1 for t in tids if t in d and d[t] is not None)


def print_paired(dbs, runs, w, tids):
    lab.header("| a | b | difference | bootstrap SE | tournaments |", "|---|---|---:|---:|---:|")
    first = dbs[0]
    for db in dbs[1:]:
        d, se = lab.paired(runs[first]["forecast"], runs[db]["forecast"], tids, w)
        print(f"| {first} | {db} | {d:+.4f} | {se:.4f} | {len(tids)} |")


def print_table(runs, w, tids, common):
    lab.header("| db | forecast | x (rating+lineup) | mu only | roster only | old system | tournaments |",
               "|---|---:|---:|---:|---:|---:|---:|")
    for db, r in runs.items():
        sel = tids if common else set(r["forecast"]) | set(r["old"])
        f, x, mu, sr, old = (mean_over(r[key], sel, w) for key in KEYS)
        print(f"| {db} | {f[0]:.4f} | {x[0]:.4f} | {mu[0]:.4f} | {sr[0]:.4f} | {old[0]:.4f} | {f[1]} / old {old[1]} |")


def title(a):
    name = "Spearman" if a.spearman else f"NDCG{'@' + str(a.k) if a.k else ''}, relevance 1/place"
    how = "unweighted" if a.unweighted else "weighted by " + a.weights
    return f"{name}, in-person, {a.frm} to {a.to}, {how}{', common tournaments only' if a.common else ''}\n"


def main():
    a = lab.cli(__doc__, OPTIONS, intermixed=True)
    w = None if a.unweighted else lab.load_weights(a.weights)
    print(title(a))
    pred = old_forecasts()
    runs = {db: run(db, pred, a.frm, a.to, a.k, spearman if a.spearman else ndcg) for db in a.dbs}
    # --common: every column averaged over the tournaments all databases forecast.
    tids = set.intersection(*(set(r["forecast"]) for r in runs.values()))
    if a.paired:
        print_paired(a.dbs, runs, w, tids)
    else:
        print_table(runs, w, tids, a.common)


if __name__ == "__main__":
    main()
