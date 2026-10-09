# /// script
# requires-python = ">=3.10"
# dependencies = ["httpx"]
# ///
"""Players entered under two ids that rating.chgk.info has since merged.

Finds pairs of ids in buff.db with the same full name who played for a common
team and never at the same tournament, then asks api.rating.chgk.info about
both. Where one id is gone and the other answers, the official site has merged
them, and the gone one becomes an alias of the other in data/player_aliases.csv,
which `data.player_aliases` reads.

    uv run tools/fetch/player_merges.py              # writes data/player_aliases.csv
    uv run tools/fetch/player_merges.py --dry        # prints the pairs, writes nothing
"""

import argparse
import csv
import itertools
import json
import os
import sqlite3
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import lab  # noqa: E402
from polite import Polite  # noqa: E402

API = "https://api.rating.chgk.info/players/{}"
OUT = "data/player_aliases.csv"
GAP_SECONDS = 0.4
TIMEOUT_SECONDS = 20


def full_names(b):
    """Every player's (surname, name, patronymic), lower-cased, ё as е."""
    fold = lambda s: (s or "").strip().lower().replace("ё", "е")
    rows = b.execute("select id, surname, name, patronymic from players")
    return {pid: (fold(s), fold(n), fold(p)) for pid, s, n, p in rows}


def appearances(b):
    """Per player, the teams they played for and the tournaments they played."""
    teams, tours = defaultdict(set), defaultdict(set)
    for tid, team, members in b.execute("select id, team_id, team_members from tournament_results"):
        for pid in json.loads(members or "[]"):
            teams[pid].add(team)
            tours[pid].add(tid)
    return teams, tours


def candidates(b):
    """Pairs with the same full name, a common team and no common tournament."""
    names = full_names(b)
    teams, tours = appearances(b)
    by_name = defaultdict(list)
    for pid, n in names.items():
        if n[0] and n[1] and pid in teams:
            by_name[n].append(pid)
    for ids in by_name.values():
        for a, c in itertools.combinations(sorted(ids), 2):
            if teams[a] & teams[c] and not tours[a] & tours[c]:
                yield a, c


def merged(pairs, http):
    """`(alias, player)` for each pair where exactly one id still exists upstream."""
    alive = {}
    for pid in sorted({p for pair in pairs for p in pair}):
        got = http.get(API.format(pid), missing=False, label=f"player {pid}")
        # True: the id answers; False: a 404; None: the request failed, so
        # the pair is left alone rather than guessed.
        alive[pid] = None if got is None else got is not False
    for a, c in pairs:
        if None not in (alive[a], alive[c]) and alive[a] != alive[c]:
            yield (a, c) if alive[c] else (c, a)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--dry", action="store_true", help="print the pairs, write nothing")
    a = ap.parse_args()
    b = sqlite3.connect(lab.BUFF)
    pairs = list(candidates(b))
    print(f"{len(pairs)} candidate pairs", file=sys.stderr)
    found = list(merged(pairs, Polite(GAP_SECONDS, TIMEOUT_SECONDS)))
    print(f"{len(found)} merged upstream", file=sys.stderr)
    if a.dry:
        for alias, player in found:
            print(alias, player)
        return
    with open(OUT, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["alias_id", "player_id"])
        w.writerows(sorted(found))
    print(f"wrote {OUT}", file=sys.stderr)


if __name__ == "__main__":
    main()
