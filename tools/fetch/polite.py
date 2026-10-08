"""The HTTP client the fetchers share: one request at a time with a gap between
them, a few retries with a growing pause, and a 404 read as "not there"."""

import sys
import time

import httpx

UA = "chgk-rating-research/1.0"
TRIES = 4
BACKOFF = 2
NOT_FOUND = 404


class Polite:
    def __init__(self, gap, timeout):
        self.client = httpx.Client(headers={"User-Agent": UA, "Accept": "application/json"}, timeout=timeout)
        self.gap, self.last = gap, 0.0

    def wait(self):
        left = self.gap - (time.monotonic() - self.last)
        if left > 0:
            time.sleep(left)
        self.last = time.monotonic()

    def get(self, url, missing=None, label=None):
        """The JSON at `url`, `missing` on a 404, None once every try has failed."""
        for attempt in range(TRIES):
            self.wait()
            try:
                return self.fetch(url, missing)
            except (httpx.HTTPError, ValueError) as e:
                if attempt == TRIES - 1:
                    print(f"FAILED {label or url}: {e}", file=sys.stderr, flush=True)
                    return None
                time.sleep(BACKOFF * (attempt + 1))
        return None

    def fetch(self, url, missing):
        r = self.client.get(url)
        if r.status_code == NOT_FOUND:
            return missing
        r.raise_for_status()
        return r.json()
