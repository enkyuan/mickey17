"""Build and read minimal Next.js App Router pages for tests."""
import json
import re


def page_with(stream, chunks=1):
    """A page whose flight stream is split across `chunks` push() calls."""
    step = max(1, len(stream) // chunks)
    parts = [stream[i:i + step] for i in range(0, len(stream), step)]
    pushes = "".join(f"<script>self.__next_f.push({json.dumps([1, p])})</script>" for p in parts)
    return f"<html><body><h1>Acme</h1>{pushes}</body></html>"


def decoded(page):
    out = ""
    for m in re.finditer(r"self\.__next_f\.push\((\[.*?\])\)</script>", page, re.S):
        arr = json.loads(m.group(1))
        if len(arr) > 1:
            out += arr[1]
    return out
