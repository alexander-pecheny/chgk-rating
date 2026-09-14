"""Top teams of the last release, side by side, for out/team_ndcg/report.md.

    uv run tools/team_top20.py out/team_ndcg/base.db out/team_ndcg/pull_kq010.db
"""

import sqlite3
import sys


def top(db, n):
    c = sqlite3.connect(db)
    c.execute("attach database 'buff.db' as buff")
    rel = c.execute("select max(release_id) from team_ratings").fetchone()[0]
    return c.execute(
        "select t.name, r.display from team_ratings r join buff.teams t on t.id = r.team_id "
        "where r.release_id = ? and r.sigma <= 0.3 order by r.mu desc limit ?",
        (rel, n),
    ).fetchall()


def main():
    dbs = [a for a in sys.argv[1:] if not a.startswith("--")]
    n = 20
    cols = [top(db, n) for db in dbs]
    print("| # | " + " | ".join(dbs) + " |")
    print("|---:|" + "|".join(["---"] * len(dbs)) + "|")
    for i in range(n):
        cells = [f"{c[i][0]} {c[i][1]:.0f}" if i < len(c) else "-" for c in cols]
        print(f"| {i + 1} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
