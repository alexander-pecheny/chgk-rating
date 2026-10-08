"""Fetches the town table of the old system's API into `data/towns.csv`.

`tournaments.town_id` in the mirror is a numeric town id and
`tournament_results.team_current_town` is a town *name*, so telling a home team
from a visitor needs the id-to-name map, and the mirror does not carry it.
The map is small (1683 rows), stable, and saved once.

    uv run tools/fetch/towns.py
"""

import argparse
import csv
import json
import urllib.request

URL = "https://api.rating.chgk.info/towns?itemsPerPage=500&page={}"
OUT = "data/towns.csv"
TIMEOUT = 60


def pages():
    """Every town the API lists, page by page until an empty one."""
    page = 1
    while True:
        with urllib.request.urlopen(URL.format(page), timeout=TIMEOUT) as r:
            batch = json.load(r)
        if not batch:
            return
        yield from batch
        page += 1


def main():
    argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter).parse_args()
    seen = {t["id"]: (t["name"], (t.get("country") or {}).get("name", "")) for t in pages()}
    with open(OUT, "w", encoding="utf-8", newline="") as f:
        w = csv.writer(f)
        w.writerow(["town_id", "name", "country"])
        w.writerows([i, *seen[i]] for i in sorted(seen))
    print(f"{len(seen)} towns written to {OUT}")


if __name__ == "__main__":
    main()
