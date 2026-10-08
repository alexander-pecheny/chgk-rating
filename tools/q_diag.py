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

import sqlite3
import statistics
from math import inf, log2

import lab

FIELD_COLS = "e.tournament_id, e.team_id, e.actual_place, e.forecast, e.mu_before, m.date, m.n_teams"
TOP = lab.TOP
# The worst teams shown per field, and the quintiles of the spread table.
WORST_TEAMS = 4
QUINTILES = 5
MIN_SPREAD_FIELD = 6
EPS = 1e-9
# The cohorts of a lineup: too short, mostly sync-only, then by the median
# player's in-person games.
NO_LINEUP = "no lineup listed"
SHORT_LINEUP = 5
HALF = 0.5
IP_COHORTS = (
    (3, "median under 3 in-person games"),
    (10, "median 3-9 in-person games"),
    (inf, "median 10+ in-person games"),
)
NAN5 = (lab.NAN,) * 5
FIELD_STATE = """
select player_id, theta_before, phi_before, sigma_before, games_ip, days_ip
from player_events where tournament_id = ?
"""
OPTIONS = [
    ("db", dict()),
    ("--teams", dict(type=int, default=0, help="the teams inside the N worst fields")),
    ("--top", dict(type=int, default=20, help="how many fields to list")),
    ("--cohorts", dict(action="store_true")),
    ("--test", dict(action="store_true", help="the test window instead of tune")),
]


def fields(db, frm, to):
    """`{tournament: {"date", "rows": [(team, place, forecast, mu)]}}` of every
    forecast in-person field of five teams or more."""
    by_t = {}
    for tid, team, place, forecast, mu, date, _ in lab.in_person(
        db, frm, to, FIELD_COLS, where="  and e.forecast is not null"
    ):
        by_t.setdefault(tid, {"date": date, "rows": []})["rows"].append((team, place, forecast, mu))
    return {t: v for t, v in by_t.items() if len(v["rows"]) >= lab.MIN_FIELD}


def scored(v, tid):
    """The field ordered by forecast: order, places, strength, NDCG and ideal DCG."""
    places = {r[0]: r[1] for r in v["rows"]}
    strength = {r[0]: r[2] for r in v["rows"]}
    order = lab.ranked(strength, strength.get, lab.Rng(tid))
    rel = [1.0 / places[t] for t in order]
    best = sorted(rel, reverse=True)
    return order, places, strength, lab.dcg(rel) / lab.dcg(best), lab.dcg(best)


def ndcg_at(order, places, k=TOP):
    rel = [1.0 / places[t] for t in order]
    return lab.dcg(rel[:k]) / lab.dcg(sorted(rel, reverse=True)[:k])


def team_cost(order, places, ideal):
    """Per team: the DCG this field would gain if that team alone were moved to
    the slot its actual place deserves, divided by the ideal DCG."""
    slot = {t: i for i, t in enumerate(order)}
    want = {t: i for i, t in enumerate(sorted(order, key=lambda t: places[t]))}
    out = {}
    for t in order:
        r = 1.0 / places[t]
        out[t] = (r / log2(want[t] + 2) - r / log2(slot[t] + 2)) / ideal
    return out


def lineups(db, tid, teams):
    """`{team: [(player, theta, phi, sigma, games_ip, days_ip)]}` for one field."""
    member = {t: ms for t, ms in lab.lineups(sqlite3.connect(lab.BUFF), tid).items() if t in teams and ms}
    st = {p: rest for p, *rest in sqlite3.connect(db).execute(FIELD_STATE, (tid,))}
    nm = lab.names({p for ms in member.values() for p in ms}, "players", "surname || ' ' || name")
    return {team: [(nm.get(p, str(p)), *st.get(p, NAN5)) for p in ms] for team, ms in member.items()}


def parse():
    a = lab.cli(__doc__, OPTIONS)
    a.frm, a.to = lab.TEST if a.test else lab.TUNE
    return a


def worst_fields(costs, w, top, n_fields, frm, to):
    total = sum(c[0] for c in costs)
    tnames = lab.names([c[1] for c in costs[:top]], "tournaments", "name")
    print(f"# The residual, {frm} to {to}, {n_fields} in-person fields\n")
    print(f"Total weighted loss {total:.1f}; the twenty worst carry "
          f"{sum(c[0] for c in costs[:20]) / total:.1%} of it.\n")
    lab.header("| # | tournament | date | teams | weight | NDCG | NDCG@3 | cost |", "|---:|---|---|---:|---:|---:|---:|---:|")
    for k, (cost, tid, nd, v, order, places, _, _) in enumerate(costs[:top], 1):
        print(f"| {k} | {tnames.get(tid, tid)} | {v['date']} | {len(order)} | "
              f"{w.get(tid, 0.0):.2f} | {nd:.4f} | {ndcg_at(order, places):.4f} | {cost:.2f} |")
    return tnames


def who(mem):
    return "; ".join(
        f"{n} θ{th:+.2f} φ{ph:+.2f} σ{sg:.2f} ip{int(gi) if gi == gi else 0}"
        f"/{int(di) if di == di and di >= 0 else -1}d"
        for n, th, ph, sg, gi, di in mem
    )


def worst_teams(db, costs, tnames, n):
    for _, tid, nd, v, order, places, _, ideal in costs[:n]:
        tc = team_cost(order, places, ideal)
        worst = sorted(tc, key=lambda t: -tc[t])[:WORST_TEAMS]
        lu = lineups(db, tid, set(worst))
        nm = lab.names(worst, "teams", "name")
        print(f"\n## {tnames.get(tid, tid)}, {v['date']}, NDCG {nd:.4f}\n")
        lab.header("| team | forecast rank | actual | cost | lineup |", "|---|---:|---:|---:|---|")
        for t in worst:
            print(f"| {nm.get(t, t)} | {order.index(t) + 1} | {places[t]:.0f} | {tc[t]:.3f} | {who(lu.get(t, []))} |")


def field_costs(fs, w):
    """Every field scored, the costliest first."""
    costs = []
    for tid, v in fs.items():
        order, places, strength, nd, ideal = scored(v, tid)
        costs.append((w.get(tid, 0.0) * (1 - nd), tid, nd, v, order, places, strength, ideal))
    costs.sort(key=lambda r: -r[0])
    return costs


def main():
    a = parse()
    w = lab.load_weights()
    fs = fields(a.db, a.frm, a.to)
    costs = field_costs(fs, w)
    tnames = worst_fields(costs, w, a.top, len(fs), a.frm, a.to)
    if a.teams:
        worst_teams(a.db, costs, tnames, a.teams)
    if a.cohorts:
        cohort_table(a.db, fs, w)
        spread_table(costs, w)


def spread_rows(costs, w):
    """`(weight, 1st-to-4th gap, NDCG)` of every weighted field of six or more."""
    rows = []
    for _, tid, nd, _, order, _, strength, _ in costs:
        if w.get(tid, 0.0) > 0 and len(order) >= MIN_SPREAD_FIELD:
            rows.append((w[tid], strength[order[0]] - strength[order[TOP]], nd))
    rows.sort(key=lambda r: r[1])
    return rows


def spread_table(costs, w):
    """How a field's NDCG goes with how far apart the forecast puts its top teams.

    A field of near-equal lineups cannot be ordered by any forecast, so this is
    the part of the loss no lever can reach."""
    rows = spread_rows(costs, w)
    print("\n## The loss against how far apart the top of the field is\n")
    lab.header("| quintile of the 1st-to-4th forecast gap | gap (points) | fields | NDCG | NDCG of the top decile of the gap |",
               "|---|---:|---:|---:|---:|")
    n = len(rows)
    for q in range(QUINTILES):
        part = rows[q * n // QUINTILES : (q + 1) * n // QUINTILES]
        dw = sum(r[0] for r in part)
        print(f"| {q + 1} | {sum(r[0] * r[1] for r in part) / dw:.0f} | {len(part)} | "
              f"{sum(r[0] * r[2] for r in part) / dw:.4f} | |")


def cohort(ms, st):
    """What a lineup looks like: its size, how much of it plays in person, and
    how many in-person games its median player has."""
    sg = [st[p] for p in ms if p in st]
    if not ms or not sg:
        return NO_LINEUP
    if len(ms) < SHORT_LINEUP:
        return "short lineup (<5)"
    if statistics.mean(1.0 if g > 0 else 0.0 for _, g, _ in sg) < HALF:
        return "half the table sync-only"
    n_ip = statistics.median(g for _, g, _ in sg)
    return next(label for top, label in IP_COHORTS if n_ip < top)


def cohort_losses(db, fs, w):
    """`{cohort: [weighted loss, weight, results]}` over every weighted field."""
    st = {t: {p: s[2:] for p, s in d.items()} for t, d in lab.player_state(sqlite3.connect(db), set(fs)).items()}
    buff = sqlite3.connect(lab.BUFF)
    buckets = {}
    for tid, v in fs.items():
        if w.get(tid, 0.0) > 0:
            order, places, _, _, ideal = scored(v, tid)
            tc, members = team_cost(order, places, ideal), lab.lineups(buff, tid)
            for t in order:
                b = buckets.setdefault(cohort(members.get(t, []), st.get(tid, {})), [0.0, 0.0, 0])
                b[0], b[1], b[2] = b[0] + w[tid] * tc[t], b[1] + w[tid], b[2] + 1
    return buckets


def cohort_table(db, fs, w):
    """The same loss cut by what the lineup looks like, over every field."""
    buckets = cohort_losses(db, fs, w)
    print("\n## The loss by cohort of lineup\n")
    lab.header("| cohort | results | share of the weighted loss | loss per result |", "|---|---:|---:|---:|")
    tot = sum(b[0] for b in buckets.values())
    for k, b in sorted(buckets.items(), key=lambda kv: -kv[1][0]):
        print(f"| {k} | {b[2]} | {b[0] / tot:.1%} | {b[0] / max(b[1], EPS):.4f} |")


if __name__ == "__main__":
    main()
