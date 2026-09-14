"""Top 20 teams and top 20 players of two databases, side by side.

    uv run tools/q_ndcg_tops.py out/shipped.db out/q_ndcg/winner.db
"""

import sqlite3
import sys

TEAMS = """
select coalesce((select nullif(trim(t.name),'') from buff.teams t where t.id=r.team_id),
                (select x.team_current_name from buff.tournament_results x
                 where x.team_id=r.team_id order by x.id desc limit 1), '?'),
       r.display
from team_ratings r
where r.release_id = (select max(release_id) from team_ratings) and r.sigma <= 0.3
order by r.mu desc limit 20
"""

PLAYERS = """
select trim(p.surname || ' ' || coalesce(p.name,'')), r.display
from player_ratings r left join buff.players p on p.id = r.player_id
where r.release_id = (select max(release_id) from player_ratings) and r.sigma <= 0.3
order by r.theta desc limit 20
"""


def top(db, q):
    c = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    c.execute("attach database 'buff.db' as buff")
    return [(n or "?", v) for n, v in c.execute(q)]


def main():
    dbs = sys.argv[1:]
    for what, q in (("teams", TEAMS), ("players", PLAYERS)):
        cols = [top(d, q) for d in dbs]
        print(f"\n### Top 20 {what}\n")
        print("| # | " + " | ".join(dbs) + " |")
        print("|---:|" + "---|" * len(dbs))
        for i in range(20):
            cells = [f"{c[i][0]} {c[i][1]:.0f}" if i < len(c) else "-" for c in cols]
            print(f"| {i + 1} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
