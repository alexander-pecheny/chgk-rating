"""Fetches the town table of the old system's API into `data/towns.csv`.

`tournaments.town_id` in the mirror is a numeric town id and
`tournament_results.team_current_town` is a town *name*, so telling a home team
from a visitor needs the id-to-name map, and the mirror does not carry it.
The map is small (1683 rows), stable, and saved once.

    uv run tools/fetch_towns.py
"""

import csv
import json
import urllib.request

URL = "https://api.rating.chgk.net/towns?itemsPerPage=500&page={}"
OUT = "data/towns.csv"


def main():
    rows, page = [], 1
    while True:
        with urllib.request.urlopen(URL.format(page), timeout=60) as r:
            batch = json.load(r)
        if not batch:
            break
        rows += batch
        page += 1
    seen = {}
    for t in rows:
        seen[t["id"]] = (t["name"], (t.get("country") or {}).get("name", ""))
    with open(OUT, "w", encoding="utf-8", newline="") as f:
        w = csv.writer(f)
        w.writerow(["town_id", "name", "country"])
        for i in sorted(seen):
            w.writerow([i, seen[i][0], seen[i][1]])
    print(f"{len(seen)} towns written to {OUT}")


if __name__ == "__main__":
    main()
