#!/usr/bin/env python
# -*- coding: utf-8 -*-
import os
import argparse
import datetime
import time

import orjson as json
import requests

from .util import _open, parse_date


TOURNAMENTS_DIR = "tournaments"
RESULTS_DIR = "tournament_results"
API = "https://api.rating.chgk.info"


def clean_personal_data(results):
    for res in results:
        for entry in res["teamMembers"]:
            player = entry.pop("player")
            entry["player_id"] = player["id"]
        synchRequest = res.get("synchRequest", None)
        if not synchRequest:
            return
        venue = synchRequest.pop("venue", None)
        if venue:
            synchRequest["venue"] = {"id": venue["id"]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--date-from", default="1970-01-01")
    parser.add_argument("--date-to", default="2099-12-31")
    args = parser.parse_args()

    if not os.path.isdir(TOURNAMENTS_DIR):
        os.mkdir(TOURNAMENTS_DIR)
    if not os.path.isdir(RESULTS_DIR):
        os.mkdir(RESULTS_DIR)
    for fn in os.listdir(TOURNAMENTS_DIR):
        if not fn.startswith("tournament") and fn.endswith(".json"):
            continue
        full_path = os.path.join(TOURNAMENTS_DIR, fn)
        with _open(full_path, "r") as f:
            info = json.loads(f.read())
        if not isinstance(info, dict):
            continue
        if (
            not parse_date(args.date_from)
            <= parse_date(info["dateStart"])
            <= parse_date(args.date_to)
        ):
            continue
        results_file_name = os.path.join(
            RESULTS_DIR, f"tournament-{info['id']}-results.json"
        )
        if os.path.isfile(results_file_name) and not (
            (datetime.date.today() - parse_date(info["dateStart"])).days <= 60
            and os.stat(results_file_name).st_size <= 2
        ):
            continue
        print(f"downloading results from tournament {info['id']} {info['name']}...")
        req = requests.get(
            f"{API}/tournaments/{info['id']}/results.json"
            "?includeMasksAndControversials=1"
            "&includeRatingB=1"
            "&includeTeamMembers=1"
            "&includeTeamFlags=1"
        ).json()
        time.sleep(1)
        if not isinstance(req, list) and req["title"] == "An error occurred":
            continue
        if not isinstance(req, list):
            import pdb; pdb.set_trace()
        clean_personal_data(req)
        with open(results_file_name, "wb") as f:
            f.write(json.dumps(req))


if __name__ == "__main__":
    main()
