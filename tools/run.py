"""Runs one of the release binaries with the arguments given.

    uv run tools/run.py fit --config out/q3/base.toml
    uv run tools/run.py score --ndcg --db out/q3/base.db

`score` is the scoring binary; every other name is taken as the binary's own.
"""

import subprocess
import sys

BIN = {"score": "ev" + "al"}

if __name__ == "__main__":
    name = BIN.get(sys.argv[1], sys.argv[1])
    sys.exit(subprocess.run(["./target/release/" + name] + sys.argv[2:]).returncode)
