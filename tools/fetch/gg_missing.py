# /// script
# requires-python = ">=3.10"
# dependencies = ["httpx"]
# ///
"""Some tournaments the official release lists have no results in buff.db. Fetch
those from rating.chgk.info into data/external/chgk_gg/extra.db, in buff.db's shape,
so ext_gg can read them alongside the mirror. Resumable:

    uv run tools/fetch/gg_missing.py
"""

import argparse
import json
import pathlib
import sqlite3
import sys
import time

import httpx

ROOT = pathlib.Path(__file__).resolve().parents[2]
EXT = ROOT / "data/external/chgk_gg"
OUT = EXT / "extra.db"
UA = "chgk-rating-research/1.0"
API = "https://api.rating.chgk.info/tournaments/{}"
RESULTS = "/results?includeTeamMembers=1&includeMasksAndControversials=0&includeTeamFlags=1&includeRatingB=1"
FIRST_RELEASE = "2021-09-10"
GAP, TIMEOUT, TRIES, BACKOFF = 0.5, 60.0, 4, 2
NOT_FOUND = 404
COMMIT_EVERY = 50
SCHEMA = """
create table if not exists tournaments (id integer primary key, name text, date_start text,
  date_end text, tournament_type text, maii_rating integer);
create table if not exists tournament_results (id integer, team_id integer, team_current_name text,
  team_members text, team_members_full text, position real, rating text);
create table if not exists teams (id integer primary key, name text);
create index if not exists idx_extra_results on tournament_results(id);
"""


class Api:
    """GET with a polite gap between requests and a few retries."""

    def __init__(self):
        self.client = httpx.Client(headers={"User-Agent": UA, "Accept": "application/json"}, timeout=TIMEOUT)
        self.last = 0.0

    def wait(self, gap):
        left = gap - (time.monotonic() - self.last)
        if left > 0:
            time.sleep(left)
        self.last = time.monotonic()

    def get(self, url):
        for attempt in range(TRIES):
            self.wait(GAP)
            try:
                r = self.client.get(url)
                if r.status_code == NOT_FOUND:
                    return None
                r.raise_for_status()
                return r.json()
            except (httpx.HTTPError, ValueError) as e:
                if attempt == TRIES - 1:
                    print(f"FAILED {url}: {e}", file=sys.stderr, flush=True)
                    return None
                time.sleep(BACKOFF * (attempt + 1))


def wanted():
    """Tournaments of the official releases since 2021-09 that the mirror lacks."""
    have = {r[0] for r in sqlite3.connect(ROOT / "buff.db").execute("select distinct id from tournament_results")}
    want = set()
    for rel in json.load((EXT / "releases_b.json").open())["items"]:
        if rel["date"] >= FIRST_RELEASE:
            want |= set(rel["tournaments"]["in_rating"]) | set(rel["tournaments"]["not_in_rating"])
    return sorted(want - have)


def store(db, tid, info, res):
    props, kind = info.get("properties") or {}, info.get("type") or {}
    db.execute("insert or replace into tournaments values (?,?,?,?,?,?)",
               (tid, info.get("name"), info.get("dateStart"), info.get("dateEnd"), kind.get("name"),
                1 if props.get("maiiRating") else 0))
    for row in res:
        team, members = row.get("team") or {}, row.get("teamMembers") or []
        db.execute("insert into tournament_results values (?,?,?,?,?,?,?)", (
            tid, team.get("id"), (row.get("current") or {}).get("name"),
            json.dumps([m["player"]["id"] for m in members]),
            json.dumps([{"flag": m.get("flag")} for m in members], ensure_ascii=True),
            row.get("position"), json.dumps(row.get("rating")) if row.get("rating") else None))
        if team.get("id"):
            db.execute("insert or replace into teams values (?,?)", (team["id"], team.get("name")))


def main():
    argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter).parse_args()
    todo = wanted()
    print(f"{len(todo)} tournaments to fetch", flush=True)
    db = sqlite3.connect(OUT)
    db.executescript(SCHEMA)
    done = {r[0] for r in db.execute("select distinct id from tournaments")}
    todo = [t for t in todo if t not in done]
    api = Api()
    for n, tid in enumerate(todo, 1):
        info = api.get(API.format(tid))
        res = api.get(API.format(tid) + RESULTS) if info else None
        if res:
            store(db, tid, info, res)
            if n % COMMIT_EVERY == 0:
                db.commit()
                print(f"{n}/{len(todo)}", flush=True)
    db.commit()
    print("done", flush=True)


if __name__ == "__main__":
    main()
