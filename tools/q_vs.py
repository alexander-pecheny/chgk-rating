"""Where another system's forecast beats ours, field by field.

The absolute residual says the loss is concentrated in fields no forecast can
order.  What matters for plan 5 is the *difference*: the fields rating.chgk.fun
or chgk.quest orders better than we do, and what their lineups have in common.

    uv run tools/q_vs.py out/q2/base.db out/ext/fun.db [--test] [--top 20]
"""

import json
import sqlite3
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ndcg as N  # noqa: E402
import q_diag as D  # noqa: E402


def per_field(db, frm, to):
    fs = D.fields(db, frm, to)
    out = {}
    for tid, v in fs.items():
        order, places, strength, nd, ideal = D.scored(v, tid)
        rel = [1.0 / places[t] for t in order]
        best = sorted(rel, reverse=True)
        out[tid] = (nd, D.dcg(rel[:3]) / D.dcg(best[:3]), v["date"], len(order))
    return out


def lineup_stats(db, tids):
    """Median in-person games and mean sigma of the lineups of each field."""
    conn = sqlite3.connect(db)
    st = {}
    for tid, p, sg, gi in conn.execute(
        "select tournament_id, player_id, sigma_before, games_ip from player_events"
    ):
        st.setdefault(tid, {})[p] = (sg, gi)
    buff = sqlite3.connect(BUFF := "buff.db")
    out = {}
    for tid in tids:
        v = []
        for team, ms in buff.execute(
            "select team_id, team_members from tournament_results where id = ?", (tid,)
        ):
            if ms and ms != "[]":
                for p in json.loads(ms):
                    if p in st.get(tid, {}):
                        v.append(st[tid][p])
        if v:
            out[tid] = (
                statistics.median(g for _, g in v),
                statistics.mean(s for s, _ in v),
                len(v),
            )
    return out


def main():
    args = sys.argv[1:]
    ours, theirs = args[0], args[1]
    frm, to = D.TUNE
    top = 20
    i = 2
    while i < len(args):
        if args[i] == "--test":
            frm, to = D.TEST
            i += 1
        elif args[i] == "--top":
            top = int(args[i + 1])
            i += 2
        else:
            i += 1

    w = N.load_weights(N.WEIGHTS)
    a, b = per_field(ours, frm, to), per_field(theirs, frm, to)
    both = sorted(set(a) & set(b))
    rows = [(w.get(t, 0.0) * (b[t][0] - a[t][0]), t) for t in both]
    rows.sort(key=lambda r: -r[0])
    den = sum(w.get(t, 0.0) for t in both)
    gap = sum(r[0] for r in rows) / den
    print(f"{len(both)} common fields, {frm} to {to}")
    print(f"weighted NDCG, theirs minus ours: {gap:+.4f}")
    lose = sum(r[0] for r in rows if r[0] > 0)
    win = -sum(r[0] for r in rows if r[0] < 0)
    print(f"they gain {lose / den:+.4f} on {sum(1 for r in rows if r[0] > 0)} fields, "
          f"we gain {win / den:+.4f} on {sum(1 for r in rows if r[0] < 0)} fields")

    ls = lineup_stats(ours, [t for _, t in rows[:top]] + [t for _, t in rows[-top:]])
    names = D.names([t for _, t in rows[:top]] + [t for _, t in rows[-top:]],
                    "tournaments", "name")
    for label, part in (("they order better", rows[:top]), ("we order better", rows[-top:])):
        print(f"\n## The {top} fields {label}\n")
        print("| tournament | date | teams | weight | ours | theirs | median in-person games | mean sigma |")
        print("|---|---|---:|---:|---:|---:|---:|---:|")
        for d, t in part:
            m = ls.get(t, (float("nan"),) * 3)
            print(
                f"| {names.get(t, t)} | {a[t][2]} | {a[t][3]} | {w.get(t, 0.0):.0f} | "
                f"{a[t][0]:.4f} | {b[t][0]:.4f} | {m[0]:.0f} | {m[1]:.3f} |"
            )

    # The same split as a cohort table over every common field.
    print("\n## The difference by cohort\n")
    ls = lineup_stats(ours, both)
    buckets = {}
    for d, t in rows:
        m = ls.get(t)
        if m is None or w.get(t, 0.0) <= 0:
            continue
        key = (
            "median under 10 in-person games"
            if m[0] < 10
            else "median 10-29"
            if m[0] < 30
            else "median 30+"
        )
        e = buckets.setdefault(key, [0.0, 0.0, 0])
        e[0] += d
        e[1] += w.get(t, 0.0)
        e[2] += 1
    for k, e in sorted(buckets.items()):
        print(f"| {k} | {e[2]} fields | theirs minus ours {e[0] / e[1]:+.4f} |")
    buckets = {}
    for d, t in rows:
        if w.get(t, 0.0) <= 0:
            continue
        n = a[t][3]
        key = "under 20 teams" if n < 20 else "20-49 teams" if n < 50 else "50+ teams"
        e = buckets.setdefault(key, [0.0, 0.0, 0])
        e[0] += d
        e[1] += w.get(t, 0.0)
        e[2] += 1
    print()
    for k, e in sorted(buckets.items()):
        print(f"| {k} | {e[2]} fields | theirs minus ours {e[0] / e[1]:+.4f} |")


if __name__ == "__main__":
    main()
