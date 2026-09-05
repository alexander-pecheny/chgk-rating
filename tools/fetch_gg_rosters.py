# /// script
# requires-python = ">=3.10"
# dependencies = ["httpx"]
# ///
"""Fetch base rosters (/teams/{id}/seasons) for every team that ever appears in a
gg release, into data/external/chgk_gg/base_rosters.jsonl. Resumable."""

import json
import pathlib
import sqlite3
import sys
import time

import httpx

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "data/external/chgk_gg/base_rosters.jsonl"
UA = "chgk-rating-research/1.0"
RATE = 2.0  # requests per second, max

con = sqlite3.connect(ROOT / "buff.db")
wanted = [r[0] for r in con.execute("select distinct team_id from ratings order by team_id")]

done = set()
if OUT.exists():
    for line in OUT.open():
        try:
            done.add(json.loads(line)["team_id"])
        except Exception:
            pass
todo = [t for t in wanted if t not in done]
print(f"{len(wanted)} teams, {len(done)} done, {len(todo)} to go", flush=True)

client = httpx.Client(headers={"User-Agent": UA, "Accept": "application/json"}, timeout=30.0)
last = 0.0
with OUT.open("a") as f:
    for n, tid in enumerate(todo, 1):
        for attempt in range(4):
            gap = 1.0 / RATE - (time.monotonic() - last)
            if gap > 0:
                time.sleep(gap)
            last = time.monotonic()
            try:
                r = client.get(f"https://api.rating.chgk.info/teams/{tid}/seasons?page=1&itemsPerPage=500")
                if r.status_code == 404:
                    rows = []
                    break
                r.raise_for_status()
                rows = r.json()
                break
            except Exception as e:
                if attempt == 3:
                    print(f"FAILED {tid}: {e}", file=sys.stderr, flush=True)
                    rows = None
                time.sleep(2 * (attempt + 1))
        if rows is None:
            continue
        f.write(json.dumps({"team_id": tid, "rows": rows}, separators=(",", ":")) + "\n")
        if n % 500 == 0:
            f.flush()
            print(f"{n}/{len(todo)}", flush=True)
print("done", flush=True)
