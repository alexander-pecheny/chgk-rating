#!/usr/bin/env python
# -*- coding: utf-8 -*-
from collections import Counter, defaultdict
import argparse
import datetime
import functools
import os
import sqlite3
import sys

import numpy as np
import scipy
import orjson

from truedl.util import parse_date, _open


CREATE_TRUEDL_TABLES = """\
create table truedl_tournament_data(
    tournament_id integer,
    tournament_name text,
    tournament_type text,
    tournament_year integer,
    team_position real,
    team_questions integer,
    total_questions integer,
    team_id integer,
    team_name text,
    team_rating integer,
    hypothetical_questions integer,
    truedl_coeff real,
    truedl real
);
"""

POSITIONS_QUERY_STUB = """\
select idteam, rating_position, rating
from chgk_release_data_balanced
where idrelease = {idrelease};    
"""

ADD_TRUEDL_DATA_STUB = """\
insert into truedl_tournament_data (
    tournament_id,
    tournament_name,
    tournament_type,
    tournament_year,
    team_position,
    team_questions,
    total_questions,
    team_id,
    team_name,
    team_rating,
    hypothetical_questions,
    truedl_coeff,
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


@functools.cache
def get_ratings(rdb_cur, release_id):
    teamid_to_position = rdb_cur.execute(
        POSITIONS_QUERY_STUB.format(
            idrelease=release_id
        )
    ).fetchall()
    return frozenset(x[2] for x in teamid_to_position)


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
                    repr(point[k])
                    for k in [
                        "tournament_id",
                        "tournament_name",
                        "tournament_type",
                        "tournament_year",
                        "team_position",
                        "team_questions",
                        "total_questions",
                        "team_id",
                        "team_name",
                        "team_rating",
                        "hypothetical_questions",
                        "truedl_coeff",
                        "truedl",
                    ]
                ]
            )
            + ")"
        )
    values = ",   ".join(values)
    query = ADD_TRUEDL_DATA_STUB.format(values=values)
    cur.execute(query)
    conn.commit()


def avg(list_):
    return sum(list_) / len(list_)


def make_point(res, total_questions, t_info, tournament_type, date_start):
    team_id = res["team"]["id"]
    team_name = res["team"]["name"]
    rg = get_rating(res)
    return {
        "tournament_id": t_info["id"],
        "tournament_name": t_info["name"],
        "tournament_type": tournament_type,
        "tournament_year": date_start.year,
        "team_position": None,
        "team_questions": res["questionsTotal"],
        "total_questions": total_questions,
        "hypothetical_questions": None,
        "team_id": team_id,
        "team_name": team_name,
        "team_rating": rg,
        "truedl_coeff": None,
        "truedl": None,
    }


def get_coeff(rank):
    return round(-0.29 * np.log(rank + 46.8) + 2.75, 2)


def calc_truedl(point, rating_to_rank):
    rank = rating_to_rank[-point["team_rating"]]
    coeff = get_coeff(rank)
    hq = int(point["team_questions"] / coeff)
    tq = point["total_questions"]
    truedl = (1 - min(hq, tq) / tq) * 10
    point["hypothetical_questions"] = hq
    point["truedl"] = truedl
    point["truedl_coeff"] = coeff
    point["team_position"] = float(rank)


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
    parser.add_argument("--truedl_db", default="truedl_tournaments_log.db")
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
        ratings = set(get_ratings(rdb_cur, release_id))
        if not ratings:
            continue
        print(f"processing tournament {t_info['id']} {t_info['name']}...")
        data_points = []
        for res in results:
            if rg := get_rating(res):
                ratings.add(rg)
                data_points.append(
                    make_point(
                        res,
                        total_questions,
                        t_info,
                        tournament_type,
                        date_start,
                    )
                )
        ratings = np.array([-1 * i for i in ratings])
        ranks = scipy.stats.rankdata(ratings)
        rating_to_rank_dict = dict(zip(ratings, ranks))
        data_points_final = []
        for dp in data_points:
            calc_truedl(dp, rating_to_rank_dict)
            if (
                dp["team_position"] <= 5000
            ):
                data_points_final.append(dp)
        if args.debug_id:
            import IPython; IPython.embed()
        if data_points_final:
            add_to_db(conn, data_points_final)


if __name__ == "__main__":
    main()
