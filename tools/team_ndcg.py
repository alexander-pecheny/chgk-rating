"""Does the team number alone order an in-person field of base-roster teams?

The published rating `mu_before` -- the number as it stood on the morning of the
tournament, with no lineup and no player ratings in it -- ranks the qualifying
teams of a tournament; NDCG with relevance 1/place scores that order against
what happened.  Only in-person, rated results of teams that turned up in their
own base roster count, so the metric asks about continuity of a team, not about
who was free that Saturday.

    uv run tools/team_ndcg.py out/ratings.db [more.db ...]
    uv run tools/team_ndcg.py out/ratings.db out/ext/gg.db out/ext/fun.db --old --common
    uv run tools/team_ndcg.py a.db b.db --boot 2000          # paired SE against a.db
    uv run tools/team_ndcg.py out/ratings.db --window tune   # 2018-09-01..2024-08-29

Both means are printed side by side; the headline is the **weighted** one, with
the weights frozen in `data/tournament_weights.csv`. `--unweighted` sets every
weight to one, so the two columns agree.
"""

import csv
import json
import math
import os
import random
import sqlite3

import lab

ROSTERS = "data/external/chgk_gg/base_rosters.csv"
CACHE = "out/team_ndcg/qualify.json"
DATE = slice(0, 10)
# MAII Prilozhenie A.4.7 and the plan's stricter fallback.
MIN_BASE = 3
MAX_LEGIONNAIRES = 3
STRICT_MIN_BASE = 4
STRICT_SLACK = 2
TOP = lab.TOP
RESULTS = "select id, team_id, team_members from tournament_results where team_members is not null"
OLD_FIELDS = """
select m.tournament_id, m.date from m.tournament_metrics m
where m.type = 'in-person' and m.date >= ? and m.date < ?
"""
OPTIONS = [
    ("dbs", dict(nargs="*")),
    ("--common", dict(action="store_true")),
    ("--old", dict(action="store_true", help="add the old system's rating from the mirror")),
    ("--unweighted", dict(action="store_true")),
    ("--rebuild", dict(action="store_true", help="rebuild the qualifying cache")),
    ("--window", dict(choices=lab.WINDOWS, default="test")),
    ("--from", dict(dest="frm")),
    ("--to", dict()),
    ("--boot", dict(type=int, default=0, help="paired bootstrap resamples")),
    ("--column", dict(default="mu_before")),
    ("--rule", dict(default="maii", help="maii, or anything else for the strict rule")),
]


# ---------------------------------------------------------------- qualifying

def base_rosters():
    """(team_id, season_id) -> [(player_id, added, removed or None)].

    `buff.db.team_seasons` only carries the two current seasons, so the roster
    history comes from the chgk.gg export and the mirror tops it up.
    """
    out = gg_rosters() if os.path.exists(ROSTERS) else {}
    q = "select team_id, season_id, player_id, date_added, date_removed from team_seasons"
    for team, season, player, added, removed in sqlite3.connect(lab.BUFF).execute(q):
        row = (player, (added or "")[DATE], (removed or None) and removed[DATE])
        if row not in out.setdefault((team, season), []):
            out[(team, season)].append(row)
    return out


def gg_rosters():
    out = {}
    with open(ROSTERS) as f:
        for r in csv.DictReader(f):
            row = (int(r["player_id"]), r["start_date"][DATE], r["end_date"][DATE] or None)
            out.setdefault((int(r["team_id"]), int(r["season_id"])), []).append(row)
    return out


def seasons():
    b = sqlite3.connect(lab.BUFF)
    return [
        (i, s[DATE], e[DATE])
        for i, s, e in b.execute("select id, date_start, date_end from seasons")
        if s and e
    ]


def season_of(secs, date):
    return next((i for i, s, e in secs if s <= date <= e), None)


def n_base(members, roster, date):
    """How many of `members` were in the base roster on `date`."""
    base = {p for p, added, removed in roster if added <= date and (removed is None or removed > date)}
    return sum(1 for m in members if m in base)


def qualifies(members, roster, date):
    """MAII Prilozhenie A.4.7: at least three base players, fewer legionnaires
    than base players, and at most three legionnaires."""
    nb = n_base(members, roster, date)
    n_leg = len(members) - nb
    return nb >= MIN_BASE and n_leg < nb and n_leg <= MAX_LEGIONNAIRES


def qualifies_strict(members, roster, date):
    """The plan's fallback rule: at least max(4, size - 2) base players."""
    nb = n_base(members, roster, date)
    return len(members) > 0 and nb >= max(STRICT_MIN_BASE, len(members) - STRICT_SLACK)


def build_qualify(rule="maii"):
    """{tournament_id: [team_id]} over every result in the mirror."""
    rosters, secs = base_rosters(), seasons()
    test = qualifies if rule == "maii" else qualifies_strict
    b = sqlite3.connect(lab.BUFF)
    dates = {i: (d or "")[DATE] for i, d in b.execute("select id, coalesce(date_end, date_start) from tournaments")}
    out = {}
    for tid, team, js in b.execute(RESULTS):
        date = dates.get(tid)
        roster = rosters.get((team, season_of(secs, date))) if date else None
        members = parse_members(js) if roster else None
        if members and test(members, roster, date):
            out.setdefault(str(tid), []).append(team)
    return out


def parse_members(js):
    try:
        return [int(x) for x in json.loads(js)]
    except (ValueError, TypeError):
        return None


def write_cache(path, q):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(q, f)
    # The same set as two columns, which `team_sweep` reads.
    with open(path.replace(".json", ".csv"), "w") as f:
        f.write("tournament_id,team_id\n")
        for tid, teams in q.items():
            for t in teams:
                f.write(f"{tid},{t}\n")


def load_qualify(rule="maii", rebuild=False):
    path = CACHE if rule == "maii" else CACHE.replace(".json", f".{rule}.json")
    if not rebuild and os.path.exists(path):
        with open(path) as f:
            q = json.load(f)
    else:
        q = build_qualify(rule)
        write_cache(path, q)
    return {int(k): set(v) for k, v in q.items()}


# ------------------------------------------------------------------ scoring

def tiekey(tid, team):
    """A deterministic tie-break, the same number in `src/bin/team_sweep.rs`."""
    return lab.mix((tid * lab.GOLDEN + team) & lab.M64)


def per_tournament(db, qual, frm, to, column="mu_before"):
    """{tournament_id: {team_id: rating}} and {tournament_id: {team_id: place}}."""
    expr = "e." + column if column.isidentifier() else column
    rows = lab.in_person(db, frm, to, f"e.tournament_id, e.team_id, {expr}, e.actual_place",
                         where="  and e.rated = 1", order=None)
    val, place = {}, {}
    for tid, team, v, pl in rows:
        if team not in qual.get(tid, ()) or v is None:
            continue
        val.setdefault(tid, {})[team] = v
        place.setdefault(tid, {})[team] = pl
    return val, place


def old_system(qual, frm, to, metrics_db):
    """The old system's rating from the mirror, at the last release before the
    tournament.  Teams the release does not list are ranked last."""
    b = sqlite3.connect(lab.BUFF)
    b.execute("attach database ? as m", (metrics_db,))
    rows = b.execute(OLD_FIELDS, (frm, to)).fetchall()
    rel = b.execute("select id, date from releases order by date").fetchall()
    out = {}
    for tid, date in rows:
        prev = [r for r in rel if r[1] < date]
        if qual.get(tid) and prev:
            have = dict(b.execute("select team_id, rating from ratings where release_id = ?", (prev[-1][0],)))
            out[tid] = {t: float(have.get(t, 0)) for t in qual[tid]}
    return out


def score(val, place, k=None):
    """NDCG per tournament over the qualifying subset, re-ranked."""
    out = {}
    for tid, strength in val.items():
        v = score_field(tid, strength, place[tid], k) if len(strength) >= lab.MIN_FIELD else None
        if v is not None:
            out[tid] = v
    return out


def score_field(tid, strength, pl, k):
    teams = list(strength)
    places = dict(zip(teams, lab.ranks([pl[t] for t in teams])))
    order = sorted(teams, key=lambda t: (-strength[t], tiekey(tid, t)))
    return lab.ndcg(order, places, k)


def weights():
    """The frozen weights, or every tournament once when the table is missing."""
    return lab.load_weights() if os.path.exists(lab.WEIGHTS) else None


def boot(a, b, tids, w, n=lab.BOOT, seed=lab.SEED):
    """Paired bootstrap SE of the difference a - b over tournaments, weighted
    and unweighted."""
    tids = [t for t in tids if t in a and t in b]
    if len(tids) < lab.MIN_FIELD:
        return lab.NAN, lab.NAN
    rng = random.Random(seed)
    w = w or {}
    samples = [[tids[rng.randrange(len(tids))] for _ in tids] for _ in range(n)]
    dw = [weighted_diff(a, b, s, w) for s in samples]
    du = [sum(a[t] - b[t] for t in s) / len(s) for s in samples]
    return sd(dw), sd(du)


def weighted_diff(a, b, s, w):
    num = sum(w.get(t, 0.0) * (a[t] - b[t]) for t in s)
    den = sum(w.get(t, 0.0) for t in s)
    return num / den if den > 0 else 0.0


def sd(x):
    m = sum(x) / len(x)
    return math.sqrt(sum((v - m) ** 2 for v in x) / (len(x) - 1))


# --------------------------------------------------------------------- main

def parse():
    a = lab.cli(__doc__, OPTIONS, intermixed=True)
    frm, to = lab.WINDOWS[a.window]
    a.frm, a.to = a.frm or frm, a.to or to
    return a


def score_all(a, qual):
    runs = {}
    for db in a.dbs:
        val, place = per_tournament(db, qual, a.frm, a.to, a.column)
        runs[db] = (score(val, place), score(val, place, TOP), val, place)
    if a.old and a.dbs:
        val0, place0 = runs[a.dbs[0]][2], runs[a.dbs[0]][3]
        ov = old_system(qual, a.frm, a.to, a.dbs[0])
        ov = {t: {k: v for k, v in d.items() if k in val0.get(t, {})} for t, d in ov.items()}
        runs["old system"] = (score(ov, place0), score(ov, place0, TOP), ov, place0)
    return runs


def print_means(runs, tids, w, common):
    lab.header("| model | NDCG (w) | NDCG | NDCG@3 (w) | NDCG@3 | tournaments | teams/t |",
               "|---|---:|---:|---:|---:|---:|---:|")
    for db, (full, at3, val, _) in runs.items():
        sel = tids if common else set(full)
        sizes = [len(val[t]) for t in sel if t in val]
        print(
            f"| {db} | {lab.wmean(full, sel, w):.4f} | {lab.wmean(full, sel):.4f} | "
            f"{lab.wmean(at3, sel, w):.4f} | {lab.wmean(at3, sel):.4f} | {len(sel & set(full))} | "
            f"{sum(sizes)/len(sizes):.1f} |"
        )


def print_boot(runs, tids, w, common, nboot):
    base, (bfull, bat3, _, _) = next(iter(runs.items()))
    print(f"\nPaired bootstrap SE of the difference against `{base}`, {nboot} resamples:\n")
    lab.header("| model | d NDCG (w) | SE (w) | d NDCG | SE | d NDCG@3 (w) | SE (w) |",
               "|---|---:|---:|---:|---:|---:|---:|")
    for db, (full, at3, _, _) in list(runs.items())[1:]:
        sel = tids if common else set(full) & set(bfull)
        (sw, su), (s3w, _) = boot(full, bfull, sel, w, nboot), boot(at3, bat3, sel, w, nboot)
        print(
            f"| {db} | {delta(full, bfull, sel, w):+.4f} | {sw:.4f} | {delta(full, bfull, sel, None):+.4f} | "
            f"{su:.4f} | {delta(at3, bat3, sel, w):+.4f} | {s3w:.4f} |"
        )


def delta(a, b, sel, w):
    return lab.wmean(a, sel, w) - lab.wmean(b, sel, w)


def main():
    a = parse()
    qual = load_qualify(a.rule, a.rebuild)
    w = None if a.unweighted else weights()
    runs = score_all(a, qual)
    tids = set.intersection(*(set(r[0]) for r in runs.values())) if runs else None
    print(f"NDCG on the team rating alone, in-person base-roster fields, {a.frm} to {a.to}")
    print(f"rule: {'MAII A.4.7' if a.rule == 'maii' else 'at least max(4, size-2) base players'}"
          f"{', common tournaments only' if a.common else ''}\n")
    print_means(runs, tids, w, a.common)
    if a.boot and len(runs) > 1:
        print_boot(runs, tids, w, a.common, a.boot)


if __name__ == "__main__":
    main()
