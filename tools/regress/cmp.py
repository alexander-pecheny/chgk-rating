"""cmp.py BASE NEW: compare two golden.sh output directories.

Databases: every table, row for row after sorting; floats must match to 1e-9
relative. Text outputs: exact, except timing lines.
"""
import math
import os
import re
import sqlite3
import sys
from pathlib import Path

base, new = Path(sys.argv[1]), Path(sys.argv[2])
os.environ.setdefault("SQLITE_TMPDIR", str(new.resolve()))
TIMING = re.compile(r"(\d+(\.\d+)?\s*(ms|s)\b|elapsed|took|in \d)", re.I)
bad = 0


def close(a, b):
    if isinstance(a, float) and isinstance(b, float):
        if math.isnan(a) and math.isnan(b):
            return True
        return abs(a - b) <= 1e-9 * max(1.0, abs(a), abs(b))
    return a == b


def cmp_db(a, b):
    """Row sets compared inside SQLite, so a table larger than memory works."""
    global bad
    c = sqlite3.connect(a)
    c.execute(f"ATTACH '{b}' AS n")
    ta = {r[0] for r in c.execute("select name from main.sqlite_master where type='table'")}
    tb = {r[0] for r in c.execute("select name from n.sqlite_master where type='table'")}
    if ta != tb:
        print(f"  {a.name}: tables differ: -{sorted(ta - tb)} +{sorted(tb - ta)}")
        bad += 1
    for t in sorted(ta & tb):
        ca_ = [r[1] for r in c.execute(f'pragma main.table_info("{t}")')]
        cb_ = [r[1] for r in c.execute(f'pragma n.table_info("{t}")')]
        if ca_ != cb_:
            bad += 1
            print(f"  {a.name}.{t}: columns differ: -{[x for x in ca_ if x not in cb_]} +{[x for x in cb_ if x not in ca_]}")
        cols = ", ".join(f'"{x}"' for x in ca_ if x in cb_)
        na = c.execute(f'select count(*) from main."{t}"').fetchone()[0]
        nb = c.execute(f'select count(*) from n."{t}"').fetchone()[0]
        diff = lambda x, y: f'select {cols} from {x}."{t}" except select {cols} from {y}."{t}"'
        only_a = c.execute(f"select count(*) from ({diff('main', 'n')})").fetchone()[0]
        only_b = c.execute(f"select count(*) from ({diff('n', 'main')})").fetchone()[0]
        if na != nb or only_a or only_b:
            bad += 1
            print(f"  {a.name}.{t}: {na} vs {nb} rows, {only_a} only in base, {only_b} only in new")
            print(f"    e.g. base {c.execute(diff('main', 'n') + ' limit 1').fetchone()}")


def cmp_text(a, b):
    global bad
    la = [l.replace(str(base), "OUT") for l in a.read_text(errors="replace").splitlines() if not TIMING.search(l)]
    lb = [l.replace(str(new), "OUT") for l in b.read_text(errors="replace").splitlines() if not TIMING.search(l)]
    if la != lb:
        bad += 1
        n = next((i for i, (x, y) in enumerate(zip(la, lb)) if x != y), min(len(la), len(lb)))
        print(f"  {a.relative_to(base)}: differs at line {n + 1} ({len(la)} vs {len(lb)} lines)")
        print(f"    base {la[n] if n < len(la) else '<eof>'}\n    new  {lb[n] if n < len(lb) else '<eof>'}")


for f in sorted(base.rglob("*")):
    g = new / f.relative_to(base)
    if f.is_dir() or f.suffix == ".err":
        continue
    if not g.exists():
        print(f"  missing {g}")
        bad += 1
    elif f.suffix == ".db":
        cmp_db(f, g)
    else:
        cmp_text(f, g)
print("SAME" if bad == 0 else f"{bad} DIFFERENCES")
