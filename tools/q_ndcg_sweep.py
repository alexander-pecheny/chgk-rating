"""Runs the plan-3 sweep: one `fit` per config variant, scored on the NDCG of `eval`.

Each experiment is a name and a list of `section.key=value` overrides against
`out/q_ndcg/base.toml`, which is `config.toml` with `data.to = 2026-09-05` and
`data.rows_every = 0`.  The database is scored and then deleted, unless the name
is listed in `--keep`, so a hundred runs cost a quarter of a gigabyte and not
twenty-five.

    uv run tools/q_ndcg_sweep.py out/q_ndcg/batch/format.txt [--jobs 2] [--keep name,name]

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
import sys

BASE = "out/q_ndcg/base.toml"
DIR = "out/q_ndcg"
RESULTS = f"{DIR}/results.tsv"


def write_config(name, overrides):
    """Base config with `section.key = value` replaced, or appended to the section."""
    lines = open(BASE, encoding="utf-8").read().splitlines()
    want = {}
    for o in overrides:
        k, v = o.split("=", 1)
        sec, key = k.rsplit(".", 1)
        want[(sec, key)] = v
    out, section, done = [], "", set()
    for line in lines:
        m = re.match(r"\s*\[([^\]]+)\]", line)
        if m:
            # Anything still unplaced for the section just closed goes in at its end.
            for (sec, key), v in want.items():
                if sec == section and (sec, key) not in done:
                    out.append(f"{key} = {v}")
                    done.add((sec, key))
            section = m.group(1)
        m = re.match(r"\s*([A-Za-z_][A-Za-z0-9_]*)\s*=", line)
        if m and (section, m.group(1)) in want:
            line = f"{m.group(1)} = {want[(section, m.group(1))]}"
            done.add((section, m.group(1)))
        out.append(line)
    for (sec, key), v in want.items():
        if (sec, key) not in done:
            if sec != section:
                out.append(f"[{sec}]")
                section = sec
            out.append(f"{key} = {v}")
    path = f"{DIR}/cfg/{name}.toml"
    os.makedirs(f"{DIR}/cfg", exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    return path


def run_one(name, overrides, keep):
    cfg = write_config(name, overrides)
    db = f"{DIR}/{name}.db"
    fit = subprocess.run(
        ["./target/release/fit", "--config", cfg, "--out", db],
        capture_output=True, text=True,
    )
    if fit.returncode != 0:
        return name, None, fit.stderr.strip().splitlines()[-3:]
    cmd = ["./target/release/eval", "--db", db, "--ndcg"]
    if name != "baseline" and os.path.exists(f"{DIR}/base.db"):
        cmd += ["--paired", f"{DIR}/base.db"]
    ev = subprocess.run(cmd, capture_output=True, text=True)
    nums = []
    for line in ev.stdout.splitlines():
        f = [float(x) for x in re.findall(r"[-+]?\d\.\d{4}", line)]
        # "ndcg <window> a @3 b unweighted c @3 d", then
        # "paired <window> minus base: d se s @3 d3 se3".
        nums += f[:4]
    if name not in keep:
        os.remove(db)
    if len(nums) not in (8, 16):
        return name, None, ev.stdout.strip() + ev.stderr.strip()
    return name, nums + [0.0] * (16 - len(nums)), " ".join(overrides)


def main():
    args = [a for a in sys.argv[1:]]
    jobs, keep, batches = 2, set(), []
    i = 0
    while i < len(args):
        if args[i] == "--jobs":
            jobs = int(args[i + 1])
            i += 2
        elif args[i] == "--keep":
            keep = set(args[i + 1].split(","))
            i += 2
        else:
            batches.append(args[i])
            i += 1
    todo = []
    for b in batches:
        for line in open(b, encoding="utf-8"):
            line = line.split("#")[0].split()
            if line:
                todo.append((line[0], line[1:]))
    os.makedirs(DIR, exist_ok=True)
    new = not os.path.exists(RESULTS)
    with open(RESULTS, "a", encoding="utf-8") as f:
        if new:
            f.write(
                "name\ttune\ttune@3\tutune\tutune@3\ttest\ttest@3\tutest\tutest@3"
                "\tdtune\tsetune\tdtune@3\tsetune@3\tdtest\tsetest\tdtest@3\tsetest@3\twhat\n"
            )
        with cf.ThreadPoolExecutor(max_workers=jobs) as ex:
            for name, nums, what in ex.map(lambda t: run_one(*t, keep), todo):
                if nums is None:
                    print(f"{name}: FAILED {what}")
                    continue
                row = "\t".join(f"{x:.4f}" for x in nums)
                f.write(f"{name}\t{row}\t{what}\n")
                f.flush()
                print(f"{name}\t{row}\t{what}")


if __name__ == "__main__":
    main()
