"""Compatibility check against real sites: clone, serve, and compare each home page with the live
one. Hits the network, so it isn't part of `pytest`. Needs the dev install (`pip install -e ".[dev]"`).

    python scripts/compat.py                 # one site per framework
    python scripts/compat.py --popular       # well-known product sites
    python scripts/compat.py huly.io astro.build
"""
import io
import json
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from PIL import Image, ImageChops
from playwright.sync_api import sync_playwright

from perfectclone.trackers import SIGNATURES

SITES = {
    "huly.io": "Next.js",
    "nextjs.org": "Next.js",
    "astro.build": "Astro",
    "svelte.dev": "SvelteKit",
    "vitepress.dev": "VitePress (Vue)",
    "docusaurus.io": "Docusaurus (React)",
    "webflow.com": "Webflow",
    "www.framer.com": "Framer",
    "wordpress.org/news": "WordPress",
    "www.paulgraham.com": "Hand-written HTML",
    "example.com": "Static",
}
POPULAR = {
    "linear.app": "Next.js", "vercel.com": "Next.js", "stripe.com": "Custom", "www.raycast.com": "Next.js",
    "cursor.com": "Next.js", "resend.com": "Next.js", "supabase.com": "Next.js", "www.anthropic.com": "Next.js",
    "openai.com": "Next.js", "arc.net": "Framer", "clerk.com": "Next.js", "ui.shadcn.com": "Next.js",
    "tailwindcss.com": "Next.js", "www.perplexity.ai": "Next.js",
}
HYDRATION = re.compile(r"hydrat|Minified React error #(418|423|425)", re.I)
# Animations never land on the same frame twice; freeze them for the comparison. Videos stay
# visible but are paused on the same frame. Third-party iframes vary per load, so they're hidden.
FREEZE = ("*,*::before,*::after{animation-play-state:paused!important;animation-delay:-1s!important;"
          "transition:none!important;caret-color:transparent!important}iframe{visibility:hidden!important}")
PAUSE_VIDEOS = """async () => {
  await Promise.all([...document.querySelectorAll('video')].map(v => new Promise(done => {
    v.pause();
    if (!v.readyState) return done();
    v.addEventListener('seeked', done, {once: true});
    v.currentTime = Math.min(1, v.duration || 1);
    setTimeout(done, 3000);
  })));
}"""


def snapshot(page, url):
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)[:160]))
    page.on("console", lambda m: m.type == "error" and errors.append(m.text[:160]))
    page.goto(url, wait_until="domcontentloaded", timeout=60000)
    try:
        page.wait_for_load_state("load", timeout=20000)
    except Exception:
        pass
    page.wait_for_timeout(3000)
    page.add_style_tag(content=FREEZE)
    page.evaluate(PAUSE_VIDEOS)
    page.wait_for_timeout(500)
    return Image.open(io.BytesIO(page.screenshot())).convert("RGB"), errors


def identical(a, b):
    if a.size != b.size:
        return 0.0
    diff = ImageChops.difference(a, b).convert("L")
    changed = sum(1 for v in diff.get_flattened_data() if v > 24)
    return 100 * (1 - changed / (a.width * a.height))


def check(site, work, port, browser):
    out = work / site.replace("/", "_")
    start = time.time()
    proc = subprocess.run([sys.executable, "-m", "perfectclone", "clone", site, "-o", str(out), "--max-pages", "3"],
                          capture_output=True, text=True)
    took = time.time() - start
    manifest = out / "manifest.json"
    if proc.returncode or not manifest.exists():
        return {"site": site, "ok": False, "error": (proc.stderr or proc.stdout).strip().splitlines()[-1:]}
    m = json.loads(manifest.read_text())
    server = subprocess.Popen([sys.executable, "-m", "perfectclone", "serve", str(out), "-p", str(port)],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1.5)
    try:
        ctx = browser.new_context(viewport={"width": 1440, "height": 900})
        live_img, live_err = snapshot(ctx.new_page(), "https://" + site)
        clone_page = ctx.new_page()
        missing, trackers = [], set()
        local = f"http://localhost:{port}"

        def on_response(r):
            if r.status == 404 and r.url.startswith(local):
                missing.append(r.url)

        def on_request(r):
            if r.url.startswith("http") and SIGNATURES.search(r.url):
                trackers.add(r.url.split("/")[2])

        clone_page.on("response", on_response)
        clone_page.on("request", on_request)
        path = "/" + site.split("/", 1)[1] if "/" in site else "/"
        clone_img, clone_err = snapshot(clone_page, f"http://localhost:{port}{path}")
        ctx.close()
    finally:
        server.terminate()
    new_errors = [e for e in clone_err if e not in live_err]
    shots = work / "shots"
    shots.mkdir(exist_ok=True)
    live_img.save(shots / f"{site.replace('/', '_')}_live.png")
    clone_img.save(shots / f"{site.replace('/', '_')}_clone.png")
    return {
        "site": site, "ok": True, "seconds": round(took), "pages": len(m["pages"]),
        "assets": m["assets_fetched"], "failed_downloads": len(m["failures"]),
        "identical_pct": round(identical(live_img, clone_img), 2),
        "hydration_errors": [e for e in new_errors if HYDRATION.search(e)],
        "new_errors": new_errors[:5], "clone_404s": missing[:5], "trackers_contacted": sorted(trackers),
    }


def main():
    args = sys.argv[1:]
    if args == ["--popular"]:
        SITES.update(POPULAR)
        args = list(POPULAR)
    sites = args or list(SITES)
    work = Path(tempfile.mkdtemp(prefix="perfectclone-compat-"))
    results = []
    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        for i, site in enumerate(sites):
            print(f"[{i + 1}/{len(sites)}] {site}", flush=True)
            try:
                results.append(check(site, work, 8700 + i, browser))
            except Exception as e:
                results.append({"site": site, "ok": False, "error": [str(e).splitlines()[0]]})
            print("   ", json.dumps(results[-1]), flush=True)
        browser.close()
    (work / "results.json").write_text(json.dumps(results, indent=2))
    header = ("site", 24), ("framework", 20), ("identical", -9), ("hydration", -9), ("new errors", -10), ("404s", -5), \
        ("trackers", -8), ("time", -6)
    print("\n" + " ".join(f"{name:<{w}}" if w > 0 else f"{name:>{-w}}" for name, w in header))
    for r in results:
        if not r["ok"]:
            print(f"{r['site']:24} {SITES.get(r['site'], ''):20} FAILED: {r['error']}")
            continue
        print(f"{r['site']:24} {SITES.get(r['site'], ''):20} {r['identical_pct']:>8}% {len(r['hydration_errors']):>9} "
              f"{len(r['new_errors']):>10} {len(r['clone_404s']):>5} {len(r['trackers_contacted']):>8} {r['seconds']:>5}s")
    print(f"\nScreenshots and results: {work}")


if __name__ == "__main__":
    main()
