"""Newcomer damping sweep: each variant is fitted twice, from 2013-01-01 and from
2013-01-13 (without the first weekend), then scored on the forecast NDCG paired
against the base, and on how much one early weekend still decides.

    uv run tools/newcomer_sweep.py --dir out/krapil --bin DIR --jobs 2 \
        base "" cap03 "player.new_cap=0.3 player.new_cap_games=5" ...

Per variant it prints: tune and test NDCG with their paired differences, the
latest theta and rank of players 16332 and 1585 (Krapil, Arkhangelskaya),
how many players the first weekend moves by more than 0.1 at the latest
release, and how far newcomers who jumped more than 0.5 in their first game
fall by their tenth.
"""

import argparse
import concurrent.futures as cf
import os
import re
import sqlite3
import subprocess

import lab

COUPLE = (16332, 1585)
LATE_FROM = "2013-01-13"


def write_config(d, name, overrides):
    with open("config.toml", encoding="utf-8") as f:
        base = ["data.to=\"2026-09-05\"", "data.rows_every=0"]
        out, _ = lab.override_toml(f.read().splitlines(), base + overrides)
    path = f"{d}/cfg/{name}.toml"
    os.makedirs(f"{d}/cfg", exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    return path


def fit(binary, cfg, out, start=None):
    cmd = [f"{binary}/fit", "--config", cfg, "--out", out]
    if start:
        cmd += ["--from", start]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(r.stderr[-2000:])


def complete(db):
    """Whether a fit at `db` ran to the end of the window, not just started."""
    if not os.path.exists(db):
        return False
    try:
        last = sqlite3.connect(db).execute("select max(date) from releases").fetchone()[0]
    except sqlite3.Error:
        return False
    return last is not None and last >= "2026-09"


def latest(db):
    c = sqlite3.connect(db)
    rel = c.execute("select max(id) from releases").fetchone()[0]
    return c, rel


def couple(db):
    c, rel = latest(db)
    out = []
    for p in COUPLE:
        th, sg, disp = c.execute("select theta, sigma, display from player_ratings where release_id=? and player_id=?", (rel, p)).fetchone()
        rank = c.execute("select count(*)+1 from player_ratings where release_id=? and display>?", (rel, disp)).fetchone()[0]
        out.append((th, sg, rank))
    return out


def sensitivity(db, late):
    c, rel = latest(db)
    c.execute("attach ? as l", (late,))
    lrel = c.execute("select max(id) from l.releases").fetchone()[0]
    return c.execute(
        """select sum(abs(a.theta-b.theta)>0.1), avg(abs(a.theta-b.theta)) from player_ratings a
           join l.player_ratings b on b.player_id=a.player_id and b.release_id=?
           where a.release_id=?""", (lrel, rel)).fetchone()


def overshoot(db):
    """Newcomers of 2017-2023 by first step: mean theta after game 1 and game 10."""
    c = sqlite3.connect(db)
    c.executescript("""
        create temp table d as select tournament_id, min(date) date from team_events group by tournament_id;
        create temp table fe as select pe.player_id, d.date, pe.theta_after-pe.theta_before step, pe.theta_after ta,
          row_number() over (partition by pe.player_id order by d.date, pe.tournament_id) n
          from player_events pe join d using(tournament_id);
        create temp table g as select a.player_id, a.step, a.ta t1, b.ta t10 from fe a join fe b
          on b.player_id=a.player_id and b.n=10 where a.n=1 and a.date>='2017' and a.date<'2024';""")
    jump = c.execute("select count(*), avg(t1-t10), max(step) from g where step>0.5").fetchone()
    flat = c.execute("select avg(t1-t10) from g where step<=0.5").fetchone()[0]
    over = jump[1] - flat if jump[1] is not None else float("nan")
    return jump[0], over, jump[2] if jump[2] is not None else float("nan")


def early_error(db):
    """RMS distance of a 2017-2023 newcomer's theta after games 5 and 10 from
    their theta after game 30, over newcomers with 30 games."""
    c = sqlite3.connect(db)
    c.executescript("""
        create temp table d as select tournament_id, min(date) date from team_events group by tournament_id;
        create temp table fe as select pe.player_id, d.date, pe.theta_after ta,
          row_number() over (partition by pe.player_id order by d.date, pe.tournament_id) n
          from player_events pe join d using(tournament_id);""")
    return c.execute("""select count(*), sqrt(avg((a.ta-z.ta)*(a.ta-z.ta))), sqrt(avg((b.ta-z.ta)*(b.ta-z.ta)))
        from fe f join fe a on a.player_id=f.player_id and a.n=5 join fe b on b.player_id=f.player_id and b.n=10
        join fe z on z.player_id=f.player_id and z.n=30 where f.n=1 and f.date>='2017' and f.date<'2024'""").fetchone()


def ndcg(binary, db, base):
    cmd = [f"{binary}/eval", "--db", db, "--ndcg"]
    if base and os.path.abspath(db) != os.path.abspath(base):
        cmd += ["--paired", base]
    r = subprocess.run(cmd, capture_output=True, text=True)
    return [ln for ln in r.stdout.splitlines() if ln.startswith(("ndcg", "paired"))]


def run(a, name, overrides):
    cfg = write_config(a.dir, name, overrides)
    db, late = f"{a.dir}/{name}.db", f"{a.dir}/{name}_late.db"
    if not (complete(db) and complete(late)):
        with cf.ThreadPoolExecutor(2) as ex:
            list(ex.map(lambda x: fit(a.bin, cfg, *x), [(db,), (late, LATE_FROM)]))
    (kt, ks, kr), (jt, js, jr) = couple(db)
    moved, mean_moved = sensitivity(db, late)
    n_jump, over, max_step = overshoot(db)
    n_early, e5, e10 = early_error(db)
    lines = ndcg(a.bin, db, f"{a.dir}/base.db")
    os.remove(late)
    return (f"== {name}  {' '.join(overrides)}\n"
            f"couple: K {kt:.3f} sd {ks:.3f} #{kr}  J {jt:.3f} sd {js:.3f} #{jr}  gap {kt - jt:.3f}\n"
            f"first weekend: {moved} players moved >0.1, mean |d| {mean_moved:.4f}\n"
            f"jumpers >0.5 (2017-23): {n_jump}, excess fall by game 10 {over:.3f}, max step {max_step:.2f}\n"
            f"early error vs game 30 (n {n_early}): game 5 {e5:.3f}, game 10 {e10:.3f}\n"
            + "\n".join(lines))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--bin", required=True)
    ap.add_argument("--jobs", type=int, default=1)
    ap.add_argument("pairs", nargs="+", help="name overrides name overrides ...")
    a = ap.parse_args()
    exps = [(a.pairs[i], a.pairs[i + 1].split()) for i in range(0, len(a.pairs), 2)]
    with cf.ThreadPoolExecutor(a.jobs) as ex:
        for res in ex.map(lambda e: run(a, *e), exps):
            print(res, flush=True)


if __name__ == "__main__":
    main()
