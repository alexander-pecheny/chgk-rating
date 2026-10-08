"""Why a residual signal does not become a gain in the metric.

Four of round 4's six levers have a residual behind them and none of them
improved the forecast.  Two things can explain that, and they are different:
the feature may be nearly **constant inside a field**, in which case no
coefficient on it can reorder anything however real the effect is; or it may
vary inside a field and simply be worth less than the noise it adds.

    uv run tools/q_diag4.py out/q4/base.db --what within
    uv run tools/q_diag4.py out/q4/base.db --what rerank

`within` reports, for each feature, the share of its variance that lies between
fields rather than inside them, and the best NDCG any linear bonus on it could
buy, found by a sweep over the coefficient on the tune window.

`rerank` re-orders every tune field by `E[1/place]` offline, at a range of
multipliers on the lineups' sd, and reports the NDCG of each.  Multiplier 0 is
the mean ordering; the question is whether the curve has an interior maximum
anywhere, and it does not.
"""

import random
import statistics

import lab
from q_mech import SCALE, TUNE, base_rows, by_field
from q_mech4 import add_field, add_sd, lever2, lever3, renorm

TOP = lab.TOP
# The coefficients each bonus is swept over. `sd` is on the theta scale; the
# others are indicators or counts.
HOME_GRID = (-400, -200, -100, -50, 50, 100, 200)
LOAD7_GRID = (-200, -100, -50, 50, 100)
LOAD30_GRID = (-100, -50, -25, 25, 50)
SD_GRID = (-2, -1, -0.5, 0.5, 1, 2)
FEATURES = [
    ("the team's own town is the tournament's", "home_team", HOME_GRID),
    ("share of the lineup local to the town", "home_share", HOME_GRID),
    ("in-person tournaments per player, last 7 days", "load7", LOAD7_GRID),
    ("the same, last 30 days", "load30", LOAD30_GRID),
    ("sd of the lineup's own strength", "sd", SD_GRID),
]
LAMBDAS = (0.0, 0.1, 0.25, 0.5, 1.0, 2.0)
DRAWS = 2000
# A vanishing pull towards the mean forecast, so equal expectations keep its order.
TIE = 1e-12
OPTIONS = [("db", dict()), ("--what", dict(choices=("within", "rerank"), default="within"))]


def forecast_fields(rows):
    """The fields every row of which has a forecast."""
    return {t: rs for t, rs in by_field(rows).items() if all(r["fc"] is not None for r in rs)}


def weighted_fields(by_t, w):
    """`(tournament, rows, weight)` of every weighted field of five or more."""
    return [(tid, rs, w[tid]) for tid, rs in by_t.items() if len(rs) >= lab.MIN_FIELD and w.get(tid, 0.0) > 0]


def score_order(order, places):
    return lab.ndcg(order, places), lab.ndcg(order, places, TOP)


def score_fields(by_t, w, key):
    """Weighted mean NDCG and NDCG@3 over the fields, ordering each by `key`."""
    num = num3 = den = 0.0
    for tid, rs, wt in weighted_fields(by_t, w):
        order = lab.ranked(rs, key, lab.Rng(tid))
        nd, nd3 = score_order([id(r) for r in order], {id(r): r["place"] for r in order})
        num, num3, den = num + wt * nd, num3 + wt * nd3, den + wt
    return num / den, num3 / den


def inside_share(rows, k):
    """The share of a feature's variance that lies inside the fields."""
    vals = [r for r in rows if r.get(k) is not None]
    if not vals:
        return None
    tot = statistics.pvariance([r[k] for r in vals])
    inside, n = 0.0, 0
    for rs in (rs for rs in by_field(vals).values() if len(rs) >= 2):
        m = statistics.mean(r[k] for r in rs)
        inside, n = inside + sum((r[k] - m) ** 2 for r in rs), n + len(rs)
    return (inside / n) / tot if tot > 0 and n else lab.NAN


def best_bonus(by_t, w, k, grid, base):
    """The best `(NDCG, NDCG@3, coefficient)` of a linear bonus on `k`."""
    best = base
    for c in grid:
        s, s3 = score_fields(by_t, w, lambda r, c=c: r["fc"] + c * (r.get(k) or 0.0))
        if s > best[0]:
            best = (s, s3, c)
    return best


def within(db, rows):
    """How much of each feature lives inside a field, and the best a linear
    bonus on it could do."""
    add_field(db, rows)
    add_sd(rows)
    lever2(db, rows)
    lever3(db, rows)
    print("\n## Where each feature lives, and the best a bonus on it could buy\n")
    w = lab.load_weights()
    by_t = forecast_fields(rows)
    base, base3 = score_fields(by_t, w, lambda r: r["fc"])
    print(f"The mean ordering scores {base:.4f} / @3 {base3:.4f} on these fields.\n")
    lab.header("| feature | share of variance inside a field | best bonus | NDCG | @3 |", "|---|---:|---:|---:|---:|")
    for label, k, grid in FEATURES:
        share = inside_share(rows, k)
        if share is not None:
            best = best_bonus(by_t, w, k, grid, (base, base3, 0.0))
            print(f"| {label} | {share:.2f} | {best[2]:+g} | {best[0]:.4f} | {best[1]:.4f} |")


def expected_relevance(rs, lam, rng):
    """Each lineup's mean `1/place` over draws of its strength at `lam` times its sd."""
    n = len(rs)
    acc, draws = [0.0] * n, DRAWS if lam > 0 else 1
    for _ in range(draws):
        x = sorted(range(n), key=lambda i: -(rs[i]["fc"] + lam * SCALE * (rs[i]["sd"] or 0.0) * rng.gauss(0, 1)))
        for place, i in enumerate(x):
            acc[i] += 1.0 / (place + 1)
    return [a / draws + TIE * rs[i]["fc"] for i, a in enumerate(acc)]


def rerank_field(tid, rs, lam):
    """The field ordered by expected relevance: NDCG, NDCG@3 and whether it moved."""
    key = expected_relevance(rs, lam, random.Random(tid))
    order = lab.ranked(range(len(rs)), key.__getitem__, lab.Rng(tid))
    mean_order = lab.ranked(range(len(rs)), lambda i: rs[i]["fc"], lab.Rng(tid))
    return (*score_order(order, {i: r["place"] for i, r in enumerate(rs)}), order != mean_order)


def rerank(db, rows):
    """The metric-targeted ordering, computed offline at a range of sd scales."""
    add_field(db, rows)
    add_sd(rows)
    fs = weighted_fields(forecast_fields(rows), lab.load_weights())
    print("## The metric-targeted ordering, offline, at a range of sd scales\n")
    print("`lambda` multiplies every lineup's sd before the draws; 0 is the mean")
    print("ordering the model ships. 2000 draws, seeded from the tournament id.\n")
    lab.header("| lambda | NDCG | NDCG@3 | fields whose order changed |", "|---|---:|---:|---:|")
    for lam in LAMBDAS:
        num = num3 = den = 0.0
        moved = 0
        for tid, rs, wt in fs:
            nd, nd3, m = rerank_field(tid, rs, lam)
            num, num3, den, moved = num + wt * nd, num3 + wt * nd3, den + wt, moved + m
        print(f"| {lam} | {num / den:.4f} | {num3 / den:.4f} | {moved} |")


def main():
    a = lab.cli(__doc__, OPTIONS)
    rows = renorm(base_rows(a.db, *TUNE))
    print(f"# Round-4 diagnosis, {TUNE[0]} to {TUNE[1]}\n")
    {"within": within, "rerank": rerank}[a.what](a.db, rows)


if __name__ == "__main__":
    main()
