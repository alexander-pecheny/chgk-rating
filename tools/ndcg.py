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
"""

import csv
import json
import math
import random
import sqlite3
import sys

WEIGHTS = "data/tournament_weights.csv"


M64 = (1 << 64) - 1
GOLDEN = 0x9E3779B97F4A7C15


class Rng:
    """`util::Rng` of the Rust side, so both implementations break ties alike.

    Forecasts do tie -- two teams with the same lineup strength -- and the rows
    are stored in finishing order, so the tie must not be broken by row order.
    """

    def __init__(self, seed):
        self.s = (seed * GOLDEN | 1) & M64

    def next_u64(self):
        self.s = (self.s + GOLDEN) & M64
        z = self.s
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
        return z ^ (z >> 31)

    def shuffle(self, v):
        for i in range(len(v) - 1, 0, -1):
            j = self.next_u64() % (i + 1)
            v[i], v[j] = v[j], v[i]


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
        rng = Rng(tid)
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


def load_weights(path):
    """tournament_id -> weight. A tournament the table does not name weighs nothing."""
    with open(path, newline="", encoding="utf-8") as f:
        return {int(r["tournament_id"]): float(r["weight"]) for r in csv.DictReader(f)}


def mean_over(d, tids, w=None):
    v = [(t, d[t]) for t in tids if t in d and d[t] is not None]
    if not v:
        return (float("nan"), 0)
    if w is None:
        return (sum(x for _, x in v) / len(v), len(v))
    den = sum(w.get(t, 0.0) for t, _ in v)
    num = sum(w.get(t, 0.0) * x for t, x in v)
    return (num / den if den > 0 else float("nan"), len(v))


def paired(a, b, w, tids, key="forecast", n=2000):
    """Difference of the weighted means and its bootstrap standard error over tournaments."""
    pairs = [(t, a[key][t], b[key][t]) for t in sorted(tids)]
    rng = random.Random(20260914)

    def diff(sample):
        den = sum(w.get(t, 0.0) if w else 1.0 for t, _, _ in sample)
        num = sum((w.get(t, 0.0) if w else 1.0) * (x - y) for t, x, y in sample)
        return num / den if den > 0 else float("nan")

    point = diff(pairs)
    draws = [diff([pairs[rng.randrange(len(pairs))] for _ in pairs]) for _ in range(n)]
    m = sum(draws) / len(draws)
    se = (sum((d - m) ** 2 for d in draws) / len(draws)) ** 0.5
    return point, se


def main():
    args = sys.argv[1:]
    frm, to, k, dbs, common, metric = "2024-08-30", "2026-08-27", None, [], False, ndcg
    wpath, weighted, do_paired = WEIGHTS, True, False
    i = 0
    while i < len(args):
        if args[i] == "--common":
            common = True
            i += 1
        elif args[i] == "--unweighted":
            weighted = False
            i += 1
        elif args[i] == "--paired":
            do_paired = True
            i += 1
        elif args[i] == "--weights":
            wpath = args[i + 1]
            i += 2
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
    w = load_weights(wpath) if weighted else None
    name = "Spearman" if metric is spearman else f"NDCG{'@' + str(k) if k else ''}, relevance 1/place"
    how = "weighted by " + wpath if weighted else "unweighted"
    print(f"{name}, in-person, {frm} to {to}, {how}"
          f"{', common tournaments only' if common else ''}\n")
    runs = {db: run(db, pred, frm, to, k, metric) for db in dbs}
    # --common: every column averaged over the tournaments all databases forecast.
    tids = None
    for r in runs.values():
        tids = set(r["forecast"]) if tids is None else tids & set(r["forecast"])
    if do_paired:
        print("| a | b | difference | bootstrap SE | tournaments |")
        print("|---|---|---:|---:|---:|")
        first = dbs[0]
        for db in dbs[1:]:
            d, se = paired(runs[first], runs[db], w, tids)
            print(f"| {first} | {db} | {d:+.4f} | {se:.4f} | {len(tids)} |")
        return
    print("| db | forecast | x (rating+lineup) | mu only | roster only | old system | tournaments |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    for db, r in runs.items():
        sel = tids if common else set(r["forecast"]) | set(r["old"])
        f, x, mu, sr, old = (mean_over(r[key], sel, w) for key in ("forecast", "x", "mu", "s_r", "old"))
        print(f"| {db} | {f[0]:.4f} | {x[0]:.4f} | {mu[0]:.4f} | {sr[0]:.4f} | {old[0]:.4f} | {f[1]} / old {old[1]} |")


if __name__ == "__main__":
    main()
