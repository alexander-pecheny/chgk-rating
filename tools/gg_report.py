# /// script
# requires-python = ">=3.10"
# dependencies = ["httpx"]
# ///
"""Fidelity check for src/bin/ext_gg.rs against rating.chgk.gg itself.

Oracle values are fetched live from the gg API (buff.db's mirror of them lags on
recent releases) and cached under data/external/chgk_gg/oracle/. Writes the numeric
half of out/ext/gg/report.md, to out/ext/gg/numbers.md, and prints it; the prose
half is written by hand.

    uv run tools/gg_report.py [out/ext/gg.db]
"""

import json
import pathlib
import sqlite3
import statistics
import sys
import time

import httpx

ROOT = pathlib.Path(__file__).resolve().parent.parent
EXT = ROOT / "data/external/chgk_gg"
CACHE = EXT / "oracle"
CACHE.mkdir(exist_ok=True)
DB = sys.argv[1] if len(sys.argv) > 1 else str(ROOT / "out/ext/gg.db")
UA = "chgk-rating-research/1.0"

client = httpx.Client(headers={"User-Agent": UA}, timeout=120.0)
_last = [0.0]


def api(path, name):
    f = CACHE / name
    if f.exists():
        return json.loads(f.read_text())
    gap = 0.5 - (time.monotonic() - _last[0])
    if gap > 0:
        time.sleep(gap)
    _last[0] = time.monotonic()
    r = client.get(f"https://rating.chgk.gg/api/v1/b/{path}")
    r.raise_for_status()
    f.write_text(r.text)
    return r.json()


rel = {r["date"]: r for r in json.load((EXT / "releases_b.json").open())["items"]}
con = sqlite3.connect(DB)
buff = sqlite3.connect(ROOT / "buff.db")
out = []
W = out.append

mine_dates = [r[0] for r in con.execute("select date from releases order by date")]
W(f"Reimplementation covers {len(mine_dates)} releases, {mine_dates[0]} to {mine_dates[-1]}.\n")

# (a) team ratings at releases spread over the run
picks = [d for d in ("2021-09-16", "2022-09-15", "2023-09-14", "2024-09-12", "2025-09-11", "2026-05-14", "2026-09-10") if d in rel and d in mine_dates]
W("## (a) Team ratings against the official release\n")
W("| release | teams both | exact | within 1 | within 10 | median abs | max abs | Pearson r | Spearman | listed only mine / only gg |")
W("|---|---|---|---|---|---|---|---|---|---|")
tops = {}
for d in picks:
    rid = rel[d]["id"]
    o = {i["team_id"]: i["rating"] for i in api(f"teams/{rid}?page_size=20000", f"teams_{rid}.json")["items"]}
    m = {r[0]: r[1] for r in con.execute("select team_id, display from team_ratings where release_id=(select id from releases where date=?)", (d,))}
    both = sorted(set(o) & set(m))
    diffs = [abs(m[t] - o[t]) for t in both]
    ex = sum(1 for x in diffs if x == 0)
    w1 = sum(1 for x in diffs if x <= 1)
    w10 = sum(1 for x in diffs if x <= 10)
    xs = [m[t] for t in both]
    ys = [o[t] for t in both]
    rp = statistics.correlation(xs, ys) if len(both) > 2 else float("nan")
    rs = statistics.correlation(xs, ys, method="ranked") if len(both) > 2 else float("nan")
    W(f"| {d} | {len(both)} | {ex/len(both):.1%} | {w1/len(both):.1%} | {w10/len(both):.1%} | "
      f"{statistics.median(diffs):.0f} | {max(diffs):.0f} | {rp:.5f} | {rs:.5f} | {len(set(m)-set(o))} / {len(set(o)-set(m))} |")
    tops[d] = (o, m)
W("")

d = picks[-1]
o, m = tops[d]
names = {r[0]: r[1] for r in buff.execute("select id, name from teams")}
W(f"### Top 20 at {d}, side by side\n")
W("| # | team | gg | mine | diff |")
W("|---|---|---|---|---|")
for i, t in enumerate(sorted(o, key=lambda t: -o[t])[:20], 1):
    got = m.get(t)
    W(f"| {i} | {names.get(t, t)} | {o[t]} | {got if got is not None else '—'} | "
      f"{(got - o[t]) if got is not None else '—'} |")
W("")

# every release against buff.db's mirror of the official team tables
W("### Every release, against buff.db's mirror of the official team table\n")
W("The mirror is exact for older releases (it matches the live API at release 2 to the "
  "point) and stale only for the newest one, so this is a complete week-by-week view.\n")
W("| season | releases | teams both | exact | within 1 | within 10 | median abs | listed only mine / only gg |")
W("|---|---|---|---|---|---|---|---|")
agg = {}
for rid, date in con.execute("select id, date from releases order by date"):
    o = {r[0]: r[1] for r in buff.execute("select team_id, rating from ratings where release_id=?", (rid,))}
    if not o:
        continue
    m = {r[0]: r[1] for r in con.execute("select team_id, display from team_ratings where release_id=?", (rid,))}
    both = set(o) & set(m)
    if not both:
        continue
    season = f"{int(date[:4]) - (date[5:] < '09-01')}/{(int(date[:4]) - (date[5:] < '09-01') + 1) % 100:02d}"
    a = agg.setdefault(season, [0, 0, 0, 0, 0, [], 0, 0])
    a[0] += 1
    a[1] += len(both)
    ds = [abs(m[t] - o[t]) for t in both]
    a[2] += sum(1 for x in ds if x == 0)
    a[3] += sum(1 for x in ds if x <= 1)
    a[4] += sum(1 for x in ds if x <= 10)
    a[5].append(statistics.median(ds))
    a[6] += len(set(m) - set(o))
    a[7] += len(set(o) - set(m))
for season in sorted(agg):
    a = agg[season]
    W(f"| {season} | {a[0]} | {a[1]} | {a[2]/a[1]:.1%} | {a[3]/a[1]:.1%} | {a[4]/a[1]:.1%} | "
      f"{statistics.median(a[5]):.0f} | {a[6]} / {a[7]} |")
W("")

# player ratings, the other half of the system
W("## Player ratings against the official release\n")
W("| release | players both | exact | within 1 | within 10 | median abs | max abs | Pearson r | only mine / only gg |")
W("|---|---|---|---|---|---|---|---|---|")
for d in ("2023-09-14", "2026-09-10"):
    if d not in rel or d not in mine_dates:
        continue
    rid = rel[d]["id"]
    o = {i["player_id"]: i["rating"] for i in api(f"players/{rid}?page_size=100000", f"players_{rid}.json")["items"]}
    m = {r[0]: r[1] for r in con.execute(
        "select player_id, display from player_ratings where release_id=(select id from releases where date=?)", (d,))}
    both = sorted(set(o) & set(m))
    if not both:
        continue
    ds = [abs(m[t] - o[t]) for t in both]
    W(f"| {d} | {len(both)} | {sum(1 for x in ds if x==0)/len(ds):.1%} | {sum(1 for x in ds if x<=1)/len(ds):.1%} | "
      f"{sum(1 for x in ds if x<=10)/len(ds):.1%} | {statistics.median(ds):.0f} | {max(ds):.0f} | "
      f"{statistics.correlation([m[t] for t in both], [o[t] for t in both]):.5f} | "
      f"{len(set(m)-set(o))} / {len(set(o)-set(m))} |")
W("")

# (b), (c) per-tournament forecasts and points
sample = []
for year in range(2022, 2027):
    for kind, n in (("Обычный", 3), ("Синхрон", 3), ("Строго синхронный", 1)):
        sample += [
            r[0] for r in buff.execute(
                "select t.id from tournaments t where t.maii_rating=1 and t.tournament_type=? "
                "and substr(t.date_end,1,10) between ? and ? "
                "and (select count(*) from tournament_results r where r.id=t.id) > 25 "
                "order by t.id limit ?",
                (kind, f"{year}-01-01", f"{year}-12-31", n),
            )
        ]
sample = sorted(set(sample))

rows = []
for tid in sample:
    o = api(f"tournaments/{tid}", f"tournament_{tid}.json")
    oi = {i["team_id"]: i for i in o.get("items", [])}
    mi = {
        r[0]: r for r in con.execute(
            "select team_id, forecast_place, s_r, forecast, x, mu_before, sigma_before, delta_l, s_pair, rated "
            "from team_events where tournament_id=?", (tid,))
    }
    both = set(oi) & set(mi)
    if not both:
        continue
    def rate(f):
        return sum(1 for t in both if f(oi[t], mi[t])) / len(both)
    kind = buff.execute("select tournament_type, substr(date_end,1,10) from tournaments where id=?", (tid,)).fetchone()
    rows.append((tid, kind[0], kind[1], len(both), len(set(oi) ^ set(mi)),
                 rate(lambda a, b: a["place_forecast"] == b[1]),
                 rate(lambda a, b: abs(a["place_forecast"] - b[1]) <= 1),
                 rate(lambda a, b: a["rating"] == round(b[2])),
                 rate(lambda a, b: abs(a["rating"] - b[2]) <= 5),
                 rate(lambda a, b: a["rg"] == round(b[3])),
                 rate(lambda a, b: a["rt"] == round(b[4])),
                 rate(lambda a, b: bool(a["in_rating"]) == bool(b[9]))))

W("## (b), (c) Per-tournament forecasts and points\n")
W("`MP` is the predicted place, `B` the points earned, `RG` the game rating, `RT` the "
  "technical rating of the lineup. Rates are shares of the teams present in both.\n")
W("| tournament | type | end | teams | only one side | MP exact | MP ±1 | B exact | B ±5 | RG exact | RT exact | continuity |")
W("|---|---|---|---|---|---|---|---|---|---|---|---|")
for r in rows:
    W("| {} | {} | {} | {} | {} | {:.1%} | {:.1%} | {:.1%} | {:.1%} | {:.1%} | {:.1%} | {:.1%} |".format(*r))
if rows:
    n = len(rows)
    W("| **all {}** | | | {} | {} | {:.1%} | {:.1%} | {:.1%} | {:.1%} | {:.1%} | {:.1%} | {:.1%} |".format(
        n, sum(r[3] for r in rows), sum(r[4] for r in rows),
        *[sum(r[i] * r[3] for r in rows) / sum(r[3] for r in rows) for i in range(5, 12)]))
W("")

# Bulk check against buff.db's own stored per-result JSON: stale by about 0.5%,
# but it covers every result rather than a sample.
W("## Every result against buff.db's stored `rating` JSON\n")
W("buff.db keeps gg's own per-result numbers, captured when the mirror last saw each "
  "tournament. They lag the live values (see NOTES section 6), so this is a floor on "
  "agreement, not a ceiling — but it covers every rated result rather than a sample.\n")
W("| year | results | B exact | B ±5 | MP exact | MP ±1 | D exact | D ±5 | continuity agrees |")
W("|---|---|---|---|---|---|---|---|---|")
cur = buff.execute(
    "select substr(t.date_end,1,4), r.id, r.team_id, r.rating from tournaments t "
    "join tournament_results r on r.id=t.id where substr(t.date_end,1,10) >= '2021-09-10' "
    "and r.rating is not null and r.rating != ''")
oracle = {}
for year, tid, team, js in cur:
    try:
        j = json.loads(js)
    except Exception:
        continue
    if isinstance(j, dict) and "b" in j:
        oracle[(tid, team)] = (year, j)
agg = {}
for tid, team, mp, b, d, rated in con.execute(
        "select tournament_id, team_id, forecast_place, s_r, evidence, rated from team_events"):
    hit = oracle.get((tid, team))
    if not hit:
        continue
    year, j = hit
    a = agg.setdefault(year, [0] * 8)
    a[0] += 1
    a[1] += j["b"] == round(b)
    a[2] += abs(j["b"] - b) <= 5
    a[3] += j["predictedPosition"] == mp
    a[4] += abs(j["predictedPosition"] - mp) <= 1
    a[5] += j["d"] == round(d)
    a[6] += abs(j["d"] - d) <= 5
    a[7] += bool(j["inRating"]) == bool(rated)
tot = [0] * 8
for year in sorted(agg):
    a = agg[year]
    tot = [x + y for x, y in zip(tot, a)]
    W("| {} | {} | {} |".format(year, a[0], " | ".join(f"{v/a[0]:.1%}" for v in a[1:])))
if tot[0]:
    W("| **all** | {} | {} |".format(tot[0], " | ".join(f"{v/tot[0]:.1%}" for v in tot[1:])))
W("")

# Q, the release normalising coefficient, over every release
W("## Q, the release normalising coefficient\n")
pairs = [(r[0], r[1], float(rel[r[0]]["q"])) for r in con.execute("select date, shift_cum from releases order by date")
         if r[0] in rel and rel[r[0]].get("q") is not None]
if pairs:
    ds = [abs(m - o) for _, m, o in pairs]
    W(f"Over all {len(pairs)} releases: median |diff| {statistics.median(ds):.5f}, "
      f"max {max(ds):.5f}, mean gg Q {statistics.mean(o for _, _, o in pairs):.5f}.\n")
W("| release | gg Q | mine | diff |")
W("|---|---|---|---|")
for d, m, o in pairs:
    if d in picks:
        W(f"| {d} | {o:.5f} | {m:.5f} | {m-o:+.5f} |")
W("")

path = ROOT / "out/ext/gg/numbers.md"
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text("\n".join(out))
print("\n".join(out))
