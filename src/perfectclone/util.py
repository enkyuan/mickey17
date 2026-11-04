"""Helpers shared by the crawler, mirror, editors and server."""
from pathlib import Path
from urllib.parse import urlparse

UA = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) "
      "Chrome/130.0 Safari/537.36 perfectclone")
TEXT_TYPES = {".html", ".htm", ".js", ".mjs", ".css", ".json", ".xml", ".txt", ".svg", ".webmanifest", ".rsc"}


def host_of(url):
    return urlparse(url).netloc.lower().removeprefix("www.")


def text_files(site):
    for p in Path(site).rglob("*"):
        if p.is_file() and p.suffix.lower() in TEXT_TYPES:
            yield p


def site_dir(path):
    """Accept a clone folder (with site/ inside) or the site folder itself."""
    p = Path(path)
    if not (p / "index.html").exists() and (p / "site" / "index.html").exists():
        p = p / "site"
    return p
