"""Forecast quality by tournament class, every field counted once.

The weighted metric of /method/ gives the weak fields almost nothing, so a
school or novice field never moves it. This scores each field by Pearson's r
between the forecast order and the places, in person and synchronous, and
averages it by the age class of `tools/tournament_class.py`, unweighted.

    uv run tools/class_metrics.py base=out/prestige/base.db new=out/prestige/my50.db --classes out/prestige/classes.tsv [--paired base]
"""

import csv
import math
import random
from collections import defaultdict

import lab
from rival_metrics import pearson

OPTIONS = [
    ("specs", dict(nargs="+", metavar="label=path")),
    ("--classes", dict(default="out/prestige/classes.tsv")),
    ("--paired", dict(metavar="LABEL")),
    ("--boot", dict(type=int, default=1000)),
]
MIN_FIELD = 8
AGES = ("school", "student", "novice", "young", "open")
YOUNG = ("school", "student", "novice")


def load(path, classes, frm, to):
    """`{tournament: {team: (forecast, place)}}`."""
    out = defaultdict(dict)
    q = "select tournament_id, team_id, forecast, actual_place from team_events where actual_place is not null and date >= ? and date < ?"
    with lab.connect(path) as d:
        for tid, team, f, place in d.execute(q, (frm, to)):
            if tid in classes and f is not None and math.isfinite(f):
                out[tid][team] = (f, place)
    return out


def scores(data, keep):
    """Pearson per field over the teams `keep(tid)` returns."""
    out = {}
    for tid, rows in data.items():
        teams = keep(tid)
        if len(teams) >= MIN_FIELD:
            r = pearson(lab.ranks([-rows[t][0] for t in teams]), [rows[t][1] for t in teams])
            if r is not None:
                out[tid] = r
    return out


def main(a):
    classes = {int(r["id"]): r for r in csv.DictReader(open(a.classes), delimiter="\t")}
    specs = [s.split("=", 1) for s in a.specs]
    for window, (frm, to) in lab.WINDOWS.items():
        data = {label: load(path, classes, frm, to) for label, path in specs}
        common = lambda tid: sorted(set.intersection(*(set(d.get(tid, {})) for d in data.values())))
        per = {label: scores(d, common) for label, d in data.items()}
        print(f"\n{window}, {frm} to {to}: mean per-field Pearson r, unweighted\n")
        groups = [(fmt, age) for fmt in ("in-person", "sync") for age in AGES]
        print("| system | " + " | ".join(f"{fmt} {age}" for fmt, age in groups) + " |")
        print("|---|" + "---:|" * len(groups))
        ids = {g: [t for t in per[specs[0][0]] if classes[t]["fmt"] == g[0] and (classes[t]["age"] in YOUNG if g[1] == "young" else classes[t]["age"] == g[1])] for g in groups}
        for label, _ in specs:
            cells = [f"{sum(per[label][t] for t in ids[g]) / len(ids[g]):.4f}" if ids[g] else "" for g in groups]
            print(f"| {label} | " + " | ".join(cells) + " |")
        print("| fields | " + " | ".join(str(len(ids[g])) for g in groups) + " |")
        if a.paired:
            print(f"\nDifference against {a.paired}, bootstrap SE over fields\n")
            print("| system | " + " | ".join(f"{fmt} {age}" for fmt, age in groups) + " |")
            print("|---|" + "---:|" * len(groups))
            rng = random.Random(1)
            for label, _ in specs:
                if label == a.paired:
                    continue
                cells = []
                for g in groups:
                    d = [per[label][t] - per[a.paired][t] for t in ids[g]]
                    if not d:
                        cells.append("")
                        continue
                    boots = [sum(rng.choice(d) for _ in d) / len(d) for _ in range(a.boot)]
                    m = sum(boots) / len(boots)
                    se = math.sqrt(sum((b - m) ** 2 for b in boots) / (len(boots) - 1))
                    cells.append(f"{sum(d) / len(d):+.4f} ± {se:.4f}")
                print(f"| {label} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main(lab.cli(__doc__, OPTIONS))
