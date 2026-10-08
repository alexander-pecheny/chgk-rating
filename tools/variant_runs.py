"""One full fit per config variant, scored on the forecast NDCG, the team NDCG
and the top of the team table.

    uv run tools/variant_runs.py out/variants2 base "" rel6 "question.rel_h=6" ...

Each variant is a name and a space-separated list of `section.key=value`
overrides against `config.toml`. The fit writes `DIR/NAME.db`, which is scored
and deleted; the summary goes to `DIR/results.md` and the top tables to
`DIR/NAME.top.txt`.
"""

import concurrent.futures as cf
import pathlib
import re
import sqlite3
import subprocess
import sys

import lab

TOP = 15
WATCH = {62868: "GG", 86405: "Мир", 57024: "Вжух", 46381: "Разведка"}
NAME_WIDTH = 28
ERROR_TAIL = 400
JOBS = 2
FORECAST = re.compile(r"ndcg (tune|test) (\d\.\d+)")
TEAM = re.compile(r"\b(0\.\d{4})\b")
HEAD = "| variant | forecast tune | forecast test | team tune | team test | ranks |\n|---|---:|---:|---:|---:|---|"
TABLE = """
select r.team_id, coalesce(n.name, r.team_id), r.display from team_ratings r
left join n.team_names n on n.id = r.team_id
where r.release_id = ? and r.listed = 1 and r.sigma <= 0.1 order by r.display desc
"""


def write_config(path, db, overrides):
    """`config.toml` with the overrides and the output path; every key must exist."""
    with open("config.toml", encoding="utf-8") as f:
        out, added = lab.override_toml(f.read().splitlines(), overrides + [f'data.out_path="{db}"'])
    assert not added, added
    pathlib.Path(path).write_text("\n".join(out) + "\n", encoding="utf-8")


def top_table(db):
    """The top of the last release's team table, and where the watched teams stand."""
    c = sqlite3.connect(db)
    c.execute("attach 'out/team_names.db' as n")
    rel = c.execute("select max(release_id) from team_ratings").fetchone()[0]
    rows = c.execute(TABLE, (rel,)).fetchall()
    rank = {t: i + 1 for i, (t, _, _) in enumerate(rows)}
    lines = [f"{i + 1:>3} {name[:NAME_WIDTH]:<28} {d:>6.0f}" for i, (_, name, d) in enumerate(rows[:TOP])]
    return lines, " ".join(f"{v} {rank.get(k, '-')}" for k, v in WATCH.items())


def team_ndcg(db, w):
    out = subprocess.run(["uv", "run", "tools/team_ndcg.py", db, "--window", w], capture_output=True, text=True).stdout
    m = TEAM.findall(out)
    return m[0] if m else "?"


def run(d, name, overrides):
    cfg, db = f"{d}/{name}.toml", f"{d}/{name}.db"
    write_config(cfg, db, overrides)
    fit = subprocess.run(["./target/release/fit", "--config", cfg], capture_output=True, text=True)
    if fit.returncode:
        return name, f"fit failed: {fit.stderr[-ERROR_TAIL:]}", None
    ev = subprocess.run(["./target/release/eval", "--config", cfg, "--db", db, "--ndcg"], capture_output=True, text=True)
    fc = dict(FORECAST.findall(ev.stdout))
    tn = {w: team_ndcg(db, w) for w in ("tune", "test")}
    lines, watch = top_table(db)
    pathlib.Path(f"{d}/{name}.top.txt").write_text("\n".join(lines) + "\n")
    pathlib.Path(db).unlink(missing_ok=True)
    row = f"| {name} | {fc.get('tune', '?')} | {fc.get('test', '?')} | {tn['tune']} | {tn['test']} | {watch} |"
    return name, row, lines


def main():
    d, args = sys.argv[1], sys.argv[2:]
    pathlib.Path(d).mkdir(parents=True, exist_ok=True)
    variants = [(args[i], args[i + 1].split()) for i in range(0, len(args), 2)]
    rows = {}
    with cf.ThreadPoolExecutor(JOBS) as ex:
        for name, row, _ in ex.map(lambda v: run(d, *v), variants):
            rows[name] = row
            print(row, flush=True)
    pathlib.Path(f"{d}/results.md").write_text(HEAD + "\n" + "\n".join(rows[n] for n, _ in variants) + "\n")


if __name__ == "__main__":
    main()
