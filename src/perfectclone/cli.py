"""perfectclone command line."""
import argparse
import json
import sys
from pathlib import Path
from urllib.parse import urlparse

from . import __version__
from .edit import localize, rehost, replace
from .serve import serve
from .trackers import strip
from .util import site_dir


def cmd_clone(a):
    # Imported here so the editing commands don't pay for loading Playwright.
    from .crawl import crawl
    from .mirror import mirror

    out = Path(a.out)
    site = out / "site"
    print(f"Crawling {a.url}")
    pages, assets, external, skipped = crawl(
        a.url, max_pages=a.max_pages, respect_robots=not a.ignore_robots,
        extract_dir=out / "extract" if a.extract else None,
    )
    if not pages:
        sys.exit("No pages could be loaded." + (f" First problem: {skipped[0]}" if skipped else ""))
    print(f"Mirroring into {site}")
    saved, fetched, failures = mirror(pages, assets, site)
    localize(site, a.url)
    if a.strip_trackers:
        print("Removing trackers")
        strip(site)
    manifest = {
        "source": a.url, "pages": pages, "assets_fetched": fetched, "failures": failures,
        "skipped": skipped, "external_hosts": external,
    }
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(f"\nDone: {saved} pages, {fetched} assets → {site}")
    if failures:
        print(f"  {len(failures)} URLs failed to download (see {out / 'manifest.json'})")
    if external:
        print(f"  The original also used these hosts (embeds, CDNs, analytics): {', '.join(external)}")
    print(f"\nPreview it:  perfectclone serve {out}")


def cmd_serve(a):
    serve(a.site, a.port)


def cmd_replace(a):
    replace(site_dir(a.site), a.old, a.new, variants=not a.exact_case, whole_word=not a.substring)


def cmd_rehost(a):
    rehost(site_dir(a.site), a.old_origin, a.new_origin)


def cmd_strip(a):
    strip(site_dir(a.site))


def main(argv=None):
    p = argparse.ArgumentParser(
        prog="perfectclone", description="Clone any website 1:1 with a real browser, then edit it safely.")
    p.add_argument("--version", action="version", version=__version__)
    sub = p.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("clone", help="crawl a site and save a working local copy")
    c.add_argument("url")
    c.add_argument("-o", "--out", default=None, help="output folder (default: the site's hostname)")
    c.add_argument("--max-pages", type=int, default=200)
    c.add_argument("--extract", action="store_true", help="also save each page's text, metadata and a full-page screenshot")
    c.add_argument("--keep-trackers", dest="strip_trackers", action="store_false", help="keep the original analytics/ad pixels")
    c.add_argument("--ignore-robots", action="store_true", help="crawl pages robots.txt disallows")
    c.set_defaults(fn=cmd_clone)

    s = sub.add_parser("serve", help="serve a clone locally")
    s.add_argument("site")
    s.add_argument("-p", "--port", type=int, default=8000)
    s.set_defaults(fn=cmd_serve)

    r = sub.add_parser("replace", help="replace text across the clone (skips URLs, Next.js-safe)")
    r.add_argument("site")
    r.add_argument("old")
    r.add_argument("new")
    r.add_argument("--exact-case", action="store_true", help="don't also replace UPPER/lower/Capitalized variants")
    r.add_argument("--substring", action="store_true", help="match inside words too")
    r.set_defaults(fn=cmd_replace)

    h = sub.add_parser("rehost", help="point canonical/og/sitemap URLs at a new origin")
    h.add_argument("site")
    h.add_argument("old_origin")
    h.add_argument("new_origin")
    h.set_defaults(fn=cmd_rehost)

    t = sub.add_parser("strip-trackers", help="remove analytics and ad pixels")
    t.add_argument("site")
    t.set_defaults(fn=cmd_strip)

    a = p.parse_args(argv)
    if a.cmd == "clone":
        if "://" not in a.url:
            a.url = "https://" + a.url
        if not a.out:
            a.out = urlparse(a.url).netloc.removeprefix("www.") or "clone"
    a.fn(a)


if __name__ == "__main__":
    main()
