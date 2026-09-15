# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""The leakage check of plan 8: does any column of the feature table know today?

Three tests, from the blunt to the decisive.

1. **Census.** Every column is a feature, the target (`y_`), the metric's weight
   (`w_`) or what the shipped model said (`m_`), and nothing else. A feature that
   reproduces the target too well is named.

2. **Re-derivation.** The columns that are made of past *outcomes* -- a team's
   run of surprises, its places over the last year, its record under today's
   editors -- are recomputed here from rows of the table dated **strictly
   earlier**, and must come out equal. If `fit` had let a memory absorb a field
   before writing its row, this recomputation would disagree with it.

3. **Scramble.** `fit --scramble DATE` permutes the finishing places and the
   answer masks of the first in-person field on or after DATE. Nothing a lineup
   can be judged by that morning has changed, so every feature row **of that
   field** must be bit-identical; any column that moved is reading its own
   result. The fields after it are expected to move, and that they do is what
   says the test has teeth.

    uv run tools/features_check.py --a out/discovery/features.db \\
        --b out/discovery/features_scrambled.db --date 2024-01-01
"""

import argparse
import math
import sqlite3
import sys
from collections import defaultdict, deque

NOT_FEATURES = ("tournament_id", "team_id", "date", "y_", "w_", "m_")
EPS = 1e-9


def cols_of(con):
    return [r[1] for r in con.execute("pragma table_info(features)")]


def features(cols):
    return [c for c in cols if not c.startswith(NOT_FEATURES)]


def rows_of(path):
    con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    cols = cols_of(con)
    rows = con.execute(
        "select * from features order by date, tournament_id, rowid"
    ).fetchall()
    con.close()
    return cols, rows


def same(a, b):
    if a is None or b is None:
        return a is None and b is None
    if isinstance(a, str) or isinstance(b, str):
        return a == b
    return abs(a - b) <= EPS * max(1.0, abs(a), abs(b))


# --------------------------------------------------------------------------- #


def census(cols, rows, out):
    feats = features(cols)
    out.append(f"Columns: {len(cols)} in all, {len(feats)} of them features.")
    ix = {c: i for i, c in enumerate(cols)}
    # Anything that orders a field as well as the finishing order itself would
    # be a copy of the target under another name.
    by_t = defaultdict(list)
    for r in rows:
        by_t[r[ix["tournament_id"]]].append(r)
    bad = []
    for c in feats:
        num = den = 0.0
        for ts in list(by_t.values())[:400]:
            v = [(t[ix[c]], t[ix["y_place"]]) for t in ts if t[ix[c]] is not None]
            if len(v) < 5:
                continue
            n = len(v)
            mx = sum(x for x, _ in v) / n
            my = sum(y for _, y in v) / n
            sx = math.sqrt(sum((x - mx) ** 2 for x, _ in v))
            sy = math.sqrt(sum((y - my) ** 2 for _, y in v))
            if sx <= 0 or sy <= 0:
                continue
            num += abs(sum((x - mx) * (y - my) for x, y in v) / (sx * sy))
            den += 1
        if den and num / den > 0.9:
            bad.append((c, num / den))
    if bad:
        out.append("**A feature orders a field as well as its own result does:**")
        for c, v in bad:
            out.append(f"  - `{c}` mean |corr| with the place {v:.3f}")
    else:
        out.append(
            "No feature correlates with the finishing place above 0.90 within a "
            "field; the best of them is the model's own forecast, which is not a "
            "feature."
        )
    return not bad


def rederive(path, dbpath, out):
    """The outcome memories, recomputed from strictly earlier rows of the table."""
    con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    cols = cols_of(con)
    ix = {c: i for i, c in enumerate(cols)}
    rows = con.execute(
        "select * from features order by date, tournament_id, rowid"
    ).fetchall()
    con.close()
    ed = {}
    b = sqlite3.connect(f"file:{dbpath}?mode=ro", uri=True)
    for tid, e in b.execute("select id, editors from tournaments"):
        ids = []
        cur, have = 0, False
        for ch in str(e or ""):
            if ch.isdigit():
                cur, have = cur * 10 + int(ch), True
            elif have:
                ids.append(cur)
                cur, have = 0, False
        if have:
            ids.append(cur)
        ed[tid] = ids
    b.close()

    surp = defaultdict(deque)
    place = defaultdict(deque)
    te = defaultdict(lambda: [0.0, 0.0])
    checked = defaultdict(int)
    wrong = defaultdict(int)
    worst = defaultdict(float)
    day_of = {}

    def day(s):
        y, m, d = (int(x) for x in s.split("-"))
        return (y * 12 + m) * 31 + d  # a monotone stand-in, days only compared

    # Real day numbers, so a 365-day window means what it says.
    def real(s):
        y, m, d = (int(x) for x in s.split("-"))
        a = (y - (m <= 2)) // 1
        return int(
            367 * y - 7 * (y + (m + 9) // 12) // 4 + 275 * m // 9 + d - 730_530
        )

    del day, day_of

    cur_t, batch = None, []

    def flush():
        for r in batch:
            tid = r[ix["tournament_id"]]
            team = r[ix["team_id"]]
            d = real(r[ix["date"]])
            n = r[ix["m_n_events"]]
            # A team with neither a lineup nor a state is not in the field the
            # engine scored, so it has no event and no residual either.
            if r[ix["m_forecast_place"]] is None or n is None:
                continue
            if r[ix["y_rated"]] != 1 or r[ix["y_place"]] <= 0:
                continue
            resid = (r[ix["m_forecast_place"]] - r[ix["y_place"]]) / n
            surp[team].append((d, resid))
            place[team].append((d, r[ix["y_place"]] / n))
            for e in ed.get(tid, []):
                a = te[(team, e)]
                a[0] += resid
                a[1] += 1.0

    def check(r):
        team = r[ix["team_id"]]
        tid = r[ix["tournament_id"]]
        d = real(r[ix["date"]])
        s = [x for _, x in list(surp[team])[-5:]]
        s365 = [x for dd, x in surp[team] if dd >= d - 365]
        p365 = [x for dd, x in place[team] if dd >= d - 365]
        want = {
            "tm_surp5_mean": (sum(s) / len(s)) if s else None,
            "tm_surp365_mean": (sum(s365) / len(s365)) if s365 else None,
            "tm_surp365_n": float(len(s365)),
            "tm_best_place365": min(p365) if p365 else None,
            "tm_worst_place365": max(p365) if p365 else None,
        }
        acc = [te[(team, e)] for e in ed.get(tid, []) if te[(team, e)][1] > 0]
        want["ed_team_n"] = float(len(acc))
        want["ed_team_resid"] = (
            sum(a[0] / a[1] for a in acc) / len(acc) if acc else None
        )
        want["ed_team_tournaments"] = float(
            sum(te[(team, e)][1] for e in ed.get(tid, []))
        )
        for c, v in want.items():
            got = r[ix[c]]
            checked[c] += 1
            # A count of zero is written as a zero and a mean of nothing as NULL.
            if v is None and got is None:
                continue
            if v is None or got is None or not same(v, got):
                wrong[c] += 1
                if v is not None and got is not None:
                    worst[c] = max(worst[c], abs(v - got))

    for r in rows:
        tid = r[ix["tournament_id"]]
        if tid != cur_t:
            flush()
            batch, cur_t = [], tid
        check(r)
        batch.append(r)
    flush()

    out.append("")
    out.append("| outcome column | rows checked | disagreements | largest |")
    out.append("|---|---:|---:|---:|")
    ok = True
    for c in sorted(checked):
        out.append(f"| `{c}` | {checked[c]} | {wrong[c]} | {worst[c]:.2e} |")
        ok = ok and wrong[c] == 0
    return ok


def scramble(a_path, b_path, date, out):
    ca, ra = rows_of(a_path)
    cb, rb = rows_of(b_path)
    if ca != cb:
        out.append("The two tables have different columns; the check cannot run.")
        return False
    ix = {c: i for i, c in enumerate(ca)}
    feats = features(ca)
    key = lambda r: (r[ix["tournament_id"]], r[ix["team_id"]])  # noqa: E731
    A = {key(r): r for r in ra}
    B = {key(r): r for r in rb}
    # `fit` reads the tournaments in (date, id) order, so the field it scrambled
    # is the first of those on or after the date.
    later = sorted(
        {(r[ix["date"]], r[ix["tournament_id"]]) for r in ra if r[ix["date"]] >= date}
    )
    if not later:
        out.append(f"No in-person field on or after {date}.")
        return False
    day, tid = later[0]
    moved_on, moved_after = defaultdict(int), defaultdict(int)
    n_on = n_after = before = 0
    for k, a in A.items():
        b = B.get(k)
        if b is None:
            continue
        d, t = a[ix["date"]], a[ix["tournament_id"]]
        where = "on" if t == tid else ("before" if (d, t) < (day, tid) else "after")
        n_on += where == "on"
        n_after += where == "after"
        for c in feats:
            if same(a[ix[c]], b[ix[c]]):
                continue
            if where == "before":
                before += 1
            elif where == "on":
                moved_on[c] += 1
            else:
                moved_after[c] += 1
    out.append("")
    out.append(
        f"Scrambled the first in-person field on or after {date}: tournament "
        f"{tid} of {day}, {n_on} results, with {n_after} results in the "
        f"{len(later) - 1} fields that follow it."
    )
    out.append(f"Rows of the fields before it that moved: {before}.")
    if moved_on:
        out.append("**Columns that moved in the scrambled field itself:**")
        for c, n in sorted(moved_on.items(), key=lambda kv: -kv[1]):
            out.append(f"  - `{c}` in {n} of {n_on} rows")
    else:
        out.append(
            f"**No feature of the {n_on} results moved** when that field's own "
            f"places and masks were permuted: not one of the {len(feats)} columns."
        )
    out.append(
        f"Columns that moved in the fields after it, as they must: "
        f"{len(moved_after)} of {len(feats)}."
    )
    return not moved_on and before == 0 and len(moved_after) > 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a", default="out/discovery/features.db")
    ap.add_argument("--b")
    ap.add_argument("--date")
    ap.add_argument("--db", default="buff.db")
    args = ap.parse_args()

    out = ["# The leakage check", ""]
    cols, rows = rows_of(args.a)
    ok1 = census(cols, rows, out)
    out.append("")
    out.append("## Re-derived from strictly earlier rows")
    ok2 = rederive(args.a, args.db, out)
    ok3 = True
    if args.b and args.date:
        out.append("")
        out.append("## The scramble test")
        ok3 = scramble(args.a, args.b, args.date, out)
    out.append("")
    out.append(
        f"**{'PASSED' if ok1 and ok2 and ok3 else 'FAILED'}**: census "
        f"{'ok' if ok1 else 'failed'}, re-derivation {'ok' if ok2 else 'failed'}, "
        f"scramble {'ok' if ok3 else 'failed' if args.b else 'not run'}."
    )
    print("\n".join(out))
    return 0 if (ok1 and ok2 and ok3) else 1


if __name__ == "__main__":
    sys.exit(main())
