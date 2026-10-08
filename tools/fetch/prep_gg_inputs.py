# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Turn the two fetched gg artifacts into the flat CSVs that ext_gg reads.

  players_release2.json  -> bootstrap_players.csv  (player_id,tournament_id,initial,weeks)
  base_rosters.jsonl     -> base_rosters.csv       (team_id,player_id,season_id,start,end)

`weeks` is the decay age the official system carried at release 2. It follows from
the tournament's end date, except for five tournaments whose end date in buff.db is
a week off gg's; there we take the week count that reproduces the published score.
Run once, after `tools/fetch/gg_rosters.py`:

    uv run tools/fetch/prep_gg_inputs.py
"""

import argparse
import csv
import datetime
import json
import pathlib
import sqlite3

ROOT = pathlib.Path(__file__).resolve().parents[2]
EXT = ROOT / "data/external/chgk_gg"
LAST_OLD = datetime.date(2020, 4, 3)
FIRST_NEW = datetime.date(2021, 9, 9)
FRIDAY = 4
WEEK = 7
# The old system's weekly decay, and how far the week search may stray.
DECAY = 0.99
WEEK_SLACK = 8
# Weeks from the last old release to the first Friday after an event, plus two.
WEEK_OFFSET = 2


def next_friday(d):
    a = FRIDAY - d.weekday()
    return d + datetime.timedelta(a + WEEK if a <= 0 else a)


def weeks_of(t, end):
    """The decay age at release 2, refitted where the end date is a week off."""
    w = 0 if end > LAST_OLD else (LAST_OLD - next_friday(end)).days // WEEK + WEEK_OFFSET
    if round(t["initial_rating"] * DECAY**w) == t["current_rating"]:
        return w, False
    cands = [k for k in range(max(0, w - WEEK_SLACK), w + WEEK_SLACK + 1)
             if round(t["initial_rating"] * DECAY**k) == t["current_rating"]]
    return (min(cands, key=lambda k: abs(k - w)), True) if cands else (w, False)


def bootstrap_players(con):
    ends = {r[0]: datetime.date.fromisoformat(r[1][:10]) for r in con.execute("select id,date_end from tournaments")}
    rows, fixed = [], 0
    for p in json.load((EXT / "players_release2.json").open())["items"]:
        for t in p["tournaments"]:
            w, refit = weeks_of(t, ends[t["tournament_id"]])
            fixed += refit
            rows.append((p["player_id"], t["tournament_id"], t["initial_rating"], w))
    write_csv(EXT / "bootstrap_players.csv", ["player_id", "tournament_id", "initial_score", "weeks"], rows)
    print(f"bootstrap_players.csv: {len(rows)} rows, {fixed} weeks refitted")


def write_csv(path, head, rows):
    with path.open("w", newline="") as f:
        wr = csv.writer(f)
        wr.writerow(head)
        wr.writerows(rows)


def fetched_rosters():
    seen, out = set(), []
    for line in (EXT / "base_rosters.jsonl").open():
        rec = json.loads(line)
        seen.add(rec["team_id"])
        out += [(r["idteam"], r["idplayer"], r["idseason"], (r["dateAdded"] or "")[:10], (r["dateRemoved"] or "")[:10])
                for r in rec["rows"]]
    return seen, out


def base_rosters(con):
    seen, out = fetched_rosters()
    local = 0
    # player_id 0 is buff.db's marker for "this team has no base roster this season".
    for team_id, season_id, player_id, added, removed in con.execute(
        "select team_id, season_id, player_id, date_added, date_removed from team_seasons where player_id != 0"
    ):
        if team_id not in seen:
            local += 1
            out.append((team_id, player_id, season_id, (added or "")[:10], (removed or "")[:10]))
    write_csv(EXT / "base_rosters.csv", ["team_id", "player_id", "season_id", "start_date", "end_date"], out)
    print(f"base_rosters.csv: {len(out)} rows for {len(seen)} fetched teams plus {local} local rows")


def main():
    argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter).parse_args()
    con = sqlite3.connect(ROOT / "buff.db")
    bootstrap_players(con)
    base_rosters(con)


if __name__ == "__main__":
    main()
