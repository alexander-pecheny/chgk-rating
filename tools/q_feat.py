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
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ndcg as N  # noqa: E402

TUNE = ("2018-09-01", "2024-08-29")
TEST = ("2024-08-30", "2026-08-27")
BUFF = "buff.db"

FIELD = """
select e.tournament_id, e.team_id, e.actual_place, e.forecast, m.date
from team_events e
join tournament_metrics m on m.tournament_id = e.tournament_id
where m.type = 'in-person' and m.date >= ? and m.date < ? and e.actual_place > 0
  and e.forecast is not null
order by e.tournament_id
"""


def load(db, frm, to):
    conn = sqlite3.connect(db)
    by_t = {}
    for tid, team, place, fc, date in conn.execute(FIELD, (frm, to)):
        by_t.setdefault(tid, []).append((team, place, fc))
    by_t = {t: v for t, v in by_t.items() if len(v) >= 5}
    st = {}
    for tid, p, th, ph, sg, gi, di in conn.execute(
        "select tournament_id, player_id, theta_before, phi_before, sigma_before,"
        " games_ip, days_ip from player_events"
    ):
        st.setdefault(tid, {})[p] = (th, ph, sg, gi, di)
    buff = sqlite3.connect(BUFF)
    mem = {}
    for tid, team, ms in buff.execute(
        "select id, team_id, team_members from tournament_results"
    ):
        if ms and ms != "[]":
            mem.setdefault(tid, {})[team] = json.loads(ms)
    return by_t, st, mem


def features(ms, s):
    v = [s[p] for p in ms if p in s]
    if not v:
        return None
    th = [x[0] for x in v]
    ph = [x[1] for x in v]
    sg = [x[2] for x in v]
    gi = [x[3] for x in v]
    di = [x[4] for x in v if x[4] >= 0]
    return {
        "seats": float(len(ms)),
        "theta_max": max(th),
        "theta_mean": statistics.mean(th),
        "theta_sd": statistics.pstdev(th) if len(th) > 1 else 0.0,
        "phi_mean": statistics.mean(ph),
        "sigma_mean": statistics.mean(sg),
        "sigma_max": max(sg),
        "ip_median": float(statistics.median(gi)),
        "ip_min": float(min(gi)),
        "sync_only": sum(1.0 for g in gi if g == 0) / len(gi),
        "thin_ip": sum(1.0 for g in gi if g < 5) / len(gi),
        "days_ip": float(statistics.median(di)) if di else 999.0,
    }


def main():
    args = sys.argv[1:]
    db = args[0]
    frm, to = TUNE
    topk = 0
    i = 1
    while i < len(args):
        if args[i] == "--test":
            frm, to = TEST
            i += 1
        elif args[i] == "--top":
            topk = int(args[i + 1])
            i += 2
        else:
            i += 1

    w = N.load_weights(N.WEIGHTS)
    by_t, st, mem = load(db, frm, to)
    rows = []
    for tid, v in by_t.items():
        wt = w.get(tid, 0.0)
        if wt <= 0:
            continue
        order = [t for t, _, _ in v]
        N.Rng(tid).shuffle(order)
        fc = {t: f for t, _, f in v}
        place = {t: p for t, p, _ in v}
        order.sort(key=lambda t: -fc[t])
        rank_f = {t: i + 1 for i, t in enumerate(order)}
        rank_a = {t: i + 1 for i, t in enumerate(sorted(order, key=lambda t: place[t]))}
        n = len(order)
        for t in order:
            if topk and rank_a[t] > topk:
                continue
            f = features(mem.get(tid, {}).get(t, []), st.get(tid, {}))
            if f is None:
                continue
            f["resid"] = (rank_f[t] - rank_a[t]) / n
            f["fpct"] = (rank_f[t] - 0.5) / n
            f["w"] = wt
            f["n"] = float(n)
            rows.append(f)

    # A team forecast first can only disappoint and a team forecast last can only
    # surprise, so the raw residual is a function of where the forecast put the
    # team. Subtracting the mean residual of its forecast-percentile bin leaves
    # the part a feature could still explain.
    bins = {}
    for r in rows:
        bins.setdefault(min(19, int(r["fpct"] * 20)), []).append(r)
    for b in bins.values():
        m = sum(r["w"] * r["resid"] for r in b) / sum(r["w"] for r in b)
        for r in b:
            r["resid"] -= m

    keys = [
        "seats",
        "theta_max",
        "theta_mean",
        "theta_sd",
        "phi_mean",
        "sigma_mean",
        "sigma_max",
        "ip_median",
        "ip_min",
        "sync_only",
        "thin_ip",
        "days_ip",
    ]
    print(f"{len(rows)} results, {frm} to {to}" + (f", top {topk} only" if topk else ""))
    print("\n| feature | weighted corr with the residual | mean | mean in the worst decile |")
    print("|---|---:|---:|---:|")
    worst = sorted(rows, key=lambda r: -r["resid"])[: max(1, len(rows) // 10)]
    for k in keys:
        x = [r[k] for r in rows]
        y = [r["resid"] for r in rows]
        ww = [r["w"] for r in rows]
        sw = sum(ww)
        mx = sum(a * b for a, b in zip(ww, x)) / sw
        my = sum(a * b for a, b in zip(ww, y)) / sw
        cov = sum(c * (a - mx) * (b - my) for a, b, c in zip(x, y, ww)) / sw
        vx = sum(c * (a - mx) ** 2 for a, c in zip(x, ww)) / sw
        vy = sum(c * (b - my) ** 2 for b, c in zip(y, ww)) / sw
        r = cov / (vx * vy) ** 0.5 if vx > 0 and vy > 0 else float("nan")
        print(
            f"| {k} | {r:+.3f} | {mx:.3f} | "
            f"{sum(q[k] for q in worst) / len(worst):.3f} |"
        )
    my = sum(r['w'] * r['resid'] for r in rows) / sum(r['w'] for r in rows)
    print(f"\nmean residual {my:+.4f}")


if __name__ == "__main__":
    main()
