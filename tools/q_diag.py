"""Where the in-person forecast still loses: the residual, tournament by
tournament and then team by team.

Plan 5 asks for the diagnosis before the levers.  Two views over one fitted
database:

    uv run tools/q_diag.py out/q2/base.db              # the twenty worst fields
    uv run tools/q_diag.py out/q2/base.db --teams 6    # and the teams inside them
    uv run tools/q_diag.py out/q2/base.db --cohorts    # the misses by cohort

A field's cost is `weight * (1 - NDCG)`; the weights are the frozen ones of
`data/tournament_weights.csv` and the NDCG is the metric of `src/ndcg.rs`, tie
break included.  A team's cost inside a field is what the field's DCG gains if
that team alone is moved to its actual place, which is the share of the loss it
is responsible for.
"""

import json
import sqlite3
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ndcg as N  # noqa: E402

BUFF = "buff.db"
# The windows of src/ndcg.rs.
TUNE = ("2018-09-01", "2024-08-29")
TEST = ("2024-08-30", "2026-08-27")
FIELD = """
select e.tournament_id, e.team_id, e.actual_place, e.forecast, e.mu_before,
       m.date, m.n_teams
from team_events e
join tournament_metrics m on m.tournament_id = e.tournament_id
where m.type = 'in-person' and m.date >= ? and m.date < ? and e.actual_place > 0
  and e.forecast is not null
order by e.tournament_id
"""


def dcg(rel):
    from math import log2

    return sum(r / log2(i + 2) for i, r in enumerate(rel))


def order_of(ts, strength):
    """The ranked order of `ts`, with the tie-break of tools/ndcg.py."""
    tid, teams = ts
    order = list(teams)
    N.Rng(tid).shuffle(order)
    order.sort(key=lambda t: -strength[t])
    return order


def fields(db, frm, to):
    rows = sqlite3.connect(db).execute(FIELD, (frm, to)).fetchall()
    by_t = {}
    for tid, team, place, forecast, mu, date, n in rows:
        by_t.setdefault(tid, {"date": date, "rows": []})["rows"].append(
            (team, place, forecast, mu)
        )
    return {t: v for t, v in by_t.items() if len(v["rows"]) >= 5}


def scored(v, tid):
    places = {r[0]: r[1] for r in v["rows"]}
    strength = {r[0]: r[2] for r in v["rows"]}
    order = order_of((tid, list(strength)), strength)
    rel = [1.0 / places[t] for t in order]
    best = sorted(rel, reverse=True)
    return order, places, strength, dcg(rel) / dcg(best), dcg(best)


def team_cost(order, places, ideal):
    """Per team: the DCG this field would gain if that team alone were moved to
    the slot its actual place deserves, divided by the ideal DCG."""
    from math import log2

    slot = {t: i for i, t in enumerate(order)}
    ranked = sorted(order, key=lambda t: places[t])
    want = {t: i for i, t in enumerate(ranked)}
    out = {}
    for t in order:
        r = 1.0 / places[t]
        gain = r / log2(want[t] + 2) - r / log2(slot[t] + 2)
        out[t] = gain / ideal
    return out


def names(ids, table, col):
    if not ids:
        return {}
    c = sqlite3.connect(BUFF)
    q = f"select id, {col} from {table} where id in ({','.join('?' * len(ids))})"
    return dict(c.execute(q, list(ids)).fetchall())


def lineups(db, tid, teams):
    """`{team: [(player, theta, phi, sigma, games_ip, days_ip)]}` for one field."""
    c = sqlite3.connect(BUFF)
    rows = c.execute(
        "select team_id, team_members from tournament_results where id = ?", (tid,)
    ).fetchall()
    member = {}
    for team, ms in rows:
        if team in teams and ms:
            member[team] = json.loads(ms)
    st = dict(
        (p, (th, ph, sg, gi, di))
        for p, th, ph, sg, gi, di in sqlite3.connect(db).execute(
            "select player_id, theta_before, phi_before, sigma_before, games_ip, days_ip"
            " from player_events where tournament_id = ?",
            (tid,),
        )
    )
    nm = names(
        {p for ms in member.values() for p in ms}, "players", "surname || ' ' || name"
    )
    out = {}
    for team, ms in member.items():
        out[team] = [
            (nm.get(p, str(p)),) + st.get(p, (float("nan"),) * 5) for p in ms
        ]
    return out


def main():
    args = sys.argv[1:]
    db = args[0]
    top = 20
    teams_for = 0
    cohorts = False
    frm, to = TUNE
    i = 1
    while i < len(args):
        if args[i] == "--teams":
            teams_for = int(args[i + 1])
            i += 2
        elif args[i] == "--top":
            top = int(args[i + 1])
            i += 2
        elif args[i] == "--cohorts":
            cohorts = True
            i += 1
        elif args[i] == "--test":
            frm, to = TEST
            i += 1
        else:
            i += 1

    w = N.load_weights(N.WEIGHTS)
    fs = fields(db, frm, to)
    costs = []
    for tid, v in fs.items():
        order, places, strength, nd, ideal = scored(v, tid)
        costs.append((w.get(tid, 0.0) * (1 - nd), tid, nd, v, order, places, strength, ideal))
    costs.sort(key=lambda r: -r[0])
    total = sum(c[0] for c in costs)
    tnames = names([c[1] for c in costs[:top]], "tournaments", "name")

    print(f"# The residual, {frm} to {to}, {len(fs)} in-person fields\n")
    print(
        f"Total weighted loss {total:.1f}; the twenty worst carry "
        f"{sum(c[0] for c in costs[:20]) / total:.1%} of it.\n"
    )
    print("| # | tournament | date | teams | weight | NDCG | NDCG@3 | cost |")
    print("|---:|---|---|---:|---:|---:|---:|---:|")
    for k, (cost, tid, nd, v, order, places, strength, ideal) in enumerate(costs[:top], 1):
        rel = [1.0 / places[t] for t in order]
        best = sorted(rel, reverse=True)
        nd3 = dcg(rel[:3]) / dcg(best[:3])
        print(
            f"| {k} | {tnames.get(tid, tid)} | {v['date']} | {len(order)} | "
            f"{w.get(tid, 0.0):.2f} | {nd:.4f} | {nd3:.4f} | {cost:.2f} |"
        )

    if teams_for:
        for cost, tid, nd, v, order, places, strength, ideal in costs[:teams_for]:
            tc = team_cost(order, places, ideal)
            worst = sorted(tc, key=lambda t: -tc[t])[:4]
            lu = lineups(db, tid, set(worst))
            nm = names(worst, "teams", "name")
            print(f"\n## {tnames.get(tid, tid)}, {v['date']}, NDCG {nd:.4f}\n")
            print("| team | forecast rank | actual | cost | lineup |")
            print("|---|---:|---:|---:|---|")
            for t in worst:
                mem = lu.get(t, [])
                who = "; ".join(
                    f"{n} θ{th:+.2f} φ{ph:+.2f} σ{sg:.2f} ip{int(gi) if gi == gi else 0}"
                    f"/{int(di) if di == di and di >= 0 else -1}d"
                    for n, th, ph, sg, gi, di in mem
                )
                print(
                    f"| {nm.get(t, t)} | {order.index(t) + 1} | {places[t]:.0f} | "
                    f"{tc[t]:.3f} | {who} |"
                )

    if cohorts:
        cohort_table(db, fs, w, frm, to)
        spread_table(costs, w)


def spread_table(costs, w):
    """How a field's NDCG goes with how far apart the forecast puts its top teams.

    A field of near-equal lineups cannot be ordered by any forecast, so this is
    the part of the loss no lever can reach."""
    rows = []
    for cost, tid, nd, v, order, places, strength, ideal in costs:
        wt = w.get(tid, 0.0)
        if wt <= 0 or len(order) < 6:
            continue
        s = [strength[t] for t in order]
        rows.append((wt, s[0] - s[3], nd, len(order)))
    rows.sort(key=lambda r: r[1])
    print("\n## The loss against how far apart the top of the field is\n")
    print("| quintile of the 1st-to-4th forecast gap | gap (points) | fields | NDCG | NDCG of the top decile of the gap |")
    print("|---|---:|---:|---:|---:|")
    n = len(rows)
    for q in range(5):
        part = rows[q * n // 5 : (q + 1) * n // 5]
        dw = sum(r[0] for r in part)
        print(
            f"| {q + 1} | {sum(r[0] * r[1] for r in part) / dw:.0f} | {len(part)} | "
            f"{sum(r[0] * r[2] for r in part) / dw:.4f} | |"
        )


def cohort_table(db, fs, w, frm, to):
    """The same loss cut by what the lineup looks like, over every field."""
    import statistics

    conn = sqlite3.connect(db)
    rows = conn.execute(
        "select tournament_id, player_id, sigma_before, games_ip, days_ip"
        " from player_events where tournament_id in (%s)" % ",".join("?" * len(fs)),
        list(fs),
    ).fetchall()
    st = {}
    for tid, p, sg, gi, di in rows:
        st[(tid, p)] = (sg, gi, di)
    c = sqlite3.connect(BUFF)
    members = {}
    for tid in fs:
        for team, ms in c.execute(
            "select team_id, team_members from tournament_results where id = ?", (tid,)
        ):
            members[(tid, team)] = json.loads(ms) if ms else []

    buckets = {}
    for tid, v in fs.items():
        order, places, strength, nd, ideal = scored(v, tid)
        tc = team_cost(order, places, ideal)
        wt = w.get(tid, 0.0)
        if wt <= 0:
            continue
        for t in order:
            ms = members.get((tid, t), [])
            if not ms:
                key = "no lineup listed"
            else:
                sg = [st[(tid, p)] for p in ms if (tid, p) in st]
                if not sg:
                    key = "no lineup listed"
                else:
                    ip = statistics.mean(1.0 if g > 0 else 0.0 for _, g, _ in sg)
                    n_ip = statistics.median(g for _, g, _ in sg)
                    if len(ms) < 5:
                        key = "short lineup (<5)"
                    elif ip < 0.5:
                        key = "half the table sync-only"
                    elif n_ip < 3:
                        key = "median under 3 in-person games"
                    elif n_ip < 10:
                        key = "median 3-9 in-person games"
                    else:
                        key = "median 10+ in-person games"
            b = buckets.setdefault(key, [0.0, 0.0, 0])
            b[0] += wt * tc[t]
            b[1] += wt
            b[2] += 1
    print("\n## The loss by cohort of lineup\n")
    print("| cohort | results | share of the weighted loss | loss per result |")
    print("|---|---:|---:|---:|")
    tot = sum(b[0] for b in buckets.values())
    for k, b in sorted(buckets.items(), key=lambda kv: -kv[1][0]):
        print(f"| {k} | {b[2]} | {b[0] / tot:.1%} | {b[0] / max(b[1], 1e-9):.4f} |")


if __name__ == "__main__":
    main()
