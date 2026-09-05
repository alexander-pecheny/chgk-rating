import datetime
import functools
from dataclasses import dataclass

_open = functools.partial(open, encoding="utf8")

API = "https://api.rating.chgk.net"
RESULTS_DIR = "tournament_results"
INFO_DIR = "tournaments"
APPEALS_DIR = "tournament_appeals"


def parse_date(str_):
    str_ = str_.split("T")[0]
    return datetime.datetime.strptime(str_, "%Y-%m-%d").date()


@dataclass
class TournamentDataPoint:
    id: int
    name: str
    teams: int
    controversials: int
    controversials_accept_rate: float
    appeals: int
    appeals_accept_rate: float
    editors: str
    game_jury: str
    appeal_jury: str
