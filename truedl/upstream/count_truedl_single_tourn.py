#!/usr/bin/env python
# -*- coding: utf-8 -*-
import argparse
import bisect
import datetime
import json
import math
import os
import time
from collections import defaultdict

import openpyxl
import requests
from mpmath.tests.test_functions import te

RESULTS_URL = (
    "https://api.rating.chgk.info/tournaments/{id}/results.json"
    "?includeMasksAndControversials=1&includeRatingB=1"
)
TRUEDL_COEFF_PAIRS = [
    (10, 1.61),
    (25, 1.52),
    (50, 1.43),
    (100, 1.32),
    (250, 1.16),
    (500, 1.0),
    (1000, 0.81),
    (2000, 0.6),
    (3000, 0.43),
    (5000, 0.31),
]
TRUEDL_COEFF_KEYS = [x[0] for x in TRUEDL_COEFF_PAIRS]
TRUEDL_COEFF_VALUES = [x[1] for x in TRUEDL_COEFF_PAIRS]


def parse_date(s):
    return datetime.datetime.strptime(s.split("T")[0], "%Y-%m-%d").date()


def get_release_id(dateStart):
    releases = requests.get("https://rating.chgk.gg/api/v1/b/releases.json").json()
    time.sleep(0.5)
    for release in releases["items"]:
        if parse_date(release["date"]) <= dateStart:
            break
    return release["id"]


def download_release(release_id):
    script_dir = os.path.dirname(os.path.abspath(__file__))
    cache_path = os.path.join(script_dir, f"release_cache_{release_id}.json")

    if os.path.exists(cache_path):
        print(f"loading release {release_id} from cache...")
        with open(cache_path, "r") as f:
            data = json.load(f)
            return {int(k): v for k, v in data.items()}

    result = {}
    page = 1
    while True:
        print(f"downloading page {page} of release {release_id}...")
        ratings = requests.get(
            f"https://rating.chgk.gg/api/v1/b/teams/{release_id}.json?page={page}"
        ).json()
        time.sleep(0.5)
        for item in ratings["items"]:
            result[item["team_id"]] = item
        if ratings["current_page"] >= ratings["pages"]:
            break
        else:
            page += 1

    with open(cache_path, "w") as f:
        json.dump(result, f)

    return result


def get_truedl_coeff(place, log_coeff=False):
    if place >= 5000:
        return
    if log_coeff:
        return round(-0.29 * math.log(place + 46.8) + 2.75, 2)
    else:
        return TRUEDL_COEFF_VALUES[bisect.bisect_left(TRUEDL_COEFF_KEYS, place)]


def get_real_truedl(questions, total_questions, place_in_rating, log_coeff=False):
    coeff = get_truedl_coeff(place_in_rating, log_coeff=log_coeff)
    return round(
        (1 - min(questions / coeff, total_questions) / total_questions) * 10.0, 1
    )


def parse_row(rd):
    rd = {k.value: v.value for k, v in rd.items()}
    return {
        "team": {"id": rd["IDteam"]},
        "current": {"name": rd["Название"]},
        "questionsTotal": int(rd["Вопросов"] or "0"),
        "rating": {
            "inRating": isinstance(rd["Релизный Б"], int),
            "rg": rd["Рейтинг Б"],
        },
    }


def parse_xlsx(filepath):
    wb = openpyxl.load_workbook(filepath)
    sheet = wb.active
    header = None
    result = []
    for row in sheet.rows:
        if header is None:
            header = row
        else:
            result.append(parse_row(dict(zip(header, row))))
    return result


def parse_detailed_results(xlsx_file_path):
    wb = openpyxl.load_workbook(xlsx_file_path)
    sheet = wb.active
    header = None
    result = defaultdict(
        lambda: {
            "team": {"id": None},
            "current": {"name": None},
            "mask": None,
        }
    )
    tour_lengths = defaultdict(list)
    for row_ in sheet.rows:
        if header is None:
            header = row_
        else:
            row = [x.value for x in row_ if x.value is not None]
            if not row:
                continue
            team_id = row[0]
            team_name = row[1]
            tour_number = row[3]
            if tour_number == "Тур":
                continue
            mask = "".join([str(x) for x in row[4:]])
            tour_lengths[tour_number].append(len(mask))
            result[team_id][tour_number] = mask
            result[team_id]["team"]["id"] = team_id
            result[team_id]["current"]["name"] = team_name
    for tour_num in tour_lengths:
        tour_lengths[tour_num] = max(tour_lengths[tour_num])

    for team_id in result:
        by_tour = result[team_id]
        final_masks = []
        for tour_num in sorted(tour_lengths):
            mask = by_tour.get(tour_num) or ""
            if len(mask) < tour_lengths[tour_num]:
                mask += "0" * (tour_lengths[tour_num] - len(mask))
            final_masks.append(mask)
        result[team_id]["mask"] = "".join(final_masks)
        result[team_id]["questionsTotal"] = len(
            [x for x in result[team_id]["mask"] if x == "1"]
        )
    return list(result.values())


def parse_questions_range(str_):
    sp = [x.strip() for x in str_.split(",")]
    result = []
    for el in sp:
        if "-" in el:
            from_, to_ = el.split("-", 1)
            result.extend(list(range(int(from_), int(to_) + 1)))
        else:
            result.append(int(el))
    return result


def get_result_from_mask(mask, question_numbers):
    mask = [(i + 1, v) for i, v in enumerate(list(mask)) if i + 1 in question_numbers]
    return len([x for x in mask if x[1] == "1"])


def get_old_dl(questions, total_questions, rg):
    return round(rg * (total_questions - questions) / total_questions / 500)


def get_avg_dl(dls):
    return round(sum(dls) / len(dls), 1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tournament_id", "-t", required=True)
    parser.add_argument("--results_from_xlsx")
    parser.add_argument("--results_from_xlsx_detailed")
    parser.add_argument("--question_numbers", "-q", default="all")
    parser.add_argument("--log_coeff", action="store_true")
    parser.add_argument("--use_all_teams", action="store_true")
    args = parser.parse_args()

    info = requests.get(
        f"https://api.rating.chgk.info/tournaments/{args.tournament_id.split(',')[0]}.json"
    ).json()
    dateStart = parse_date(info["dateStart"])

    release_id = get_release_id(dateStart)
    print(f"using release {release_id}")
    ratings = download_release(release_id)

    if args.question_numbers == "all":
        total_questions = sum(info["questionQty"].values())
    else:
        total_questions = len(parse_questions_range(args.question_numbers))
    results = []
    if args.results_from_xlsx_detailed:
        results = parse_detailed_results(
            args.results_from_xlsx_detailed,
        )
    else:
        for t_id in args.tournament_id.split(","):
            results.extend(requests.get(RESULTS_URL.format(id=t_id)).json())
    old_dls = []
    truedls = []
    truedl_by_tour = defaultdict(list)
    for res in results:
        team_id = res["team"]["id"]
        team_repr = f"{team_id} {res['current']['name']}"
        if not (res.get("rating") or {}).get("inRating") and not args.use_all_teams:
            print(f"skipping team {team_repr}, not in rating")
            continue
        if team_id not in ratings:
            print(f"skipping team {team_repr}, not found in rating")
            continue
        place_in_rating = ratings[team_id]["place"]
        if args.question_numbers == "all":
            questions = res["questionsTotal"]
        else:
            questions = get_result_from_mask(
                res["mask"], parse_questions_range(args.question_numbers)
            )
        try:
            rg = res["rating"]["rg"]
            dl = None
            truedl = None
            if (rg or 0) > 3000:
                dl = get_old_dl(questions, total_questions, rg)
                if dl is not None:
                    old_dls.append(dl)
            else:
                dl = None
        except:
            dl = None
        truedl = get_real_truedl(
            questions, total_questions, place_in_rating, log_coeff=args.log_coeff
        )
        if truedl is not None:
            truedls.append(truedl)
        offset = 0
        if args.question_numbers == "all":
            for tour_id in range(len(info["questionQty"])):
                questions_in_tour = info["questionQty"][str(tour_id + 1)]
                next_offset = offset + questions_in_tour
                mask_fragment = res["mask"][offset:next_offset]
                team_questions_in_tour = len([x for x in mask_fragment if x == "1"])
                truedl_by_tour[tour_id].append(
                    get_real_truedl(
                        team_questions_in_tour,
                        questions_in_tour,
                        place_in_rating,
                        log_coeff=args.log_coeff,
                    )
                )
                offset = next_offset
        if dl or truedl:
            print(
                f"team {team_id} {res['current']['name']}, questions {questions}, place {place_in_rating}, dl {dl}, truedl {truedl}"
            )
    if old_dls:
        old_avg = get_avg_dl(old_dls)
    else:
        old_avg = None
    if truedls:
        true_avg = get_avg_dl(truedls)
    else:
        true_avg = None
    print(f"final results:\ndl: {old_avg}, truedl: {true_avg}")
    for tour_id in sorted(truedl_by_tour):
        print(f"tour {tour_id + 1}: {get_avg_dl(truedl_by_tour[tour_id])}")


if __name__ == "__main__":
    main()
