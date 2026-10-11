# /// script
# requires-python = ">=3.10"
# dependencies = ["httpx"]
# ///
"""Download the tournament pages of the London Elo (https://elo-chgk.uk) and
write what they say into out/elo_site.db, in the shape `tools/ndcg.py` reads.

A page lists, for every team, its place, its score, the expected score the
model gave it and each player's Elo on the morning of the tournament, and for
every question the fitted difficulty and sport. The site publishes no
algorithm for its rating updates, so it cannot be replayed like fun or quest;
its own pages are the forecast. Pages are cached under out/elo_pages/, so a
rerun only parses. Tournaments are the in-person ones `--ratings` dates in the
window; `--cached` parses only the pages already downloaded. The pages are a
check on `ext_elo`, the replay of the rating (see `tools/elo_check.py`):

    uv run tools/fetch/elo_pages.py [--ratings out/ratings.db] [--from 2024-08-30 --to 2026-08-27] [--cached]
"""

import argparse
import html
import pathlib
import re
import sqlite3
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
CACHE = ROOT / "out/elo_pages"
OUT = ROOT / "out/elo_site.db"
URL = "https://elo-chgk.uk/tournaments/{}"
UA = "chgk-rating-research/1.0 (ap@pecheny.me)"
GAP = 1.0
TRIES = 4
TIMEOUT = 60.0

TAGS = re.compile(r"<[^>]+>")
TABLES = re.compile(r"<table[^>]*>(.*?)</table>", re.S)
ROWS = re.compile(r"<tr>(.*?)</tr>", re.S)
CELLS = re.compile(r"<td[^>]*>(.*?)</td>", re.S)
PLAYER = re.compile(r'href="/players/(\d+)"[^>]*>[^<]*</a>\s*—\s*([-\d.]+)\s*(?:<span[^>]*>\s*\(([-+\d.]+)\))?')
TEAM = re.compile(r'href="/teams/(\d+)"')
FRACTION = re.compile(r"([\d.]+)\s*/\s*(\d+)")
NUMBER = re.compile(r"^[-+]?\d+(\.\d+)?$")

SCHEMA = """
create table tournament_metrics (tournament_id integer primary key, date text, type text);
create table team_events (tournament_id integer, team_id integer, forecast real,
    actual_place real, score real, expected real, x real, mu_before real, s_r real);
create table lineups (tournament_id integer, team_id integer, player_id integer, elo real);
create table questions (tournament_id integer, number integer, difficulty real, width real, sport real, taken text);
"""


def text(s):
    return " ".join(html.unescape(TAGS.sub(" ", s)).split())


def page(client, tid):
    """The cached page of tournament `tid`, fetched on a miss; None if it is not there."""
    import httpx

    path = CACHE / f"{tid}.html"
    if path.exists():
        return path.read_text(encoding="utf-8") or None
    for attempt in range(TRIES):
        time.sleep(GAP)
        try:
            r = client.get(URL.format(tid))
            if r.status_code == 404:
                path.write_text("", encoding="utf-8")
                return None
            r.raise_for_status()
            path.write_text(r.text, encoding="utf-8")
            return r.text
        except httpx.HTTPError as e:
            if attempt == TRIES - 1:
                print(f"FAILED {tid}: {e}", file=sys.stderr, flush=True)
                return None
            time.sleep(2 * (attempt + 1))
    return None


def parse(src):
    """The team rows and the question rows of one page."""
    teams, questions = [], []
    for table in TABLES.findall(src):
        head = text(table.split("</thead>")[0]) if "</thead>" in table else ""
        body = table.split("<tbody>")[-1]
        if "Ожид." in head:
            for row in ROWS.findall(body):
                c = CELLS.findall(row)
                team = TEAM.search(row)
                place = FRACTION.search(text(c[0]))
                score = FRACTION.search(text(c[4])) if len(c) > 4 else None
                exp = text(c[5]) if len(c) > 5 else ""
                if not (team and place):
                    continue
                teams.append(dict(
                    team=int(team.group(1)),
                    place=float(place.group(1)),
                    score=float(score.group(1)) if score else None,
                    expected=float(exp) if NUMBER.match(exp) else None,
                    players=[(int(p), float(e)) for p, e, _ in PLAYER.findall(row)],
                    deltas=[float(d) if d else None for _, _, d in PLAYER.findall(row)],
                ))
        elif "Sport" in head and "Width" in head:
            for row in ROWS.findall(body):
                c = [text(x) for x in CELLS.findall(row)]
                if len(c) >= 5 and NUMBER.match(c[1]):
                    questions.append((int(c[0]), float(c[1]), float(c[2]), float(c[3]), c[4]))
    return teams, questions


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ratings", default="out/ratings.db")
    ap.add_argument("--from", dest="frm", default="2024-08-30")
    ap.add_argument("--to", default="2026-08-27")
    ap.add_argument("--cached", action="store_true", help="parse the pages already cached, fetch nothing")
    ap.add_argument("--out", default=str(OUT))
    a = ap.parse_args()
    out = pathlib.Path(a.out)
    CACHE.mkdir(parents=True, exist_ok=True)
    out.parent.mkdir(parents=True, exist_ok=True)
    q = "select tournament_id, date from tournament_metrics where type = 'in-person' and date >= ? and date < ? order by date"
    todo = sqlite3.connect(a.ratings).execute(q, (a.frm, a.to)).fetchall()
    if a.cached:
        todo = [(t, d) for t, d in todo if (CACHE / f"{t}.html").exists()]
    out.unlink(missing_ok=True)
    db = sqlite3.connect(out)
    db.executescript(SCHEMA)
    import httpx

    client = httpx.Client(headers={"User-Agent": UA}, timeout=TIMEOUT, follow_redirects=True)
    got = 0
    for i, (tid, date) in enumerate(todo):
        src = page(client, tid)
        teams, questions = parse(src) if src else ([], [])
        if not teams:
            print(f"{tid} {date}: no results", file=sys.stderr)
            continue
        got += 1
        db.execute("insert into tournament_metrics values (?, ?, 'in-person')", (tid, date))
        for t in teams:
            elos = [e for _, e in t["players"]]
            db.execute("insert into team_events values (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                       (tid, t["team"], t["expected"], t["place"], t["score"], t["expected"],
                        None, sum(elos) / len(elos) if elos else None, None))
            db.executemany("insert into lineups values (?, ?, ?, ?)", [(tid, t["team"], p, e) for p, e in t["players"]])
        db.executemany("insert into questions values (?, ?, ?, ?, ?, ?)", [(tid, *row) for row in questions])
        if (i + 1) % 25 == 0:
            db.commit()
            print(f"{i + 1}/{len(todo)}", file=sys.stderr, flush=True)
    db.commit()
    print(f"{got} of {len(todo)} tournaments into {out}")


if __name__ == "__main__":
    main()
