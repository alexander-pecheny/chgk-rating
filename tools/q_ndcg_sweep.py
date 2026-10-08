"""Runs the plan-3 sweep: one `fit` per config variant, scored on the NDCG of `eval`.

Each experiment is a name and a list of `section.key=value` overrides against
`out/q_ndcg/base.toml`, which is `config.toml` with `data.to = 2026-09-05` and
`data.rows_every = 0`.  The database is scored and then deleted, unless the name
is listed in `--keep`, so a hundred runs cost a quarter of a gigabyte and not
twenty-five.

    uv run tools/q_ndcg_sweep.py out/q_ndcg/batch/format.txt [--jobs 2] [--keep name,name]

`--dir D` moves the working directory (base config, scratch databases, results
table) somewhere else, which is how plan 5 keeps its runs apart from plan 3's.

A batch file holds one experiment per line:

    w_sync_025  data.w_sync=0.25
    w_async_01  data.w_async=0.1

Results are appended to `out/q_ndcg/results.tsv`: name, tune NDCG and NDCG@3,
test NDCG and NDCG@3, weighted and unweighted, and the overrides.
"""

import concurrent.futures as cf
import os
import re
import subprocess

import lab

DIR = "out/q_ndcg"
# `eval --ndcg` prints four numbers a window, `--paired` four more.
SCORES, PAIRED_SCORES = 8, 16
NUMBER = re.compile(r"[-+]?\d\.\d{4}")
ERROR_LINES = 3
HEAD = ("name\ttune\ttune@3\tutune\tutune@3\ttest\ttest@3\tutest\tutest@3"
        "\tdtune\tsetune\tdtune@3\tsetune@3\tdtest\tsetest\tdtest@3\tsetest@3\twhat\n")
OPTIONS = [
    ("batches", dict(nargs="*", help="batch files, one `name section.key=value ...` per line")),
    ("--dir", dict(default=DIR, help="the working directory: base config, scratch databases, results")),
    ("--jobs", dict(type=int, default=2)),
    ("--keep", dict(default="", help="comma list of experiment names whose databases are kept")),
]


def paths():
    return f"{DIR}/base.toml", f"{DIR}/results.tsv"


def write_config(name, overrides):
    """Base config with `section.key = value` replaced, or appended to the section."""
    with open(paths()[0], encoding="utf-8") as f:
        out, _ = lab.override_toml(f.read().splitlines(), overrides)
    path = f"{DIR}/cfg/{name}.toml"
    os.makedirs(f"{DIR}/cfg", exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    return path


def scores(stdout):
    """The first four numbers of every line: "ndcg <window> a @3 b unweighted c @3 d",
    then "paired <window> minus base: d se s @3 d3 se3"."""
    nums = []
    for line in stdout.splitlines():
        nums += [float(x) for x in NUMBER.findall(line)][:4]
    return nums


def evaluate(name, db):
    cmd = ["./target/release/eval", "--db", db, "--ndcg"]
    if name != "baseline" and os.path.exists(f"{DIR}/base.db"):
        cmd += ["--paired", f"{DIR}/base.db"]
    return subprocess.run(cmd, capture_output=True, text=True)


def run_one(name, overrides, keep):
    cfg = write_config(name, overrides)
    db = f"{DIR}/{name}.db"
    fit = subprocess.run(["./target/release/fit", "--config", cfg, "--out", db], capture_output=True, text=True)
    if fit.returncode != 0:
        return name, None, fit.stderr.strip().splitlines()[-ERROR_LINES:]
    ev = evaluate(name, db)
    nums = scores(ev.stdout)
    if name not in keep:
        os.remove(db)
    if len(nums) not in (SCORES, PAIRED_SCORES):
        return name, None, ev.stdout.strip() + ev.stderr.strip()
    return name, nums + [0.0] * (PAIRED_SCORES - len(nums)), " ".join(overrides)


def experiments(batches):
    todo = []
    for b in batches:
        with open(b, encoding="utf-8") as f:
            todo += [(w[0], w[1:]) for w in (line.split("#")[0].split() for line in f) if w]
    return todo


def main():
    global DIR
    a = lab.cli(__doc__, OPTIONS, intermixed=True)
    DIR, keep = a.dir.rstrip("/"), set(a.keep.split(",")) if a.keep else set()
    todo = experiments(a.batches)
    _, results = paths()
    os.makedirs(DIR, exist_ok=True)
    new = not os.path.exists(results)
    with open(results, "a", encoding="utf-8") as f, cf.ThreadPoolExecutor(max_workers=a.jobs) as ex:
        if new:
            f.write(HEAD)
        for name, nums, what in ex.map(lambda t: run_one(*t, keep), todo):
            line = f"{name}: FAILED {what}" if nums is None else f"{name}\t" + "\t".join(f"{x:.4f}" for x in nums) + f"\t{what}"
            if nums is not None:
                f.write(line + "\n")
                f.flush()
            print(line)


if __name__ == "__main__":
    main()
