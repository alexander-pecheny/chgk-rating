"""Where another system's forecast beats ours, field by field.

The absolute residual says the loss is concentrated in fields no forecast can
order.  What matters for plan 5 is the *difference*: the fields rating.chgk.fun
or chgk.quest orders better than we do, and what their lineups have in common.

    uv run tools/q_vs.py out/q2/base.db out/ext/fun.db [--test] [--top 20]
"""

import sqlite3
import statistics
from math import inf

import lab
import q_diag as D

NAN3 = (lab.NAN,) * 3
# Cohorts of a field by its lineups' median in-person games, and by its size.
GAMES = ((10, "median under 10 in-person games"), (30, "median 10-29"), (inf, "median 30+"))
SIZES = ((20, "under 20 teams"), (50, "20-49 teams"), (inf, "50+ teams"))
OPTIONS = [
    ("ours", dict()),
    ("theirs", dict()),
    ("--test", dict(action="store_true", help="the test window instead of tune")),
    ("--top", dict(type=int, default=20, help="how many fields each way")),
]


def per_field(db, frm, to):
    """`{tournament: (NDCG, NDCG@3, date, teams)}`."""
    out = {}
    for tid, v in D.fields(db, frm, to).items():
        order, places, _, nd, _ = D.scored(v, tid)
        out[tid] = (nd, D.ndcg_at(order, places), v["date"], len(order))
    return out


def lineup_stats(db, tids):
    """Median in-person games, mean sigma and seats of the lineups of each field."""
    st = lab.player_state(sqlite3.connect(db))
    buff = sqlite3.connect(lab.BUFF)
    out = {}
    for tid in tids:
        s = st.get(tid, {})
        v = [s[p] for ms in lab.lineups(buff, tid).values() for p in ms if p in s]
        if v:
            out[tid] = (statistics.median(x[3] for x in v), statistics.mean(x[2] for x in v), len(v))
    return out


def print_fields(a, b, w, ls, label, part, top):
    names = lab.names([t for _, t in part], "tournaments", "name")
    print(f"\n## The {top} fields {label}\n")
    lab.header("| tournament | date | teams | weight | ours | theirs | median in-person games | mean sigma |",
               "|---|---|---:|---:|---:|---:|---:|---:|")
    for _, t in part:
        m = ls.get(t, NAN3)
        print(f"| {names.get(t, t)} | {a[t][2]} | {a[t][3]} | {w.get(t, 0.0):.0f} | "
              f"{a[t][0]:.4f} | {b[t][0]:.4f} | {m[0]:.0f} | {m[1]:.3f} |")


def print_cohorts(rows, w, key):
    """The difference summed by `key(tid)`, weighted."""
    buckets = {}
    for d, t in rows:
        k = key(t) if w.get(t, 0.0) > 0 else None
        if k is not None:
            e = buckets.setdefault(k, [0.0, 0.0, 0])
            e[0], e[1], e[2] = e[0] + d, e[1] + w[t], e[2] + 1
    for k, e in sorted(buckets.items()):
        print(f"| {k} | {e[2]} fields | theirs minus ours {e[0] / e[1]:+.4f} |")


def cohort(x, cuts):
    return next(label for top, label in cuts if x < top)


def summary(rows, both, w, frm, to):
    den = sum(w.get(t, 0.0) for t in both)
    lose = sum(r[0] for r in rows if r[0] > 0)
    win = -sum(r[0] for r in rows if r[0] < 0)
    print(f"{len(both)} common fields, {frm} to {to}")
    print(f"weighted NDCG, theirs minus ours: {sum(r[0] for r in rows) / den:+.4f}")
    print(f"they gain {lose / den:+.4f} on {sum(1 for r in rows if r[0] > 0)} fields, "
          f"we gain {win / den:+.4f} on {sum(1 for r in rows if r[0] < 0)} fields")


def main():
    o = lab.cli(__doc__, OPTIONS)
    frm, to = lab.TEST if o.test else lab.TUNE
    w = lab.load_weights()
    a, b = per_field(o.ours, frm, to), per_field(o.theirs, frm, to)
    both = sorted(set(a) & set(b))
    rows = sorted(((w.get(t, 0.0) * (b[t][0] - a[t][0]), t) for t in both), key=lambda r: -r[0])
    summary(rows, both, w, frm, to)
    ls = lineup_stats(o.ours, [t for _, t in rows[: o.top]] + [t for _, t in rows[-o.top :]])
    for label, part in (("they order better", rows[: o.top]), ("we order better", rows[-o.top :])):
        print_fields(a, b, w, ls, label, part, o.top)
    # The same split as a cohort table over every common field.
    print("\n## The difference by cohort\n")
    ls = lineup_stats(o.ours, both)
    print_cohorts(rows, w, lambda t: cohort(ls[t][0], GAMES) if t in ls else None)
    print()
    print_cohorts(rows, w, lambda t: cohort(a[t][3], SIZES))


if __name__ == "__main__":
    main()
