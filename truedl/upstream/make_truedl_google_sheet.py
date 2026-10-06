#!/usr/bin/env python
# -*- coding: utf-8 -*-
import argparse
import bisect
import datetime
import math
import time
from collections import defaultdict

import gspread
import openpyxl
import requests
from google_sheet_writer import Cf, Checkbox, FCell, GoogleSheetWriter

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
    result = defaultdict(dict)
    tour_lengths = defaultdict(list)
    for row_ in sheet.rows:
        if header is None:
            header = row_
        else:
            row = [x.value for x in row_ if x.value is not None]
            team_id = row[0]
            tour_number = row[3]
            mask = "".join([str(x) for x in row[4:]])
            tour_lengths[tour_number].append(len(mask))
            result[team_id][tour_number] = mask
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
        result[team_id] = "".join(final_masks)
    return result


def patch_results_from_detailed_xlsx(results, xlsx_file_path):
    detailed_results = parse_detailed_results(xlsx_file_path)
    for res in results:
        res["mask"] = detailed_results[res["team"]["id"]]


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


def get_result_from_mask(mask, question_numbers=None):
    mask = [
        (i + 1, v)
        for i, v in enumerate(list(mask))
        if (question_numbers is None) or (i + 1 in question_numbers)
    ]
    mask_1_0 = [(1 if x[1] == "1" else 0) for x in mask]
    return mask_1_0, sum(mask_1_0)


def get_old_dl(questions, total_questions, rg):
    return round(rg * (total_questions - questions) / total_questions / 500)


class Writer(GoogleSheetWriter):
    def make_table(self, args):
        wks = self.get_worksheet("truedl", remove=True)
        cur = wks.cursor
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
        start_checkboxes = wks.spawn_cursor(y=1, x=6)
        tq = f'COUNTIF({start_checkboxes}:{start_checkboxes.clone_shift(x=total_questions)}, "TRUE")'
        cur.hblock_n(
            [
                None,
                FCell("Вопросов:", Cf.RIGHT),
                "=" + tq,
                FCell("Итого:", Cf.RIGHT),
                None,
            ]
            + [FCell(Checkbox(True), width=30) for _ in range(total_questions)]
        )
        cur.hblock_n(
            ["Команда", "Место в рейтинге", "Коэффициент", "Сумма", "Truedl"]
            + list(range(1, total_questions + 1))
        )
        results = []
        if args.results_from_xlsx:
            for fn in args.results_from_xlsx.split(","):
                results.extend(parse_xlsx(fn))
            if args.question_numbers != "all":
                assert args.results_from_xlsx_detailed
                patch_results_from_detailed_xlsx(
                    results,
                    args.results_from_xlsx_detailed,
                )
        else:
            for t_id in args.tournament_id.split(","):
                results.extend(requests.get(RESULTS_URL.format(id=t_id)).json())
        for res in results:
            team_id = res["team"]["id"]
            team_repr = f"{team_id} {res['current']['name']}"
            if not (res["rating"] or {}).get("inRating"):
                print(f"skipping team {team_repr}, not in rating")
                continue
            if team_id not in ratings:
                print(f"skipping team {team_repr}, not found in rating")
                continue
            place_in_rating = ratings[team_id]["place"]
            if args.question_numbers == "all":
                qn = None
            else:
                qn = parse_questions_range(args.question_numbers)
            mask, _ = get_result_from_mask(res["mask"], qn)
            coeff = get_truedl_coeff(place_in_rating)
            if not coeff:
                print(f"skipping team {team_repr} because it has no coeff")
                continue
            cur.hblock_n(
                [
                    team_repr,
                    place_in_rating,
                    coeff,
                    lambda x: f"=SUMPRODUCT({start_checkboxes}:{start_checkboxes.clone_shift(x=total_questions)}, {x.clone_shift(x=2)}:{x.clone_shift(x=2 + total_questions)})",
                    lambda x: f"=ROUND((1 - MIN({x.clone_shift(x=-1)} / {x.clone_shift(x=-2)}, {tq}) / {tq}) * 10.0, 1)",
                ]
                + mask
            )
        n_teams = cur.y - 3
        start_truedls = wks.spawn_cursor(y=2, x=5)
        truedl_cur = wks.spawn_cursor(y=1, x=5)
        truedl_cur.hblock_n(
            [
                f"=ROUND(SUM({start_truedls}:{start_truedls.clone_shift(y=n_teams)}) / {n_teams}, 1)"
            ]
        )
        truedl_cur.freeze_col()
        truedl_cur.freeze_row()
        self.submit()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tournament_id", "-t", required=True)
    parser.add_argument("--results_from_xlsx")
    parser.add_argument("--results_from_xlsx_detailed")
    parser.add_argument("--service_account", default="service_account.json")
    parser.add_argument("--google_sheet_id", "-g", required=True)
    parser.add_argument("--question_numbers", "-q", default="all")
    parser.add_argument("--log_coeff", action="store_true")
    args = parser.parse_args()

    gc = gspread.service_account(args.service_account)

    writer = Writer(gc, args.google_sheet_id)
    writer.make_table(args)


if __name__ == "__main__":
    main()
