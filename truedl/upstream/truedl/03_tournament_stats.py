#!/usr/bin/env python
# -*- coding: utf-8 -*-
import argparse
import os
from collections import Counter

import orjson as json
import requests
import pyexcel

from .util import (
    _open,
    parse_date,
    RESULTS_DIR,
    INFO_DIR,
    APPEALS_DIR,
    TournamentDataPoint,
    API,
)


def get_controversials(res):
    result = {}
    for element in res:
        if "controversials" in element:
            for controversial in element["controversials"]:
                result[controversial["id"]] = controversial["status"]
    return result


def get_people(info, field):
    return ", ".join(f"{p['name']} {p['surname']}" for p in info[field])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--date-from")
    parser.add_argument("--date-to")
    parser.add_argument("--top-n", type=int, default=100)
    args = parser.parse_args()

    args.date_from = parse_date(args.date_from)
    args.date_to = parse_date(args.date_to)

    if not os.path.isdir(APPEALS_DIR):
        os.mkdir(APPEALS_DIR)

    data = []
    for fn in os.listdir(RESULTS_DIR):
        if not fn.endswith(".json"):
            continue
        full_path = os.path.join(RESULTS_DIR, fn)
        tourn_id = int(fn.split("-")[1])
        info_path = os.path.join(INFO_DIR, f"tournament-{tourn_id}.json")
        with _open(full_path, "r") as f:
            res = json.loads(f.read())
        with _open(info_path, "r") as f:
            info = json.loads(f.read())
        if not (
            args.date_from <= parse_date(info["dateStart"]) <= args.date_to
            and args.date_from <= parse_date(info["dateEnd"]) <= args.date_to
        ):
            continue
        controversials = get_controversials(res)
        accepted_controversials = Counter(controversials.values())["A"]
        appeals_file_path = os.path.join(APPEALS_DIR, fn)
        if not os.path.isfile(appeals_file_path):
            print(f"downloading appeals for {tourn_id} {info['name']}...")
            appeals = requests.get(f"{API}/tournaments/{tourn_id}/appeals.json").json()
            with open(appeals_file_path, "wb") as f:
                f.write(json.dumps(appeals))
        else:
            with _open(appeals_file_path, "r") as f:
                appeals = json.loads(f.read())
        accepted_appeals = len([x for x in appeals if x["status"] == "A"])
        editors = get_people(info, "editors")
        game_jury = get_people(info, "gameJury")
        appeal_jury = get_people(info, "appealJury")
        data_point = TournamentDataPoint(
            id=tourn_id,
            name=info["name"],
            teams=len(res),
            controversials=len(controversials),
            controversials_accept_rate=round(
                accepted_controversials / len(controversials) * 100.0, 2
            )
            if controversials
            else 0,
            appeals=len(appeals),
            appeals_accept_rate=round(accepted_appeals / len(appeals) * 100.0, 2)
            if appeals
            else 0,
            editors=editors,
            game_jury=game_jury,
            appeal_jury=appeal_jury,
        )
        data.append(data_point)
    srt = sorted(data, key=lambda x: x.appeals, reverse=True)
    with open(f"tournament_stats_{args.date_from}_{args.date_to}.json", "wb") as f:
        f.write(json.dumps(srt))
    headers = [
        "num",
        "id",
        "name",
        "teams",
        "controversials",
        "controversials_accept_rate",
        "appeals",
        "appeals_accept_rate",
        "editors",
        "game_jury",
        "appeal_jury",
        "appeals_running_sum",
        "controversials_running_sum"
    ]
    excel_data = [headers]
    appeals_running_sum = 0
    controversials_running_sum = 0
    for i, el in enumerate(srt[:200]):
        row = [i + 1] + [getattr(el, h) for h in headers[1:-2]]
        appeals_running_sum += el.appeals
        controversials_running_sum += el.controversials
        row.append(appeals_running_sum)
        row.append(controversials_running_sum)
        excel_data.append(row)
    pyexcel.save_as(
        array=excel_data,
        dest_file_name=f"tournament_stats_{args.date_from}_{args.date_to}.xlsx",
    )


if __name__ == "__main__":
    main()
