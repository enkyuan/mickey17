"""Edit a cloned site in place without breaking it.

Text replacement leaves URLs alone (renaming "Acme" must not break https://acme.com/logo.png or
/acme/app.js) and goes through the flight payload on Next.js pages so the server HTML and the
React payload stay identical.
"""
import re

from .flight import edit_flight, has_flight, map_rows, outside_flight
from .util import text_files

# URLs, root-relative asset paths and mailto/tel targets are never rewritten.
PROTECTED = re.compile(r"""(https?://[^\s"'\\<>)]+|(?<=["'(=])/[^\s"'\\<>)]*|mailto:[^\s"'\\<>]+)""")


def case_variants(old, new):
    """Acme → Globex also maps ACME → GLOBEX and acme → globex."""
    pairs = [(old, new)]
    for f in (str.upper, str.lower, str.capitalize):
        if f(old) != old and (f(old), f(new)) not in pairs:
            pairs.append((f(old), f(new)))
    return pairs


def replacer(pairs, whole_word=True):
    rx = [(re.compile(rf"\b{re.escape(o)}\b" if whole_word else re.escape(o)), n) for o, n in pairs]

    def swap(text):
        parts = PROTECTED.split(text)
        for i in range(0, len(parts), 2):                      # odd indices are protected
            for r, n in rx:
                parts[i] = r.sub(n, parts[i])
        return "".join(parts)
    return swap


def apply(site, fn, html_fn=None, log=print):
    """Run fn over every text file. Flight data (in HTML, or saved .rsc navigation payloads) is
    edited row by row so length-prefixed rows stay valid; html_fn, if given, is used for the HTML
    outside the payload instead of fn."""
    changed = 0
    for p in text_files(site):
        text = p.read_text(errors="surrogateescape")
        suffix = p.suffix.lower()
        if suffix == ".rsc":
            new = map_rows(text, fn)
        elif suffix in (".html", ".htm"):
            new = outside_flight(edit_flight(text, fn), html_fn or fn) if has_flight(text) else (html_fn or fn)(text)
        else:
            new = fn(text)
        if new != text:
            p.write_text(new, errors="surrogateescape")
            changed += 1
    log(f"  changed {changed} files")
    return changed


def replace(site, old, new, variants=True, whole_word=True, log=print):
    pairs = case_variants(old, new) if variants else [(old, new)]
    return apply(site, replacer(pairs, whole_word), log=log)


def origin_variants(origin):
    m = re.match(r"https?://(?:www\.)?([^/]+)", origin.rstrip("/"))
    host = m.group(1) if m else origin
    return re.compile(rf"https?://(?:www\.)?{re.escape(host)}(?![\w.-])")


def rehost(site, old_origin, new_origin, log=print):
    """Point absolute URLs at a new origin (canonical, og:url, JSON-LD, sitemap)."""
    rx = origin_variants(old_origin)
    new_origin = new_origin.rstrip("/")
    return apply(site, lambda t: rx.sub(new_origin, t), log=log)


ASSET_TAG = re.compile(r"<(?:img|script|source|video|audio|track|link|image|use)\b[^>]*>", re.I)
# Module scripts need CORS like fonts do, and import maps list them by absolute URL.
STYLE_BLOCK = re.compile(r"(<style\b[^>]*>|<script\b[^>]*type=[\"']?importmap[^>]*>)(.*?)(</style>|</script>)", re.S | re.I)
DOC_LINK = re.compile(r"""rel=["']?(?:canonical|alternate|shortlink|prev|next|author|me)\b""", re.I)


def localize(site, origin, log=print):
    """Make the site's own absolute asset URLs root-relative, so the clone loads its local copies.

    Browsers block cross-origin fonts without CORS headers, so a stylesheet or @font-face still
    pointing at the original domain renders in a fallback font. Page links and metadata keep
    their absolute URLs; `rehost` is for those.
    """
    rx = re.compile(origin_variants(origin).pattern + r"(?=/)")

    def tag(m):
        t = m.group(0)
        return t if t[:5].lower() == "<link" and DOC_LINK.search(t) else rx.sub("", t)

    def html(t):
        t = ASSET_TAG.sub(tag, t)
        return STYLE_BLOCK.sub(lambda m: m.group(1) + rx.sub("", m.group(2)) + m.group(3), t)

    changed = 0
    for p in text_files(site):
        suffix = p.suffix.lower()
        if suffix not in (".css", ".html", ".htm"):
            continue
        text = p.read_text(errors="surrogateescape")
        new = rx.sub("", text) if suffix == ".css" else outside_flight(text, html)
        if new != text:
            p.write_text(new, errors="surrogateescape")
            changed += 1
    return changed
