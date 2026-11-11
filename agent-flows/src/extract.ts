// Value extraction: every request and response is flattened into (location, value) slots.
// Locations are stable strings, identical across runs, so slots can be aligned and compared.
//
//   url.path[i], url.query:k[#n]         decoded path segment / query parameter
//   header:name[#n]                       header (`cookie` is split into cookie:name)
//   cookie:name, setcookie:name           request cookie / Set-Cookie
//   json:$.a.b[0]                         JSON leaf (["k"] for keys that are not plain identifiers)
//   form:name[#n], multipart:name[@filename]
//   html:input[name=x], html:meta[name=x] hidden inputs and meta tags in HTML responses
//   body                                  an opaque request body
//   <loc>|path[i], <loc>|query:k          components of an absolute URL held in any value

import { MAX_LEN, MIN_LEN } from "./evidence.ts";
import { type Entry, mimeBase, parseCookieHeader, type Part } from "./har.ts";
import { tryParseJson, type JVal } from "./json.ts";
import { cmp, decodeComponent } from "./util.ts";

/** JSON type of a body value, so a constant can be replayed as the number or null it was. */
export type JsonType = "string" | "number" | "boolean" | "null" | "object" | "array";

export interface Slot {
    loc: string;
    value: string;
    /** Only on JSON body values. Empty containers are slots with value `{}` / `[]`. */
    jt?: JsonType;
}

/** Headers determined by other parts of the message or by the transport: no independent value. */
const IGNORED_HEADERS = new Set([
    "content-length", "host", "connection", "keep-alive", "te", "transfer-encoding", "date", "age", "vary", "server-timing",
]);

const ignoredHeader = (n: string): boolean => n.startsWith(":") || IGNORED_HEADERS.has(n);

class Out {
    slots: Slot[] = [];

    push(loc: string, value: string, jt?: JsonType): void {
        this.slots.push({ loc, value, ...(jt ? { jt } : {}) });
        // An absolute URL inside any value contributes its path and query components.
        if (value.length < 4096 && (value.startsWith("http://") || value.startsWith("https://"))) {
            let u: URL;
            try {
                u = new URL(value);
            } catch {
                return;
            }
            u.pathname
                .split("/")
                .slice(1)
                .forEach((seg, i) => {
                    const s = decodeComponent(seg);
                    if (s) this.slots.push({ loc: `${loc}|path[${i}]`, value: s });
                });
            const seen = new Map<string, number>();
            for (const [k, v] of u.searchParams) {
                const n = seen.get(k) ?? 0;
                seen.set(k, n + 1);
                this.slots.push({ loc: `${loc}|query:${k.replace(/#/g, "%23")}${n === 0 ? "" : `#${n}`}`, value: v });
            }
        }
    }
}

function keyed(prefix: string, name: string, counts: Map<string, number>): string {
    const key = `${prefix}${name.replace(/#/g, "%23")}`;
    const n = counts.get(key) ?? 0;
    counts.set(key, n + 1);
    return n === 0 ? key : `${key}#${n}`;
}

export function pathSegments(url: URL): string[] {
    return url.pathname.split("/").slice(1).map(decodeComponent);
}

const bySlotLoc = (a: Slot, b: Slot): number => cmp(a.loc, b.loc);

/** Request slots, sorted by location (stable for equal locations). */
export function requestSlots(e: Entry): Slot[] {
    const out = new Out();
    pathSegments(e.url).forEach((seg, i) => {
        if (seg) out.push(`url.path[${i}]`, seg);
    });
    let counts = new Map<string, number>();
    for (const [k, v] of e.url.searchParams) out.push(keyed("url.query:", k, counts), v);
    counts = new Map();
    let cookiePairs: [string, string][] = [];
    for (const [n, v] of e.reqHeaders) {
        if (ignoredHeader(n)) continue;
        if (n === "cookie") {
            cookiePairs.push(...parseCookieHeader(v));
            continue;
        }
        out.push(keyed("header:", n, counts), v);
    }
    if (cookiePairs.length === 0) cookiePairs = e.reqCookies;
    counts = new Map();
    for (const [n, v] of cookiePairs) out.push(keyed("cookie:", n, counts), v);
    if (e.reqBody) bodySlots(out, e.reqBody.mime, e.reqBody.text, e.reqBody.parts, true);
    return out.slots.sort(bySlotLoc);
}

/** Response slots usable as sources, sorted by location. */
export function responseSlots(e: Entry): Slot[] {
    const out = new Out();
    const counts = new Map<string, number>();
    for (const [n, v] of e.respHeaders) {
        if (ignoredHeader(n) || n === "set-cookie") continue;
        out.push(keyed("header:", n, counts), v);
    }
    const cookieCounts = new Map<string, number>();
    for (const [n, v] of e.respSetCookies) out.push(keyed("setcookie:", n, cookieCounts), v);
    if (e.respBody) bodySlots(out, e.respBody.mime, e.respBody.text, e.respBody.parts, false);
    return out.slots.sort(bySlotLoc);
}

function bodySlots(out: Out, mime: string, text: string, parts: Part[], request: boolean): void {
    const base = mimeBase(mime);
    const trimmed = text.trimStart();
    if (base.includes("json") || trimmed.startsWith("{") || trimmed.startsWith("[")) {
        const v = tryParseJson(text);
        if (v) {
            jsonLeaves(out, "$", v);
            return;
        }
    }
    if (base === "application/x-www-form-urlencoded" || (request && base === "" && text.includes("="))) {
        const counts = new Map<string, number>();
        const pairs: [string, string][] =
            parts.length > 0 && text === "" ? parts.map((p) => [p.name, p.value]) : [...new URLSearchParams(text)];
        for (const [k, v] of pairs) out.push(keyed("form:", k, counts), v);
        return;
    }
    if (base === "multipart/form-data") {
        const counts = new Map<string, number>();
        for (const p of parts) {
            const loc = keyed("multipart:", p.name, counts);
            out.push(loc, p.value);
            if (p.filename !== null) out.push(`${loc}@filename`, p.filename);
        }
        return;
    }
    if (request) {
        if (text) out.push("body", text);
        return;
    }
    if (base === "text/html" || base === "application/xhtml+xml") htmlSlots(out, text);
}

function pathKey(base: string, key: string): string {
    return /^[A-Za-z_][A-Za-z0-9_-]*$/.test(key) ? `${base}.${key}` : `${base}[${JSON.stringify(key)}]`;
}

function jsonLeaves(out: Out, path: string, v: JVal): void {
    switch (v.t) {
        case "obj":
            if (v.entries.size === 0) out.push(`json:${path}`, "{}", "object");
            for (const [k, child] of v.entries) jsonLeaves(out, pathKey(path, k), child);
            break;
        case "arr":
            if (v.items.length === 0) out.push(`json:${path}`, "[]", "array");
            v.items.forEach((child, i) => jsonLeaves(out, `${path}[${i}]`, child));
            break;
        case "str":
            out.push(`json:${path}`, v.v, "string");
            break;
        case "num":
            out.push(`json:${path}`, v.raw, "number");
            break;
        case "bool":
            out.push(`json:${path}`, String(v.v), "boolean");
            break;
        case "null":
            out.push(`json:${path}`, "null", "null");
            break;
    }
}

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };

function decodeEntities(s: string): string {
    return s.replace(/&(#x[0-9a-fA-F]+|#[0-9]+|[A-Za-z]+);/g, (m, g: string) => {
        if (g.startsWith("#x")) return safeCp(parseInt(g.slice(2), 16), m);
        if (g.startsWith("#")) return safeCp(parseInt(g.slice(1), 10), m);
        return Object.hasOwn(ENTITIES, g) ? ENTITIES[g]! : m;
    });
}

const safeCp = (n: number, fallback: string): string => {
    try {
        return String.fromCodePoint(n);
    } catch {
        return fallback;
    }
};

/**
 * Attributes of every `<tag ...>` outside comments, scripts and styles. A tolerant single-pass
 * scanner (linear time, whatever the input), not a full HTML parser. Every tag is parsed through its
 * quoted attributes, so markup inside an attribute value is never mistaken for a tag.
 */
/** Elements whose content is text (or an inert fragment), never form controls of the live DOM. */
const RAW_TEXT = new Set(["script", "style", "textarea", "title", "xmp", "noscript", "template", "plaintext"]);

function scanTags(html: string, tag: string): Map<string, string>[] {
    const out: Map<string, string>[] = [];
    let i = 0;
    let bang = -2; // next `--!>` position; -1 once none remains
    while (i < html.length) {
        const lt = html.indexOf("<", i);
        if (lt < 0) break;
        if (html.startsWith("<!-->", lt)) {
            i = lt + 5;
            continue;
        }
        if (html.startsWith("<!--->", lt)) {
            i = lt + 6;
            continue;
        }
        if (html.startsWith("<!--", lt)) {
            const a = html.indexOf("-->", lt + 4);
            // `--!>` also ends a comment; its position is cached so a document with none is scanned once.
            if (bang !== -1 && bang < lt + 4) bang = html.indexOf("--!>", lt + 4);
            const b = bang;
            const end = a < 0 ? b : b < 0 ? a : Math.min(a, b);
            if (end < 0) break;
            i = end + (html.startsWith("--!>", end) ? 4 : 3);
            continue;
        }
        if (html[lt + 1] === "!" || html[lt + 1] === "?") {
            // `<!DOCTYPE`, `<![CDATA[` and `<?php` are bogus comments: they end at the next `>`.
            const end = html.indexOf(">", lt + 2);
            if (end < 0) break;
            i = end + 1;
            continue;
        }
        const first = html[lt + 1] ?? "";
        if (!/[A-Za-z]/.test(first)) {
            i = lt + 1;
            continue;
        }
        // Tag name, then attributes up to the closing `>`, honouring quotes.
        let p = lt + 1;
        while (p < html.length && !/[\s/>]/.test(html[p]!)) p++;
        const name = html.slice(lt + 1, p).toLowerCase();
        const attrs = new Map<string, string>();
        let closed = false;
        while (p < html.length) {
            const c = html[p]!;
            if (c === ">") {
                closed = true;
                p++;
                break;
            }
            if (/[\s/]/.test(c)) {
                p++;
                continue;
            }
            let q = p;
            while (q < html.length && !/[\s/>=]/.test(html[q]!)) q++;
            if (q === p) q = p + 1; // a stray `=`
            const attr = html.slice(p, q).toLowerCase();
            p = q;
            while (p < html.length && /\s/.test(html[p]!)) p++;
            let value = "";
            if (html[p] === "=") {
                p++;
                while (p < html.length && /\s/.test(html[p]!)) p++;
                const quote = html[p];
                if (quote === '"' || quote === "'") {
                    const end = html.indexOf(quote, p + 1);
                    if (end < 0) return out;
                    value = html.slice(p + 1, end);
                    p = end + 1;
                } else {
                    q = p;
                    while (q < html.length && !/[\s>]/.test(html[q]!)) q++;
                    value = html.slice(p, q);
                    p = q;
                }
            }
            if (attr && !attrs.has(attr)) attrs.set(attr, decodeEntities(value));
        }
        if (!closed) break;
        i = p;
        if (RAW_TEXT.has(name) && html[p - 2] !== "/") {
            // Raw text up to the matching end tag; nothing inside is markup.
            const re = new RegExp(`</${name}[\\s/>]`, "gi");
            re.lastIndex = i;
            const end = re.exec(html);
            if (!end) break;
            i = end.index + 2 + name.length;
            continue;
        }
        if (name === tag) out.push(attrs);
    }
    return out;
}

function htmlSlots(out: Out, text: string): void {
    let counts = new Map<string, number>();
    for (const a of scanTags(text, "input")) {
        const name = a.get("name");
        const value = a.get("value");
        if (a.get("type")?.toLowerCase() === "hidden" && name !== undefined && value !== undefined) {
            out.push(`${keyed("html:input[name=", name, counts)}]`, value);
        }
    }
    counts = new Map();
    for (const a of scanTags(text, "meta")) {
        const name = a.get("name");
        const content = a.get("content");
        if (name !== undefined && content !== undefined) out.push(`${keyed("html:meta[name=", name, counts)}]`, content);
    }
}

/** Whether a response body is searched as free text for tokens (HTML, scripts, text not already split into structured slots). */
export function isFreeText(e: Entry): boolean {
    const b = e.respBody;
    if (!b) return false;
    const base = mimeBase(b.mime);
    const trimmed = b.text.trimStart();
    if (base.includes("json") || (trimmed.startsWith("{") && tryParseJson(b.text) !== null)) return false;
    return (
        base === "text/html" || base === "application/xhtml+xml" || base.startsWith("text/") || base.includes("javascript") || base.includes("xml")
    );
}

/**
 * Tokens of a free-text body at two granularities: words (`[A-Za-z0-9_-]`) and blobs (anything
 * between whitespace, quotes, angle brackets, `&`, `;`, `,`, parentheses and braces, so base64
 * and percent-encoded tokens stay whole).
 */
export function bodyTokens(text: string): Set<string> {
    const set = new Set<string>();
    const add = (t: string) => {
        if (t.length >= MIN_LEN && t.length <= MAX_LEN) set.add(t);
    };
    for (const t of text.split(/[^A-Za-z0-9_-]/)) add(t);
    for (const t of text.split(/[\s"'<>&;,(){}[\]|\\`]/)) {
        add(t);
        for (const piece of t.split(/[=:]/)) add(piece);
    }
    return set;
}
