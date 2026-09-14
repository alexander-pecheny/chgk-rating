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
import random as _random
import sqlite3
import sys

BUFF = "buff.db"
ROSTERS = "data/external/chgk_gg/base_rosters.csv"
WEIGHTS = "data/tournament_weights.csv"
CACHE = "out/team_ndcg/qualify.json"
WINDOWS = {
    "tune": ("2018-09-01", "2024-08-29"),
    "test": ("2024-08-30", "2026-08-27"),
}


# ---------------------------------------------------------------- qualifying

def base_rosters():
    """(team_id, season_id) -> [(player_id, added, removed or None)].

    `buff.db.team_seasons` only carries the two current seasons, so the roster
    history comes from the chgk.gg export and the mirror tops it up.
    """
    out = {}
    if os.path.exists(ROSTERS):
        with open(ROSTERS) as f:
            for r in csv.DictReader(f):
                out.setdefault((int(r["team_id"]), int(r["season_id"])), []).append(
                    (int(r["player_id"]), r["start_date"][:10], r["end_date"][:10] or None)
                )
    b = sqlite3.connect(BUFF)
    for team, season, player, added, removed in b.execute(
        "select team_id, season_id, player_id, date_added, date_removed from team_seasons"
    ):
        key = (team, season)
        row = (player, (added or "")[:10], (removed or None) and removed[:10])
        if row not in out.setdefault(key, []):
            out[key].append(row)
    return out


def seasons():
    b = sqlite3.connect(BUFF)
    return [
        (i, s[:10], e[:10])
        for i, s, e in b.execute("select id, date_start, date_end from seasons")
        if s and e
    ]


def season_of(secs, date):
    for i, s, e in secs:
        if s <= date <= e:
            return i
    return None


def qualifies(members, roster, date):
    """MAII Prilozhenie A.4.7: at least three base players, fewer legionnaires
    than base players, and at most three legionnaires."""
    base = {
        p
        for p, added, removed in roster
        if added <= date and (removed is None or removed > date)
    }
    n_base = sum(1 for m in members if m in base)
    n_leg = len(members) - n_base
    return n_base >= 3 and n_leg < n_base and n_leg <= 3


def qualifies_strict(members, roster, date):
    """The plan's fallback rule: at least max(4, size - 2) base players."""
    base = {
        p
        for p, added, removed in roster
        if added <= date and (removed is None or removed > date)
    }
    n_base = sum(1 for m in members if m in base)
    return len(members) > 0 and n_base >= max(4, len(members) - 2)


def build_qualify(rule="maii"):
    """{tournament_id: {team_id: 1}} over every result in the mirror."""
    rosters, secs = base_rosters(), seasons()
    test = qualifies if rule == "maii" else qualifies_strict
    b = sqlite3.connect(BUFF)
    dates = {
        i: (d or "")[:10]
        for i, d in b.execute("select id, coalesce(date_end, date_start) from tournaments")
    }
    out = {}
    for tid, team, js in b.execute(
        "select id, team_id, team_members from tournament_results where team_members is not null"
    ):
        date = dates.get(tid)
        if not date:
            continue
        sid = season_of(secs, date)
        roster = rosters.get((team, sid))
        if not roster:
            continue
        try:
            members = [int(x) for x in json.loads(js)]
        except (ValueError, TypeError):
            continue
        if members and test(members, roster, date):
            out.setdefault(str(tid), []).append(team)
    return out


def load_qualify(rule="maii", rebuild=False):
    path = CACHE if rule == "maii" else CACHE.replace(".json", f".{rule}.json")
    if not rebuild and os.path.exists(path):
        with open(path) as f:
            return {int(k): set(v) for k, v in json.load(f).items()}
    q = build_qualify(rule)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(q, f)
    # The same set as two columns, which `team_sweep` reads.
    with open(path.replace(".json", ".csv"), "w") as f:
        f.write("tournament_id,team_id\n")
        for tid, teams in q.items():
            for t in teams:
                f.write(f"{tid},{t}\n")
    return {int(k): set(v) for k, v in q.items()}


# ------------------------------------------------------------------ scoring

def avg_ranks(vals):
    """1-based ranks of `vals` ascending, ties sharing the mean rank."""
    order = sorted(range(len(vals)), key=lambda i: vals[i])
    r = [0.0] * len(vals)
    i = 0
    while i < len(order):
        j = i
        while j + 1 < len(order) and vals[order[j + 1]] == vals[order[i]]:
            j += 1
        for t in order[i : j + 1]:
            r[t] = (i + j) / 2 + 1
        i = j + 1
    return r


M64 = (1 << 64) - 1


def tiekey(tid, team):
    """A deterministic tie-break, the same number in `src/bin/team_sweep.rs`."""
    x = (tid * 0x9E3779B97F4A7C15 + team) & M64
    x ^= x >> 30
    x = (x * 0xBF58476D1CE4E5B9) & M64
    x ^= x >> 27
    x = (x * 0x94D049BB133111EB) & M64
    return x ^ (x >> 31)


def ndcg(order, places, k=None):
    rel = [1.0 / places[t] for t in order]
    k = k or len(rel)
    dcg = sum(r / math.log2(i + 2) for i, r in enumerate(rel[:k]))
    idcg = sum(r / math.log2(i + 2) for i, r in enumerate(sorted(rel, reverse=True)[:k]))
    return dcg / idcg if idcg > 0 else None


def per_tournament(db, qual, frm, to, column="mu_before"):
    """{tournament_id: {team_id: rating}} and {tournament_id: {team_id: place}}."""
    c = sqlite3.connect(db)
    rows = c.execute(
        f"select e.tournament_id, e.team_id, {column if not column.isidentifier() else 'e.' + column}, e.actual_place "
        "from team_events e join tournament_metrics m on m.tournament_id = e.tournament_id "
        "where m.type = 'in-person' and m.date >= ? and m.date < ? "
        "  and e.actual_place > 0 and e.rated = 1",
        (frm, to),
    ).fetchall()
    val, place = {}, {}
    for tid, team, v, pl in rows:
        if team not in qual.get(tid, ()):
            continue
        if v is None:
            continue
        val.setdefault(tid, {})[team] = v
        place.setdefault(tid, {})[team] = pl
    return val, place


def old_system(qual, frm, to, metrics_db):
    """The old system's rating from the mirror, at the last release before the
    tournament.  Teams the release does not list are ranked last."""
    b = sqlite3.connect(BUFF)
    b.execute("attach database ? as m", (metrics_db,))
    rows = b.execute(
        "select m.tournament_id, m.date from m.tournament_metrics m "
        "where m.type = 'in-person' and m.date >= ? and m.date < ?",
        (frm, to),
    ).fetchall()
    rel = b.execute("select id, date from releases order by date").fetchall()
    out = {}
    for tid, date in rows:
        teams = qual.get(tid)
        if not teams:
            continue
        prev = [r for r in rel if r[1] < date]
        if not prev:
            continue
        rid = prev[-1][0]
        have = dict(
            b.execute("select team_id, rating from ratings where release_id = ?", (rid,))
        )
        out[tid] = {t: float(have.get(t, 0)) for t in teams}
    return out


def score(val, place, k=None):
    """NDCG per tournament over the qualifying subset, re-ranked."""
    out = {}
    for tid, strength in val.items():
        teams = list(strength)
        if len(teams) < 5:
            continue
        pl = place[tid]
        ranks = avg_ranks([pl[t] for t in teams])
        places = dict(zip(teams, ranks))
        order = sorted(teams, key=lambda t: (-strength[t], tiekey(tid, t)))
        v = ndcg(order, places, k)
        if v is not None:
            out[tid] = v
    return out


def weights():
    w = {}
    if os.path.exists(WEIGHTS):
        with open(WEIGHTS) as f:
            for r in csv.DictReader(f):
                w[int(r["tournament_id"])] = float(r["weight"])
    return w


def wmean(d, tids, w):
    """An empty weight table means every tournament counts once."""
    num = sum(w.get(t, 1.0 if not w else 0.0) * d[t] for t in tids if t in d)
    den = sum(w.get(t, 1.0 if not w else 0.0) for t in tids if t in d)
    return num / den if den > 0 else float("nan")


def umean(d, tids):
    v = [d[t] for t in tids if t in d]
    return sum(v) / len(v) if v else float("nan")


def boot(a, b, tids, w, n=2000, seed=20260914):
    """Paired bootstrap SE of the difference a - b over tournaments."""
    tids = [t for t in tids if t in a and t in b]
    if len(tids) < 5:
        return float("nan"), float("nan")
    rng = _random.Random(seed)
    dw, du = [], []
    for _ in range(n):
        s = [tids[rng.randrange(len(tids))] for _ in tids]
        num = sum(w.get(t, 0.0) * (a[t] - b[t]) for t in s)
        den = sum(w.get(t, 0.0) for t in s)
        dw.append(num / den if den > 0 else 0.0)
        du.append(sum(a[t] - b[t] for t in s) / len(s))
    def sd(x):
        m = sum(x) / len(x)
        return (sum((v - m) ** 2 for v in x) / (len(x) - 1)) ** 0.5
    return sd(dw), sd(du)


# --------------------------------------------------------------------- main

def main():
    args = sys.argv[1:]
    frm, to = WINDOWS["test"]
    dbs, common, use_old, nboot = [], False, False, 0
    column, rule, rebuild, unweighted = "mu_before", "maii", False, False
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--common":
            common, i = True, i + 1
        elif a == "--old":
            use_old, i = True, i + 1
        elif a == "--unweighted":
            unweighted, i = True, i + 1
        elif a == "--rebuild":
            rebuild, i = True, i + 1
        elif a == "--window":
            frm, to = WINDOWS[args[i + 1]]
            i += 2
        elif a == "--from":
            frm, i = args[i + 1], i + 2
        elif a == "--to":
            to, i = args[i + 1], i + 2
        elif a == "--boot":
            nboot, i = int(args[i + 1]), i + 2
        elif a == "--column":
            column, i = args[i + 1], i + 2
        elif a == "--rule":
            rule, i = args[i + 1], i + 2
        else:
            dbs.append(a)
            i += 1

    qual = load_qualify(rule, rebuild)
    w = {} if unweighted else weights()
    runs = {}
    for db in dbs:
        val, place = per_tournament(db, qual, frm, to, column)
        runs[db] = (score(val, place), score(val, place, 3), val, place)
    if use_old and dbs:
        val0, place0 = runs[dbs[0]][2], runs[dbs[0]][3]
        ov = old_system(qual, frm, to, dbs[0])
        ov = {t: {k: v for k, v in d.items() if k in val0.get(t, {})} for t, d in ov.items()}
        runs["old system"] = (score(ov, place0), score(ov, place0, 3), ov, place0)

    tids = None
    for r in runs.values():
        tids = set(r[0]) if tids is None else tids & set(r[0])
    print(f"NDCG on the team rating alone, in-person base-roster fields, {frm} to {to}")
    print(f"rule: {'MAII A.4.7' if rule == 'maii' else 'at least max(4, size-2) base players'}"
          f"{', common tournaments only' if common else ''}\n")
    print("| model | NDCG (w) | NDCG | NDCG@3 (w) | NDCG@3 | tournaments | teams/t |")
    print("|---|---:|---:|---:|---:|---:|---:|")
    base = None
    for db, (full, at3, val, _) in runs.items():
        sel = tids if common else set(full)
        n = len(sel & set(full))
        sizes = [len(val[t]) for t in sel if t in val]
        print(
            f"| {db} | {wmean(full, sel, w):.4f} | {umean(full, sel):.4f} | "
            f"{wmean(at3, sel, w):.4f} | {umean(at3, sel):.4f} | {n} | "
            f"{sum(sizes)/len(sizes):.1f} |"
        )
        if base is None:
            base = (db, full, at3)
    if nboot and len(runs) > 1:
        print(f"\nPaired bootstrap SE of the difference against `{base[0]}`, {nboot} resamples:\n")
        print("| model | d NDCG (w) | SE (w) | d NDCG | SE | d NDCG@3 (w) | SE (w) |")
        print("|---|---:|---:|---:|---:|---:|---:|")
        for db, (full, at3, _, _) in runs.items():
            if db == base[0]:
                continue
            sel = (tids if common else set(full) & set(base[1]))
            sw, su = boot(full, base[1], sel, w, nboot)
            s3w, _ = boot(at3, base[2], sel, w, nboot)
            print(
                f"| {db} | {wmean(full, sel, w) - wmean(base[1], sel, w):+.4f} | {sw:.4f} | "
                f"{umean(full, sel) - umean(base[1], sel):+.4f} | {su:.4f} | "
                f"{wmean(at3, sel, w) - wmean(base[2], sel, w):+.4f} | {s3w:.4f} |"
            )


if __name__ == "__main__":
    main()
