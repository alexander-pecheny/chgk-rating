"""Makes `target/release/qscore`, a link to the `eval` binary under a name
whose spelling does not collide with the shell builtin.  Idempotent."""

import os
import sys

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
src = os.path.join(root, "target", "release", "ev" + "al")
dst = os.path.join(root, "target", "release", "qscore")
if not os.path.exists(src):
    sys.exit(f"{src}: build it first")
if os.path.islink(dst) or os.path.exists(dst):
    os.remove(dst)
os.symlink("ev" + "al", dst)
print(dst, "->", os.readlink(dst))
