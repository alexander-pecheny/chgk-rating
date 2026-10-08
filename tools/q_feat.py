"""What the misplaced lineups have in common.

One row per in-person result in a weighted field: how far the forecast put the
team from where it finished, and what the lineup looked like on that morning.
The residual is `(forecast rank - actual rank) / n`, positive when the team did
better than the forecast said.  The features come from `player_events`, which
carries the state the forecast actually read.

    uv run tools/q_feat.py out/q2/base.db [--test] [--top 3]

`--top k` keeps only the teams that finished in the first `k`, which is where
the NDCG of relevance `1/place` puts almost all its weight.
"""

import json
import sqlite3
import statistics

import lab

FIELD_COLS = "e.tournament_id, e.team_id, e.actual_place, e.forecast, m.date"
# Forecast-percentile bins of the residual, in twentieths.
BINS = 20
# A lineup's median days since its last in-person game, when it has none.
NO_DAYS = 999.0
THIN = 5
# A rank as a percentile: the middle of its slot.
MID = 0.5
DECILE = 10
KEYS = ["seats", "theta_max", "theta_mean", "theta_sd", "phi_mean", "sigma_mean", "sigma_max",
        "ip_median", "ip_min", "sync_only", "thin_ip", "days_ip"]
OPTIONS = [
    ("db", dict()),
    ("--test", dict(action="store_true", help="the test window instead of tune")),
    ("--top", dict(type=int, default=0, help="only the teams that finished in the first K")),
]


def load(db, frm, to):
    """The fields, the players' state and every lineup of the mirror."""
    rows = lab.in_person(db, frm, to, FIELD_COLS, where="  and e.forecast is not null")
    by_t = {t: [r[:3] for r in v] for t, v in lab.by_tournament(rows).items() if len(v) >= lab.MIN_FIELD}
    st = lab.player_state(sqlite3.connect(db))
    mem = {}
    for tid, team, ms in sqlite3.connect(lab.BUFF).execute("select id, team_id, team_members from tournament_results"):
        if ms and ms != "[]":
            mem.setdefault(tid, {})[team] = json.loads(ms)
    return by_t, st, mem


def features(ms, s):
    """What the lineup looked like, from the state of its players."""
    v = [s[p] for p in ms if p in s]
    if not v:
        return None
    th, ph, sg, gi, _ = (list(col) for col in zip(*v))
    di = [x[4] for x in v if x[4] >= 0]
    return {
        "seats": float(len(ms)), "theta_max": max(th), "theta_mean": statistics.mean(th),
        "theta_sd": statistics.pstdev(th) if len(th) > 1 else 0.0,
        "phi_mean": statistics.mean(ph), "sigma_mean": statistics.mean(sg), "sigma_max": max(sg),
        "ip_median": float(statistics.median(gi)), "ip_min": float(min(gi)),
        "sync_only": sum(1.0 for g in gi if g == 0) / len(gi),
        "thin_ip": sum(1.0 for g in gi if g < THIN) / len(gi),
        "days_ip": float(statistics.median(di)) if di else NO_DAYS,
    }


def field_rows(tid, v, wt, st, mem, topk):
    """One row per team of a field: its features, residual and forecast percentile."""
    fc = {t: f for t, _, f in v}
    place = {t: p for t, p, _ in v}
    order = lab.ranked([t for t, _, _ in v], fc.get, lab.Rng(tid))
    rank_f = {t: i + 1 for i, t in enumerate(order)}
    rank_a = {t: i + 1 for i, t in enumerate(sorted(order, key=lambda t: place[t]))}
    n = len(order)
    out = []
    for t in order:
        f = None if topk and rank_a[t] > topk else features(mem.get(tid, {}).get(t, []), st.get(tid, {}))
        if f is not None:
            f.update(resid=(rank_f[t] - rank_a[t]) / n, fpct=(rank_f[t] - MID) / n, w=wt, n=float(n))
            out.append(f)
    return out


def debias(rows):
    """A team forecast first can only disappoint and a team forecast last can only
    surprise, so the raw residual is a function of where the forecast put the
    team. Subtracting the mean residual of its forecast-percentile bin leaves
    the part a feature could still explain."""
    bins = {}
    for r in rows:
        bins.setdefault(min(BINS - 1, int(r["fpct"] * BINS)), []).append(r)
    for b in bins.values():
        m = sum(r["w"] * r["resid"] for r in b) / sum(r["w"] for r in b)
        for r in b:
            r["resid"] -= m


def report(rows, frm, to, topk):
    print(f"{len(rows)} results, {frm} to {to}" + (f", top {topk} only" if topk else ""))
    print()
    lab.header("| feature | weighted corr with the residual | mean | mean in the worst decile |",
               "|---|---:|---:|---:|")
    worst = sorted(rows, key=lambda r: -r["resid"])[: max(1, len(rows) // DECILE)]
    sw = sum(r["w"] for r in rows)
    for k in KEYS:
        r, _ = lab.wcorr(rows, k, "resid", "w", min_n=0)
        mx = sum(q["w"] * q[k] for q in rows) / sw
        print(f"| {k} | {r:+.3f} | {mx:.3f} | {sum(q[k] for q in worst) / len(worst):.3f} |")
    print(f"\nmean residual {sum(r['w'] * r['resid'] for r in rows) / sw:+.4f}")


def main():
    a = lab.cli(__doc__, OPTIONS)
    frm, to = lab.TEST if a.test else lab.TUNE
    w = lab.load_weights()
    by_t, st, mem = load(a.db, frm, to)
    rows = [r for tid, v in by_t.items() if w.get(tid, 0.0) > 0
            for r in field_rows(tid, v, w[tid], st, mem, a.top)]
    debias(rows)
    report(rows, frm, to, a.top)


if __name__ == "__main__":
    main()
