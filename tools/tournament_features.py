"""One row per tournament of the features a prestige and age classifier reads.

    uv run tools/tournament_features.py out/prestige/all.db > out/prestige/features.tsv

The fit (`all.db`, a run with include_unrated) gives lineup strengths and
question difficulties; buff.db gives names, towns, base rosters and every
player's first game.
"""

import datetime
import json
import math
from collections import Counter, defaultdict

import lab

OPTIONS = [("db", dict()), ("--buff", dict(default="buff.db")), ("--elite", dict(type=int, default=30)), ("--strong", dict(type=int, default=100))]
FORMAT = {"Обычный": "in-person", "Региональный": "in-person", "Синхрон": "sync", "Строго синхронный": "sync", "Асинхрон": "async"}


def members(s):
    try:
        return [int(p) for p in json.loads(s or "[]")]
    except ValueError:
        return []


def main(args):
    b = lab.connect(args.buff)
    d = lab.connect(args.db)
    country = dict(b.execute("select id, country_id from towns"))
    tour = {}
    for tid, name, fmt, rated, date, town in b.execute(
        "select id, name, tournament_type, in_rating, substr(date_start,1,10), town_id from tournaments where date_start >= '2000'"
    ):
        if fmt in FORMAT and date:
            tour[tid] = dict(name=(name or "").replace("\t", " ").strip(), fmt=FORMAT[fmt], rated=rated or 0, date=date, town=town)

    first = {}
    lineups = defaultdict(list)
    for tid, team, town, ms in b.execute("select id, team_id, team_current_town_id, team_members from tournament_results"):
        t = tour.get(tid)
        if t is None:
            continue
        ps = members(ms)
        for p in ps:
            if p not in first or t["date"] < first[p]:
                first[p] = t["date"]
        if t["date"] >= "2015-01-01":
            lineups[tid].append((team, town, ps))

    history = defaultdict(list)
    for tid, ls in lineups.items():
        for team, _, ps in ls:
            history[team].append((tour[tid]["date"], tid, ps))
    for h in history.values():
        h.sort()

    def core_share(team, tid, date, ps):
        """Share of the team's four most frequent players of the past year present in this lineup."""
        d0 = (datetime.date.fromisoformat(date) - datetime.timedelta(days=365)).isoformat()
        past = [g for dt, t2, g in history[team] if d0 <= dt < date and t2 != tid]
        if len(past) < 5:
            return None
        core = [p for p, _ in Counter(p for g in past for p in g).most_common(4)]
        return len(set(ps) & set(core)) / len(core)

    x = defaultdict(dict)
    per_year = defaultdict(lambda: defaultdict(list))
    for tid, date, team, xx in d.execute("select tournament_id, date, team_id, x from team_events"):
        if xx is not None and math.isfinite(xx):
            x[tid][team] = xx
            if tour.get(tid, {}).get("fmt") in ("in-person", "sync"):
                per_year[int(date[:4])][team].append(xx)
    top = {}
    for y, teams in per_year.items():
        ranked = sorted(((sum(v) / len(v), t) for t, v in teams.items() if len(v) >= 3), reverse=True)
        top[y + 1] = [t for _, t in ranked[: args.strong]]
    diff = {tid: (sum(bs) / len(bs), len(bs)) for tid, bs in _group(d.execute("select tournament_id, b from question_params where b is not null"))}

    cols = ["id", "date", "fmt", "rated", "n", "top10", "elite", "strong", "core_strong", "travel", "towns", "countries", "b", "n_q", "career_med", "new3", "name"]
    print("\t".join(cols))
    for tid, ls in sorted(lineups.items(), key=lambda kv: tour[kv[0]]["date"]):
        t = tour[tid]
        if t["fmt"] == "async" or len(ls) < 5:
            continue
        y = int(t["date"][:4])
        tops = top.get(y, [])
        elite, strong = set(tops[: args.elite]), set(tops)
        teams = {team for team, _, _ in ls}
        xs = sorted(x.get(tid, {}).values(), reverse=True)
        shares = [c for team, _, ps in ls if team in strong for c in [core_share(team, tid, t["date"], ps)] if c is not None]
        towns = [town for _, town, _ in ls if town]
        away = [town != t["town"] for town in towns] if t["fmt"] == "in-person" and t["town"] else []
        careers = sorted(
            ((datetime.date.fromisoformat(t["date"]) - datetime.date.fromisoformat(first[p])).days / 365.25 for _, _, ps in ls for p in ps if p in first),
        )
        bm, nq = diff.get(tid, (float("nan"), 0))
        row = [
            tid, t["date"], t["fmt"], t["rated"], len(ls),
            f"{sum(xs[:10]) / len(xs[:10]):.0f}" if xs else "",
            len(teams & elite), len(teams & strong),
            f"{sum(shares) / len(shares):.3f}" if shares else "",
            f"{sum(away) / len(away):.3f}" if away else "",
            len(set(towns)), len({country.get(tw) for tw in towns}),
            f"{bm:.3f}" if nq else "", nq,
            f"{careers[len(careers) // 2]:.2f}" if careers else "",
            f"{sum(c < 3 for c in careers) / len(careers):.3f}" if careers else "",
            t["name"],
        ]
        print("\t".join(map(str, row)))


def _group(rows):
    g = defaultdict(list)
    for k, v in rows:
        g[k].append(v)
    return g.items()


if __name__ == "__main__":
    main(lab.cli(__doc__, OPTIONS))
