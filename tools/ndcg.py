"""Top-weighted forecast quality: NDCG of the forecast order against actual places.

Relevance of a team is 1/actual_place, so the first three places carry most of
the weight and a swap at places 27 and 30 is nearly free.  Usage:

    uv run tools/ndcg.py out/ratings.db [more.db ...] [--from 2024-08-30 --to 2026-08-27] [--k 10]
"""

import json
import math
import random
import sqlite3
import sys


def ndcg(order, places, k=None, values=None):
    """`order`: teams sorted by forecast, best first; `places`: actual place per team."""
    rel = [1.0 / places[t] for t in order]
    k = k or len(rel)
    dcg = sum(r / math.log2(i + 2) for i, r in enumerate(rel[:k]))
    idcg = sum(r / math.log2(i + 2) for i, r in enumerate(sorted(rel, reverse=True)[:k]))
    return dcg / idcg if idcg > 0 else None


def ranks(vals):
    order = sorted(range(len(vals)), key=lambda i: vals[i])
    r = [0.0] * len(vals)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and vals[order[j + 1]] == vals[order[i]]:
            j += 1
        for t in order[i : j + 1]:
            r[t] = (i + j) / 2 + 1
        i = j + 1
    return r


def spearman(order, places, _k=None, values=None):
    """Spearman of the forecast values (ties averaged) against actual place."""
    fr = ranks([-values[t] for t in order])
    ar = ranks([places[t] for t in order])
    n = len(order)
    if n < 2:
        return None
    mf, ma = sum(fr) / n, sum(ar) / n
    cov = sum((a - mf) * (b - ma) for a, b in zip(fr, ar))
    vf = sum((a - mf) ** 2 for a in fr) ** 0.5
    va = sum((b - ma) ** 2 for b in ar) ** 0.5
    return cov / (vf * va) if vf and va else None


def old_forecasts():
    b = sqlite3.connect("buff.db")
    pred = {}
    for tid, team, js in b.execute(
        "select id, team_id, rating from tournament_results where rating like '%predictedPosition%'"
    ):
        p = json.loads(js).get("predictedPosition")
        if isinstance(p, (int, float)):
            pred[(tid, team)] = float(p)
    return pred


def run(db, pred, frm, to, k, metric=ndcg):
    c = sqlite3.connect(db)
    rows = c.execute(
        "select e.tournament_id, e.team_id, e.forecast, e.actual_place, e.x, e.mu_before, e.s_r "
        "from team_events e join tournament_metrics m on m.tournament_id = e.tournament_id "
        "where m.type = 'in-person' and m.date >= ? and m.date < ? and e.actual_place > 0 "
        "order by e.tournament_id",
        (frm, to),
    ).fetchall()
    by_t = {}
    for row in rows:
        by_t.setdefault(row[0], []).append(row[1:])
    acc = {"forecast": {}, "x": {}, "mu": {}, "s_r": {}, "old": {}}
    for tid, ts in by_t.items():
        if len(ts) < 5:
            continue
        places = {t[0]: t[2] for t in ts}
        # Rows are stored in finishing order, so ties must not be broken by row order.
        rng = random.Random(tid)
        for key, col in (("forecast", 1), ("x", 3), ("mu", 4), ("s_r", 5)):
            if any(t[col] is None for t in ts):
                continue
            strength = {t[0]: t[col] for t in ts}
            order = list(strength)
            rng.shuffle(order)
            order.sort(key=lambda t: -strength[t])
            acc[key][tid] = metric(order, places, k, strength)
        strength = {t[0]: pred.get((tid, t[0])) for t in ts}
        if all(v is not None for v in strength.values()):
            strength = {t: -v for t, v in strength.items()}
            order = list(strength)
            rng.shuffle(order)
            order.sort(key=lambda t: -strength[t])
            acc["old"][tid] = metric(order, places, k, strength)
    return acc


def mean_over(d, tids):
    v = [d[t] for t in tids if t in d and d[t] is not None]
    return (sum(v) / len(v) if v else float("nan"), len(v))


def main():
    args = sys.argv[1:]
    frm, to, k, dbs, common, metric = "2024-08-30", "2026-08-27", None, [], False, ndcg
    i = 0
    while i < len(args):
        if args[i] == "--common":
            common = True
            i += 1
        elif args[i] == "--spearman":
            metric = spearman
            i += 1
        elif args[i] in ("--from", "--to", "--k"):
            val = args[i + 1]
            frm, to, k = (val, to, k) if args[i] == "--from" else (frm, val, k) if args[i] == "--to" else (frm, to, int(val))
            i += 2
        else:
            dbs.append(args[i])
            i += 1
    pred = old_forecasts()
    name = "Spearman" if metric is spearman else f"NDCG{'@' + str(k) if k else ''}, relevance 1/place"
    print(f"{name}, in-person, {frm} to {to}{', common tournaments only' if common else ''}\n")
    runs = {db: run(db, pred, frm, to, k, metric) for db in dbs}
    # --common: every column averaged over the tournaments all databases forecast.
    tids = None
    for r in runs.values():
        tids = set(r["forecast"]) if tids is None else tids & set(r["forecast"])
    print("| db | forecast | x (rating+lineup) | mu only | roster only | old system | tournaments |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    for db, r in runs.items():
        sel = tids if common else set(r["forecast"]) | set(r["old"])
        f, x, mu, sr, old = (mean_over(r[key], sel) for key in ("forecast", "x", "mu", "s_r", "old"))
        print(f"| {db} | {f[0]:.4f} | {x[0]:.4f} | {mu[0]:.4f} | {sr[0]:.4f} | {old[0]:.4f} | {f[1]} / old {old[1]} |")


if __name__ == "__main__":
    main()
