# /// script
# requires-python = ">=3.10"
# dependencies = ["httpx"]
# ///
"""Fetch base rosters (/teams/{id}/seasons) for every team that ever appears in a
gg release, into data/external/chgk_gg/base_rosters.jsonl. Resumable:

    uv run tools/fetch/gg_rosters.py
"""

import argparse
import json
import pathlib
import sqlite3

from polite import Polite

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "data/external/chgk_gg/base_rosters.jsonl"
URL = "https://api.rating.chgk.info/teams/{}/seasons?page=1&itemsPerPage=500"
RATE = 2.0  # requests per second, max
TIMEOUT = 30.0
FLUSH_EVERY = 500


def done_teams():
    """Teams already in the output, so a rerun resumes."""
    done = set()
    for line in OUT.open() if OUT.exists() else ():
        try:
            done.add(json.loads(line)["team_id"])
        except (ValueError, KeyError):
            pass
    return done


def fetch(todo):
    api = Polite(1.0 / RATE, TIMEOUT)
    with OUT.open("a") as f:
        for n, tid in enumerate(todo, 1):
            rows = api.get(URL.format(tid), missing=[], label=tid)
            if rows is not None:
                f.write(json.dumps({"team_id": tid, "rows": rows}, separators=(",", ":")) + "\n")
                if n % FLUSH_EVERY == 0:
                    f.flush()
                    print(f"{n}/{len(todo)}", flush=True)


def main():
    argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter).parse_args()
    con = sqlite3.connect(ROOT / "buff.db")
    wanted = [r[0] for r in con.execute("select distinct team_id from ratings order by team_id")]
    done = done_teams()
    todo = [t for t in wanted if t not in done]
    print(f"{len(wanted)} teams, {len(done)} done, {len(todo)} to go", flush=True)
    fetch(todo)
    print("done", flush=True)


if __name__ == "__main__":
    main()
