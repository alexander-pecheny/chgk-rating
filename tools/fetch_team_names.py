# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""A team's name as rating.chgk.info itself publishes it, into out/team_names.db.

Neither `tournament_results.team_current_name` nor buff.db's `teams` table says
what a team is called: the mirror builds `teams` from each team's latest result,
so a team that alternates between two names alternates in the mirror too. Team
308 is "Номер 6" to the API and "Номер 2" to its last result. The API's paginated
listing is the authority and is cheap in bulk — 500 teams a page, about 0.8 s a
page, ~150 pages for 74 000 teams."""

import json
import os
import pathlib
import sqlite3
import sys
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "out/team_names.db"
# Per-process, so two runs at once cannot corrupt each other's half-built file.
TMP = OUT.with_name(f"{OUT.name}.{os.getpid()}.tmp")
API = "https://api.rating.chgk.net/teams?page={}&itemsPerPage=500"
UA = "chgk-rating-research/1.0"
PAUSE = 0.3
TRIES = 3


def get(url):
    for attempt in range(TRIES):
        req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                return json.loads(r.read())
        except Exception as e:
            if attempt == TRIES - 1:
                raise SystemExit(f"giving up on {url}: {e}")
            print(f"retrying {url}: {e}", file=sys.stderr, flush=True)
            time.sleep(2 * (attempt + 1))


# A half-written file must never replace a good one: build beside it, rename last.
OUT.parent.mkdir(parents=True, exist_ok=True)
TMP.unlink(missing_ok=True)
db = sqlite3.connect(TMP)
db.execute(
    "create table team_names (id integer primary key, name text, town text, fetched text)"
)
fetched = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())

n = 0
page = 1
try:
    while True:
        rows = get(API.format(page))
        if not rows:
            break
        db.executemany(
            "insert or replace into team_names values (?,?,?,?)",
            [
                (t["id"], t.get("name") or "", (t.get("town") or {}).get("name") or "", fetched)
                for t in rows
                if t.get("id")
            ],
        )
        n += len(rows)
        if page % 10 == 0:
            db.commit()
            print(f"page {page}: {n} teams", flush=True)
        page += 1
        time.sleep(PAUSE)
    db.commit()
    db.close()
except BaseException:
    db.close()
    TMP.unlink(missing_ok=True)
    raise

os.replace(TMP, OUT)
print(f"{n} teams in {OUT}", flush=True)
