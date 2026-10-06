#!/usr/bin/env python
# -*- coding: utf-8 -*-
import os
import time

import orjson as json
import requests
import sys

from .util import _open

API = "https://api.rating.chgk.info"
SLEEP = 1


def clean_org(tourn):
    for key in ("orgcommittee", "editors", "gameJury", "appealJury"):
        if not isinstance(tourn, dict):
            return
        list_ = tourn.get(key, None)
        if not list_ or not isinstance(list_, list):
            continue
        tourn[key] = [{"id": x["id"]} for x in list_]
    return tourn


def main():

    if not os.path.isdir("tournaments"):
        os.mkdir("tournaments")

    if os.path.isfile("tournaments/tournaments.json"):
        with _open("tournaments/tournaments.json", "r") as f:
            tournaments = json.loads(f.read() or "[]")
    else:
        tournaments = []
    tournament_ids = {x["id"] for x in tournaments}
    ITEMS_PER_PAGE = 100
    page = len(tournaments) // ITEMS_PER_PAGE + 1
    req = requests.get(
        f"{API}/tournaments.json?itemsPerPage={ITEMS_PER_PAGE}&page={page}"
    )
    res = req.json()
    time.sleep(SLEEP)
    for x in res:
        if x["id"] not in tournament_ids:
            tournaments.append(x)
            tournament_ids.add(x["id"])
    while len(res) == 100:
        page += 1
        print(f"processing page {page}...")
        req = requests.get(
            f"{API}/tournaments.json?itemsPerPage={ITEMS_PER_PAGE}&page={page}"
        )
        res = req.json()
        for x in res:
            if x["id"] not in tournament_ids:
                tournaments.append(x)
                tournament_ids.add(x["id"])
        time.sleep(SLEEP)
    with open("tournaments/tournaments.json", "wb") as f:
        f.write(json.dumps([clean_org(x) for x in tournaments]))

    for t in tournaments:
        dest_file_name = f"tournaments/tournament-{t['id']}.json"
        if os.path.isfile(dest_file_name):
            continue
        else:
            print(f"downloading tournament {t['id']} {t['name']}...")
            url = f"{API}/tournaments/{t['id']}.json"
            req = requests.get(url)
            time.sleep(SLEEP)
            try:
                res = req.json()
            except Exception as e:
                sys.stderr.write(
                    f"error while trying to get info from {url}: {type(e)} {e}\n"
                )
                continue
            with open(dest_file_name, "wb") as f:
                f.write(json.dumps(clean_org(res)))


if __name__ == "__main__":
    main()
