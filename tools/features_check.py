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
import datetime as dt
import math
import sqlite3
import sys
from collections import defaultdict
from operator import itemgetter

NOT_FEATURES = ("tournament_id", "team_id", "date", "y_", "w_", "m_")
EPS = 1e-9
# The census reads this many fields, and names a feature whose mean |corr| with
# the place is above the bar.
CENSUS_FIELDS = 400
CENSUS_BAR = 0.9
# The re-derived memories: the last five surprises, and the last year.
LAST = 5
YEAR = 365
DECIMAL = 10
ROWS = "select * from features order by date, tournament_id, rowid"


def cols_of(con):
    return [r[1] for r in con.execute("pragma table_info(features)")]


def features(cols):
    return [c for c in cols if not c.startswith(NOT_FEATURES)]


def rows_of(path):
    con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    cols, rows = cols_of(con), con.execute(ROWS).fetchall()
    con.close()
    return cols, rows


def same(a, b):
    if a is None or b is None:
        return a is None and b is None
    if isinstance(a, str) or isinstance(b, str):
        return a == b
    return abs(a - b) <= EPS * max(1.0, abs(a), abs(b))


# --------------------------------------------------------------------------- #


def abs_corr(v):
    """|Pearson| of `(x, y)` pairs, or None when either side is constant."""
    n = len(v)
    mx, my = sum(x for x, _ in v) / n, sum(y for _, y in v) / n
    sx = math.sqrt(sum((x - mx) ** 2 for x, _ in v))
    sy = math.sqrt(sum((y - my) ** 2 for _, y in v))
    if sx <= 0 or sy <= 0:
        return None
    return abs(sum((x - mx) * (y - my) for x, y in v) / (sx * sy))


def mean_corr(fields, i, place):
    """Mean |corr| of column `i` with the place within each field of five or more."""
    cs = [abs_corr(v) for v in ([(t[i], t[place]) for t in ts if t[i] is not None] for ts in fields) if len(v) >= 5]
    cs = [c for c in cs if c is not None]
    return sum(cs) / len(cs) if cs else 0.0


CLEAN_CENSUS = (
    "No feature correlates with the finishing place above 0.90 within a "
    "field; the best of them is the model's own forecast, which is not a "
    "feature."
)


def first_fields(rows, ix):
    """The rows of the first `CENSUS_FIELDS` tournaments, field by field."""
    by_t = defaultdict(list)
    for r in rows:
        by_t[r[ix["tournament_id"]]].append(r)
    return list(by_t.values())[:CENSUS_FIELDS]


def census(cols, rows, out):
    """Anything that orders a field as well as the finishing order itself would
    be a copy of the target under another name."""
    feats = features(cols)
    out.append(f"Columns: {len(cols)} in all, {len(feats)} of them features.")
    ix = {c: i for i, c in enumerate(cols)}
    fields = first_fields(rows, ix)
    bad = [(c, v) for c in feats for v in [mean_corr(fields, ix[c], ix["y_place"])] if v > CENSUS_BAR]
    if bad:
        out.append("**A feature orders a field as well as its own result does:**")
    out += [f"  - `{c}` mean |corr| with the place {v:.3f}" for c, v in bad] or [CLEAN_CENSUS]
    return not bad


def editor_ids(e):
    """The integers in the mirror's `editors` text, whatever its shape."""
    ids, cur, have = [], 0, False
    for ch in str(e or "") + " ":
        if ch.isdigit():
            cur, have = cur * DECIMAL + int(ch), True
        elif have:
            ids.append(cur)
            cur, have = 0, False
    return ids


def day_number(s):
    """A real day number, so a 365-day window means what it says."""
    return dt.date.fromisoformat(s).toordinal()


class Memories:
    """The outcome memories of `fit`, rebuilt from strictly earlier rows."""

    def __init__(self, ix, ed):
        self.ix, self.ed = ix, ed
        self.surp, self.place = defaultdict(list), defaultdict(list)
        self.te = defaultdict(lambda: [0.0, 0.0])

    def absorb(self, r):
        """Add one result, once its whole field has been checked."""
        ix = self.ix
        n, fp, place = r[ix["m_n_events"]], r[ix["m_forecast_place"]], r[ix["y_place"]]
        # A team with neither a lineup nor a state is not in the field the
        # engine scored, so it has no event and no residual either.
        if fp is None or n is None or r[ix["y_rated"]] != 1 or place <= 0:
            return
        team, d, resid = r[ix["team_id"]], day_number(r[ix["date"]]), (fp - place) / n
        self.surp[team].append((d, resid))
        self.place[team].append((d, place / n))
        for e in self.ed.get(r[ix["tournament_id"]], []):
            a = self.te[(team, e)]
            a[0], a[1] = a[0] + resid, a[1] + 1.0

    def want(self, r):
        """What every outcome column of `r` should say."""
        ix = self.ix
        team, tid, d = r[ix["team_id"]], r[ix["tournament_id"]], day_number(r[ix["date"]])
        s = [x for _, x in self.surp[team][-LAST:]]
        s365 = [x for dd, x in self.surp[team] if dd >= d - YEAR]
        p365 = [x for dd, x in self.place[team] if dd >= d - YEAR]
        acc = [self.te[(team, e)] for e in self.ed.get(tid, []) if self.te[(team, e)][1] > 0]
        return {
            "tm_surp5_mean": mean(s), "tm_surp365_mean": mean(s365), "tm_surp365_n": float(len(s365)),
            "tm_best_place365": min(p365) if p365 else None, "tm_worst_place365": max(p365) if p365 else None,
            "ed_team_n": float(len(acc)), "ed_team_resid": mean([a[0] / a[1] for a in acc]),
            "ed_team_tournaments": float(sum(self.te[(team, e)][1] for e in self.ed.get(tid, []))),
        }


def mean(v):
    return sum(v) / len(v) if v else None


class Tally:
    """Rows checked, disagreements and the largest gap per column."""

    def __init__(self):
        self.checked, self.wrong, self.worst = defaultdict(int), defaultdict(int), defaultdict(float)

    def check(self, r, ix, want):
        for c, v in want.items():
            got = r[ix[c]]
            self.checked[c] += 1
            # A count of zero is written as a zero and a mean of nothing as NULL.
            if (v is None) != (got is None) or (v is not None and not same(v, got)):
                self.wrong[c] += 1
                if v is not None and got is not None:
                    self.worst[c] = max(self.worst[c], abs(v - got))


def editors(dbpath):
    b = sqlite3.connect(f"file:{dbpath}?mode=ro", uri=True)
    ed = {tid: editor_ids(e) for tid, e in b.execute("select id, editors from tournaments")}
    b.close()
    return ed


def tally_rows(rows, ix, mem):
    """Check every row against the memories, absorbing a field only once the
    next one starts, so a row never sees its own field."""
    tally, cur_t, batch = Tally(), None, []
    for r in rows:
        if r[ix["tournament_id"]] != cur_t:
            for b in batch:
                mem.absorb(b)
            batch, cur_t = [], r[ix["tournament_id"]]
        tally.check(r, ix, mem.want(r))
        batch.append(r)
    return tally


def rederive(path, dbpath, out):
    """The outcome memories, recomputed from strictly earlier rows of the table."""
    cols, rows = rows_of(path)
    ix = {c: i for i, c in enumerate(cols)}
    tally = tally_rows(rows, ix, Memories(ix, editors(dbpath)))
    out += ["", "| outcome column | rows checked | disagreements | largest |", "|---|---:|---:|---:|"]
    out += [f"| `{c}` | {tally.checked[c]} | {tally.wrong[c]} | {tally.worst[c]:.2e} |" for c in sorted(tally.checked)]
    return all(tally.wrong[c] == 0 for c in tally.checked)


def moved_columns(A, B, ix, feats, where):
    """Per place relative to the scrambled field, how often each column moved."""
    moved = {"before": defaultdict(int), "on": defaultdict(int), "after": defaultdict(int)}
    counts = defaultdict(int)
    for k, a in A.items():
        b = B.get(k)
        if b is not None:
            w = where(a[ix["date"]], a[ix["tournament_id"]])
            counts[w] += 1
            for c in (c for c in feats if not same(a[ix[c]], b[ix[c]])):
                moved[w][c] += 1
    return moved, counts


def scramble_report(out, date, field, moved, counts, n_feats):
    """`field` is `(day, tournament, fields after it)`."""
    day, tid, n_later = field
    before = sum(moved["before"].values())
    out.append("")
    out.append(
        f"Scrambled the first in-person field on or after {date}: tournament "
        f"{tid} of {day}, {counts['on']} results, with {counts['after']} results in the "
        f"{n_later} fields that follow it."
    )
    out.append(f"Rows of the fields before it that moved: {before}.")
    out += moved_on(moved["on"], counts["on"], n_feats)
    out.append(f"Columns that moved in the fields after it, as they must: {len(moved['after'])} of {n_feats}.")
    return not moved["on"] and before == 0 and len(moved["after"]) > 0


def moved_on(moved, n_on, n_feats):
    if not moved:
        return [f"**No feature of the {n_on} results moved** when that field's own "
                f"places and masks were permuted: not one of the {n_feats} columns."]
    return ["**Columns that moved in the scrambled field itself:**"] + [
        f"  - `{c}` in {n} of {n_on} rows" for c, n in sorted(moved.items(), key=lambda kv: -kv[1])]


def scrambled_field(ra, ix, date):
    """`fit` reads the tournaments in (date, id) order, so the field it scrambled
    is the first of those on or after the date: `(day, tournament, fields after)`."""
    later = sorted({(r[ix["date"]], r[ix["tournament_id"]]) for r in ra if r[ix["date"]] >= date})
    return (*later[0], len(later) - 1) if later else None


def scramble(a_path, b_path, date, out):
    (ca, ra), (cb, rb) = rows_of(a_path), rows_of(b_path)
    if ca != cb:
        out.append("The two tables have different columns; the check cannot run.")
        return False
    ix = {c: i for i, c in enumerate(ca)}
    field = scrambled_field(ra, ix, date)
    if field is None:
        out.append(f"No in-person field on or after {date}.")
        return False
    key = itemgetter(ix["tournament_id"], ix["team_id"])
    moved, counts = moved_columns({key(r): r for r in ra}, {key(r): r for r in rb}, ix, features(ca), placement(*field[:2]))
    return scramble_report(out, date, field, moved, counts, len(features(ca)))


def placement(day, tid):
    """Where a field stands against the scrambled one: before, on or after it."""
    return lambda d, t: "on" if t == tid else ("before" if (d, t) < (day, tid) else "after")


OPTIONS = [
    ("--a", dict(default="out/discovery/features.db")),
    ("--b", dict()),
    ("--date", dict()),
    ("--db", dict(default="buff.db")),
]


def verdict(ok1, ok2, ok3, scrambled):
    return (
        f"**{'PASSED' if ok1 and ok2 and ok3 else 'FAILED'}**: census "
        f"{'ok' if ok1 else 'failed'}, re-derivation {'ok' if ok2 else 'failed'}, "
        f"scramble {'ok' if ok3 else 'failed' if scrambled else 'not run'}."
    )


def parse():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for name, kw in OPTIONS:
        ap.add_argument(name, **kw)
    return ap.parse_args()


def main():
    args = parse()
    out = ["# The leakage check", ""]
    ok1 = census(*rows_of(args.a), out)
    out += ["", "## Re-derived from strictly earlier rows"]
    ok2 = rederive(args.a, args.db, out)
    ok3 = True
    if args.b and args.date:
        out += ["", "## The scramble test"]
        ok3 = scramble(args.a, args.b, args.date, out)
    out += ["", verdict(ok1, ok2, ok3, args.b)]
    print("\n".join(out))
    return 0 if (ok1 and ok2 and ok3) else 1


if __name__ == "__main__":
    sys.exit(main())
