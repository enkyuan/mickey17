"""Remove the original owner's analytics and ad pixels, which would otherwise report a clone's
visitors to their accounts. Next.js next/script trackers exist only in the flight payload.
"""
import re

from .flight import edit_json_rows, has_flight, json_row_editor, map_rows, outside_flight
from .util import text_files

SIGNATURES = re.compile(
    r"googletagmanager\.com|google-analytics\.com|\bgtag\(|\bga\(['\"]|doubleclick\.net"
    r"|connect\.facebook\.net|facebook\.com/tr|\bfbq\("
    r"|_linkedin_partner_id|snap\.licdn\.com|px\.ads\.linkedin\.com"
    r"|static\.hotjar\.com|\bhj\(|clarity\.ms"
    r"|cdn\.segment\.com|analytics\.load\("
    r"|reb2b|ddwl4m2hdecbv\.cloudfront\.net"
    r"|posthog|plausible\.io|cdn\.mxpnl\.com|mixpanel"
    r"|js\.hs-scripts\.com|js\.hs-analytics\.net|hs-banner"
    r"|analytics\.tiktok\.com|\bttq\.|bat\.bing\.com|\buetq\b"
    r"|static\.ads-twitter\.com|\btwq\(|redditstatic\.com/ads|\brdt\("
    r"|static\.cloudflareinsights\.com|data-cf-beacon"
    r"|cdn\.amplitude\.com|heapanalytics|fullstory\.com|\bFS\.identify|cdn\.lr-ingest|logrocket",
    re.I,
)

# Script loaders that bundles inject at runtime (e.g. a GoogleTagManager React component).
# Pointing them at a data: URL holding a JS comment lets the injecting code run untouched
# while nothing loads. Beacon/collect endpoints are left alone: rewriting those can throw.
LOADER = re.compile(
    r"(?:https?:)?//(?:www\.)?(?:googletagmanager\.com/(?:gtm|gtag/js)|connect\.facebook\.net/[\w_]+/fbevents\.js"
    r"|snap\.licdn\.com/li\.lms-analytics/insight\.min\.js|static\.hotjar\.com/c/hotjar-|www\.clarity\.ms/tag/"
    r"|cdn\.segment\.com/analytics\.js|js\.hs-scripts\.com/|static\.cloudflareinsights\.com/beacon"
    r"|ddwl4m2hdecbv\.cloudfront\.net/b/|plausible\.io/js/|cdn\.amplitude\.com/|cdn\.mxpnl\.com/"
    r"|analytics\.tiktok\.com/i18n/pixel|static\.ads-twitter\.com/uwt\.js|bat\.bing\.com/bat\.js)"
)
SCRIPT = re.compile(r"<script\b[^>]*>.*?</script>", re.S | re.I)
NOSCRIPT = re.compile(r"<noscript\b[^>]*>.*?</noscript>", re.S | re.I)
HINT = re.compile(r"<link\b[^>]*rel=[\"']?(?:preload|preconnect|dns-prefetch|prefetch)[^>]*>", re.I)


def _strip_html(text):
    def drop(m):
        tag = m.group(0)
        if "__next_f" in tag or "application/ld+json" in tag:
            return tag
        return "" if SIGNATURES.search(tag) else tag

    def empty(m):
        # The element may be part of a React tree, so removing it breaks hydration. React never
        # hydrates noscript children, so emptying it is safe and stops the no-JS pixel.
        tag = m.group(0)
        return tag[: tag.index(">") + 1] + "</noscript>" if SIGNATURES.search(tag) else tag

    text = SCRIPT.sub(drop, text)
    text = NOSCRIPT.sub(empty, text)
    return HINT.sub(drop, text)


def _is_tracker_element(node):
    """A React element ["$", type, key, props] for a script whose src/id/body is a tracker."""
    if not (isinstance(node, list) and len(node) == 4 and node[0] == "$" and isinstance(node[3], dict)):
        return False
    props = node[3]
    is_script = node[1] == "script" or "strategy" in props            # next/script carries strategy
    if not is_script:
        return False
    body = " ".join(str(props.get(k, "")) for k in ("src", "id", "children"))
    body += str((props.get("dangerouslySetInnerHTML") or {}).get("__html", ""))
    return bool(SIGNATURES.search(body))


def _drop(node):
    return None if _is_tracker_element(node) else node


def strip(site, log=print):
    changed = 0
    for p in text_files(site):
        text = p.read_text(errors="surrogateescape")
        suffix = p.suffix.lower()
        if suffix in (".js", ".mjs"):
            new = LOADER.sub(lambda m: "data:text/javascript,//" + m.group(0).split("//", 1)[1], text)
        elif suffix == ".rsc":
            new = map_rows(text, json_row_editor(_drop))
        elif suffix in (".html", ".htm"):
            new = outside_flight(edit_json_rows(text, _drop), _strip_html) if has_flight(text) else _strip_html(text)
            changed += new != text
        else:
            continue
        if new != text:
            p.write_text(new, errors="surrogateescape")
    if changed:
        log(f"  removed trackers from {changed} pages")
    return changed
