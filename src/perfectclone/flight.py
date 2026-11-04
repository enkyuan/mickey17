"""Read and rewrite the React Server Components "flight" payload that Next.js inlines in HTML.

An App Router page ships its server-rendered HTML *and* a serialized copy of the same React tree
in `<script>self.__next_f.push([1,"..."])</script>` chunks. On load React hydrates the HTML
against that payload. Change the HTML without changing the payload (or the other way round) and
React throws a hydration error and re-renders from the payload, undoing the edit.

The payload is a stream of rows:
    <id>:<json>\n              most rows
    <id>:T<hex byte length>,<text>   long text, length-prefixed in UTF-8 bytes, no newline
Rows can span push() chunks, and a text row's length must be recomputed after any edit.
"""
import json
import re

PUSH = re.compile(r"<script[^>]*>self\.__next_f\.push\((\[.*?\])\)</script>", re.S)
TEXT_ROW = re.compile(rb"([0-9a-f]*):T([0-9a-f]+),")


def has_flight(page):
    return "self.__next_f.push(" in page


def map_rows(stream, fn):
    """Apply fn(str) -> str to every row of a decoded flight stream."""
    b, out, i = stream.encode(), [], 0
    while i < len(b):
        m = TEXT_ROW.match(b, i, min(len(b), i + 40))
        if m:
            start = m.end()
            end = start + int(m.group(2), 16)
            text = fn(b[start:end].decode()).encode()
            out.append(m.group(1) + b":T" + format(len(text), "x").encode() + b"," + text)
            i = end
        else:
            j = b.find(b"\n", i)
            j = len(b) if j < 0 else j + 1
            out.append(fn(b[i:j].decode()).encode())
            i = j
    return b"".join(out).decode()


def edit_flight(page, fn):
    """Apply fn row by row to a page's flight payload and write it back.

    The decoded stream is written back as a single push() in place of the first data chunk, so a
    row that used to straddle two chunks is handled like any other.
    """
    chunks = []
    for m in PUSH.finditer(page):
        arr = json.loads(m.group(1))
        if len(arr) > 1 and arr[0] == 1 and isinstance(arr[1], str):
            chunks.append((m, arr[1]))
    if not chunks:
        return page
    stream = "".join(s for _, s in chunks)
    new = map_rows(stream, fn)
    if new == stream:
        return page
    enc = json.dumps([1, new], ensure_ascii=False, separators=(",", ":"))
    # Same escaping Next.js applies so the payload can't close the <script> early.
    enc = enc.replace("<", "\\u003c").replace("\u2028", "\\u2028").replace("\u2029", "\\u2029")
    first = chunks[0][0].group(0)
    tag = first[: first.index(">") + 1]                       # keep any nonce attribute
    out, last = [], 0
    for k, (m, _) in enumerate(chunks):
        out.append(page[last:m.start()])
        if k == 0:
            out.append(f"{tag}self.__next_f.push({enc})</script>")
        last = m.end()
    out.append(page[last:])
    return "".join(out)


def outside_flight(page, fn):
    """Apply fn to the parts of the page that are not flight chunks."""
    out, last = [], 0
    for m in PUSH.finditer(page):
        out.append(fn(page[last:m.start()]))
        out.append(m.group(0))
        last = m.end()
    out.append(fn(page[last:]))
    return "".join(out)


def edit_json_rows(page, fn):
    """Rewrite each JSON row's tree in a page's payload with fn(node) -> node (None drops an element)."""
    return edit_flight(page, json_row_editor(fn))


def json_row_editor(fn):
    def row(text):
        m = re.match(r"([0-9a-f]+):(.*?)(\n?)$", text, re.S)
        if not m or not m.group(2).startswith(("[", "{")):
            return text
        try:
            tree = json.loads(m.group(2))
        except ValueError:
            return text
        new = walk(tree, fn)
        if new == tree:
            return text
        return f"{m.group(1)}:{json.dumps(new, ensure_ascii=False, separators=(',', ':'))}{m.group(3)}"
    return row


def walk(node, fn):
    node = fn(node)
    if isinstance(node, list):
        return [walk(n, fn) for n in node]
    if isinstance(node, dict):
        return {k: walk(v, fn) for k, v in node.items()}
    return node
