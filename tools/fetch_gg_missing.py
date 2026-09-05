# /// script
# requires-python = ">=3.10"
# dependencies = ["httpx"]
# ///
"""Some tournaments the official release lists have no results in buff.db. Fetch
those from rating.chgk.info into data/external/chgk_gg/extra.db, in buff.db's shape,
so ext_gg can read them alongside the mirror."""

import json
import pathlib
import sqlite3
import sys
import time

import httpx

ROOT = pathlib.Path(__file__).resolve().parent.parent
EXT = ROOT / "data/external/chgk_gg"
OUT = EXT / "extra.db"
UA = "chgk-rating-research/1.0"

buff = sqlite3.connect(ROOT / "buff.db")
have = {r[0] for r in buff.execute("select distinct id from tournament_results")}
wanted = set()
for rel in json.load((EXT / "releases_b.json").open())["items"]:
    if rel["date"] < "2021-09-10":
        continue
    wanted |= set(rel["tournaments"]["in_rating"]) | set(rel["tournaments"]["not_in_rating"])
todo = sorted(wanted - have)
print(f"{len(todo)} tournaments to fetch", flush=True)

db = sqlite3.connect(OUT)
db.executescript("""
create table if not exists tournaments (id integer primary key, name text, date_start text,
  date_end text, tournament_type text, maii_rating integer);
create table if not exists tournament_results (id integer, team_id integer, team_current_name text,
  team_members text, team_members_full text, position real, rating text);
create table if not exists teams (id integer primary key, name text);
create index if not exists idx_extra_results on tournament_results(id);
""")
done = {r[0] for r in db.execute("select distinct id from tournaments")}
todo = [t for t in todo if t not in done]

client = httpx.Client(headers={"User-Agent": UA, "Accept": "application/json"}, timeout=60.0)
last = 0.0


def get(url):
    global last
    for attempt in range(4):
        gap = 0.5 - (time.monotonic() - last)
        if gap > 0:
            time.sleep(gap)
        last = time.monotonic()
        try:
            r = client.get(url)
            if r.status_code == 404:
                return None
            r.raise_for_status()
            return r.json()
        except Exception as e:
            if attempt == 3:
                print(f"FAILED {url}: {e}", file=sys.stderr, flush=True)
                return None
            time.sleep(2 * (attempt + 1))


for n, tid in enumerate(todo, 1):
    info = get(f"https://api.rating.chgk.info/tournaments/{tid}")
    if not info:
        continue
    res = get(
        f"https://api.rating.chgk.info/tournaments/{tid}/results"
        "?includeTeamMembers=1&includeMasksAndControversials=0&includeTeamFlags=1&includeRatingB=1"
    )
    if not res:
        continue
    db.execute(
        "insert or replace into tournaments values (?,?,?,?,?,?)",
        (
            tid,
            info.get("name"),
            info.get("dateStart"),
            info.get("dateEnd"),
            (info.get("type") or {}).get("name"),
            1 if (info.get("properties") or {}).get("maiiRating") else 0,
        ),
    )
    for row in res:
        team = row.get("team") or {}
        members = row.get("teamMembers") or []
        db.execute(
            "insert into tournament_results values (?,?,?,?,?,?,?)",
            (
                tid,
                team.get("id"),
                (row.get("current") or {}).get("name"),
                json.dumps([m["player"]["id"] for m in members]),
                json.dumps([{"flag": m.get("flag")} for m in members], ensure_ascii=True),
                row.get("position"),
                json.dumps(row.get("rating")) if row.get("rating") else None,
            ),
        )
        if team.get("id"):
            db.execute("insert or replace into teams values (?,?)", (team["id"], team.get("name")))
    if n % 50 == 0:
        db.commit()
        print(f"{n}/{len(todo)}", flush=True)
db.commit()
print("done", flush=True)
