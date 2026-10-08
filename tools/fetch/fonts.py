"""Download LiterataFix and LiterataMath from the fonts-fixing repo into fonts/.

They are Literata patched for Russian (stress-mark anchors, kerning, static
instances) and live only in a private Forgejo repository, so this needs a token
with read access in ~/.config/forgejo/token. `report/report.typ` builds with
them:

    uv run tools/fetch/fonts.py
"""

import argparse
import json
import pathlib
import time
import urllib.request

TOKEN_FILE = pathlib.Path.home() / ".config/forgejo/token"
API = "https://code.pecheny.me/api/v1/repos/pecheny/fonts-fixing/contents/"
DEST = pathlib.Path(__file__).resolve().parents[2] / "fonts"
FAMILIES = ("LiterataFix", "LiterataMath")
TIMEOUT = 60
GAP = 0.5


def get(url, token):
    req = urllib.request.Request(url, headers={"Authorization": f"token {token}"})
    with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
        return r.read()


def fetch_family(family, token):
    """Every file of one family that is missing or has the wrong size."""
    out = DEST / family
    out.mkdir(parents=True, exist_ok=True)
    for entry in json.loads(get(API + "fonts/" + family, token)):
        target = out / entry["name"]
        if entry["type"] != "file" or (target.exists() and target.stat().st_size == entry["size"]):
            continue
        target.write_bytes(get(entry["download_url"], token))
        print(f"{family}/{entry['name']}\t{entry['size']}")
        time.sleep(GAP)


def main():
    argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter).parse_args()
    token = TOKEN_FILE.read_text().strip()
    for family in FAMILIES:
        fetch_family(family, token)


if __name__ == "__main__":
    main()
