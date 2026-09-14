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

import math
import random
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ndcg as N  # noqa: E402
from q_mech import TUNE, base_rows, wcorr  # noqa: E402
from q_mech4 import add_field, add_sd, lever2, lever3, renorm  # noqa: E402


def fields(rows):
    by_t = {}
    for r in rows:
        by_t.setdefault(r["tid"], []).append(r)
    return by_t


def ndcg_of(order, places, k=None):
    rel = [1.0 / places[t] for t in order]
    k = len(rel) if k is None else min(k, len(rel))
    dcg = sum(x / math.log2(i + 2) for i, x in enumerate(rel[:k]))
    best = sorted(rel, reverse=True)
    idcg = sum(x / math.log2(i + 2) for i, x in enumerate(best[:k]))
    return dcg / idcg if idcg > 0 else float("nan")


def score_fields(by_t, w, key):
    """Weighted mean NDCG and NDCG@3 over the fields, ordering each by `key`."""
    num = num3 = den = 0.0
    for tid, rs in by_t.items():
        if len(rs) < 5:
            continue
        wt = w.get(tid, 0.0)
        if wt <= 0:
            continue
        order = list(rs)
        N.Rng(tid).shuffle(order)
        order.sort(key=lambda r: -key(r))
        places = {id(r): r["place"] for r in order}
        ids = [id(r) for r in order]
        num += wt * ndcg_of(ids, places)
        num3 += wt * ndcg_of(ids, places, 3)
        den += wt
    return num / den, num3 / den


def within(db, rows):
    """How much of each feature lives inside a field, and the best a linear
    bonus on it could do."""
    add_field(db, rows)
    add_sd(rows)
    lever2(db, rows)
    lever3(db, rows)
    print("\n## Where each feature lives, and the best a bonus on it could buy\n")
    w = N.load_weights(N.WEIGHTS)
    by_t = {t: rs for t, rs in fields(rows).items() if all(r["fc"] is not None for r in rs)}
    base, base3 = score_fields(by_t, w, lambda r: r["fc"])
    print(f"The mean ordering scores {base:.4f} / @3 {base3:.4f} on these fields.\n")
    print("| feature | share of variance inside a field | best bonus | NDCG | @3 |")
    print("|---|---:|---:|---:|---:|")
    feats = [
        ("the team's own town is the tournament's", "home_team", (-400, -200, -100, -50, 50, 100, 200)),
        ("share of the lineup local to the town", "home_share", (-400, -200, -100, -50, 50, 100, 200)),
        ("in-person tournaments per player, last 7 days", "load7", (-200, -100, -50, 50, 100)),
        ("the same, last 30 days", "load30", (-100, -50, -25, 25, 50)),
        ("sd of the lineup's own strength", "sd", (-2, -1, -0.5, 0.5, 1, 2)),
    ]
    for lab, k, grid in feats:
        vals = [r for r in rows if r.get(k) is not None]
        if not vals:
            continue
        tot = statistics.pvariance([r[k] for r in vals])
        inside = 0.0
        n = 0
        for rs in fields(vals).values():
            if len(rs) < 2:
                continue
            m = statistics.mean(r[k] for r in rs)
            inside += sum((r[k] - m) ** 2 for r in rs)
            n += len(rs)
        share = (inside / n) / tot if tot > 0 and n else float("nan")
        best = (base, base3, 0.0)
        for c in grid:
            # `sd` is on the theta scale; the others are indicators or counts.
            s, s3 = score_fields(
                by_t, w, lambda r, c=c, k=k: r["fc"] + c * (r.get(k) or 0.0)
            )
            if s > best[0]:
                best = (s, s3, c)
        print(f"| {lab} | {share:.2f} | {best[2]:+g} | {best[0]:.4f} | {best[1]:.4f} |")


def rerank(db, rows):
    """The metric-targeted ordering, computed offline at a range of sd scales."""
    add_field(db, rows)
    add_sd(rows)
    w = N.load_weights(N.WEIGHTS)
    by_t = {t: rs for t, rs in fields(rows).items() if all(r["fc"] is not None for r in rs)}
    # The sd is propagated on the theta scale; the forecast is display points.
    scale = 2454.8828
    print("## The metric-targeted ordering, offline, at a range of sd scales\n")
    print("`lambda` multiplies every lineup's sd before the draws; 0 is the mean")
    print("ordering the model ships. 2000 draws, seeded from the tournament id.\n")
    print("| lambda | NDCG | NDCG@3 | fields whose order changed |")
    print("|---|---:|---:|---:|")
    for lam in (0.0, 0.1, 0.25, 0.5, 1.0, 2.0):
        num = num3 = den = 0.0
        moved = 0
        for tid, rs in by_t.items():
            if len(rs) < 5:
                continue
            wt = w.get(tid, 0.0)
            if wt <= 0:
                continue
            rng = random.Random(tid)
            n = len(rs)
            acc = [0.0] * n
            draws = 2000 if lam > 0 else 1
            for _ in range(draws):
                x = sorted(
                    range(n),
                    key=lambda i: -(
                        rs[i]["fc"]
                        + lam * scale * (rs[i]["sd"] or 0.0) * rng.gauss(0, 1)
                    ),
                )
                for place, i in enumerate(x):
                    acc[i] += 1.0 / (place + 1)
            key = [a / draws + 1e-12 * rs[i]["fc"] for i, a in enumerate(acc)]
            order = list(range(n))
            N.Rng(tid).shuffle(order)
            order.sort(key=lambda i: -key[i])
            mean_order = list(range(n))
            N.Rng(tid).shuffle(mean_order)
            mean_order.sort(key=lambda i: -rs[i]["fc"])
            moved += order != mean_order
            places = {i: rs[i]["place"] for i in range(n)}
            num += wt * ndcg_of(order, places)
            num3 += wt * ndcg_of(order, places, 3)
            den += wt
        print(f"| {lam} | {num / den:.4f} | {num3 / den:.4f} | {moved} |")


def main():
    args = sys.argv[1:]
    db = args[0]
    what = args[args.index("--what") + 1] if "--what" in args else "within"
    rows = renorm(base_rows(db, *TUNE))
    print(f"# Round-4 diagnosis, {TUNE[0]} to {TUNE[1]}\n")
    {"within": within, "rerank": rerank}[what](db, rows)


if __name__ == "__main__":
    main()
