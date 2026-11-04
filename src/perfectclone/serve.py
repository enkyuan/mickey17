"""Serve a clone locally the way a static host would."""
import functools
import http.server
import io
import os
import re
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

from .util import site_dir

RANGE = re.compile(r"bytes=(\d*)-(\d*)$")


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, ".rsc": "text/x-component"}

    def end_headers(self):
        self.send_header("Accept-Ranges", "bytes")
        super().end_headers()

    def send_head(self):
        self.redirect_to = None
        self.route()
        if self.redirect_to:
            self.send_response(302)
            self.send_header("Location", self.redirect_to)
            self.end_headers()
            return None
        if self.headers.get("Range"):
            return self.send_range()
        return super().send_head()

    def send_range(self):
        # Safari only plays <video> from servers that answer byte-range requests with 206.
        path = self.translate_path(self.path)
        m = RANGE.match(self.headers["Range"].strip())
        if not m or not os.path.isfile(path) or m.group(1) == m.group(2) == "":
            return super().send_head()
        size = os.path.getsize(path)
        if m.group(1):
            start, end = int(m.group(1)), min(int(m.group(2) or size - 1), size - 1)
        else:
            start, end = max(size - int(m.group(2)), 0), size - 1
        if start > end:
            self.send_response(416)
            self.send_header("Content-Range", f"bytes */{size}")
            self.end_headers()
            return None
        with open(path, "rb") as f:
            f.seek(start)
            body = f.read(end - start + 1)
        self.send_response(206)
        self.send_header("Content-Type", self.guess_type(path))
        self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        return io.BytesIO(body)

    def route(self):
        """Map the request onto a file the way a static host would."""
        url = urlparse(self.path)
        if "_rsc" in parse_qs(url.query) or self.headers.get("RSC") == "1":
            rsc = Path(self.directory) / unquote(url.path).strip("/") / "index.rsc"
            if rsc.is_file():
                self.path = url.path.rstrip("/") + "/index.rsc"
                return
        if url.path == "/_next/image":
            # The mirror keeps original images, not optimizer output. Remote originals (a CMS
            # CDN) were never on the site, so the browser is sent to them.
            src = parse_qs(url.query).get("url", [""])[0]
            if src.startswith(("http://", "https://")):
                self.redirect_to = src
            elif src.startswith("/"):
                self.path = src
        elif not Path(url.path).suffix:
            local = Path(self.directory) / unquote(url.path).lstrip("/")
            if (local / "index.html").is_file():
                self.path = url.path.rstrip("/") + "/index.html"

    def log_message(self, fmt, *args):
        if args and str(args[1]).startswith(("4", "5")):
            super().log_message(fmt, *args)


def serve(path, port=8000):
    site = site_dir(path)
    handler = functools.partial(Handler, directory=str(site.resolve()))
    for attempt in range(port, port + 20):
        try:
            httpd = http.server.ThreadingHTTPServer(("127.0.0.1", attempt), handler)
            break
        except OSError:
            continue
    else:
        raise SystemExit(f"No free port between {port} and {port + 19}.")
    with httpd:
        print(f"Serving {site} at http://localhost:{attempt}  (Ctrl+C to stop)")
        try:
            httpd.serve_forever()
        except KeyboardInterrupt:
            pass
