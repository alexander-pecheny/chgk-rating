"""What the research tools share: paths, the two windows, the frozen weights,
the in-person field query and the NDCG metric of `src/ndcg.rs`.

Every tool runs from the repository root, so the paths here are relative to it.
A tool that imports this module needs nothing else on `sys.path`: Python puts
the script's own directory, `tools/`, there.
"""

import argparse
import csv
import json
import math
import random
import re
import sqlite3

BUFF = "buff.db"
WEIGHTS = "data/tournament_weights.csv"

# The windows of src/ndcg.rs: decisions are made on tune, test is read once.
TUNE = ("2018-09-01", "2024-08-29")
TEST = ("2024-08-30", "2026-08-27")
WINDOWS = {"tune": TUNE, "test": TEST}

# A field of fewer teams than this is not scored.
MIN_FIELD = 5
# NDCG@3, the top-weighted second number of every table.
TOP = 3
# Paired bootstrap: resamples and seed, the same as `eval --paired`.
BOOT = 2000
SEED = 20260914
# Weighted correlations over fewer rows than this are not reported.
MIN_CORR = 30

M64 = (1 << 64) - 1
GOLDEN = 0x9E3779B97F4A7C15
# splitmix64's two multipliers and three shifts.
MIX = (0xBF58476D1CE4E5B9, 0x94D049BB133111EB)
SHIFTS = (30, 27, 31)
NAN = float("nan")


def cli(doc, options, intermixed=False):
    """Parse `sys.argv` by `options`, a list of `(name, add_argument kwargs)`;
    `--help` prints the tool's docstring."""
    ap = argparse.ArgumentParser(description=doc, formatter_class=argparse.RawDescriptionHelpFormatter)
    for name, kw in options:
        ap.add_argument(name, **kw)
    return ap.parse_intermixed_args() if intermixed else ap.parse_args()


def window(spec):
    """`tune`, `test`, or `FROM:TO`."""
    return WINDOWS.get(spec) or tuple(spec.split(":"))


def connect(path):
    """A read-only connection; a missing file is an error, not an empty database."""
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def mix(z):
    """The splitmix64 finaliser of `util::Rng`."""
    a, b, c = SHIFTS
    z = ((z ^ (z >> a)) * MIX[0]) & M64
    z = ((z ^ (z >> b)) * MIX[1]) & M64
    return z ^ (z >> c)


class Rng:
    """`util::Rng` of the Rust side, so both implementations break ties alike.

    Forecasts do tie -- two teams with the same lineup strength -- and the rows
    are stored in finishing order, so the tie must not be broken by row order.
    """

    def __init__(self, seed):
        self.s = (seed * GOLDEN | 1) & M64

    def next_u64(self):
        self.s = (self.s + GOLDEN) & M64
        return mix(self.s)

    def shuffle(self, v):
        for i in range(len(v) - 1, 0, -1):
            j = self.next_u64() % (i + 1)
            v[i], v[j] = v[j], v[i]


def ranked(items, score, rng):
    """`items` best first by `score`, ties broken by a shuffle from `rng`.

    `rng` is usually `Rng(tournament_id)`, so the stored finishing order never
    leaks into the rank."""
    order = list(items)
    rng.shuffle(order)
    order.sort(key=lambda t: -score(t))
    return order


def dcg(rel):
    return sum(r / math.log2(i + 2) for i, r in enumerate(rel))


def ndcg(order, places, k=None, values=None):
    """`order`: teams sorted by forecast, best first; `places`: actual place per
    team. Relevance is `1/place`. `values` is unused; it lets `spearman` share
    the signature."""
    rel = [1.0 / places[t] for t in order]
    k = k or len(rel)
    idcg = dcg(sorted(rel, reverse=True)[:k])
    return dcg(rel[:k]) / idcg if idcg > 0 else None


def ranks(vals):
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


def load_weights(path=WEIGHTS):
    """tournament_id -> weight. A tournament the table does not name weighs nothing."""
    with open(path, newline="", encoding="utf-8") as f:
        return {int(r["tournament_id"]): float(r["weight"]) for r in csv.DictReader(f)}


def weigher(w):
    """The weight of a tournament under `w`; `None` counts every one once."""
    return (lambda t: 1.0) if w is None else (lambda t: w.get(t, 0.0))


def wmean(d, tids, w=None):
    """Mean of `d` over the `tids` it has a value for, weighted as `weigher`."""
    wt = weigher(w)
    v = [(t, d[t]) for t in tids if t in d and d[t] is not None]
    den = sum(wt(t) for t, _ in v)
    num = sum(wt(t) * x for t, x in v)
    return num / den if den > 0 else NAN


def paired(a, b, keys, w=None, n=BOOT):
    """Weighted mean of `a - b` over `keys` and its paired bootstrap standard
    error, resampling tournaments. `w` as in `wmean`."""
    keys = sorted(keys)
    rng = random.Random(SEED)
    point = wdiff(a, b, keys, weigher(w))
    draws = [wdiff(a, b, [keys[rng.randrange(len(keys))] for _ in keys], weigher(w)) for _ in range(n)]
    m = sum(draws) / len(draws)
    return point, (sum((d - m) ** 2 for d in draws) / len(draws)) ** 0.5


def wdiff(a, b, sample, wt):
    den = sum(wt(t) for t in sample)
    num = sum(wt(t) * (a[t] - b[t]) for t in sample)
    return num / den if den > 0 else NAN


def wcorr(rows, x, y, w, min_n=MIN_CORR):
    """Weighted Pearson correlation of two keys over the rows that have both,
    and how many those are; `nan` under `min_n` rows."""
    v = [(r[w], r[x], r[y]) for r in rows if r[x] is not None and r[y] is not None]
    sw = sum(t[0] for t in v) if len(v) >= min_n else 0
    return (wpearson(v, sw) if sw > 0 else NAN), len(v)


def wpearson(v, sw):
    """Pearson of `(weight, x, y)` triples whose weights sum to `sw`."""
    mx = sum(t[0] * t[1] for t in v) / sw
    my = sum(t[0] * t[2] for t in v) / sw
    cov = sum(t[0] * (t[1] - mx) * (t[2] - my) for t in v)
    vx = sum(t[0] * (t[1] - mx) ** 2 for t in v)
    vy = sum(t[0] * (t[2] - my) ** 2 for t in v)
    return cov / math.sqrt(vx * vy) if vx > 0 and vy > 0 else NAN


def header(names, aligns):
    """A markdown table's two header lines."""
    print(names)
    print(aligns)


def side_by_side(labels, cols, n):
    """`n` ranked `(name, value)` rows of each column, one table, `-` past the end."""
    header("| # | " + " | ".join(labels) + " |", "|---:|" + "---|" * len(labels))
    for i in range(n):
        cells = [f"{c[i][0]} {c[i][1]:.0f}" if i < len(c) else "-" for c in cols]
        print(f"| {i + 1} | " + " | ".join(cells) + " |")


# --------------------------------------------------------------- the configs

HEADER = re.compile(r"\s*\[([^\]]+)\]")
KEY = re.compile(r"\s*([A-Za-z_][A-Za-z0-9_]*)\s*=")


def override_toml(lines, overrides):
    """`lines` of a config with `section.key=value` overrides applied: a key is
    replaced where it stands, a key its section lacks goes at the section's end,
    and a section the config lacks is appended. Returns the new lines and the
    sections that had to be appended."""
    want = dict(parse_override(o) for o in overrides)
    out, section, done = [], "", set()
    for line in lines:
        m = HEADER.match(line)
        if m:
            place_missing(out, want, done, section)
            section = m.group(1)
        k = KEY.match(line)
        if k and (section, k.group(1)) in want:
            line = f"{k.group(1)} = {want[(section, k.group(1))]}"
            done.add((section, k.group(1)))
        out.append(line)
    return out, append_missing(out, want, done, section)


def parse_override(o):
    """`section.key=value` as `((section, key), value)`."""
    k, v = o.split("=", 1)
    return tuple(k.rsplit(".", 1)), v


def place_missing(out, want, done, section):
    """Anything still unplaced for the section just closed goes in at its end."""
    for (sec, key), v in want.items():
        if sec == section and (sec, key) not in done:
            out.append(f"{key} = {v}")
            done.add((sec, key))


def append_missing(out, want, done, section):
    added = []
    for (sec, key), v in want.items():
        if (sec, key) not in done:
            if sec != section:
                out.append(f"[{sec}]")
                section = sec
                added.append(sec)
            out.append(f"{key} = {v}")
    return added


# ----------------------------------------------------------------- the data

IN_PERSON = """
select {cols}
from team_events e
join tournament_metrics m on m.tournament_id = e.tournament_id
where m.type = 'in-person' and m.date >= ? and m.date < ? and e.actual_place > 0
{where}
{order}
"""
PLAYER_STATE = """
select tournament_id, player_id, theta_before, phi_before, sigma_before, games_ip, days_ip
from player_events
"""


def in_person(db, frm, to, cols, where="", order="e.tournament_id"):
    """Placed results of in-person tournaments dated in `[frm, to)`: the join
    every metric is computed over. `where` adds `and ...` clauses."""
    sql = IN_PERSON.format(cols=cols, where=where, order=f"order by {order}" if order else "")
    return sqlite3.connect(db).execute(sql, (frm, to)).fetchall()


def by_tournament(rows):
    """`{tournament_id: [rest of the row]}`, in row order."""
    out = {}
    for row in rows:
        out.setdefault(row[0], []).append(row[1:])
    return out


def player_state(conn, tids=None):
    """`{tournament: {player: (theta, phi, sigma, games_ip, days_ip)}}` as the
    forecast read them on that morning."""
    out = {}
    for tid, p, *st in conn.execute(PLAYER_STATE):
        if tids is None or tid in tids:
            out.setdefault(tid, {})[p] = tuple(st)
    return out


def lineups(buff, tid):
    """`{team: [player ids]}` of one tournament, from the mirror."""
    q = "select team_id, team_members from tournament_results where id = ?"
    return {team: json.loads(ms) if ms else [] for team, ms in buff.execute(q, (tid,))}


def names(ids, table, col):
    """`{id: col}` from a table of the mirror."""
    if not ids:
        return {}
    q = f"select id, {col} from {table} where id in ({','.join('?' * len(ids))})"
    return dict(sqlite3.connect(BUFF).execute(q, list(ids)).fetchall())
