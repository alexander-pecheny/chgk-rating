"""The mechanism behind each round-4 lever, checked on the residual before the sweep.

Same object as `tools/q_mech.py`: one row per in-person result in a weighted
tune field, the residual `(forecast rank - actual rank) / n` with the mean
residual of the team's forecast-percentile bin subtracted.  Lever 1 is the
exception -- it is a claim about the *metric*, not about the mean residual, so
it is checked on the realised relevance `1/place` instead.

    uv run tools/q_mech4.py out/q4/base.db --lever 1   # expected relevance
    uv run tools/q_mech4.py out/q4/base.db --lever 2   # home and away
    uv run tools/q_mech4.py out/q4/base.db --lever 3   # player load
    uv run tools/q_mech4.py out/q4/base.db --lever 4   # season timing
    uv run tools/q_mech4.py out/q4/base.db --lever 5   # the editor memory
    uv run tools/q_mech4.py out/q4/base.db --lever 6   # a field-conditional quantile
"""

import bisect
import csv
import json
import math
import sqlite3
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from q_mech import BUFF, TUNE, add_theta, base_rows, wcorr  # noqa: E402

TOWNS = "data/towns.csv"


def renorm(rows):
    """Re-derives the residual from *ranks on both sides*.

    `q_mech.base_rows` keeps only the results a field forecasts and then scores
    `(forecast rank - actual place) / n` against that shortened field, so a
    field of 120 teams of which 6 carry a lineup produces residuals near -19.
    Round 3 read those through weighted correlations and never printed a cohort
    mean; this round does, so the actual place is replaced by its rank among the
    rows kept, ties shared, and the forecast-bin means are subtracted again.
    """
    by_t = {}
    for r in rows:
        by_t.setdefault(r["tid"], []).append(r)
    for rs in by_t.values():
        n = len(rs)
        order = sorted(rs, key=lambda r: r["place"])
        i = 0
        while i < n:
            j = i
            while j + 1 < n and order[j + 1]["place"] == order[i]["place"]:
                j += 1
            rank = (i + j) / 2.0 + 1.0
            for r in order[i : j + 1]:
                r["arank"] = rank
            i = j + 1
        best = min(r["place"] for r in rs)
        for r in rs:
            r["n"] = n
            r["raw"] = (r["fc_rank"] - r["arank"]) / n
            r["rel"] = 1.0 / r["arank"]
            r["won"] = 1.0 if r["place"] == best else 0.0
    bins = {}
    for r in rows:
        bins.setdefault(int(r["pct"] * 20), []).append(r["raw"])
    mean = {k: sum(v) / len(v) for k, v in bins.items()}
    for r in rows:
        r["res"] = r["raw"] - mean[int(r["pct"] * 20)]
    return rows


def mean_se(v):
    """Mean and its standard error; `nan` for fewer than two values."""
    if len(v) < 2:
        return float("nan"), float("nan")
    return statistics.mean(v), statistics.pstdev(v) / math.sqrt(len(v))


def forecasts(db):
    """`{tournament: {team: forecast}}`, the display-point forecast per result."""
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    out = {}
    for tid, team, f in conn.execute(
        "select tournament_id, team_id, forecast from team_events where forecast is not null"
    ):
        out.setdefault(tid, {})[team] = f
    return out


def add_field(db, rows):
    """Per field: the 1st-to-4th forecast gap (tightness) and the mean forecast
    of its ten best lineups (strength).  Both in display points."""
    fc = forecasts(db)
    by_t = {}
    for r in rows:
        by_t.setdefault(r["tid"], []).append(r)
    gap, strength = {}, {}
    for tid, rs in by_t.items():
        v = sorted((fc.get(tid, {}).get(r["team"]) for r in rs), reverse=True)
        v = [x for x in v if x is not None]
        if len(v) < 4:
            continue
        gap[tid] = v[0] - v[3]
        strength[tid] = statistics.mean(v[:10])
    for r in rows:
        r["fc"] = fc.get(r["tid"], {}).get(r["team"])
        r["gap"] = gap.get(r["tid"])
        r["strength"] = strength.get(r["tid"])


def add_sd(rows, tau=0.7):
    """`sd(Q(L))`, the sd of the tempered roster estimate propagated from the
    players' sigma, on the theta scale.  The same quantity `roster_sd` computes."""
    add_theta(rows)
    for r in rows:
        th, sg = r["theta"], r["sigma"]
        if not th:
            r["sd"] = None
            continue
        m = max(th)
        e = [math.exp((x - m) / tau) for x in th]
        s = sum(e)
        r["sd"] = math.sqrt(sum(((x / s) * b) ** 2 for x, b in zip(e, sg)))


# ---------------------------------------------------------------- lever 1


def lever1(db, rows):
    """NDCG pays for `1/place`, which is convex in the place.  Two lineups with
    the same mean strength and different spread therefore have different
    expected relevance, and the wider one is worth more -- but only where the
    order is actually in doubt.  If that is real, the wider lineup of a tight
    field wins more often than the narrower one at the same forecast rank."""
    add_field(db, rows)
    add_sd(rows)
    v = [r for r in rows if r["sd"] is not None and r["gap"] is not None]
    print("## Lever 1: does the spread of a lineup buy first places\n")
    print("Among the four lineups each field's forecast puts highest, by the tightness")
    print("of the field and the spread of the lineup. `won` is the share that finished")
    print("first; `1/place` is the relevance the metric pays.\n")
    top = [r for r in v if r["fc_rank"] <= 4 and r["n"] >= 10]
    tight = sorted({r["tid"]: r["gap"] for r in top}.values())
    if not tight:
        return
    cut = tight[len(tight) // 3], tight[2 * len(tight) // 3]
    print("| field | lineup spread | results | won | 1/place | mean residual |")
    print("|---|---|---:|---:|---:|---:|")
    for flab, fsel in [
        ("tight (1st-to-4th gap in the lowest third)", lambda r: r["gap"] <= cut[0]),
        ("middle", lambda r: cut[0] < r["gap"] <= cut[1]),
        ("loose (the highest third)", lambda r: r["gap"] > cut[1]),
    ]:
        part = [r for r in top if fsel(r)]
        if len(part) < 30:
            continue
        s = sorted(r["sd"] for r in part)
        med = s[len(s) // 2]
        for slab, ssel in [
            ("narrow", lambda r: r["sd"] <= med),
            ("wide", lambda r: r["sd"] > med),
        ]:
            q = [r for r in part if ssel(r)]
            if not q:
                continue
            w, sw = mean_se([r["won"] for r in q])
            rl, _ = mean_se([r["rel"] for r in q])
            rs, _ = mean_se([r["res"] for r in q])
            print(
                f"| {flab} | {slab} | {len(q)} | {w:.3f} ± {sw:.3f} | {rl:.3f} | {rs:+.4f} |"
            )
    for lab, sel in [
        ("the four highest forecasts", lambda r: r["fc_rank"] <= 4),
        ("the same, in the tightest third of fields", lambda r: r["fc_rank"] <= 4 and r["gap"] <= cut[0]),
        ("every result", lambda r: True),
    ]:
        part = [r for r in v if sel(r)]
        c, n = wcorr(part, "sd", "rel", "w")
        c2, _ = wcorr(part, "sd", "res", "w")
        print(f"\ncorr(sd, 1/place) over {lab}: {c:+.3f}; corr(sd, residual) {c2:+.3f}; {n} results.")


# ---------------------------------------------------------------- lever 2


def town_map():
    return {int(r["town_id"]): r["name"] for r in csv.DictReader(open(TOWNS, encoding="utf-8"))}


def lever2(db, rows):
    """A team playing at home against one that travelled.  Two readings of
    "home": the town the team is registered in, and the towns its players have
    played out of before today."""
    add_field(db, rows)
    tm = town_map()
    buff = sqlite3.connect(f"file:{BUFF}?mode=ro", uri=True)
    ttown = {tid: tm.get(t) for tid, t in buff.execute("select id, town_id from tournaments")}
    rtown = {}
    for tid, team, town in buff.execute(
        "select id, team_id, team_current_town from tournament_results"
    ):
        rtown[(tid, team)] = town
    # A player's town is the town they have played out of most often before
    # today, so nothing in the field enters its own forecast.
    hist = []
    for date, tid, team, town, ms in buff.execute(
        "select substr(t.date_start,1,10), t.id, r.team_id, r.team_current_town, r.team_members"
        " from tournament_results r join tournaments t on t.id = r.id"
        " where t.in_rating = 1 and substr(t.date_start,1,10) >= '2013-01-01'"
        " and r.team_members is not null order by substr(t.date_start,1,10)"
    ):
        hist.append((date, town, json.loads(ms) if ms else []))
    rows.sort(key=lambda r: r["date"])
    cnt, k = {}, 0
    for r in rows:
        while k < len(hist) and hist[k][0] < r["date"]:
            _, town, ms = hist[k]
            if town:
                for p in ms:
                    d = cnt.setdefault(p, {})
                    d[town] = d.get(town, 0) + 1
            k += 1
        home = ttown.get(r["tid"])
        r["home_team"] = None if home is None else float(rtown.get((r["tid"], r["team"])) == home)
        share = []
        for p in r["members"]:
            d = cnt.get(p)
            if d:
                share.append(1.0 if max(d, key=d.get) == home else 0.0)
        r["home_share"] = statistics.mean(share) if share and home else None
    print("## Lever 2: home and away\n")
    have = [r for r in rows if r["home_team"] is not None]
    print(f"{len(have)} of {len(rows)} results have a town on both sides; "
          f"{statistics.mean(r['home_team'] for r in have):.1%} of them are at home.\n")
    print("| cohort | results | mean residual | won |")
    print("|---|---:|---:|---:|")
    strong = sorted({r["tid"]: r["strength"] for r in have if r["strength"]}.items(),
                    key=lambda kv: -kv[1])
    topfifth = {t for t, _ in strong[: max(1, len(strong) // 5)]}
    for lab, sel in [
        ("the team's own town, at home", lambda r: r["home_team"] > 0.5),
        ("the team's own town, away", lambda r: r["home_team"] < 0.5),
        ("at home, in the strongest fifth of fields", lambda r: r["home_team"] > 0.5 and r["tid"] in topfifth),
        ("away, in the strongest fifth of fields", lambda r: r["home_team"] < 0.5 and r["tid"] in topfifth),
        ("at home, among the four highest forecasts", lambda r: r["home_team"] > 0.5 and r["fc_rank"] <= 4),
        ("away, among the four highest forecasts", lambda r: r["home_team"] < 0.5 and r["fc_rank"] <= 4),
    ]:
        part = [r for r in have if sel(r)]
        m, se = mean_se([r["res"] for r in part])
        w, _ = mean_se([r["won"] for r in part])
        print(f"| {lab} | {len(part)} | {m:+.4f} ± {se:.4f} | {w:.3f} |")
    v = [r for r in rows if r["home_share"] is not None]
    print(f"\n{len(v)} results have a town for at least one player. By the share of the")
    print("lineup that has played out of the tournament's own town before:\n")
    print("| share of the lineup from the town | results | mean residual | won |")
    print("|---|---:|---:|---:|")
    for lab, lo, hi in [("none", -0.01, 0.01), ("under a half", 0.01, 0.5),
                        ("a half or more", 0.5, 0.999), ("all of it", 0.999, 1.01)]:
        part = [r for r in v if lo <= r["home_share"] < hi]
        m, se = mean_se([r["res"] for r in part])
        w, _ = mean_se([r["won"] for r in part])
        print(f"| {lab} | {len(part)} | {m:+.4f} ± {se:.4f} | {w:.3f} |")
    c, n = wcorr(v, "home_share", "res", "w")
    print(f"\nweighted corr(share of the lineup from the town, residual): {c:+.3f} over {n} results.")


# ---------------------------------------------------------------- lever 3


def lever3(db, rows):
    """How much the players have already played: in-person tournaments in the
    trailing 7 and 30 days, and whether the event itself runs over more than a day."""
    add_field(db, rows)
    buff = sqlite3.connect(f"file:{BUFF}?mode=ro", uri=True)
    played = {}
    for date, ms in buff.execute(
        "select substr(t.date_start,1,10), r.team_members from tournament_results r"
        " join tournaments t on t.id = r.id where t.in_rating = 1"
        " and t.tournament_type in ('Обычный','Региональный')"
        " and substr(t.date_start,1,10) >= '2012-01-01' and r.team_members is not null"
    ):
        for p in json.loads(ms) if ms else []:
            played.setdefault(p, []).append(date)
    for v in played.values():
        v.sort()
    span = {}
    for tid, a, b in buff.execute("select id, substr(date_start,1,10), substr(date_end,1,10) from tournaments"):
        span[tid] = 0 if not b or b <= a else (
            (int(b[:4]) - int(a[:4])) * 365 + (int(b[5:7]) - int(a[5:7])) * 31 + int(b[8:10]) - int(a[8:10])
        )
    import datetime as dt

    for r in rows:
        d = dt.date.fromisoformat(r["date"])
        w7, w30 = str(d - dt.timedelta(days=7)), str(d - dt.timedelta(days=30))
        a, b = [], []
        for p in r["members"]:
            h = played.get(p, [])
            hi = bisect.bisect_left(h, r["date"])
            a.append(hi - bisect.bisect_left(h, w7))
            b.append(hi - bisect.bisect_left(h, w30))
        r["load7"] = statistics.mean(a) if a else None
        r["load30"] = statistics.mean(b) if b else None
        r["span"] = span.get(r["tid"], 0)
    print("## Lever 3: how much the players have already played\n")
    print("| feature | weighted corr with the residual | results |")
    print("|---|---:|---:|")
    for lab, key in [("in-person tournaments per player, last 7 days", "load7"),
                     ("the same, last 30 days", "load30"),
                     ("days the event itself spans", "span")]:
        c, n = wcorr(rows, key, "res", "w")
        print(f"| {lab} | {c:+.3f} | {n} |")
    v = [r for r in rows if r["load7"] is not None]
    print("\n| mean in-person tournaments per player in the last 7 days | results | mean residual |")
    print("|---|---:|---:|")
    for lab, lo, hi in [("0", -0.01, 0.01), ("up to 1", 0.01, 1.001),
                        ("1 to 2", 1.001, 2.001), ("over 2", 2.001, 99)]:
        part = [r for r in v if lo <= r["load7"] < hi]
        m, se = mean_se([r["res"] for r in part])
        print(f"| {lab} | {len(part)} | {m:+.4f} ± {se:.4f} |")
    print("\n| days the event spans | results | mean residual |")
    print("|---|---:|---:|")
    for lab, sel in [("one", lambda r: r["span"] == 0), ("two", lambda r: r["span"] == 1),
                     ("three or more", lambda r: r["span"] >= 2)]:
        part = [r for r in rows if sel(r)]
        m, se = mean_se([r["res"] for r in part])
        print(f"| {lab} | {len(part)} | {m:+.4f} ± {se:.4f} |")


# ---------------------------------------------------------------- lever 4


def lever4(db, rows):
    """Rust in August, fatigue in May: is the residual a function of the month?"""
    add_field(db, rows)
    add_theta(rows)
    print("## Lever 4: the month of the season\n")
    print("| month | results | mean residual | mean theta of the lineup |")
    print("|---|---:|---:|---:|")
    for m in list(range(8, 13)) + list(range(1, 8)):
        part = [r for r in rows if int(r["date"][5:7]) == m]
        if not part:
            continue
        mu, se = mean_se([r["res"] for r in part])
        th = [r["mean_theta"] for r in part if r["mean_theta"] is not None]
        print(f"| {m:02d} | {len(part)} | {mu:+.4f} ± {se:.4f} | {statistics.mean(th):.3f} |")
    sp = [abs(mean_se([r["res"] for r in rows if int(r["date"][5:7]) == m])[0])
          for m in range(1, 13) if any(int(r["date"][5:7]) == m for r in rows)]
    se = [mean_se([r["res"] for r in rows if int(r["date"][5:7]) == m])[1]
          for m in range(1, 13) if any(int(r["date"][5:7]) == m for r in rows)]
    print(f"\nLargest monthly deviation {max(sp):.4f} against a typical standard error of "
          f"{statistics.median(se):.4f}.")


# ---------------------------------------------------------------- lever 5


def editors_of(tids):
    buff = sqlite3.connect(f"file:{BUFF}?mode=ro", uri=True)
    out = {}
    for tid, e in buff.execute("select id, editors from tournaments"):
        if tid in tids:
            try:
                out[tid] = json.loads(e) if e else []
            except (ValueError, TypeError):
                out[tid] = []
    return out


def lever5(db, rows):
    """The one live lever of round 3, and the three keys it was not tried on:
    the team instead of its players, the editors' pair instead of each editor,
    and a memory that forgets."""
    add_field(db, rows)
    add_theta(rows)
    eds = editors_of({r["tid"] for r in rows})
    rows.sort(key=lambda r: r["date"])
    import datetime as dt

    acc = {k: {} for k in ("pe", "te", "pp", "pe2", "pe4")}

    def read(store, keys, min_n, day=None, half=None):
        v = []
        for k in keys:
            a = store.get(k)
            if not a:
                continue
            s, c, last = a
            if half is not None:
                f = 0.5 ** ((day - last) / (365.25 * half))
                s, c = s * f, c * f
            if c >= min_n:
                v.append(s / c)
        return statistics.mean(v) if v else None

    def write(store, keys, res, day, half=None):
        for k in keys:
            a = store.setdefault(k, [0.0, 0.0, day])
            if half is not None:
                f = 0.5 ** ((day - a[2]) / (365.25 * half))
                a[0], a[1] = a[0] * f, a[1] * f
            a[0] += res
            a[1] += 1.0
            a[2] = day

    for r in rows:
        es = eds.get(r["tid"], [])
        day = dt.date.fromisoformat(r["date"]).toordinal()
        pe = [(p, e) for p in r["members"] for e in es]
        te = [(r["team"], e) for e in es]
        pp = [(p, tuple(sorted(es))) for p in r["members"]] if len(es) >= 2 else []
        r["m_pe"] = read(acc["pe"], pe, 2)
        r["m_te"] = read(acc["te"], te, 2)
        r["m_pp"] = read(acc["pp"], pp, 2)
        r["m_pe2"] = read(acc["pe2"], pe, 2, day, 2.0)
        r["m_pe4"] = read(acc["pe4"], pe, 2, day, 4.0)
        write(acc["pe"], pe, r["res"], day)
        write(acc["te"], te, r["res"], day)
        write(acc["pp"], pp, r["res"], day)
        write(acc["pe2"], pe, r["res"], day, 2.0)
        write(acc["pe4"], pe, r["res"], day, 4.0)
    v = [r for r in rows if r["mean_theta"] is not None]
    v.sort(key=lambda r: -r["mean_theta"])
    top5 = v[: len(v) // 5]
    top20 = v[: len(v) // 20]
    print("## Lever 5: the editor memory, on four keys\n")
    print("| key | all lineups | the strongest fifth | the strongest twentieth | results |")
    print("|---|---:|---:|---:|---:|")
    for lab, key in [
        ("player x editor (round 3's)", "m_pe"),
        ("team x editor", "m_te"),
        ("player x the editors' set", "m_pp"),
        ("player x editor, half-life 2 years", "m_pe2"),
        ("player x editor, half-life 4 years", "m_pe4"),
    ]:
        c, n = wcorr(rows, key, "res", "w")
        c5, _ = wcorr(top5, key, "res", "w")
        c20, _ = wcorr(top20, key, "res", "w")
        print(f"| {lab} | {c:+.3f} | {c5:+.3f} | {c20:+.3f} | {n} |")
    for lab, key in [("player x editor", "m_pe"), ("team x editor", "m_te"),
                     ("the editors' set", "m_pp")]:
        share = sum(1 for r in rows if r[key] is not None) / len(rows)
        print(f"\n{share:.1%} of the results have a {lab} memory at all.")


# ---------------------------------------------------------------- lever 6


def lever6(db, rows):
    """`sigma_c` is one number.  Should it be two -- one for a field whose top is
    bunched and one for a field with a clear favourite?"""
    add_field(db, rows)
    add_theta(rows)
    v = [r for r in rows if r["gap"] is not None and r["mean_sigma"] is not None]
    g = sorted({r["tid"]: r["gap"] for r in v}.values())
    cut = g[len(g) // 3], g[2 * len(g) // 3]
    print("## Lever 6: should the quantile depend on the field\n")
    print("| field, by the 1st-to-4th forecast gap | gap | results | corr(mean sigma, residual) |")
    print("|---|---:|---:|---:|")
    for lab, sel in [
        ("tight (lowest third)", lambda r: r["gap"] <= cut[0]),
        ("middle", lambda r: cut[0] < r["gap"] <= cut[1]),
        ("loose (highest third)", lambda r: r["gap"] > cut[1]),
    ]:
        part = [r for r in v if sel(r)]
        c, n = wcorr(part, "mean_sigma", "res", "w")
        gg = statistics.mean(r["gap"] for r in part)
        print(f"| {lab} | {gg:.0f} | {n} | {c:+.3f} |")
    print("\nThe same among the four highest forecasts of a field, where the quantile decides:\n")
    print("| field | results | corr(mean sigma, residual) | corr(mean sigma, 1/place) |")
    print("|---|---:|---:|---:|")
    for lab, sel in [
        ("tight", lambda r: r["gap"] <= cut[0]),
        ("middle", lambda r: cut[0] < r["gap"] <= cut[1]),
        ("loose", lambda r: r["gap"] > cut[1]),
    ]:
        part = [r for r in v if sel(r) and r["fc_rank"] <= 4]
        c, n = wcorr(part, "mean_sigma", "res", "w")
        c2, _ = wcorr(part, "mean_sigma", "rel", "w")
        print(f"| {lab} | {n} | {c:+.3f} | {c2:+.3f} |")


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
    rows = renorm(base_rows(db, *TUNE))
    print(f"# Round-4 mechanisms on the round-3 residual, {TUNE[0]} to {TUNE[1]}\n")
    print(f"{len(rows)} results in {len({r['tid'] for r in rows})} weighted in-person fields.\n")
    for k in levers or [1, 2, 3, 4, 5, 6]:
        {1: lever1, 2: lever2, 3: lever3, 4: lever4, 5: lever5, 6: lever6}[k](db, rows)
        print()


if __name__ == "__main__":
    main()
