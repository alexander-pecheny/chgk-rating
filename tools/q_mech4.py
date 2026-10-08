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
import datetime as dt
import json
import math
import statistics

import lab
from lab import wcorr
from q_mech import BUFF, TAU, TUNE, add_theta, base_rows, by_field, debias, editors, forecasts, softmax, terciles

TOWNS = "data/towns.csv"
TOP4 = 4
THIRDS = 3
DECAY = 0.5
TOP_FIELD = 10
MIN_PART = 30
FIFTHS = 5
TWENTIETH = 20
HOME_FROM = "2013-01-01"
LOAD_FROM = "2012-01-01"
LOAD_DAYS = (7, 30)
# Rough day counts for the span of a multi-day event.
YEAR_DAYS, MONTH_DAYS = 365, 31
DAYS_PER_YEAR = 365.25
HALF_LIVES = (2.0, 4.0)
SEASON = list(range(8, 13)) + list(range(1, 8))
MONTHS = range(1, 13)
MONTH = slice(5, 7)
OPTIONS = [
    ("db", dict()),
    ("--lever", dict(type=int, action="append", default=[], help="one lever; repeatable; all by default")),
]
MEAN_HEAD = ("| cohort | results | mean residual | won |", "|---|---:|---:|---:|")
HOME_RESULTS = """
select substr(t.date_start,1,10), t.id, r.team_id, r.team_current_town, r.team_members
from tournament_results r join tournaments t on t.id = r.id
where t.in_rating = 1 and substr(t.date_start,1,10) >= ?
and r.team_members is not null order by substr(t.date_start,1,10)
"""
IN_PERSON_LINEUPS = """
select substr(t.date_start,1,10), r.team_members from tournament_results r
join tournaments t on t.id = r.id where t.in_rating = 1
and t.tournament_type in ('Обычный','Региональный')
and substr(t.date_start,1,10) >= ? and r.team_members is not null
"""


def renorm(rows):
    """Re-derives the residual from *ranks on both sides*.

    `q_mech.base_rows` keeps only the results a field forecasts and then scores
    `(forecast rank - actual place) / n` against that shortened field, so a
    field of 120 teams of which 6 carry a lineup produces residuals near -19.
    Round 3 read those through weighted correlations and never printed a cohort
    mean; this round does, so the actual place is replaced by its rank among the
    rows kept, ties shared, and the forecast-bin means are subtracted again.
    """
    for rs in by_field(rows).values():
        n, best = len(rs), min(r["place"] for r in rs)
        for r, rank in zip(rs, lab.ranks([r["place"] for r in rs])):
            r["arank"], r["n"] = rank, n
            r["raw"] = (r["fc_rank"] - rank) / n
            r["rel"] = 1.0 / rank
            r["won"] = 1.0 if r["place"] == best else 0.0
    return debias(rows)


def mean_se(v):
    """Mean and its standard error; `nan` for fewer than two values."""
    if len(v) < 2:
        return lab.NAN, lab.NAN
    return statistics.mean(v), statistics.pstdev(v) / math.sqrt(len(v))


def mean_rows(cohorts, rows):
    """`| label | results | mean residual ± se | won |` per `(label, selector)`."""
    for lab_, sel in cohorts:
        part = [r for r in rows if sel(r)]
        m, se = mean_se([r["res"] for r in part])
        print(f"| {lab_} | {len(part)} | {m:+.4f} ± {se:.4f} | {mean_se([r['won'] for r in part])[0]:.3f} |")


def binned(rows, key, bins):
    """`| label | results | mean residual ± se |` per `(label, lo, hi)` of `key`."""
    for label, lo, hi in bins:
        part = [r for r in rows if lo <= r[key] < hi]
        m, se = mean_se([r["res"] for r in part])
        print(f"| {label} | {len(part)} | {m:+.4f} ± {se:.4f} |")


def field_stats(fc, rs):
    """The 1st-to-4th forecast gap and the mean forecast of the ten best, or None."""
    v = [x for x in sorted((fc.get(r["team"]) for r in rs), reverse=True) if x is not None]
    return (v[0] - v[TOP4 - 1], statistics.mean(v[:TOP_FIELD])) if len(v) >= TOP4 else None


def add_field(db, rows):
    """Per field: the 1st-to-4th forecast gap (tightness) and the mean forecast
    of its ten best lineups (strength).  Both in display points."""
    fc = forecasts(db)
    stats = {tid: field_stats(fc.get(tid, {}), rs) for tid, rs in by_field(rows).items()}
    for r in rows:
        r["fc"] = fc.get(r["tid"], {}).get(r["team"])
        r["gap"], r["strength"] = stats[r["tid"]] or (None, None)


def add_sd(rows, tau=TAU):
    """`sd(Q(L))`, the sd of the tempered roster estimate propagated from the
    players' sigma, on the theta scale.  The same quantity `roster_sd` computes."""
    add_theta(rows)
    for r in rows:
        wts = softmax(r["theta"], tau) if r["theta"] else None
        r["sd"] = math.sqrt(sum((x * b) ** 2 for x, b in zip(wts, r["sigma"]))) if wts else None


# ---------------------------------------------------------------- lever 1


def thirds(v):
    """The two cuts of a sorted list into thirds, and the selectors they make."""
    cut = v[len(v) // THIRDS], v[2 * len(v) // THIRDS]
    return cut, [
        lambda r: r["gap"] <= cut[0],
        lambda r: cut[0] < r["gap"] <= cut[1],
        lambda r: r["gap"] > cut[1],
    ]


LEVER1_FIELDS = ["tight (1st-to-4th gap in the lowest third)", "middle", "loose (the highest third)"]


def spread_rows(part):
    """Narrow and wide lineups of one third of the fields: won, 1/place, residual."""
    s = sorted(r["sd"] for r in part)
    med = s[len(s) // 2]
    for slab, q in (("narrow", [r for r in part if r["sd"] <= med]), ("wide", [r for r in part if r["sd"] > med])):
        if q:
            w, sw = mean_se([r["won"] for r in q])
            rl, rs = mean_se([r["rel"] for r in q])[0], mean_se([r["res"] for r in q])[0]
            yield slab, len(q), f"{w:.3f} ± {sw:.3f} | {rl:.3f} | {rs:+.4f}"


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
    top = [r for r in v if r["fc_rank"] <= TOP4 and r["n"] >= TOP_FIELD]
    tight = sorted({r["tid"]: r["gap"] for r in top}.values())
    if tight:
        lever1_tables(v, top, *thirds(tight))


def lever1_tables(v, top, cut, sels):
    lab.header("| field | lineup spread | results | won | 1/place | mean residual |", "|---|---|---:|---:|---:|---:|")
    for flab, fsel in zip(LEVER1_FIELDS, sels):
        part = [r for r in top if fsel(r)]
        for slab, n, cells in spread_rows(part) if len(part) >= MIN_PART else ():
            print(f"| {flab} | {slab} | {n} | {cells} |")
    for lab_, sel in [
        ("the four highest forecasts", lambda r: r["fc_rank"] <= TOP4),
        ("the same, in the tightest third of fields", lambda r: r["fc_rank"] <= TOP4 and r["gap"] <= cut[0]),
        ("every result", lambda r: True),
    ]:
        part = [r for r in v if sel(r)]
        (c, n), (c2, _) = wcorr(part, "sd", "rel", "w"), wcorr(part, "sd", "res", "w")
        print(f"\ncorr(sd, 1/place) over {lab_}: {c:+.3f}; corr(sd, residual) {c2:+.3f}; {n} results.")


# ---------------------------------------------------------------- lever 2


def town_map():
    with open(TOWNS, encoding="utf-8") as f:
        return {int(r["town_id"]): r["name"] for r in csv.DictReader(f)}


def player_towns(rows, buff):
    """Yields each row with `{player: {town: results}}` as of that morning, so
    nothing in the field enters its own forecast."""
    hist = [(date, town, json.loads(ms) if ms else []) for date, _, _, town, ms in buff.execute(HOME_RESULTS, (HOME_FROM,))]
    cnt, k = {}, 0
    for r in rows:
        while k < len(hist) and hist[k][0] < r["date"]:
            _, town, ms = hist[k]
            for p in ms if town else ():
                d = cnt.setdefault(p, {})
                d[town] = d.get(town, 0) + 1
            k += 1
        yield r, cnt


def add_home(rows):
    """`home_team`: the team's own town is the tournament's; `home_share`: the
    share of the lineup whose most frequent town it is."""
    tm, buff = town_map(), lab.connect(BUFF)
    ttown = {tid: tm.get(t) for tid, t in buff.execute("select id, town_id from tournaments")}
    rtown = {(tid, team): town for tid, team, town in buff.execute("select id, team_id, team_current_town from tournament_results")}
    rows.sort(key=lambda r: r["date"])
    for r, cnt in player_towns(rows, buff):
        home = ttown.get(r["tid"])
        r["home_team"] = None if home is None else float(rtown.get((r["tid"], r["team"])) == home)
        share = [1.0 if max(cnt[p], key=cnt[p].get) == home else 0.0 for p in r["members"] if cnt.get(p)]
        r["home_share"] = statistics.mean(share) if share and home else None


HOME_COHORTS = [
    ("the team's own town, at home", lambda r, top: r["home_team"] > 0.5),
    ("the team's own town, away", lambda r, top: r["home_team"] < 0.5),
    ("at home, in the strongest fifth of fields", lambda r, top: r["home_team"] > 0.5 and r["tid"] in top),
    ("away, in the strongest fifth of fields", lambda r, top: r["home_team"] < 0.5 and r["tid"] in top),
    ("at home, among the four highest forecasts", lambda r, top: r["home_team"] > 0.5 and r["fc_rank"] <= TOP4),
    ("away, among the four highest forecasts", lambda r, top: r["home_team"] < 0.5 and r["fc_rank"] <= TOP4),
]
SHARE_BINS = [("none", -0.01, 0.01), ("under a half", 0.01, 0.5), ("a half or more", 0.5, 0.999), ("all of it", 0.999, 1.01)]


def lever2(db, rows):
    """A team playing at home against one that travelled.  Two readings of
    "home": the town the team is registered in, and the towns its players have
    played out of before today."""
    add_field(db, rows)
    add_home(rows)
    print("## Lever 2: home and away\n")
    have = [r for r in rows if r["home_team"] is not None]
    print(f"{len(have)} of {len(rows)} results have a town on both sides; "
          f"{statistics.mean(r['home_team'] for r in have):.1%} of them are at home.\n")
    lab.header(*MEAN_HEAD)
    strong = sorted({r["tid"]: r["strength"] for r in have if r["strength"]}.items(), key=lambda kv: -kv[1])
    top = {t for t, _ in strong[: max(1, len(strong) // FIFTHS)]}
    mean_rows([(label, lambda r, s=sel: s(r, top)) for label, sel in HOME_COHORTS], have)
    lever2_share(rows)


def lever2_share(rows):
    v = [r for r in rows if r["home_share"] is not None]
    print(f"\n{len(v)} results have a town for at least one player. By the share of the")
    print("lineup that has played out of the tournament's own town before:\n")
    lab.header("| share of the lineup from the town | results | mean residual | won |", "|---|---:|---:|---:|")
    mean_rows([(label, lambda r, lo=lo, hi=hi: lo <= r["home_share"] < hi) for label, lo, hi in SHARE_BINS], v)
    c, n = wcorr(v, "home_share", "res", "w")
    print(f"\nweighted corr(share of the lineup from the town, residual): {c:+.3f} over {n} results.")


# ---------------------------------------------------------------- lever 3


def played(buff):
    """`{player: [dates]}` of rated in-person results."""
    out = {}
    for date, ms in buff.execute(IN_PERSON_LINEUPS, (LOAD_FROM,)):
        for p in json.loads(ms) if ms else []:
            out.setdefault(p, []).append(date)
    for v in out.values():
        v.sort()
    return out


def span_days(a, b):
    """Days an event spans, by the rough calendar the round used."""
    if not b or b <= a:
        return 0
    return (int(b[:4]) - int(a[:4])) * YEAR_DAYS + (int(b[MONTH]) - int(a[MONTH])) * MONTH_DAYS + int(b[8:10]) - int(a[8:10])


def add_load(rows):
    """In-person tournaments per player in the trailing 7 and 30 days, and the
    event's own span."""
    buff = lab.connect(BUFF)
    pl = played(buff)
    span = {tid: span_days(a, b) for tid, a, b in buff.execute(
        "select id, substr(date_start,1,10), substr(date_end,1,10) from tournaments")}
    for r in rows:
        d = dt.date.fromisoformat(r["date"])
        hi = [bisect.bisect_left(pl.get(p, []), r["date"]) for p in r["members"]]
        for key, days in zip(("load7", "load30"), LOAD_DAYS):
            lo = str(d - dt.timedelta(days=days))
            v = [h - bisect.bisect_left(pl.get(p, []), lo) for p, h in zip(r["members"], hi)]
            r[key] = statistics.mean(v) if v else None
        r["span"] = span.get(r["tid"], 0)


LOAD_BINS = [("0", -0.01, 0.01), ("up to 1", 0.01, 1.001), ("1 to 2", 1.001, 2.001), ("over 2", 2.001, 99)]
SPAN_BINS = [("one", 0, 1), ("two", 1, 2), ("three or more", 2, math.inf)]


def lever3(db, rows):
    """How much the players have already played: in-person tournaments in the
    trailing 7 and 30 days, and whether the event itself runs over more than a day."""
    add_field(db, rows)
    add_load(rows)
    print("## Lever 3: how much the players have already played\n")
    lab.header("| feature | weighted corr with the residual | results |", "|---|---:|---:|")
    for label, key in [("in-person tournaments per player, last 7 days", "load7"),
                       ("the same, last 30 days", "load30"), ("days the event itself spans", "span")]:
        c, n = wcorr(rows, key, "res", "w")
        print(f"| {label} | {c:+.3f} | {n} |")
    print()
    lab.header("| mean in-person tournaments per player in the last 7 days | results | mean residual |", "|---|---:|---:|")
    binned([r for r in rows if r["load7"] is not None], "load7", LOAD_BINS)
    print()
    lab.header("| days the event spans | results | mean residual |", "|---|---:|---:|")
    binned(rows, "span", SPAN_BINS)


# ---------------------------------------------------------------- lever 4


def lever4(db, rows):
    """Rust in August, fatigue in May: is the residual a function of the month?"""
    add_field(db, rows)
    add_theta(rows)
    month = {m: [r for r in rows if int(r["date"][MONTH]) == m] for m in MONTHS}
    print("## Lever 4: the month of the season\n")
    lab.header("| month | results | mean residual | mean theta of the lineup |", "|---|---:|---:|---:|")
    for m in SEASON:
        if month[m]:
            mu, se = mean_se([r["res"] for r in month[m]])
            th = [r["mean_theta"] for r in month[m] if r["mean_theta"] is not None]
            print(f"| {m:02d} | {len(month[m])} | {mu:+.4f} ± {se:.4f} | {statistics.mean(th):.3f} |")
    stats = [mean_se([r["res"] for r in month[m]]) for m in MONTHS if month[m]]
    print(f"\nLargest monthly deviation {max(abs(m) for m, _ in stats):.4f} against a typical standard error of "
          f"{statistics.median(se for _, se in stats):.4f}.")


# ---------------------------------------------------------------- lever 5


class Memory:
    """A running mean of residuals per key, optionally forgetting with a half-life in years."""

    def __init__(self, half=None):
        self.store, self.half = {}, half

    def fade(self, day, last):
        return DECAY ** ((day - last) / (DAYS_PER_YEAR * self.half)) if self.half is not None else 1.0

    def read(self, keys, day, min_n=2):
        v = []
        for s, c, last in (self.store[k] for k in keys if self.store.get(k)):
            f = self.fade(day, last)
            if c * f >= min_n:
                v.append((s * f) / (c * f))
        return statistics.mean(v) if v else None

    def write(self, keys, res, day):
        for k in keys:
            a = self.store.setdefault(k, [0.0, 0.0, day])
            f = self.fade(day, a[2])
            a[:] = [a[0] * f + res, a[1] * f + 1.0, day]


MEMORY_KEYS = [
    ("player x editor (round 3's)", "m_pe"), ("team x editor", "m_te"), ("player x the editors' set", "m_pp"),
    ("player x editor, half-life 2 years", "m_pe2"), ("player x editor, half-life 4 years", "m_pe4"),
]


def memory_keys(r, es):
    pe = [(p, e) for p in r["members"] for e in es]
    pp = [(p, tuple(sorted(es))) for p in r["members"]] if len(es) >= 2 else []
    return {"m_pe": pe, "m_te": [(r["team"], e) for e in es], "m_pp": pp, "m_pe2": pe, "m_pe4": pe}


def add_memories(rows):
    """The editor memory on its four keys, each read before the result is written."""
    eds = editors()
    mem = {"m_pe": Memory(), "m_te": Memory(), "m_pp": Memory(),
           "m_pe2": Memory(HALF_LIVES[0]), "m_pe4": Memory(HALF_LIVES[1])}
    rows.sort(key=lambda r: r["date"])
    for r in rows:
        day, keys = dt.date.fromisoformat(r["date"]).toordinal(), memory_keys(r, eds.get(r["tid"], []))
        for name, m in mem.items():
            r[name] = m.read(keys[name], day)
        for name, m in mem.items():
            m.write(keys[name], r["res"], day)


def lever5(db, rows):
    """The one live lever of round 3, and the three keys it was not tried on:
    the team instead of its players, the editors' pair instead of each editor,
    and a memory that forgets."""
    add_field(db, rows)
    add_theta(rows)
    add_memories(rows)
    v = sorted((r for r in rows if r["mean_theta"] is not None), key=lambda r: -r["mean_theta"])
    top5, top20 = v[: len(v) // FIFTHS], v[: len(v) // TWENTIETH]
    print("## Lever 5: the editor memory, on four keys\n")
    lab.header("| key | all lineups | the strongest fifth | the strongest twentieth | results |", "|---|---:|---:|---:|---:|")
    for label, key in MEMORY_KEYS:
        (c, n), (c5, _), (c20, _) = (wcorr(part, key, "res", "w") for part in (rows, top5, top20))
        print(f"| {label} | {c:+.3f} | {c5:+.3f} | {c20:+.3f} | {n} |")
    for label, key in [("player x editor", "m_pe"), ("team x editor", "m_te"), ("the editors' set", "m_pp")]:
        share = sum(1 for r in rows if r[key] is not None) / len(rows)
        print(f"\n{share:.1%} of the results have a {label} memory at all.")


# ---------------------------------------------------------------- lever 6


LEVER6_FIELDS = (["tight (lowest third)", "middle", "loose (highest third)"], ["tight", "middle", "loose"])


def lever6(db, rows):
    """`sigma_c` is one number.  Should it be two -- one for a field whose top is
    bunched and one for a field with a clear favourite?"""
    add_field(db, rows)
    add_theta(rows)
    v = [r for r in rows if r["gap"] is not None and r["mean_sigma"] is not None]
    _, sels = thirds(sorted({r["tid"]: r["gap"] for r in v}.values()))
    print("## Lever 6: should the quantile depend on the field\n")
    lab.header("| field, by the 1st-to-4th forecast gap | gap | results | corr(mean sigma, residual) |", "|---|---:|---:|---:|")
    for label, sel in zip(LEVER6_FIELDS[0], sels):
        part = [r for r in v if sel(r)]
        c, n = wcorr(part, "mean_sigma", "res", "w")
        print(f"| {label} | {statistics.mean(r['gap'] for r in part):.0f} | {n} | {c:+.3f} |")
    print("\nThe same among the four highest forecasts of a field, where the quantile decides:\n")
    lab.header("| field | results | corr(mean sigma, residual) | corr(mean sigma, 1/place) |", "|---|---:|---:|---:|")
    for label, sel in zip(LEVER6_FIELDS[1], sels):
        part = [r for r in v if sel(r) and r["fc_rank"] <= TOP4]
        (c, n), (c2, _) = wcorr(part, "mean_sigma", "res", "w"), wcorr(part, "mean_sigma", "rel", "w")
        print(f"| {label} | {n} | {c:+.3f} | {c2:+.3f} |")


LEVERS = {1: lever1, 2: lever2, 3: lever3, 4: lever4, 5: lever5, 6: lever6}


def main():
    a = lab.cli(__doc__, OPTIONS)
    rows = renorm(base_rows(a.db, *TUNE))
    print(f"# Round-4 mechanisms on the round-3 residual, {TUNE[0]} to {TUNE[1]}\n")
    print(f"{len(rows)} results in {len({r['tid'] for r in rows})} weighted in-person fields.\n")
    for k in a.lever or LEVERS:
        LEVERS[k](a.db, rows)
        print()


if __name__ == "__main__":
    main()
