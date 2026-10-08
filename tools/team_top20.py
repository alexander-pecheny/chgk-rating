"""Top teams of the last release, side by side, for out/team_ndcg/report.md.

    uv run tools/team_top20.py out/team_ndcg/base.db out/team_ndcg/pull_kq010.db
"""

import sqlite3

import lab

TOP = 20
QUERY = """
select t.name, r.display from team_ratings r join buff.teams t on t.id = r.team_id
where r.release_id = (select max(release_id) from team_ratings) and r.sigma <= 0.3
order by r.mu desc limit ?
"""
OPTIONS = [("dbs", dict(nargs="+")), ("--top", dict(type=int, default=TOP))]


def top(db, n):
    c = sqlite3.connect(db)
    c.execute(f"attach database '{lab.BUFF}' as buff")
    return c.execute(QUERY, (n,)).fetchall()


def main():
    a = lab.cli(__doc__, OPTIONS)
    lab.side_by_side(a.dbs, [top(db, a.top) for db in a.dbs], a.top)


if __name__ == "__main__":
    main()
