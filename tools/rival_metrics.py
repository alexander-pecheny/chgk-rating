"""Forecast quality of several systems on the same in-person fields, by the
measures of /method/ and those the London Elo (https://elo-chgk.uk) reports:

- NDCG, NDCG@3 and NDCG@10 of /method/: the team at forecast position i has
  relevance 1/(|i - place| + 1), a shared place standing at the centre of its
  range, discounted by log2(i + 1), over the same sum for the actual order;
- Pearson's r between the forecast place and the actual place;
- the pairwise score over every pair of teams in the field, and over the pairs
  among the best 10% of the field by actual result (at least five teams,
  ties at the cut kept). A pair scores 1 when the forecast orders it as the result
  does, a tie included, 0.5 when only one of the two is a tie, 0 otherwise; the
  score is the share of the maximum.

A spec `label=path:x` orders the field by the `x` column instead of
`forecast`; for `ext_elo` that is the expected score on a fixed pack, which
reads nothing of the tournament itself.

Every system is scored on the teams all of them forecast, in the tournaments all
of them cover, with places from the first database. Tournaments are weighted as
in `tools/ndcg.py`; `--unweighted` is the plain mean. `--old` adds the old
system's `predictedPosition` as a row of its own, on the teams it forecast.

    uv run tools/rival_metrics.py "ours=out/ratings.db" "elo=out/ext/elo.db" ... [--paired ours]
"""

import json
import math
import sqlite3

import lab
from lab import Rng, ranked

COLUMNS = ("forecast", "x")
OLD = "old system"
TOP_SHARE = 0.10
TOP_MIN = 5
METRICS = ("ndcg", "ndcg3", "ndcg10", "pearson", "pairwise", "pairwise_top")
TITLES = ("NDCG", "NDCG@3", "NDCG@10", "Pearson r", "pair", "pair@10%")
CUTS = {"ndcg": None, "ndcg3": 3, "ndcg10": 10}
OPTIONS = [
    ("specs", dict(nargs="+", metavar="label=path")),
    ("--unweighted", dict(action="store_true")),
    ("--weights", dict(default=lab.WEIGHTS)),
    ("--paired", dict(metavar="LABEL")),
    ("--old", dict(action="store_true", help="add the old system's forecast")),
    ("--from", dict(dest="frm", default=lab.TEST[0])),
    ("--to", dict(default=lab.TEST[1])),
    ("--json", dict(action="store_true", help="print the rows and their paired differences as JSON")),
]


def hits(order, places, k):
    """NDCG of /method/: relevance falls with the distance to the actual place."""
    k = min(k or len(order), len(order))

    def dcg(o):
        return sum(1 / ((abs(pos - places[t]) + 1) * math.log2(pos + 1)) for pos, t in enumerate(o[:k], 1))

    return dcg(order) / dcg(sorted(order, key=places.get))


def pearson(xs, ys):
    n = len(xs)
    mx, my = sum(xs) / n, sum(ys) / n
    sxy = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    sxx = sum((x - mx) ** 2 for x in xs)
    syy = sum((y - my) ** 2 for y in ys)
    return sxy / math.sqrt(sxx * syy) if sxx > 0 and syy > 0 else None


def sign(x):
    return (x > 0) - (x < 0)


def pairwise(teams, value, places):
    """Share of the maximum: 1 per pair ordered as the result, 0.5 when exactly one side ties."""
    got, n = 0.0, 0
    for i, a in enumerate(teams):
        for b in teams[i + 1:]:
            f = sign(value[a] - value[b])
            r = sign(places[b] - places[a])
            got += 1.0 if f == r else 0.5 if f == 0 or r == 0 else 0.0
            n += 1
    return got / n if n else None


def top(teams, places):
    """The best tenth of the field by actual place, at least `TOP_MIN` teams, ties at the cut kept."""
    k = max(TOP_MIN, math.ceil(TOP_SHARE * len(teams)))
    cut = sorted(places[t] for t in teams)[k - 1]
    return [t for t in teams if places[t] <= cut]


def score(tid, teams, value, places):
    """Every measure for one field; `value` is higher-is-better."""
    order = ranked(teams, value.get, Rng(tid))
    fplace = lab.ranks([-value[t] for t in teams])
    return {
        **{m: hits(order, places, k) for m, k in CUTS.items()},
        "pearson": pearson(fplace, [places[t] for t in teams]),
        "pairwise": pairwise(teams, value, places),
        "pairwise_top": pairwise(top(teams, places), value, places),
    }


def load(spec, frm, to):
    """`{tournament: {team: (place, forecast)}}` of placed in-person results."""
    path, _, col = spec.partition(":")
    col = col or "forecast"
    assert col in COLUMNS, col
    out = {}
    for tid, team, place, f in lab.in_person(path, frm, to, f"e.tournament_id, e.team_id, e.actual_place, e.{col}"):
        if f is not None:
            out.setdefault(tid, {})[team] = (place, f)
    return out


def old_forecasts(tids):
    q = "select id, team_id, rating from tournament_results where rating like '%predictedPosition%'"
    import json
    pred = {}
    for tid, team, js in sqlite3.connect(lab.BUFF).execute(q):
        if tid in tids:
            p = json.loads(js).get("predictedPosition")
            if isinstance(p, (int, float)):
                pred.setdefault(tid, {})[team] = -float(p)
    return pred


def main():
    a = lab.cli(__doc__, OPTIONS, intermixed=True)
    w = None if a.unweighted else lab.load_weights(a.weights)
    specs = [s.split("=", 1) for s in a.specs]
    data = {label: load(path, a.frm, a.to) for label, path in specs}
    first = data[specs[0][0]]
    tids = set.intersection(*(set(d) for d in data.values()))
    old = old_forecasts(tids) if a.old else {}
    labels = [label for label, _ in specs] + ([OLD] if a.old else [])
    res = {label: {m: {} for m in METRICS} for label in labels}
    used = set()
    for tid in sorted(tids):
        teams = set(first[tid])
        for d in data.values():
            teams &= set(d[tid])
        if a.old:
            teams &= set(old.get(tid, {}))
        teams = sorted(teams)
        if len(teams) < lab.MIN_FIELD:
            continue
        used.add(tid)
        places = {t: first[tid][t][0] for t in teams}
        for label in labels:
            value = {t: old[tid][t] for t in teams} if label == OLD else {t: data[label][tid][t][1] for t in teams}
            for m, v in score(tid, teams, value, places).items():
                res[label][m][tid] = v
    if a.json:
        print(json.dumps(as_json(a, w, labels, res, used), ensure_ascii=False))
        return
    how = "unweighted" if w is None else "weighted by " + a.weights
    print(f"In-person, {a.frm} to {a.to}, {how}, {len(used)} tournaments every system forecasts\n")
    print("| system | " + " | ".join(TITLES) + " |")
    print("|---|" + "---:|" * len(TITLES))
    for label in labels:
        cells = [f"{lab.wmean(res[label][m], used, w):.4f}" for m in METRICS]
        print(f"| {label} | " + " | ".join(cells) + " |")
    if a.paired:
        print(f"\nDifference against {a.paired}, paired bootstrap SE over tournaments\n")
        print("| system | " + " | ".join(TITLES) + " |")
        print("|---|" + "---:|" * len(TITLES))
        for label in labels:
            if label == a.paired:
                continue
            cells = []
            for m in METRICS:
                keys = [t for t in used if res[label][m].get(t) is not None and res[a.paired][m].get(t) is not None]
                d, se = lab.paired(res[label][m], res[a.paired][m], keys, w)
                cells.append(f"{d:+.4f} ± {se:.4f}")
            print(f"| {label} | " + " | ".join(cells) + " |")


def as_json(a, w, labels, res, used):
    """The table for /method/'s metrics block: each row's values and, against
    every other row, its differences with their paired bootstrap SEs."""
    done = {}

    def vs(label, base):
        # b against a is a against b with the sign flipped: one bootstrap per pair.
        if (base, label) in done:
            d = done[(base, label)]
            return {"diff": [-x for x in d["diff"]], "se": d["se"]}
        pairs = [lab.paired(res[label][m], res[base][m],
                            [t for t in used if res[label][m].get(t) is not None and res[base][m].get(t) is not None], w)
                 for m in METRICS]
        done[(label, base)] = {"diff": [d for d, _ in pairs], "se": [se for _, se in pairs]}
        return done[(label, base)]
    rows = [{"label": label, "value": [lab.wmean(res[label][m], used, w) for m in METRICS],
             "vs": {base: vs(label, base) for base in labels if base != label}} for label in labels]
    return {"from": a.frm, "to": a.to, "weighted": w is not None, "n": len(used), "rows": rows}


if __name__ == "__main__":
    main()
