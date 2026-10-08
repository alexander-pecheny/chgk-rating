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

from polite import Polite

ROOT = pathlib.Path(__file__).resolve().parents[2]
EXT = ROOT / "data/external/chgk_gg"
OUT = EXT / "extra.db"
API = "https://api.rating.chgk.info/tournaments/{}"
RESULTS = "/results?includeTeamMembers=1&includeMasksAndControversials=0&includeTeamFlags=1&includeRatingB=1"
FIRST_RELEASE = "2021-09-10"
GAP = 0.5
TIMEOUT = 60.0
COMMIT_EVERY = 50
SCHEMA = """
create table if not exists tournaments (id integer primary key, name text, date_start text,
  date_end text, tournament_type text, maii_rating integer);
create table if not exists tournament_results (id integer, team_id integer, team_current_name text,
  team_members text, team_members_full text, position real, rating text);
create table if not exists teams (id integer primary key, name text);
create index if not exists idx_extra_results on tournament_results(id);
"""


def wanted():
    """Tournaments of the official releases since 2021-09 that the mirror lacks."""
    have = {r[0] for r in sqlite3.connect(ROOT / "buff.db").execute("select distinct id from tournament_results")}
    want = set()
    for rel in json.load((EXT / "releases_b.json").open())["items"]:
        if rel["date"] >= FIRST_RELEASE:
            want |= set(rel["tournaments"]["in_rating"]) | set(rel["tournaments"]["not_in_rating"])
    return sorted(want - have)


def result_row(tid, row):
    """One result in the mirror's `tournament_results` shape."""
    team, members = row.get("team") or {}, row.get("teamMembers") or []
    return (
        tid, team.get("id"), (row.get("current") or {}).get("name"),
        json.dumps([m["player"]["id"] for m in members]),
        json.dumps([{"flag": m.get("flag")} for m in members], ensure_ascii=True),
        row.get("position"), json.dumps(row.get("rating")) if row.get("rating") else None,
    )


def store(db, tid, info, res):
    props, kind = info.get("properties") or {}, info.get("type") or {}
    db.execute("insert or replace into tournaments values (?,?,?,?,?,?)",
               (tid, info.get("name"), info.get("dateStart"), info.get("dateEnd"), kind.get("name"),
                1 if props.get("maiiRating") else 0))
    for row in res:
        db.execute("insert into tournament_results values (?,?,?,?,?,?,?)", result_row(tid, row))
        team = row.get("team") or {}
        if team.get("id"):
            db.execute("insert or replace into teams values (?,?)", (team["id"], team.get("name")))


def open_store():
    """The output database, and the tournaments it already holds."""
    db = sqlite3.connect(OUT)
    db.executescript(SCHEMA)
    return db, {r[0] for r in db.execute("select distinct id from tournaments")}


def fetch(db, todo):
    api = Polite(GAP, TIMEOUT)
    for n, tid in enumerate(todo, 1):
        info = api.get(API.format(tid))
        res = api.get(API.format(tid) + RESULTS) if info else None
        if res:
            store(db, tid, info, res)
            if n % COMMIT_EVERY == 0:
                db.commit()
                print(f"{n}/{len(todo)}", flush=True)
    db.commit()


def main():
    argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter).parse_args()
    todo = wanted()
    print(f"{len(todo)} tournaments to fetch", flush=True)
    db, done = open_store()
    fetch(db, [t for t in todo if t not in done])
    print("done", flush=True)


if __name__ == "__main__":
    main()
