"""Install the Zola binary into ~/.local/bin."""
import io
import pathlib
import tarfile
import urllib.request

URL = "https://github.com/getzola/zola/releases/download/v0.23.4/zola-v0.23.4-x86_64-unknown-linux-gnu.tar.gz"
DEST = pathlib.Path.home() / ".local/bin"

with urllib.request.urlopen(URL, timeout=120) as r:
    data = r.read()
with tarfile.open(fileobj=io.BytesIO(data)) as tar:
    member = tar.getmember("zola")
    DEST.mkdir(parents=True, exist_ok=True)
    (DEST / "zola").write_bytes(tar.extractfile(member).read())
(DEST / "zola").chmod(0o755)
print("installed", DEST / "zola")
