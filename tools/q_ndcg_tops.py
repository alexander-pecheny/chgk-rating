"""Top 20 teams and top 20 players of two (or more) databases, side by side.

The tables of section 5 of out/final_report.md: the last release of each
database, provisional ratings left out, a team named from the mirror.

    uv run tools/q_ndcg_tops.py out/shipped.db out/q_ndcg/winner.db
"""

import lab

TOP = 20
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
OPTIONS = [("dbs", dict(nargs="+"))]


def top(db, q):
    c = lab.connect(db)
    c.execute(f"attach database '{lab.BUFF}' as buff")
    return [(n or "?", v) for n, v in c.execute(q)]


def main():
    dbs = lab.cli(__doc__, OPTIONS).dbs
    for what, q in (("teams", TEAMS), ("players", PLAYERS)):
        print(f"\n### Top 20 {what}\n")
        lab.side_by_side(dbs, [top(d, q) for d in dbs], TOP)


if __name__ == "__main__":
    main()
