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

TOP = 15
WATCH = {62868: "GG", 86405: "Мир", 57024: "Вжух", 46381: "Разведка"}


def write_config(path, db, overrides):
    lines = open("config.toml", encoding="utf-8").read().splitlines()
    want = {}
    for o in overrides + [f'data.out_path="{db}"']:
        k, v = o.split("=", 1)
        sec, key = k.rsplit(".", 1)
        want[(sec.replace(".", "\0"), key)] = v
    out, section, done = [], "", set()

    def flush():
        for (sec, key), v in want.items():
            if sec == section and (sec, key) not in done:
                out.append(f"{key} = {v}")
                done.add((sec, key))

    for line in lines:
        m = re.match(r"\s*\[([^\]]+)\]", line)
        if m:
            flush()
            section = m.group(1).replace(".", "\0")
        k = re.match(r"\s*([A-Za-z_0-9]+)\s*=", line)
        if k and (section, k.group(1)) in want:
            key = (section, k.group(1))
            out.append(f"{k.group(1)} = {want[key]}")
            done.add(key)
            continue
        out.append(line)
    flush()
    missing = set(want) - done
    assert not missing, missing
    open(path, "w", encoding="utf-8").write("\n".join(out) + "\n")


def top_table(db):
    c = sqlite3.connect(db)
    c.execute("attach 'out/team_names.db' as n")
    rel = c.execute("select max(release_id) from team_ratings").fetchone()[0]
    rows = c.execute(
        "select r.team_id, coalesce(n.name, r.team_id), r.display from team_ratings r "
        "left join n.team_names n on n.id = r.team_id "
        "where r.release_id = ? and r.listed = 1 and r.sigma <= 0.1 order by r.display desc",
        (rel,),
    ).fetchall()
    rank = {t: i + 1 for i, (t, _, _) in enumerate(rows)}
    lines = [f"{i + 1:>3} {name[:28]:<28} {d:>6.0f}" for i, (_, name, d) in enumerate(rows[:TOP])]
    watch = " ".join(f"{v} {rank.get(k, '-')}" for k, v in WATCH.items())
    return lines, watch


def run(d, name, overrides):
    cfg, db = f"{d}/{name}.toml", f"{d}/{name}.db"
    write_config(cfg, db, overrides)
    fit = subprocess.run(["./target/release/fit", "--config", cfg], capture_output=True, text=True)
    if fit.returncode:
        return name, f"fit failed: {fit.stderr[-400:]}", None
    ev = subprocess.run(
        ["./target/release/eval", "--config", cfg, "--db", db, "--ndcg"], capture_output=True, text=True
    ).stdout
    fc = {w: m for w, m in re.findall(r"ndcg (tune|test) (\d\.\d+)", ev)}
    tn = {}
    for w in ("tune", "test"):
        out = subprocess.run(
            ["uv", "run", "tools/team_ndcg.py", db, "--window", w], capture_output=True, text=True
        ).stdout
        m = re.findall(r"\b(0\.\d{4})\b", out)
        tn[w] = m[0] if m else "?"
    lines, watch = top_table(db)
    pathlib.Path(f"{d}/{name}.top.txt").write_text("\n".join(lines) + "\n")
    pathlib.Path(db).unlink(missing_ok=True)
    row = f"| {name} | {fc.get('tune', '?')} | {fc.get('test', '?')} | {tn['tune']} | {tn['test']} | {watch} |"
    return name, row, lines


def main():
    d, args = sys.argv[1], sys.argv[2:]
    pathlib.Path(d).mkdir(parents=True, exist_ok=True)
    variants = [(args[i], args[i + 1].split()) for i in range(0, len(args), 2)]
    head = "| variant | forecast tune | forecast test | team tune | team test | ranks |\n|---|---:|---:|---:|---:|---|"
    rows = {}
    with cf.ThreadPoolExecutor(2) as ex:
        for name, row, _ in ex.map(lambda v: run(d, *v), variants):
            rows[name] = row
            print(row, flush=True)
    pathlib.Path(f"{d}/results.md").write_text(head + "\n" + "\n".join(rows[n] for n, _ in variants) + "\n")


main()
