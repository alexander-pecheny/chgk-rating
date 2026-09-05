"""Download LiterataFix and LiterataMath from the fonts-fixing repo into fonts/."""
import json
import pathlib
import time
import urllib.request

TOKEN = pathlib.Path.home().joinpath(".config/forgejo/token").read_text().strip()
API = "https://code.pecheny.me/api/v1/repos/pecheny/fonts-fixing/contents/"
DEST = pathlib.Path(__file__).resolve().parent.parent / "fonts"


def get(url):
    req = urllib.request.Request(url, headers={"Authorization": f"token {TOKEN}"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


for family in ("LiterataFix", "LiterataMath"):
    out = DEST / family
    out.mkdir(parents=True, exist_ok=True)
    for entry in json.loads(get(API + "fonts/" + family)):
        if entry["type"] != "file":
            continue
        target = out / entry["name"]
        if target.exists() and target.stat().st_size == entry["size"]:
            continue
        target.write_bytes(get(entry["download_url"]))
        print(f"{family}/{entry['name']}\t{entry['size']}")
        time.sleep(0.5)
