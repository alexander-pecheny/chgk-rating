#!/usr/bin/env python
# -*- coding: utf-8 -*-
from collections import Counter, defaultdict
import argparse
import datetime
import os
import sqlite3
import sys

import orjson

from truedl.util import parse_date, _open


CREATE_TRUEDL_TABLES = """\
create table truedl_data(
    tournament_id integer,
    tournament_year integer,
    team_category text,
    result_abs real,
    ratio_to_top10 real
);
"""

POSITIONS_QUERY_STUB = """\
select idteam, rating_position
from chgk_release_data_balanced
where idrelease = {idrelease}
and idteam in ({team_ids});    
"""

ADD_TRUEDL_DATA_STUB = """\
insert into truedl_data (
    tournament_id, tournament_year, team_category, result_abs, ratio_to_top10
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


def get_rating_positions(rdb_cur, release_id, team_ids):
    teamid_to_position = rdb_cur.execute(POSITIONS_QUERY_STUB.format(
        idrelease=release_id,
        team_ids = ",".join(str(x) for x in team_ids)
    )).fetchall()
    return {
        x[0]: get_category(x[1]) for x in teamid_to_position
    }


def add_to_db(conn, data_points):
    cur = conn.cursor()
    tournament_ids = sorted({x["tournament_id"] for x in data_points})
    tournament_ids = "(" + ",".join(str(x) for x in tournament_ids) + ")"
    existing_data = cur.execute(
        f"select * from truedl_data where tournament_id in {tournament_ids};"
    ).fetchall()
    if existing_data:
        cur.execute(
            f"delete from truedl_data where tournament_id in {tournament_ids};"
        )
        conn.commit()
    values = []
    for point in data_points:
        values.append(
            "(" + ",".join([
                repr(point["tournament_id"]),
                repr(point["tournament_year"]),
                repr(point["team_category"]),
                repr(point["result_abs"]),
                repr(point["ratio_to_top10"]),
            ]) + ")"
        )
    values = ",   ".join(values)
    try:
        cur.execute(ADD_TRUEDL_DATA_STUB.format(
            values=values
        ))
    except:
        import pdb; pdb.set_trace()
    conn.commit()


def avg(list_):
    return sum(list_) / len(list_)


def is_in_rating(res):
    return (res.get("rating") or {}).get("inRating")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tournament_info_dir", default="tournaments")
    parser.add_argument("--tournament_results_dir", default="tournament_results")
    parser.add_argument("--rating_db", default="rating.db")
    parser.add_argument("--truedl_db", default="truedl.db")
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
            continue
        date_start = parse_date(t_info["dateStart"])
        if date_start not in releases:
            continue
        release_id = releases[date_start]["id"]
        results_filepath = (
            "tournament_results/"
            f"tournament-{t_info['id']}-results.json"
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
        team_ids = {x["team"]["id"] for x in results if is_in_rating(x)}
        rating_positions = get_rating_positions(rdb_cur, release_id, team_ids)
        if "1-10" not in rating_positions.values():
            continue
        by_category = defaultdict(list)
        for res in results:
            if not is_in_rating(res):
                continue
            team_id = res["team"]["id"]
            category = rating_positions.get(team_id)
            if not category:
                continue
            by_category[category].append(res["questionsTotal"] or 0)
        top10_avg = avg(by_category["1-10"])
        if not top10_avg:
            continue
        data_points = []
        for cat in by_category:
            if cat == "1-10":
                continue
            list_ = by_category[cat]
            if len(list_) < 5:
                continue
            cat_avg = avg(list_)
            data_points.append({
                "tournament_id": t_info["id"],
                "tournament_year": date_start.year,
                "team_category": cat,
                "result_abs": cat_avg,
                "ratio_to_top10": cat_avg / top10_avg
            })
        if data_points:
            add_to_db(conn, data_points)






    
    



if __name__ == "__main__":
    main()
