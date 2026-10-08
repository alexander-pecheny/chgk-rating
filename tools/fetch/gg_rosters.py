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
import sys
import time

import httpx

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "data/external/chgk_gg/base_rosters.jsonl"
UA = "chgk-rating-research/1.0"
URL = "https://api.rating.chgk.info/teams/{}/seasons?page=1&itemsPerPage=500"
RATE = 2.0  # requests per second, max
TIMEOUT, TRIES, BACKOFF = 30.0, 4, 2
NOT_FOUND = 404
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


class Fetcher:
    def __init__(self):
        self.client = httpx.Client(headers={"User-Agent": UA, "Accept": "application/json"}, timeout=TIMEOUT)
        self.last = 0.0

    def wait(self):
        gap = 1.0 / RATE - (time.monotonic() - self.last)
        if gap > 0:
            time.sleep(gap)
        self.last = time.monotonic()

    def seasons(self, tid):
        """The team's season rows, `[]` for an unknown team, None on failure."""
        for attempt in range(TRIES):
            self.wait()
            try:
                r = self.client.get(URL.format(tid))
                if r.status_code == NOT_FOUND:
                    return []
                r.raise_for_status()
                return r.json()
            except (httpx.HTTPError, ValueError) as e:
                if attempt == TRIES - 1:
                    print(f"FAILED {tid}: {e}", file=sys.stderr, flush=True)
                time.sleep(BACKOFF * (attempt + 1))
        return None


def main():
    argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter).parse_args()
    con = sqlite3.connect(ROOT / "buff.db")
    wanted = [r[0] for r in con.execute("select distinct team_id from ratings order by team_id")]
    done = done_teams()
    todo = [t for t in wanted if t not in done]
    print(f"{len(wanted)} teams, {len(done)} done, {len(todo)} to go", flush=True)
    fetcher = Fetcher()
    with OUT.open("a") as f:
        for n, tid in enumerate(todo, 1):
            rows = fetcher.seasons(tid)
            if rows is None:
                continue
            f.write(json.dumps({"team_id": tid, "rows": rows}, separators=(",", ":")) + "\n")
            if n % FLUSH_EVERY == 0:
                f.flush()
                print(f"{n}/{len(todo)}", flush=True)
    print("done", flush=True)


if __name__ == "__main__":
    main()
