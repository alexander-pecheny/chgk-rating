#!/usr/bin/env python
# -*- coding: utf-8 -*-
from collections import Counter, defaultdict
import argparse
import datetime
import functools
import os
import sqlite3
import sys

import orjson

from truedl.util import parse_date, _open


CREATE_TRUEDL_TABLES = """\
create table truedl_tournament_data(
    tournament_id integer,
    tournament_name text,
    tournament_type text,
    tournament_year integer,
    team_category text,
    team_id integer,
    team_name text,
    result_abs integer,
    hypothetical_result integer,
    truedl real
);
"""

POSITIONS_QUERY_STUB = """\
select idteam, rating_position
from chgk_release_data_balanced
where idrelease = {idrelease};    
"""

ADD_TRUEDL_DATA_STUB = """\
insert into truedl_tournament_data (
    tournament_id,
    tournament_name,
    tournament_type,
    tournament_year,
    team_category,
    team_id,
    team_name,
    result_abs,
    hypothetical_result,
    truedl
)
values 
    {values};
"""

DELETE_TRUEDL_DATA_STUB = """\
delete from truedl_data
where tournament_id in {tournament_ids};
"""


def create_truedl_tables(conn):
    cur = conn.cursor()
    cur.execute(CREATE_TRUEDL_TABLES)
    conn.commit()


def make_release_lookup(releases):
    prev_release = None
    result = {}
    for release in releases:
        if prev_release:
            curr_date = parse_date(release["date"])
            prev_date = parse_date(prev_release["date"])
            while prev_date < curr_date:
                result[prev_date] = prev_release
                prev_date += datetime.timedelta(days=1)
        prev_release = release
    return result


def get_category(position):
    if not position or position <= 0:
        return "???"
    if position <= 10:
        return "1-10"
    if position <= 25:
        return "11-25"
    if position <= 50:
        return "26-50"
    if position <= 100:
        return "51-100"
    if position <= 250:
        return "101-250"
    if position <= 500:
        return "251-500"
    if position <= 1000:
        return "501-1000"
    if position <= 2000:
        return "1001-2000"
    if position <= 3000:
        return "2001-3000"
    if position <= 4000:
        return "3001-5000"
    if position <= 5000:
        return "3001-5000"
    return "5001+"


@functools.cache
def get_rating_positions(rdb_cur, release_id):
    teamid_to_position = rdb_cur.execute(
        POSITIONS_QUERY_STUB.format(
            idrelease=release_id
        )
    ).fetchall()
    return tuple({x[0]: get_category(x[1]) for x in teamid_to_position}.items())


def add_to_db(conn, data_points):
    cur = conn.cursor()
    tournament_ids = sorted({x["tournament_id"] for x in data_points})
    tournament_ids = "(" + ",".join(str(x) for x in tournament_ids) + ")"
    existing_data = cur.execute(
        f"select * from truedl_tournament_data where tournament_id in {tournament_ids};"
    ).fetchall()
    if existing_data:
        cur.execute(
            f"delete from truedl_tournament_data where tournament_id in {tournament_ids};"
        )
        conn.commit()
    values = []
    for point in data_points:
        values.append(
            "("
            + ",".join(
                [
                    repr(point["tournament_id"]),
                    repr(point["tournament_name"]),
                    repr(point["tournament_type"]),
                    repr(point["tournament_year"]),
                    repr(point["team_category"]),
                    repr(point["team_id"]),
                    repr(point["team_name"]),
                    repr(point["result_abs"]),
                    repr(point["hypothetical_result"]),
                    repr(point["truedl"]),
                ]
            )
            + ")"
        )
    values = ",   ".join(values)
    cur.execute(ADD_TRUEDL_DATA_STUB.format(values=values))
    conn.commit()


truedl_dict = {
    "1-10": 1.61,
    "11-25": 1.52,
    "26-50": 1.43,
    "51-100": 1.32,
    "101-250": 1.16,
    "251-500": 1.0,
    "501-1000": 0.81,
    "1001-2000": 0.6,
    "2001-3000": 0.43,
    "3001-5000": 0.31,
}


def avg(list_):
    return sum(list_) / len(list_)


def make_point(
    res, category, truedl_coeff, total_questions, t_info, tournament_type, date_start
):
    team_id = res["team"]["id"]
    team_name = res["team"]["name"]
    hypothetical_result = min(
        int(res["questionsTotal"] / truedl_coeff), total_questions
    )
    truedl = round((1.0 - (hypothetical_result / total_questions)) * 10.0, 1)
    return {
        "tournament_id": t_info["id"],
        "tournament_name": t_info["name"],
        "tournament_type": tournament_type,
        "tournament_year": date_start.year,
        "team_category": category,
        "total_questions": total_questions,
        "team_id": team_id,
        "team_name": team_name,
        "result_abs": res["questionsTotal"],
        "hypothetical_result": hypothetical_result,
        "truedl": truedl,
    }


def get_uncategorized_category(res, ratings_by_category):
    srt = sorted(ratings_by_category, key=lambda x: max(ratings_by_category[x]))
    rg = get_rating(res)
    if not rg:
        return
    for cat in srt:
        ratings = ratings_by_category[cat]
        if min(ratings) <= rg <= max(ratings):
            return cat


def get_rating(res):
    try:
        return res["rating"]["rg"]
    except (KeyError, TypeError):
        return


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tournament_info_dir", default="tournaments")
    parser.add_argument("--tournament_results_dir", default="tournament_results")
    parser.add_argument("--rating_db", default="rating.db")
    parser.add_argument("--truedl_db", default="truedl_tournaments.db")
    parser.add_argument("--debug_id", type=int, default=None)
    args = parser.parse_args()

    if not os.path.isfile(args.truedl_db):
        conn = sqlite3.connect(args.truedl_db)
        create_truedl_tables(conn)
    else:
        conn = sqlite3.connect(args.truedl_db)

    if not os.path.isfile(args.rating_db):
        sys.stderr.write(f"rating db not found at {args.rating_db}\n")
        sys.exit(1)
    rdb_conn = sqlite3.connect(args.rating_db)
    rdb_cur = rdb_conn.cursor()

    with _open("tournaments/releases.json") as f:
        releases = make_release_lookup(orjson.loads(f.read()))

    with _open("tournaments/tournaments.json") as f:
        t_infos = orjson.loads(f.read())

    for t_info in t_infos:
        if not t_info.get("tournamentInRatingBalanced"):
            prefix = ""
        else:
            prefix = "(R) "
        if args.debug_id and t_info["id"] != args.debug_id:
            continue
        tournament_type = f"{prefix}{t_info['type']['name']}"
        date_start = parse_date(t_info["dateStart"])
        if date_start not in releases:
            continue
        release_id = releases[date_start]["id"]
        results_filepath = (
            "tournament_results/" f"tournament-{t_info['id']}-results.json"
        )
        if not os.path.isfile(results_filepath):
            continue
        with _open(results_filepath) as f:
            results = orjson.loads(f.read())
        if not results:
            continue
        if not results[0].get("mask"):
            continue
        mask = results[0]["mask"]
        total_questions = len(mask) - Counter(mask)["X"]
        if not total_questions:
            continue
        print(f"processing tournament {t_info['id']} {t_info['name']}...")
        rating_positions = dict(get_rating_positions(rdb_cur, release_id))
        data_points = []
        uncategorized_teams = []
        ratings_by_category = defaultdict(list)
        for res in results:
            team_id = res["team"]["id"]
            category = rating_positions.get(team_id)
            if not category and res.get("teamMembers"):
                uncategorized_teams.append(res)
                continue
            truedl_coeff = truedl_dict.get(category)
            if not truedl_coeff:
                continue
            if rg := get_rating(res):
                ratings_by_category[category].append(rg)
            data_points.append(
                make_point(
                    res,
                    category,
                    truedl_coeff,
                    total_questions,
                    t_info,
                    tournament_type,
                    date_start,
                )
            )
        for res in uncategorized_teams:
            category = get_uncategorized_category(res, ratings_by_category)
            truedl_coeff = truedl_dict.get(category)
            if not truedl_coeff:
                continue
            data_points.append(
                make_point(
                    res,
                    category,
                    truedl_coeff,
                    total_questions,
                    t_info,
                    tournament_type,
                    date_start,
                )
            )
        if args.debug_id:
            import IPython; IPython.embed()
        if data_points:
            add_to_db(conn, data_points)


if __name__ == "__main__":
    main()
