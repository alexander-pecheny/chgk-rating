"""How close the reimplemented London Elo (`ext_elo`) comes to the site.

Reads the cached tournament pages of `tools/fetch/elo_pages.py` (out/elo_pages/)
and compares, tournament by tournament, each player's Elo before, each player's
change, each lineup's expected score and each question's difficulty and width
with what the replay wrote:

    uv run tools/elo_check.py out/ext/elo.db [--pages out/elo_pages]
"""

import argparse
import math
import pathlib
import sqlite3
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent / "fetch"))
import elo_pages  # noqa: E402

LN10 = math.log(10)


def corr(xs, ys):
    n = len(xs)
    if n < 3:
        return float("nan")
    mx, my = sum(xs) / n, sum(ys) / n
    sxy = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    sxx = sum((x - mx) ** 2 for x in xs)
    syy = sum((y - my) ** 2 for y in ys)
    return sxy / math.sqrt(sxx * syy) if sxx > 0 and syy > 0 else float("nan")


def summary(name, pairs):
    if not pairs:
        print(f"| {name} | 0 | | | |")
        return
    site, ours = zip(*pairs)
    mae = sum(abs(a - b) for a, b in pairs) / len(pairs)
    bias = sum(b - a for a, b in pairs) / len(pairs)
    print(f"| {name} | {len(pairs)} | {corr(site, ours):.4f} | {mae:.2f} | {bias:+.2f} |")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("db")
    ap.add_argument("--pages", default="out/elo_pages")
    a = ap.parse_args()
    db = sqlite3.connect(a.db)
    elo, delta, expected, diff, width = [], [], [], [], []
    for page in sorted(pathlib.Path(a.pages).glob("*.html")):
        src = page.read_text(encoding="utf-8")
        if not src:
            continue
        tid = int(page.stem)
        teams, qs = elo_pages.parse(src)
        ours = {p: (b, af) for p, b, af in db.execute(
            "select player_id, theta_before, theta_after from player_events where tournament_id = ?", (tid,))}
        fc = dict(db.execute("select team_id, forecast from team_events where tournament_id = ?", (tid,)))
        qp = {q + 1: (d, s) for q, d, s in db.execute(
            "select q, b, a from question_params where tournament_id = ?", (tid,))}
        for t in teams:
            if t["team"] in fc and t["expected"] is not None:
                expected.append((t["expected"], fc[t["team"]]))
            for (p, e), d in zip(t["players"], t["deltas"]):
                if p in ours:
                    elo.append((e, ours[p][0]))
                    if d is not None:
                        delta.append((d, ours[p][1] - ours[p][0]))
        for num, d, w, s, _ in qs:
            if num in qp and d not in (0.0, 5000.0):
                diff.append((d, qp[num][0]))
                width.append((w, LN10 / qp[num][1]))
    print("| quantity | n | correlation | mean abs. difference | ours minus site |")
    print("|---|---:|---:|---:|---:|")
    summary("player Elo before", elo)
    summary("player change", delta)
    summary("expected score", expected)
    summary("question difficulty", diff)
    summary("question width", width)


if __name__ == "__main__":
    main()
