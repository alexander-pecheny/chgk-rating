#!/usr/bin/env python
# -*- coding: utf-8 -*-
import os
import time

import orjson as json
import requests

from .util import _open

API = "https://api.rating.chgk.net"
SLEEP = 1


def main():

    if not os.path.isdir("tournaments"):
        os.mkdir("tournaments")

    if os.path.isfile("tournaments/releases.json"):
        with _open("tournaments/releases.json", "r") as f:
            releases = json.loads(f.read() or "[]")
    else:
        releases = []
    release_ids = {x["id"] for x in releases}
    ITEMS_PER_PAGE = 100
    page = len(releases) // ITEMS_PER_PAGE + 1
    req = requests.get(f"{API}/releases.json?itemsPerPage={ITEMS_PER_PAGE}&page={page}")
    res = req.json()
    time.sleep(SLEEP)
    for x in res:
        if x["id"] not in release_ids:
            releases.append(x)
            release_ids.add(x["id"])
    while len(res) == 100:
        page += 1
        print(f"processing page {page}...")
        req = requests.get(f"{API}/releases.json?itemsPerPage={ITEMS_PER_PAGE}&page={page}")
        res = req.json()
        for x in res:
            if x["id"] not in release_ids:
                releases.append(x)
                release_ids.add(x["id"])
        time.sleep(SLEEP)
    with open("tournaments/releases.json", "wb") as f:
        f.write(json.dumps(releases))


if __name__ == "__main__":
    main()
