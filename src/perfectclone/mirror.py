"""Download crawled pages and assets into a folder that serves as a working copy.

Pages are saved as the server's HTML, not the rendered DOM, because that is what the site's
JavaScript hydrates. Assets keep their URL paths so root-relative references resolve unchanged.
"""
import html
import re
from pathlib import Path
from urllib.parse import parse_qs, unquote, urljoin, urlparse

from playwright.sync_api import sync_playwright

from .flight import has_flight
from .util import UA, host_of

REF = re.compile(
    r"""(?:src|poster|data-src)=["']([^"']+)["']"""
    r"""|url\(\s*["']?([^"')]+?)["']?\s*\)"""
    r"""|(?:srcset|srcSet|imagesrcset|data-srcset)=["']([^"']+)["']"""
    r"""|href=["']([^"']+)["']"""
)
# Pages link to each other with href too; those are left to the crawler so --max-pages and
# robots.txt still apply. From an href, only files that are clearly assets are mirrored.
ASSET_HREF = re.compile(
    r"\.(css|js|mjs|ico|png|jpe?g|gif|svg|webp|avif|woff2?|ttf|otf|json|webmanifest|xml|pdf|mp4|webm)(\?|$)", re.I)
# Lazy chunks and other files loaded at runtime appear only as string literals inside scripts.
# Bundlers leave node_modules source paths in strings too; those aren't served.
CHUNK = re.compile(
    r"""["'`](?:\./)?(/?(?!node_modules/)[\w-]+/[^"'`\s?]+?"""
    r"""\.(?:js|mjs|css|woff2?|ttf|otf|png|jpe?g|webp|avif|svg|gif|mp4|webm|json|wasm|riv|bin|glb|gltf)"""
    r"""(?:\?[^"'`\s]*)?)["'`]""")
TEXT_TYPES = (".html", ".css", ".js", ".mjs", ".svg", ".json", ".xml", ".txt", ".webmanifest")
# Files a site often has that no page references directly (service workers are registered
# from script, browsers request icons and manifests on their own).
EXTRAS = ("/sw.js", "/service-worker.js", "/manifest.json", "/site.webmanifest", "/favicon.ico",
          "/apple-touch-icon.png", "/robots.txt")
# String literals that look like paths but are templates ("/static/${id}.js") or fragments.
BAD_URL = re.compile(r"[\s{}<>`$\\^|()*#]|%23")


def contained(site, out):
    """out, if it stays inside site once resolved; percent-encoded ../ must not escape."""
    out = out.resolve()
    return out if out.is_relative_to(site.resolve()) else None


def page_path(site, url):
    path = unquote(urlparse(url).path)
    if Path(path).suffix in (".html", ".htm"):
        return contained(site, site / path.lstrip("/"))
    return contained(site, site / path.strip("/") / "index.html")


def asset_path(site, url):
    path = unquote(urlparse(url).path)
    if path.endswith("/"):
        path += "index.html"
    return contained(site, site / path.lstrip("/"))


def refs(text, base, host):
    """Same-site URLs referenced from HTML, CSS or JS."""
    out = set()
    for m in REF.finditer(text):
        val = html.unescape(next(g for g in m.groups() if g)).strip().strip("\"'")
        if val.startswith("data:"):
            continue
        if m.group(4) and not ASSET_HREF.search(urlparse(val).path + ("?" if urlparse(val).query else "")):
            continue
        # Only srcset is a comma list; a data: URL or query string may contain commas too.
        for part in val.split(", ") if m.group(3) else [val]:
            u = part.strip().split(" ")[0]
            if u and not u.startswith(("data:", "#", "mailto:", "tel:", "javascript:", "blob:")):
                out.add(urljoin(base, u))
    for m in CHUNK.finditer(text):
        out.add(urljoin(base, "/" + m.group(1).lstrip("/")))
    # /_next/image responses are keyed by query string and often refused to direct requests, so
    # mirror the original image instead; `serve` maps the optimizer URL back to it.
    for u in [u for u in out if urlparse(u).path == "/_next/image"]:
        out.discard(u)
        src = parse_qs(urlparse(u).query).get("url", [""])[0]
        if src.startswith("/"):
            out.add(urljoin(base, src))
    return {u for u in out if u.startswith("http") and host_of(u) == host and not BAD_URL.search(u)}


def mirror(pages, assets, out_dir, log=print):
    """Fetch pages and assets into out_dir. Returns (saved_pages, urls_tried, failures)."""
    site = Path(out_dir)
    site.mkdir(parents=True, exist_ok=True)
    host = host_of(pages[0]["url"])
    page_paths = {urlparse(p["url"]).path.rstrip("/") or "/" for p in pages}
    todo, done, failures = set(assets), set(), []
    saved_pages = 0

    with sync_playwright() as pw:
        req = pw.request.new_context(user_agent=UA)

        def fetch(url, headers=None, quiet=False):
            try:
                r = req.get(url, headers=headers, timeout=30000)
            except Exception as e:
                if not quiet:
                    failures.append({"url": url, "error": str(e).splitlines()[0]})
                return None
            if not r.ok:
                if not quiet:
                    failures.append({"url": url, "status": r.status})
                return None
            return r

        for p in pages:
            r = fetch(p["url"])
            if r is None:
                continue
            body = r.text()
            out = page_path(site, p["url"])
            if out is None:
                failures.append({"url": p["url"], "error": "path escapes the output folder"})
                continue
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(body)
            saved_pages += 1
            todo |= refs(body, p["url"], host)
            if has_flight(body) and out.name == "index.html":
                # Next.js navigates between pages by fetching this payload (?_rsc=...) instead
                # of HTML; without it every client-side link 404s and falls back to a reload.
                rsc = fetch(p["url"], headers={"RSC": "1"}, quiet=True)
                if rsc and "text/x-component" in rsc.headers.get("content-type", ""):
                    out.with_name("index.rsc").write_bytes(rsc.body())

        for path in EXTRAS:
            r = fetch(urljoin(pages[0]["url"], path), quiet=True)
            out = asset_path(site, urljoin(pages[0]["url"], path))
            if r and out and not out.exists() and "text/html" not in r.headers.get("content-type", ""):
                out.parent.mkdir(parents=True, exist_ok=True)
                out.write_bytes(r.body())
                done.add(urljoin(pages[0]["url"], path))

        while todo:
            url = todo.pop()
            parsed = urlparse(url)
            key = parsed._replace(fragment="").geturl()
            if key in done or (parsed.path.rstrip("/") or "/") in page_paths:
                continue
            done.add(key)
            if "/cdn-cgi/" in parsed.path:                    # Cloudflare's own endpoints
                continue
            r = fetch(key)
            if r is None:
                continue
            if "text/html" in r.headers.get("content-type", "") and not Path(parsed.path).suffix:
                continue                                       # a page the crawl didn't reach
            out = asset_path(site, key)
            if out is None:
                failures.append({"url": key, "error": "path escapes the output folder"})
                continue
            if out.is_dir():
                out = out / "index"
            out.parent.mkdir(parents=True, exist_ok=True)
            body = r.body()
            out.write_bytes(body)
            if out.suffix in TEXT_TYPES:
                todo |= refs(body.decode("utf-8", "ignore"), key, host) - done
        req.dispose()

    log(f"  mirrored {saved_pages} pages, {len(done) - len(failures)} assets")
    return saved_pages, len(done), failures
