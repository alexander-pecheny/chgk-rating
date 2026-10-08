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

import bisect
import json
import math
import statistics

import lab
from lab import wcorr

BUFF = lab.BUFF
TUNE = lab.TUNE
# Forecast-percentile bins of the residual, in twentieths.
BINS = 20
# A rank as a percentile: the middle of its slot.
MID = 0.5
THIRDS = 3
# Lever 7 cuts the lineups at two and four fifths.
TWO = 2
FOUR = 4
# The tempered roster estimate of the engine, and the theta-to-display scale.
TAU = 0.7
SCALE = 2454.8828
# Lever 1 counts pairs from here on; lever 3 looks back this many years.
PAIRS_FROM = "2013-01-01"
UNRATED_FROM = "2016-01-01"
UNRATED_YEARS = 2
# Lever 5's fields of ten or more, in deciles; lever 6's memory and cohorts.
MIN_FIELD5 = 10
DECILES = 10
MEMORY = 5
HISTORY_CUTS = (5, 10, 20)
TOP_FORECASTS = 5
FIELD_COLS = "e.tournament_id, e.team_id, e.actual_place, e.forecast, m.date"
RATED_LINEUPS = """
select r.team_members from tournament_results r join tournaments t on t.id = r.id
where t.in_rating = 1 and substr(t.date_start,1,10) >= ?
and substr(t.date_start,1,10) < ? and r.team_members is not null
"""
UNRATED_LINEUPS = """
select substr(t.date_start,1,10), r.team_members from tournament_results r
join tournaments t on t.id = r.id where t.in_rating = 0
and t.tournament_type in ('Обычный','Региональный','Синхрон','Строго синхронный')
and substr(t.date_start,1,10) >= ? and r.team_members is not null
"""
SURPRISES = """
select e.tournament_id, e.team_id, e.forecast_place, e.actual_place, m.date, m.n_teams
from team_events e join tournament_metrics m on m.tournament_id = e.tournament_id
where m.type = 'in-person' and e.actual_place > 0 and e.forecast is not null
order by m.date, e.tournament_id
"""
CORR_HEAD = ("| feature | weighted corr with the residual | results |", "|---|---:|---:|")
OPTIONS = [
    ("db", dict()),
    ("--lever", dict(type=int, action="append", default=[], help="one lever; repeatable; all by default")),
]


def corr_table(rows, features, head=CORR_HEAD):
    """One weighted correlation with the residual per `(label, key)`."""
    lab.header(*head)
    for name, key in features:
        c, n = wcorr(rows, key, "res", "w")
        print(f"| {name} | {c:+.3f} | {n} |")


def by_field(rows):
    out = {}
    for r in rows:
        out.setdefault(r["tid"], []).append(r)
    return out


def fields(db, frm, to):
    """`{tournament: {"date", "rows": [(team, place, forecast)]}}`, five teams or more."""
    by_t = {}
    for tid, team, place, fc, date in lab.in_person(
        db, frm, to, FIELD_COLS, where="  and e.forecast is not null", order="m.date, e.tournament_id"
    ):
        by_t.setdefault(tid, {"date": date, "rows": []})["rows"].append((team, place, fc))
    return {t: v for t, v in by_t.items() if len(v["rows"]) >= lab.MIN_FIELD}


def field_rows(tid, v, wt, members, st):
    """One row per result of a field, forecast order."""
    rs = sorted(v["rows"], key=lambda r: -r[2])
    n = len(rs)
    return [
        {"tid": tid, "team": team, "date": v["date"], "w": wt, "n": n, "fc_rank": k + 1,
         "pct": (k + MID) / n, "place": place, "raw": (k + 1 - place) / n,
         "members": members.get(team, []), "st": st.get(tid, {})}
        for k, (team, place, fc) in enumerate(rs)
    ]


def debias(rows):
    """Subtract the mean residual of each forecast-percentile bin."""
    bins = {}
    for r in rows:
        bins.setdefault(int(r["pct"] * BINS), []).append(r["raw"])
    mean = {k: sum(v) / len(v) for k, v in bins.items()}
    for r in rows:
        r["res"] = r["raw"] - mean[int(r["pct"] * BINS)]
    return rows


def base_rows(db, frm, to):
    """One row per result: tournament, team, residual net of the forecast bin,
    the field's weight, the lineup, and the date."""
    by_t = fields(db, frm, to)
    w = lab.load_weights()
    buff = lab.connect(BUFF)
    members = {tid: lab.lineups(buff, tid) for tid in by_t}
    st = lab.player_state(lab.connect(db), by_t)
    out = []
    for tid, v in by_t.items():
        if w.get(tid, 0.0) > 0:
            out += field_rows(tid, v, w[tid], members[tid], st)
    return debias(out)


def add_theta(rows):
    """Each lineup's theta (with phi), sigma, and their mean and spread."""
    for r in rows:
        known = [r["st"][p] for p in r["members"] if p in r["st"]]
        th = r["theta"] = [s[0] + s[1] for s in known]
        sg = r["sigma"] = [s[2] for s in known]
        r["mean_theta"] = statistics.mean(th) if th else None
        r["spread"] = statistics.pstdev(th) if len(th) > 1 else None
        r["mean_sigma"] = statistics.mean(sg) if sg else None


def pairs(m):
    """Every unordered pair of a lineup, smaller id first."""
    return [(min(m[a], m[b]), max(m[a], m[b])) for a in range(len(m)) for b in range(a + 1, len(m))]


def lineups_of(cur):
    for (ms,) in cur:
        yield json.loads(ms) if ms else []


# ---------------------------------------------------------------- lever 1


class PairCounter:
    """`{(i,j): games together}` over rated results, advanced day by day, so the
    table read for a field is the state on that morning."""

    def __init__(self):
        self.cnt, self.seen_to = {}, PAIRS_FROM
        self.buff = lab.connect(BUFF)

    def advance(self, to):
        for m in lineups_of(self.buff.execute(RATED_LINEUPS, (self.seen_to, to))):
            for k in pairs(m):
                self.cnt[k] = self.cnt.get(k, 0) + 1
        self.seen_to = to

    def together(self, m):
        return [math.log1p(self.cnt.get(k, 0)) for k in pairs(m)]


LEVER1_COHORTS = [
    ("forecast 1-3, finished 1-3", lambda r: r["fc_rank"] <= 3 and r["place"] <= 3),
    ("forecast 1-3, finished 4+", lambda r: r["fc_rank"] <= 3 and r["place"] > 3),
    ("forecast 4-10, finished 1", lambda r: 4 <= r["fc_rank"] <= 10 and r["place"] <= 1),
    ("forecast 4-10, finished 2-3", lambda r: 4 <= r["fc_rank"] <= 10 and 2 <= r["place"] <= 3),
    ("forecast 4-10, finished 4+", lambda r: 4 <= r["fc_rank"] <= 10 and r["place"] > 3),
]


def lever1(rows):
    """Togetherness `T(L)`, the mean over pairs of `log(1 + games together)`,
    counted over everything before the field's own date."""
    rows.sort(key=lambda r: r["date"])
    pc = PairCounter()
    for r in rows:
        if r["date"] > pc.seen_to:
            pc.advance(r["date"])
        ts = pc.together(r["members"])
        r["T"], r["Tmax"] = (statistics.mean(ts), max(ts)) if ts else (None, None)
    print("## Lever 1: chemistry, games played together\n")
    corr_table(rows, [("T(L), mean over pairs of log(1+games together)", "T"), ("the same, largest pair", "Tmax")])
    strong = [r for r in rows if r["fc_rank"] <= 10 and r["n"] >= 20]
    print("\nAmong the ten teams each field's forecast puts highest, in fields of 20+:\n")
    lab.header("| cohort | results | mean T(L) |", "|---|---:|---:|")
    for label, sel in LEVER1_COHORTS:
        v = [r["T"] for r in strong if sel(r) and r["T"] is not None]
        if v:
            print(f"| {label} | {len(v)} | {statistics.mean(v):.2f} |")


# ---------------------------------------------------------------- lever 2


def terciles(v):
    k = len(v) // THIRDS
    return [v[:k], v[k : 2 * k], v[2 * k :]]


def lever2(db, rows):
    """Does the ranking of a lopsided lineup against a balanced one depend on
    how hard the package is?  If it does, a global `m` has something to fit."""
    add_theta(rows)
    hard = dict(lab.connect(db).execute("select tournament_id, truedl from tournament_metrics"))
    for r in rows:
        r["hard"] = hard.get(r["tid"])
    v = sorted((r for r in rows if r["hard"] is not None and r["spread"] is not None), key=lambda r: r["hard"])
    print("## Lever 2: does the package's difficulty change what a lopsided lineup is worth\n")
    c, n = wcorr(rows, "spread", "res", "w")
    print(f"Spread of theta against the residual, all fields: {c:+.3f} over {n} results.\n")
    lab.header("| tercile of the field's difficulty | truedl | results | corr(spread of theta, residual) |",
               "|---|---:|---:|---:|")
    for i, part in enumerate(terciles(v)):
        c, n = wcorr(part, "spread", "res", "w")
        print(f"| {i + 1} | {statistics.mean(r['hard'] for r in part):.2f} | {n} | {c:+.3f} |")


# ---------------------------------------------------------------- lever 3


def unrated_history():
    """`{player: [dates]}` of unrated in-person and synchronous results."""
    hist = {}
    for date, ms in lab.connect(BUFF).execute(UNRATED_LINEUPS, (UNRATED_FROM,)):
        for p in json.loads(ms) if ms else []:
            hist.setdefault(p, []).append(date)
    for v in hist.values():
        v.sort()
    return hist


def lever3(rows):
    """How much of a lineup's record the rated window cannot see: unrated
    in-person and synchronous tournaments its players played in the two years
    before the field."""
    hist = unrated_history()
    for r in rows:
        d = r["date"]
        lo = f"{int(d[:4]) - UNRATED_YEARS}{d[4:]}"
        v = [bisect.bisect_left(hist.get(p, []), d) - bisect.bisect_left(hist.get(p, []), lo) for p in r["members"]]
        r["unrated"], r["unrated_mean"] = (statistics.median(v), statistics.mean(v)) if v else (None, None)
    print("## Lever 3: the record the rated window cannot see\n")
    corr_table(rows, [("median unrated tournaments per player, last 2 years", "unrated"),
                      ("the mean of the same", "unrated_mean")])
    v = [r["unrated_mean"] for r in rows if r["unrated_mean"] is not None]
    share = sum(1 for x in v if x > 0) / max(len(v), 1)
    print(f"\n{share:.1%} of the lineups have a player with at least one unrated "
          f"tournament in the window; the mean count is {statistics.mean(v):.2f}.")


# ---------------------------------------------------------------- lever 4


def softmax(th, tau):
    """Softmax weights of the tempered roster estimate, on the theta scale."""
    m = max(th)
    e = [math.exp((x - m) / tau) for x in th]
    s = sum(e)
    return [x / s for x in e]


def lever4(rows, tau=TAU, scale=SCALE):
    """The sd of the roster estimate propagated from the players' sigma, against
    the mean sigma round 1 already subtracts."""
    add_theta(rows)
    for r in rows:
        wts = softmax(r["theta"], tau) if r["theta"] else None
        r["sdQ"] = math.sqrt(sum((a * b) ** 2 for a, b in zip(wts, r["sigma"]))) if wts else None
        if wts:
            r["maxw"] = max(wts)
    print("## Lever 4: the lineup's own uncertainty against the mean of its players'\n")
    corr_table(rows, [("mean sigma of the lineup", "mean_sigma"),
                      ("sd(Q(L)), propagated through the tempered roster estimate", "sdQ")])
    v = [(r["mean_sigma"], r["sdQ"]) for r in rows if r["sdQ"] is not None and r["mean_sigma"]]
    print(f"\nsd(Q(L)) is {statistics.mean(b / a for a, b in v):.2f} of the mean sigma on "
          f"average, and the two correlate at "
          f"{statistics.correlation([a for a, _ in v], [b for _, b in v]):.3f}: "
          "they are close to the same number up to a scale.")


# ---------------------------------------------------------------- lever 5


def forecasts(db):
    """`{tournament: {team: forecast}}`, the display-point forecast per result."""
    out = {}
    q = "select tournament_id, team_id, forecast from team_events where forecast is not null"
    for tid, team, f in lab.connect(db).execute(q):
        out.setdefault(tid, {})[team] = f
    return out


def favourites(db, rows):
    """`(gap, the favourite won, weight)` of every field of ten or more."""
    fc = forecasts(db)
    fs = []
    for tid, rs in by_field(rows).items():
        if len(rs) >= MIN_FIELD5:
            rs = sorted(rs, key=lambda r: r["fc_rank"])
            g = fc[tid][rs[0]["team"]] - fc[tid][rs[1]["team"]]
            fs.append((g, 1.0 if rs[0]["place"] <= min(r["place"] for r in rs) else 0.0, rs[0]["w"]))
    return sorted(fs)


def lever5(db, rows):
    """Is the forecast's spread at the top of an in-person field too wide or too
    narrow?  If a gap of g points buys less than the model thinks, `a` is too
    large; if more, too small."""
    print("## Lever 5: how much a point of forecast gap is worth in person\n")
    lab.header("| decile of the 1st-to-2nd forecast gap | gap (points) | fields | P(the favourite wins) |",
               "|---|---:|---:|---:|")
    fs = favourites(db, rows)
    n = len(fs)
    for q in range(DECILES):
        part = fs[q * n // DECILES : (q + 1) * n // DECILES]
        print(f"| {q + 1} | {statistics.mean(p[0] for p in part):.0f} | {len(part)} | "
              f"{sum(p[2] * p[1] for p in part) / sum(p[2] for p in part):.3f} |")


# ---------------------------------------------------------------- lever 6


def surprises(db):
    """`{team: [(date, surprise)]}` over every in-person result, in date order."""
    hist = {}
    for _, team, fp, ap, date, nt in lab.connect(db).execute(SURPRISES):
        hist.setdefault(team, []).append((date, (fp - ap) / max(nt, 1)))
    return hist


def lever6(db, rows, k=MEMORY):
    """A team's mean surprise over its last k in-person results against its
    residual today.  Surprise is `(forecast place - actual place) / n`, the same
    quantity as the residual, so this is simply its autocorrelation per team."""
    hist = surprises(db)
    for r in rows:
        h = [s for d, s in hist.get(r["team"], []) if d < r["date"]]
        r["S"], r["Sn"] = statistics.mean(h[-k:]) if len(h) >= 2 else None, len(h)
    print(f"## Lever 6: series memory, the mean surprise over the last {k} in-person results\n")
    lab.header(*CORR_HEAD)
    cohorts = [(f"mean of the last {k} surprises", rows)]
    cohorts += [(f"the same, teams with {lo}+ in-person results behind them", [r for r in rows if r["Sn"] >= lo])
                for lo in HISTORY_CUTS]
    cohorts.append(("the same, the five highest forecasts of a field", [r for r in rows if r["fc_rank"] <= TOP_FORECASTS]))
    for label, v in cohorts:
        c, n = wcorr(v, "S", "res", "w")
        print(f"| {label} | {c:+.3f} | {n} |")


# ---------------------------------------------------------------- lever 7


def editors():
    """`{tournament: [editor ids]}`, empty where the mirror's list does not parse."""
    eds = {}
    for tid, e in lab.connect(BUFF).execute("select id, editors from tournaments"):
        try:
            eds[tid] = json.loads(e) if e else []
        except (ValueError, TypeError):
            eds[tid] = []
    return eds


def editor_memory(rows, eds):
    """Per (player, editor), the running mean of the residuals of the results
    that player's lineup had under that editor, strictly before today."""
    acc = {}
    for r in rows:
        keys = [(p, e) for p in r["members"] for e in eds.get(r["tid"], [])]
        v = [acc[k][0] / acc[k][1] for k in keys if k in acc and acc[k][1] >= 2]
        r["ed_past"], r["ed_n"] = statistics.mean(v) if v else None, len(v)
        for k in keys:
            a = acc.setdefault(k, [0.0, 0])
            a[0], a[1] = a[0] + r["res"], a[1] + 1


LEVER7_PARTS = ["the weakest two fifths by mean theta", "the middle two fifths", "the strongest fifth"]
TWENTIETH = 20
FIFTHS = 5


def lever7(db, rows):
    """Does a lineup's past residual under the same editors predict its residual
    here, and is that link stronger for strong lineups?"""
    add_theta(rows)
    rows.sort(key=lambda r: r["date"])
    editor_memory(rows, editors())
    print("## Lever 7: editor affinity, and whether it is a property of strong players\n")
    lab.header("| cohort | weighted corr(mean past residual under these editors, residual) | results |",
               "|---|---:|---:|")
    v = sorted((r for r in rows if r["mean_theta"] is not None and r["ed_past"] is not None),
               key=lambda r: r["mean_theta"])
    m = len(v) // FIFTHS
    cohorts = [("every lineup with any history under the editors", rows)]
    cohorts += zip(LEVER7_PARTS, [v[: TWO * m], v[TWO * m : FOUR * m], v[FOUR * m :]])
    cohorts.append(("the strongest twentieth", sorted(v, key=lambda r: -r["mean_theta"])[: len(v) // TWENTIETH]))
    for label, part in cohorts:
        c, n = wcorr(part, "ed_past", "res", "w")
        print(f"| {label} | {c:+.3f} | {n} |")
    print(f"\n{sum(1 for r in rows if r['ed_n'] > 0) / len(rows):.1%} of the results have any "
          "history under the tournament's editors at all.")


LEVERS = {1: lambda db, rows: lever1(rows), 2: lever2, 3: lambda db, rows: lever3(rows),
          4: lambda db, rows: lever4(rows), 5: lever5, 6: lever6, 7: lever7}


def main():
    a = lab.cli(__doc__, OPTIONS)
    rows = base_rows(a.db, *TUNE)
    print(f"# Round-3 mechanisms on the round-2 residual, {TUNE[0]} to {TUNE[1]}\n")
    print(f"{len(rows)} results in {len({r['tid'] for r in rows})} weighted in-person fields.\n")
    for k in a.lever or LEVERS:
        LEVERS[k](a.db, rows)
        print()


if __name__ == "__main__":
    main()
