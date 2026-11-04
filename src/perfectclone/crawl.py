"""Find a site's pages, and every asset they load, by rendering them in Chromium."""
import json
import re
import subprocess
import sys
from collections import deque
from pathlib import Path
from urllib import robotparser
from urllib.parse import urldefrag, urljoin, urlparse

from playwright.sync_api import sync_playwright

from .util import UA, host_of

NOT_PAGE = re.compile(
    r"\.(png|jpe?g|gif|svg|webp|avif|ico|pdf|zip|mp4|webm|mp3|woff2?|ttf|otf|css|js|mjs|json|xml|txt|rss)$", re.I)

EXTRACT_JS = """
() => {
  const abs = u => { try { return new URL(u, location.href).href } catch { return u } };
  const meta = {};
  document.querySelectorAll('meta').forEach(m => {
    const k = m.getAttribute('name') || m.getAttribute('property') || m.getAttribute('http-equiv');
    if (k) meta[k] = m.getAttribute('content');
  });
  return {
    title: document.title,
    lang: document.documentElement.lang,
    canonical: document.querySelector('link[rel=canonical]')?.href || null,
    meta,
    headings: [...document.querySelectorAll('h1,h2,h3,h4,h5,h6')].map(h => ({tag: h.tagName, text: h.innerText.trim()})),
    links: [...document.querySelectorAll('a[href]')].map(a => ({text: a.innerText.trim(), href: abs(a.getAttribute('href'))})),
    images: [...document.querySelectorAll('img')].map(i => ({src: i.currentSrc || abs(i.getAttribute('src') || ''), alt: i.alt})),
    forms: [...document.querySelectorAll('form')].map(f => ({action: f.action, method: f.method,
      fields: [...f.querySelectorAll('input,select,textarea')].map(i => ({tag: i.tagName, type: i.type, name: i.name}))})),
    jsonld: [...document.querySelectorAll('script[type="application/ld+json"]')].map(s => s.textContent),
    text: document.body ? document.body.innerText : '',
  };
}
"""

# Scrolling triggers lazy-loaded images, chunks and fonts so they show up in the response log.
SCROLL_JS = """async () => {
  for (let y = 0; y < document.body.scrollHeight; y += 600) { window.scrollTo(0, y); await new Promise(r => setTimeout(r, 120)); }
  window.scrollTo(0, 0);
}"""


def page_key(url, host):
    """Normalized URL if it's a same-site page worth visiting, else None."""
    url = urldefrag(url)[0]
    p = urlparse(url)
    if p.scheme not in ("http", "https") or host_of(url) != host or NOT_PAGE.search(p.path):
        return None
    return p._replace(path=p.path.rstrip("/") or "/", query="").geturl()


def slug(url):
    return re.sub(r"[^a-zA-Z0-9]+", "_", urlparse(url).path.strip("/"))[:100] or "index"


def launch(pw, log=print):
    try:
        return pw.chromium.launch()
    except Exception as e:
        if "Executable doesn't exist" not in str(e):
            raise
    log("First run: downloading Chromium for Playwright (one time, about 150 MB)...")
    subprocess.run([sys.executable, "-m", "playwright", "install", "chromium"], check=True)
    return pw.chromium.launch()


def crawl(start, max_pages=200, respect_robots=True, extract_dir=None, log=print):
    """Visit same-site pages breadth-first. Returns (pages, asset_urls, external_hosts, skipped)."""
    host = host_of(start)
    first = page_key(start, host)
    queue, seen = deque([first]), {first}
    pages, assets, external, skipped = [], set(), set(), []
    if extract_dir:
        Path(extract_dir).mkdir(parents=True, exist_ok=True)

    with sync_playwright() as pw:
        browser = launch(pw, log)
        ctx = browser.new_context(viewport={"width": 1440, "height": 900}, user_agent=UA)

        def get(url):
            try:
                r = ctx.request.get(url, timeout=20000)
                return r if r.ok else None
            except Exception:
                return None

        robots = robotparser.RobotFileParser()
        r = get(urljoin(start, "/robots.txt"))
        robots.parse(r.text().splitlines() if r else [])
        sitemaps = re.findall(r"(?im)^sitemap:\s*(\S+)", r.text()) if r else []
        for sm in sitemaps or [urljoin(start, "/sitemap.xml")]:
            s = get(urljoin(start, sm))
            if s:
                for loc in re.findall(r"<loc>\s*(.*?)\s*</loc>", s.text()):
                    u = page_key(loc, host)
                    if u and u not in seen:
                        seen.add(u)
                        queue.append(u)

        page = ctx.new_page()

        def on_response(resp):
            u = resp.url
            if not u.startswith("http"):
                return
            if host_of(u) != host:
                external.add(urlparse(u).netloc)
            elif resp.request.resource_type != "document":
                assets.add(u)

        page.on("response", on_response)

        while queue and len(pages) < max_pages:
            url = queue.popleft()
            if respect_robots and not robots.can_fetch("perfectclone", url):
                skipped.append({"url": url, "reason": "robots.txt"})
                continue
            log(f"  [{len(pages) + 1}] {url}")
            try:
                resp = page.goto(url, wait_until="domcontentloaded", timeout=45000)
                # A slow third-party image, a video or a polling connection can hold off "load"
                # and "networkidle" forever, so each gets a bounded wait.
                for state, ms in (("load", 15000), ("networkidle", 8000)):
                    try:
                        page.wait_for_load_state(state, timeout=ms)
                    except Exception:
                        pass
                page.evaluate(SCROLL_JS)
                page.wait_for_timeout(400)
                links = page.eval_on_selector_all("a[href]", "as => as.map(a => a.href)")
            except Exception as e:
                skipped.append({"url": url, "reason": str(e).splitlines()[0]})
                continue
            status = resp.status if resp else None
            ctype = resp.headers.get("content-type", "") if resp else ""
            if (status and status >= 400) or "html" not in ctype:
                skipped.append({"url": url, "reason": f"HTTP {status} {ctype}".strip()})
                continue
            pages.append({"url": url, "final_url": page.url, "status": status, "title": page.title()})

            if extract_dir:
                data = page.evaluate(EXTRACT_JS)
                name = slug(url)
                Path(extract_dir, f"{name}.json").write_text(json.dumps(data, indent=2, ensure_ascii=False))
                Path(extract_dir, f"{name}.txt").write_text(data["text"])
                page.screenshot(path=str(Path(extract_dir, f"{name}.png")), full_page=True)

            for href in links:
                u = page_key(href, host)
                if u and u not in seen:
                    seen.add(u)
                    queue.append(u)

        browser.close()
    return pages, assets, sorted(external), skipped
