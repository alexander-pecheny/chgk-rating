"""Rule-based classes of every tournament from `tournament_features.py`:
prestige (major, strong, regular), age (school, student, novice, open),
online, and pack difficulty (hard, middle, easy).

    uv run tools/tournament_class.py out/prestige/features.tsv > out/prestige/classes.tsv

Prestige reads who came, not the name: `strong` counts the previous year's
top-100 teams in the field and `elite` its top 30. An in-person major also
needs at least half its teams from out of town; a sync tournament is ranked
within its year. International marks an in-person field from many countries
that mostly travelled, such as a world or emigrant championship.
"""

import csv
import re
import sys

import lab

OPTIONS = [("features", dict())]
SCHOOL = re.compile(r"школ|ШЧР|детск|юниор|U-?1[0-9]\b|Школьнае", re.I)
STUDENT = re.compile(r"студ|СтудЧР|СЧР|МЧР|вуз|межфакультет|Студэнц|Студгульн|Студиг", re.I)
ONLINE = re.compile(r"^Онлайн|\(онлайн\)|онлайн-", re.I)
PRIOR_B = 3.82
# Who counts as a newcomer, and how many make a novice field.
NEW_YEARS_SHARE = 0.6
# A youth-named sync whose players have played this long is adults on a youth pack.
ADULT_CAREER = 5.0
# In-person: counts of last year's top-100 (`strong`) and top-30 (`elite`) teams.
IN_PERSON = dict(major_strong=15, major_elite=6, major_travel=0.5, strong_strong=5, strong_elite=2)
# Sync fields grew with the format, so a sync tournament is ranked within its
# year by `strong` and then `elite`.
SYNC_MAJOR, SYNC_STRONG = 0.015, 0.10
INTERNATIONAL = dict(countries=8, travel=0.8, n=15)
HARD_B, EASY_B = 2.6, 1.8


def num(r, k):
    return float(r[k]) if r[k] not in ("", None) else None


def prestige(r, sync_rank):
    strong, elite, travel = num(r, "strong") or 0, num(r, "elite") or 0, num(r, "travel")
    if r["fmt"] == "sync":
        q = sync_rank[r["id"]]
        return "major" if q < SYNC_MAJOR else "strong" if q < SYNC_STRONG else "regular"
    rule = IN_PERSON
    if (strong >= rule["major_strong"] or elite >= rule["major_elite"]) and (travel or 0) >= rule["major_travel"]:
        return "major"
    if strong >= rule["strong_strong"] or elite >= rule["strong_elite"]:
        return "strong"
    return "regular"


def international(r):
    rule = INTERNATIONAL
    return int(r["fmt"] == "in-person" and (num(r, "countries") or 0) >= rule["countries"] and (num(r, "travel") or 0) >= rule["travel"] and int(r["n"]) >= rule["n"])


def sync_ranks(rows):
    """Each sync tournament's share of its year's sync fields that rank above it."""
    years = {}
    for r in rows:
        if r["fmt"] == "sync":
            years.setdefault(r["date"][:4], []).append(r)
    out = {}
    for rs in years.values():
        rs.sort(key=lambda r: (-(num(r, "strong") or 0), -(num(r, "elite") or 0)))
        for i, r in enumerate(rs):
            out[r["id"]] = i / len(rs)
    return out


def age(r):
    career, new = num(r, "career_med"), num(r, "new3")
    adults = r["fmt"] == "sync" and career is not None and career >= ADULT_CAREER
    if SCHOOL.search(r["name"]) and not adults:
        return "school"
    if STUDENT.search(r["name"]) and not adults:
        return "student"
    if new is not None and new >= NEW_YEARS_SHARE:
        return "novice"
    return "open"


def difficulty(r):
    b = num(r, "b")
    if b is None or abs(b - PRIOR_B) < 1e-3:
        return ""
    return "hard" if b >= HARD_B else "easy" if b < EASY_B else "middle"


def main(args):
    rows = list(csv.DictReader(open(args.features), delimiter="\t"))
    out = csv.writer(sys.stdout, delimiter="\t", lineterminator="\n")
    out.writerow(["id", "date", "fmt", "prestige", "international", "age", "online", "difficulty", "name"])
    ranks = sync_ranks(rows)
    for r in rows:
        online = int(bool(ONLINE.search(r["name"])) or ("2020-04-01" <= r["date"] < "2021-09-01" and r["fmt"] == "in-person" and (num(r, "travel") or 0) > 0.95))
        out.writerow([r["id"], r["date"], r["fmt"], "regular" if online else prestige(r, ranks), 0 if online else international(r), age(r), online, difficulty(r), r["name"]])


if __name__ == "__main__":
    main(lab.cli(__doc__, OPTIONS))
