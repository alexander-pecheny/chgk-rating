"""The mechanism behind each round-3 lever, checked on the residual before the sweep.

Plan 6 asks for a reason before a number.  Every check below is on the same
object plan 5 used: one row per in-person result in a weighted tune field, the
residual `(forecast rank - actual rank) / n`, with the mean residual of the
team's forecast-percentile bin (twentieths) subtracted -- a team forecast first
can only disappoint, and that floor effect is not a bias.

    uv run tools/q_mech.py out/q3/base.db --lever 1   # chemistry
    uv run tools/q_mech.py out/q3/base.db --lever 2   # difficulty x lineup shape
    uv run tools/q_mech.py out/q3/base.db --lever 3   # the unrated record
    uv run tools/q_mech.py out/q3/base.db --lever 4   # the lineup's own sd
    uv run tools/q_mech.py out/q3/base.db --lever 5   # discrimination by format
    uv run tools/q_mech.py out/q3/base.db --lever 6   # series memory
    uv run tools/q_mech.py out/q3/base.db --lever 7   # editors and strength
"""

import json
import math
import sqlite3
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ndcg as N  # noqa: E402

BUFF = "buff.db"
TUNE = ("2018-09-01", "2024-08-29")

FIELD = """
select e.tournament_id, e.team_id, e.actual_place, e.forecast, m.date
from team_events e
join tournament_metrics m on m.tournament_id = e.tournament_id
where m.type = 'in-person' and m.date >= ? and m.date < ? and e.actual_place > 0
  and e.forecast is not null
order by m.date, e.tournament_id
"""


def wcorr(rows, x, y, w):
    """Weighted Pearson correlation over rows that have both features."""
    v = [(r[w], r[x], r[y]) for r in rows if r[x] is not None and r[y] is not None]
    if len(v) < 30:
        return float("nan"), len(v)
    sw = sum(t[0] for t in v)
    if sw <= 0:
        return float("nan"), len(v)
    mx = sum(t[0] * t[1] for t in v) / sw
    my = sum(t[0] * t[2] for t in v) / sw
    cov = sum(t[0] * (t[1] - mx) * (t[2] - my) for t in v)
    vx = sum(t[0] * (t[1] - mx) ** 2 for t in v)
    vy = sum(t[0] * (t[2] - my) ** 2 for t in v)
    if vx <= 0 or vy <= 0:
        return float("nan"), len(v)
    return cov / math.sqrt(vx * vy), len(v)


def base_rows(db, frm, to):
    """One row per result: tournament, team, residual net of the forecast bin,
    the field's weight, the lineup, and the date."""
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    by_t = {}
    for tid, team, place, fc, date in conn.execute(FIELD, (frm, to)):
        by_t.setdefault(tid, {"date": date, "rows": []})["rows"].append((team, place, fc))
    by_t = {t: v for t, v in by_t.items() if len(v["rows"]) >= 5}
    w = N.load_weights(N.WEIGHTS)
    buff = sqlite3.connect(f"file:{BUFF}?mode=ro", uri=True)
    members = {}
    for tid in by_t:
        for team, ms in buff.execute(
            "select team_id, team_members from tournament_results where id = ?", (tid,)
        ):
            members[(tid, team)] = json.loads(ms) if ms else []
    st = {}
    for tid, p, th, ph, sg, gi, di in conn.execute(
        "select tournament_id, player_id, theta_before, phi_before, sigma_before,"
        " games_ip, days_ip from player_events"
    ):
        if tid in by_t:
            st.setdefault(tid, {})[p] = (th, ph, sg, gi, di)
    out = []
    for tid, v in by_t.items():
        wt = w.get(tid, 0.0)
        if wt <= 0:
            continue
        rs = sorted(v["rows"], key=lambda r: -r[2])
        n = len(rs)
        for k, (team, place, fc) in enumerate(rs):
            out.append(
                {
                    "tid": tid,
                    "team": team,
                    "date": v["date"],
                    "w": wt,
                    "n": n,
                    "fc_rank": k + 1,
                    "pct": (k + 0.5) / n,
                    "place": place,
                    "raw": (k + 1 - place) / n,
                    "members": members.get((tid, team), []),
                    "st": st.get(tid, {}),
                }
            )
    # Subtract the mean residual of each forecast-percentile bin, in twentieths.
    bins = {}
    for r in out:
        bins.setdefault(int(r["pct"] * 20), []).append(r["raw"])
    mean = {k: sum(v) / len(v) for k, v in bins.items()}
    for r in out:
        r["res"] = r["raw"] - mean[int(r["pct"] * 20)]
    return out


def add_theta(rows):
    for r in rows:
        th = [r["st"][p][0] + r["st"][p][1] for p in r["members"] if p in r["st"]]
        sg = [r["st"][p][2] for p in r["members"] if p in r["st"]]
        r["theta"] = th
        r["sigma"] = sg
        r["mean_theta"] = statistics.mean(th) if th else None
        r["spread"] = statistics.pstdev(th) if len(th) > 1 else None
        r["mean_sigma"] = statistics.mean(sg) if sg else None


# ---------------------------------------------------------------- lever 1


def pair_counts(frm, to, until):
    """`{(i,j): games together}` over rated results strictly before `until`.

    One pass over the window, so the counter is the running one the engine would
    keep; `until` is a date and the table returned is the state on that morning.
    """
    buff = sqlite3.connect(f"file:{BUFF}?mode=ro", uri=True)
    cnt = {}
    for ms, in buff.execute(
        "select r.team_members from tournament_results r join tournaments t on t.id = r.id"
        " where t.in_rating = 1 and substr(t.date_start,1,10) >= ?"
        " and substr(t.date_start,1,10) < ? and r.team_members is not null",
        (frm, until),
    ):
        m = json.loads(ms) if ms else []
        for a in range(len(m)):
            for b in range(a + 1, len(m)):
                k = (m[a], m[b]) if m[a] < m[b] else (m[b], m[a])
                cnt[k] = cnt.get(k, 0) + 1
    return cnt


def lever1(rows):
    """Togetherness `T(L)`, the mean over pairs of `log(1 + games together)`,
    counted over everything before the field's own date."""
    rows.sort(key=lambda r: r["date"])
    cnt, seen_to = {}, "2013-01-01"
    buff = sqlite3.connect(f"file:{BUFF}?mode=ro", uri=True)

    def advance(to):
        nonlocal seen_to
        for ms, in buff.execute(
            "select r.team_members from tournament_results r join tournaments t on t.id = r.id"
            " where t.in_rating = 1 and substr(t.date_start,1,10) >= ?"
            " and substr(t.date_start,1,10) < ? and r.team_members is not null",
            (seen_to, to),
        ):
            m = json.loads(ms) if ms else []
            for a in range(len(m)):
                for b in range(a + 1, len(m)):
                    k = (m[a], m[b]) if m[a] < m[b] else (m[b], m[a])
                    cnt[k] = cnt.get(k, 0) + 1
        seen_to = to

    for r in rows:
        if r["date"] > seen_to:
            advance(r["date"])
        m = r["members"]
        ts = []
        for a in range(len(m)):
            for b in range(a + 1, len(m)):
                k = (m[a], m[b]) if m[a] < m[b] else (m[b], m[a])
                ts.append(math.log1p(cnt.get(k, 0)))
        r["T"] = statistics.mean(ts) if ts else None
        r["Tmax"] = max(ts) if ts else None

    print("## Lever 1: chemistry, games played together\n")
    print("| feature | weighted corr with the residual | results |")
    print("|---|---:|---:|")
    for name, key in [("T(L), mean over pairs of log(1+games together)", "T"),
                      ("the same, largest pair", "Tmax")]:
        c, n = wcorr(rows, key, "res", "w")
        print(f"| {name} | {c:+.3f} | {n} |")
    strong = [r for r in rows if r["fc_rank"] <= 10 and r["n"] >= 20]
    print("\nAmong the ten teams each field's forecast puts highest, in fields of 20+:\n")
    print("| cohort | results | mean T(L) |")
    print("|---|---:|---:|")
    for label, sel in [
        ("forecast 1-3, finished 1-3", lambda r: r["fc_rank"] <= 3 and r["place"] <= 3),
        ("forecast 1-3, finished 4+", lambda r: r["fc_rank"] <= 3 and r["place"] > 3),
        ("forecast 4-10, finished 1", lambda r: 4 <= r["fc_rank"] <= 10 and r["place"] <= 1),
        ("forecast 4-10, finished 2-3", lambda r: 4 <= r["fc_rank"] <= 10 and 2 <= r["place"] <= 3),
        ("forecast 4-10, finished 4+", lambda r: 4 <= r["fc_rank"] <= 10 and r["place"] > 3),
    ]:
        v = [r["T"] for r in strong if sel(r) and r["T"] is not None]
        if v:
            print(f"| {label} | {len(v)} | {statistics.mean(v):.2f} |")


# ---------------------------------------------------------------- lever 2


def lever2(db, rows):
    """Does the ranking of a lopsided lineup against a balanced one depend on
    how hard the package is?  If it does, a global `m` has something to fit."""
    add_theta(rows)
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    hard = dict(conn.execute("select tournament_id, truedl from tournament_metrics"))
    for r in rows:
        r["hard"] = hard.get(r["tid"])
    v = [r for r in rows if r["hard"] is not None and r["spread"] is not None]
    v.sort(key=lambda r: r["hard"])
    print("## Lever 2: does the package's difficulty change what a lopsided lineup is worth\n")
    c, n = wcorr(rows, "spread", "res", "w")
    print(f"Spread of theta against the residual, all fields: {c:+.3f} over {n} results.\n")
    print("| tercile of the field's difficulty | truedl | results | corr(spread of theta, residual) |")
    print("|---|---:|---:|---:|")
    k = len(v) // 3
    for i, part in enumerate([v[:k], v[k : 2 * k], v[2 * k :]]):
        c, n = wcorr(part, "spread", "res", "w")
        print(
            f"| {i + 1} | {statistics.mean(r['hard'] for r in part):.2f} | {n} | {c:+.3f} |"
        )


# ---------------------------------------------------------------- lever 3


def lever3(rows):
    """How much of a lineup's record the rated window cannot see: unrated
    in-person and synchronous tournaments its players played in the two years
    before the field."""
    buff = sqlite3.connect(f"file:{BUFF}?mode=ro", uri=True)
    hist = {}
    for date, ms in buff.execute(
        "select substr(t.date_start,1,10), r.team_members from tournament_results r"
        " join tournaments t on t.id = r.id where t.in_rating = 0"
        " and t.tournament_type in ('Обычный','Региональный','Синхрон','Строго синхронный')"
        " and substr(t.date_start,1,10) >= '2016-01-01' and r.team_members is not null"
    ):
        for p in json.loads(ms) if ms else []:
            hist.setdefault(p, []).append(date)
    for v in hist.values():
        v.sort()
    import bisect

    for r in rows:
        d = r["date"]
        lo = f"{int(d[:4]) - 2}{d[4:]}"
        v = []
        for p in r["members"]:
            h = hist.get(p, [])
            v.append(bisect.bisect_left(h, d) - bisect.bisect_left(h, lo))
        r["unrated"] = statistics.median(v) if v else None
        r["unrated_mean"] = statistics.mean(v) if v else None
    print("## Lever 3: the record the rated window cannot see\n")
    print("| feature | weighted corr with the residual | results |")
    print("|---|---:|---:|")
    for name, key in [
        ("median unrated tournaments per player, last 2 years", "unrated"),
        ("the mean of the same", "unrated_mean"),
    ]:
        c, n = wcorr(rows, key, "res", "w")
        print(f"| {name} | {c:+.3f} | {n} |")
    v = [r["unrated_mean"] for r in rows if r["unrated_mean"] is not None]
    share = sum(1 for x in v if x > 0) / max(len(v), 1)
    print(
        f"\n{share:.1%} of the lineups have a player with at least one unrated "
        f"tournament in the window; the mean count is {statistics.mean(v):.2f}."
    )


# ---------------------------------------------------------------- lever 4


def lever4(rows, tau=0.7, scale=2454.8828):
    """The sd of the roster estimate propagated from the players' sigma, against
    the mean sigma round 1 already subtracts."""
    add_theta(rows)
    for r in rows:
        th, sg = r["theta"], r["sigma"]
        if not th:
            r["sdQ"] = None
            continue
        # Softmax weights of the tempered roster estimate, on the theta scale.
        m = max(th)
        e = [math.exp((x - m) / tau) for x in th]
        s = sum(e)
        wts = [x / s for x in e]
        r["sdQ"] = math.sqrt(sum((a * b) ** 2 for a, b in zip(wts, sg)))
        r["maxw"] = max(wts)
    print("## Lever 4: the lineup's own uncertainty against the mean of its players'\n")
    print("| feature | weighted corr with the residual | results |")
    print("|---|---:|---:|")
    for name, key in [
        ("mean sigma of the lineup", "mean_sigma"),
        ("sd(Q(L)), propagated through the tempered roster estimate", "sdQ"),
    ]:
        c, n = wcorr(rows, key, "res", "w")
        print(f"| {name} | {c:+.3f} | {n} |")
    v = [(r["mean_sigma"], r["sdQ"]) for r in rows if r["sdQ"] is not None and r["mean_sigma"]]
    print(
        f"\nsd(Q(L)) is {statistics.mean(b / a for a, b in v):.2f} of the mean sigma on "
        f"average, and the two correlate at "
        f"{statistics.correlation([a for a, _ in v], [b for _, b in v]):.3f}: "
        "they are close to the same number up to a scale."
    )


# ---------------------------------------------------------------- lever 5


def lever5(db, rows):
    """Is the forecast's spread at the top of an in-person field too wide or too
    narrow?  If a gap of g points buys less than the model thinks, `a` is too
    large; if more, too small."""
    by_t = {}
    for r in rows:
        by_t.setdefault(r["tid"], []).append(r)
    print("## Lever 5: how much a point of forecast gap is worth in person\n")
    print("| decile of the 1st-to-2nd forecast gap | gap (points) | fields | P(the favourite wins) |")
    print("|---|---:|---:|---:|")
    fs = []
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    fc = {}
    for tid, team, f in conn.execute(
        "select tournament_id, team_id, forecast from team_events where forecast is not null"
    ):
        fc.setdefault(tid, {})[team] = f
    for tid, rs in by_t.items():
        if len(rs) < 10:
            continue
        rs = sorted(rs, key=lambda r: r["fc_rank"])
        g = fc[tid][rs[0]["team"]] - fc[tid][rs[1]["team"]]
        fs.append((g, 1.0 if rs[0]["place"] <= min(r["place"] for r in rs) else 0.0, rs[0]["w"]))
    fs.sort()
    n = len(fs)
    for q in range(10):
        part = fs[q * n // 10 : (q + 1) * n // 10]
        dw = sum(p[2] for p in part)
        print(
            f"| {q + 1} | {statistics.mean(p[0] for p in part):.0f} | {len(part)} | "
            f"{sum(p[2] * p[1] for p in part) / dw:.3f} |"
        )


# ---------------------------------------------------------------- lever 6


def lever6(db, rows, k=5):
    """A team's mean surprise over its last k in-person results against its
    residual today.  Surprise is `(forecast place - actual place) / n`, the same
    quantity as the residual, so this is simply its autocorrelation per team."""
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    hist = {}
    for tid, team, fp, ap, date, nt in conn.execute(
        "select e.tournament_id, e.team_id, e.forecast_place, e.actual_place, m.date, m.n_teams"
        " from team_events e join tournament_metrics m on m.tournament_id = e.tournament_id"
        " where m.type = 'in-person' and e.actual_place > 0 and e.forecast is not null"
        " order by m.date, e.tournament_id"
    ):
        hist.setdefault(team, []).append((date, (fp - ap) / max(nt, 1)))
    for r in rows:
        h = [s for d, s in hist.get(r["team"], []) if d < r["date"]]
        r["S"] = statistics.mean(h[-k:]) if len(h) >= 2 else None
        r["Sn"] = len(h)
    print(f"## Lever 6: series memory, the mean surprise over the last {k} in-person results\n")
    print("| feature | weighted corr with the residual | results |")
    print("|---|---:|---:|")
    c, n = wcorr(rows, "S", "res", "w")
    print(f"| mean of the last {k} surprises | {c:+.3f} | {n} |")
    for lo in (5, 10, 20):
        v = [r for r in rows if r["Sn"] >= lo]
        c, n = wcorr(v, "S", "res", "w")
        print(f"| the same, teams with {lo}+ in-person results behind them | {c:+.3f} | {n} |")
    v = [r for r in rows if r["fc_rank"] <= 5]
    c, n = wcorr(v, "S", "res", "w")
    print(f"| the same, the five highest forecasts of a field | {c:+.3f} | {n} |")


# ---------------------------------------------------------------- lever 7


def lever7(db, rows):
    """Does a lineup's past residual under the same editors predict its residual
    here, and is that link stronger for strong lineups?"""
    add_theta(rows)
    buff = sqlite3.connect(f"file:{BUFF}?mode=ro", uri=True)
    eds = {}
    for tid, e in buff.execute("select id, editors from tournaments"):
        try:
            eds[tid] = json.loads(e) if e else []
        except (ValueError, TypeError):
            eds[tid] = []
    rows.sort(key=lambda r: r["date"])
    # Per (player, editor), the running mean of the residuals of the results
    # that player's lineup had under that editor, strictly before today.
    acc = {}
    for r in rows:
        es = eds.get(r["tid"], [])
        v = []
        for p in r["members"]:
            for e in es:
                a = acc.get((p, e))
                if a and a[1] >= 2:
                    v.append(a[0] / a[1])
        r["ed_past"] = statistics.mean(v) if v else None
        r["ed_n"] = len(v)
        for p in r["members"]:
            for e in es:
                a = acc.setdefault((p, e), [0.0, 0])
                a[0] += r["res"]
                a[1] += 1
    print("## Lever 7: editor affinity, and whether it is a property of strong players\n")
    print("| cohort | weighted corr(mean past residual under these editors, residual) | results |")
    print("|---|---:|---:|")
    c, n = wcorr(rows, "ed_past", "res", "w")
    print(f"| every lineup with any history under the editors | {c:+.3f} | {n} |")
    v = [r for r in rows if r["mean_theta"] is not None and r["ed_past"] is not None]
    v.sort(key=lambda r: r["mean_theta"])
    m = len(v) // 5
    for i, part in enumerate([v[: 2 * m], v[2 * m : 4 * m], v[4 * m :]]):
        c, n = wcorr(part, "ed_past", "res", "w")
        lab = ["the weakest two fifths by mean theta", "the middle two fifths",
               "the strongest fifth"][i]
        print(f"| {lab} | {c:+.3f} | {n} |")
    top = sorted(v, key=lambda r: -r["mean_theta"])[: len(v) // 20]
    c, n = wcorr(top, "ed_past", "res", "w")
    print(f"| the strongest twentieth | {c:+.3f} | {n} |")
    print(
        f"\n{sum(1 for r in rows if r['ed_n'] > 0) / len(rows):.1%} of the results have any "
        "history under the tournament's editors at all."
    )


def main():
    args = sys.argv[1:]
    db = args[0]
    levers = []
    i = 1
    while i < len(args):
        if args[i] == "--lever":
            levers.append(int(args[i + 1]))
            i += 2
        else:
            i += 1
    rows = base_rows(db, *TUNE)
    print(f"# Round-3 mechanisms on the round-2 residual, {TUNE[0]} to {TUNE[1]}\n")
    print(f"{len(rows)} results in {len({r['tid'] for r in rows})} weighted in-person fields.\n")
    for k in levers or [1, 2, 3, 4, 5, 6, 7]:
        {1: lambda: lever1(rows), 2: lambda: lever2(db, rows), 3: lambda: lever3(rows),
         4: lambda: lever4(rows), 5: lambda: lever5(db, rows), 6: lambda: lever6(db, rows),
         7: lambda: lever7(db, rows)}[k]()
        print()


if __name__ == "__main__":
    main()
