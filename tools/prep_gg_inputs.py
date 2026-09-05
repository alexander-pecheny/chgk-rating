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
"""

import csv
import datetime
import json
import pathlib
import sqlite3

ROOT = pathlib.Path(__file__).resolve().parent.parent
EXT = ROOT / "data/external/chgk_gg"
LAST_OLD = datetime.date(2020, 4, 3)
FIRST_NEW = datetime.date(2021, 9, 9)


def next_friday(d):
    a = 4 - d.weekday()
    return d + datetime.timedelta(a + 7 if a <= 0 else a)


con = sqlite3.connect(ROOT / "buff.db")
ends = {r[0]: datetime.date.fromisoformat(r[1][:10]) for r in con.execute("select id,date_end from tournaments")}

rows = []
fixed = 0
for p in json.load((EXT / "players_release2.json").open())["items"]:
    for t in p["tournaments"]:
        end = ends[t["tournament_id"]]
        w = 0 if end > LAST_OLD else (LAST_OLD - next_friday(end)).days // 7 + 2
        if round(t["initial_rating"] * 0.99**w) != t["current_rating"]:
            cands = [k for k in range(max(0, w - 8), w + 9) if round(t["initial_rating"] * 0.99**k) == t["current_rating"]]
            if cands:
                w = min(cands, key=lambda k: abs(k - w))
                fixed += 1
        rows.append((p["player_id"], t["tournament_id"], t["initial_rating"], w))

with (EXT / "bootstrap_players.csv").open("w", newline="") as f:
    wr = csv.writer(f)
    wr.writerow(["player_id", "tournament_id", "initial_score", "weeks"])
    wr.writerows(rows)
print(f"bootstrap_players.csv: {len(rows)} rows, {fixed} weeks refitted")

seen = set()
out = []
for line in (EXT / "base_rosters.jsonl").open():
    rec = json.loads(line)
    seen.add(rec["team_id"])
    for r in rec["rows"]:
        out.append(
            (r["idteam"], r["idplayer"], r["idseason"], (r["dateAdded"] or "")[:10], (r["dateRemoved"] or "")[:10])
        )
local = 0
# player_id 0 is buff.db's marker for "this team has no base roster this season".
for team_id, season_id, player_id, added, removed in con.execute(
    "select team_id, season_id, player_id, date_added, date_removed from team_seasons where player_id != 0"
):
    if team_id in seen:
        continue
    local += 1
    out.append((team_id, player_id, season_id, (added or "")[:10], (removed or "")[:10]))

with (EXT / "base_rosters.csv").open("w", newline="") as f:
    wr = csv.writer(f)
    wr.writerow(["team_id", "player_id", "season_id", "start_date", "end_date"])
    wr.writerows(out)
print(f"base_rosters.csv: {len(out)} rows for {len(seen)} fetched teams plus {local} local rows")
