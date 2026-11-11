// Recording input: HAR 1.2, an interaction trace and a starting cookie jar. Entries are sorted
// by start time then id, so the order of the input file is irrelevant.

import { asArray, asNumber, asObject, asString, parseJson, type JVal } from "./json.ts";
import { cmp, sha256Hex } from "./util.ts";

export interface Part {
    name: string;
    value: string;
    filename: string | null;
}

export interface Body {
    mime: string;
    text: string;
    parts: Part[];
}

export interface Entry {
    /** `_requestId` when present, else `c-<hash of method, url and body>`. */
    id: string;
    /** True when `id` is a `_requestId` (stable across runs), false when derived from content. */
    explicitId: boolean;
    /** Hash of the whole entry (request and response): the last tie-break, so equal ids and start times still sort the same. */
    key: string;
    startedUs: number;
    durationUs: number;
    method: string;
    url: URL;
    /** Lower-cased names, original order. */
    reqHeaders: [string, string][];
    reqCookies: [string, string][];
    reqBody: Body | null;
    status: number;
    respHeaders: [string, string][];
    respSetCookies: [string, string][];
    respBody: Body | null;
}

export interface TraceEvent {
    id: string;
    tUs: number;
    kind: string;
    target: string;
    /** What the user supplied at this event, flattened to dotted names. */
    inputs: Map<string, string>;
    requests: string[];
    /** Hash of the whole event: the last sort tie-break (recorders may reuse ids). */
    key: string;
}

export interface Recording {
    id: string;
    params: Map<string, string>;
    entries: Entry[];
    events: TraceEvent[];
    initialCookies: { name: string; value: string }[];
    /** Entries that could not be used (not an http(s) URL): reported, never silently dropped. */
    skipped: string[];
}

/** Natural string order: digit runs compare as numbers, so `100.9` sorts before `100.10` in every run. */
export function natCmp(a: string, b: string): number {
    const ta = a.match(/\d+|\D+/g) ?? [];
    const tb = b.match(/\d+|\D+/g) ?? [];
    for (let i = 0; i < Math.min(ta.length, tb.length); i++) {
        const x = ta[i]!;
        const y = tb[i]!;
        if (x === y) continue;
        if (/^\d/.test(x) && /^\d/.test(y)) {
            // Numeric order without parsing: drop leading zeros, then shorter is smaller, then lexical.
            const nx = x.replace(/^0+(?=\d)/, "");
            const ny = y.replace(/^0+(?=\d)/, "");
            if (nx.length !== ny.length) return nx.length < ny.length ? -1 : 1;
            const lex = cmp(nx, ny);
            if (lex !== 0) return lex;
            continue;
        }
        return cmp(x, y);
    }
    return ta.length - tb.length;
}

export function endUs(e: Entry): number {
    return e.startedUs + Math.max(0, e.durationUs);
}

/** The event that caused entry `entryId` (earliest event wins). */
export function eventOf(rec: Recording, entryId: string): TraceEvent | undefined {
    return rec.events.find((e) => e.requests.includes(entryId));
}

/** Parse one run: its HAR, its trace (which names the run and carries its parameters) and optionally the starting cookie jar. */
export function parseRecording(har: string, trace: string, cookies?: string | null): Recording {
    const harV = parseJson(har);
    const traceV = parseJson(trace);
    const t = asObject(traceV);
    if (!t) throw new Error("trace must be a JSON object");
    const id = runId(t);
    const params = new Map<string, string>();
    for (const key of ["params", "inputs"]) flatten("", t.get(key), params);
    const skipped: string[] = [];
    const entries = parseEntries(harV, skipped);
    entries.sort((a, b) => a.startedUs - b.startedUs || natCmp(a.id, b.id) || cmp(a.id, b.id) || cmp(a.key, b.key));
    const events = parseEvents(t);
    events.sort((a, b) => a.tUs - b.tUs || natCmp(a.id, b.id) || cmp(a.id, b.id) || cmp(a.key, b.key));
    const jar = cookies ? parseCookieJar(cookies) : [];
    jar.sort((a, b) => cmp(a.name, b.name) || cmp(a.value, b.value));
    return { id, params, entries, events, initialCookies: jar, skipped };
}

function runId(t: Map<string, JVal>): string {
    const run = t.get("run");
    if (run?.t === "str" && run.v) return run.v;
    if (run?.t === "num") {
        const sc = asString(t.get("scenario"));
        return sc ? `${sc}.run${run.raw}` : `run${run.raw}`;
    }
    throw new Error("trace.run (the run id) is required");
}

function scalar(v: JVal): string | null {
    if (v.t === "str") return v.v;
    if (v.t === "num") return v.raw;
    if (v.t === "bool") return String(v.v);
    return null;
}

/** Nested parameter objects become dotted names: `address.city`, `items[0]`. */
function flatten(prefix: string, v: JVal | undefined, out: Map<string, string>): void {
    if (!v) return;
    if (v.t === "obj") {
        for (const [k, child] of v.entries) flatten(prefix ? `${prefix}.${k}` : k, child, out);
    } else if (v.t === "arr") {
        v.items.forEach((child, i) => flatten(`${prefix}[${i}]`, child, out));
    } else {
        const s = scalar(v);
        if (s !== null && prefix) out.set(prefix, s);
    }
}

function pairs(v: JVal | undefined): [string, string][] {
    const out: [string, string][] = [];
    for (const h of asArray(v) ?? []) {
        const o = asObject(h);
        const name = asString(o?.get("name"));
        if (name === null) continue;
        out.push([name, asString(o?.get("value")) ?? ""]);
    }
    return out;
}

const lower = (p: [string, string][]): [string, string][] => p.map(([n, v]) => [n.toLowerCase(), v]);

function parseEntries(har: JVal, skipped: string[]): Entry[] {
    const log = asObject(asObject(har)?.get("log"));
    const list = asArray(log?.get("entries"));
    if (!list) throw new Error("HAR has no log.entries array");
    const out: Entry[] = [];
    list.forEach((e, i) => {
        const eo = asObject(e);
        const req = asObject(eo?.get("request"));
        if (!eo || !req) throw new Error(`HAR entry ${i} has no request`);
        let url: URL;
        try {
            url = new URL(asString(req.get("url")) ?? "");
        } catch {
            skipped.push("an entry with an invalid request.url was skipped");
            return;
        }
        // data:, blob: and similar entries are not network requests; they are skipped without a warning.
        if (url.protocol !== "http:" && url.protocol !== "https:") return;
        const started = parseIsoUs(asString(eo.get("startedDateTime")) ?? "");
        if (started === null) throw new Error(`entry ${i}: startedDateTime is missing or not ISO 8601`);
        const time = asNumber(eo.get("time"));
        const method = (asString(req.get("method")) ?? "GET").toUpperCase();
        const reqBody = req.has("postData") ? parsePostData(req.get("postData")) : null;
        const resp = asObject(eo.get("response"));
        const respHeaders = lower(pairs(resp?.get("headers")));
        let setCookies: [string, string][] = [];
        for (const [n, v] of respHeaders) {
            if (n !== "set-cookie") continue;
            const c = parseSetCookie(v);
            if (c) setCookies.push(c);
        }
        if (setCookies.length === 0) setCookies = pairs(resp?.get("cookies"));
        const rid = asString(eo.get("_requestId"));
        out.push({
            id: rid ? rid : contentId(method, asString(req.get("url")) ?? "", reqBody),
            explicitId: Boolean(rid),
            key: sha256Hex(stable(e)),
            startedUs: started,
            durationUs: time === null || !Number.isFinite(time) ? 0 : Math.round(Math.min(Math.max(0, time), MAX_DURATION_MS) * 1000),
            method,
            url,
            reqHeaders: lower(pairs(req.get("headers"))),
            reqCookies: pairs(req.get("cookies")),
            reqBody,
            status: validStatus(asNumber(resp?.get("status"))),
            respHeaders,
            respSetCookies: setCookies,
            respBody: parseContent(asObject(resp?.get("content"))),
        });
    });
    return out;
}

/** An event without its per-run `inputs`: ids that clash must tie-break the same way in every run. */
function withoutInputs(e: JVal): JVal {
    if (e.t !== "obj") return e;
    const entries = new Map(e.entries);
    entries.delete("inputs");
    return { t: "obj", entries };
}

/** JSON text of a parsed value with object keys sorted, so key order in the file never matters. */
function stable(v: JVal): string {
    switch (v.t) {
        case "obj":
            return `{${[...v.entries.keys()].sort(cmp).map((k) => `${JSON.stringify(k)}:${stable(v.entries.get(k)!)}`).join(",")}}`;
        case "arr":
            return `[${v.items.map(stable).join(",")}]`;
        case "str":
            return JSON.stringify(v.v);
        case "num":
            return v.raw;
        default:
            return v.t === "bool" ? String(v.v) : "null";
    }
}

/** A response longer than a day is a hostile or corrupt `time`; it would let any response look finished. */
const MAX_DURATION_MS = 86_400_000;

const validStatus = (n: number | null): number => (n !== null && Number.isInteger(n) && n >= 100 && n <= 599 ? n : 0);

function contentId(method: string, url: string, body: Body | null): string {
    let s = `${method}\n${url}\n`;
    if (body) {
        s += body.text;
        for (const p of body.parts) s += p.name + p.value;
    }
    return `c-${sha256Hex(s).slice(0, 16)}`;
}

function parsePostData(pd: JVal | undefined): Body {
    const o = asObject(pd);
    const mime = asString(o?.get("mimeType")) ?? "";
    const text = asString(o?.get("text")) ?? "";
    let parts: Part[] = [];
    if (text === "") {
        for (const p of asArray(o?.get("params")) ?? []) {
            const po = asObject(p);
            const name = asString(po?.get("name"));
            if (name !== null) {
                parts.push({ name, value: asString(po?.get("value")) ?? "", filename: asString(po?.get("fileName")) });
            }
        }
    } else if (mimeBase(mime) === "multipart/form-data") {
        parts = parseMultipart(mime, text);
    }
    return { mime, text, parts };
}

function parseContent(c: Map<string, JVal> | null): Body | null {
    if (!c) return null;
    const raw = asString(c.get("text"));
    if (raw === null) return null;
    let text = raw;
    if (asString(c.get("encoding")) === "base64") {
        const buf = Buffer.from(raw, "base64");
        try {
            text = new TextDecoder("utf-8", { fatal: true }).decode(buf);
        } catch {
            return null;
        }
    }
    return { mime: asString(c.get("mimeType")) ?? "", text, parts: [] };
}

/** Media type without parameters, lower-cased. */
export function mimeBase(mime: string): string {
    return (mime.split(";")[0] ?? "").trim().toLowerCase();
}

export function parseSetCookie(line: string): [string, string] | null {
    const first = line.split(";")[0] ?? "";
    const i = first.indexOf("=");
    if (i < 0) return null;
    const n = first.slice(0, i).trim();
    return n ? [n, first.slice(i + 1).trim()] : null;
}

export function parseCookieHeader(line: string): [string, string][] {
    const out: [string, string][] = [];
    for (const p of line.split(";")) {
        const i = p.indexOf("=");
        if (i < 0) continue;
        const n = p.slice(0, i).trim();
        if (n) out.push([n, p.slice(i + 1).trim()]);
    }
    return out;
}

function boundaryOf(mime: string): string | null {
    const i = mime.toLowerCase().indexOf("boundary=");
    if (i < 0) return null;
    const rest = (mime.slice(i + 9).split(";")[0] ?? "").trim().replace(/^"|"$/g, "");
    return rest || null;
}

export function parseMultipart(mime: string, text: string): Part[] {
    const boundary = boundaryOf(mime);
    if (!boundary) return [];
    const parts: Part[] = [];
    for (const raw of text.split(`--${boundary}`).slice(1)) {
        const chunk = raw.replace(/^[\r\n]+/, "");
        if (chunk.startsWith("--") || chunk.trim() === "") continue;
        let sep = chunk.indexOf("\r\n\r\n");
        let sepLen = 4;
        if (sep < 0) {
            sep = chunk.indexOf("\n\n");
            sepLen = 2;
        }
        if (sep < 0) continue;
        const head = chunk.slice(0, sep);
        let body = chunk.slice(sep + sepLen);
        body = body.endsWith("\r\n") ? body.slice(0, -2) : body.endsWith("\n") ? body.slice(0, -1) : body;
        let name: string | null = null;
        let filename: string | null = null;
        for (const line of head.split(/\r?\n/)) {
            if (line.toLowerCase().startsWith("content-disposition:")) {
                name = dispositionParam(line, "name");
                filename = dispositionParam(line, "filename");
            }
        }
        if (name !== null) parts.push({ name, value: body, filename });
    }
    return parts;
}

function dispositionParam(line: string, key: string): string | null {
    const needle = `${key}="`;
    let from = 0;
    for (;;) {
        const i = line.indexOf(needle, from);
        if (i < 0) return null;
        const prev = i > 0 ? line[i - 1] : "";
        if (prev === " " || prev === ";" || prev === "\t") {
            const rest = line.slice(i + needle.length);
            const end = rest.indexOf('"');
            return end < 0 ? null : rest.slice(0, end);
        }
        from = i + needle.length;
    }
}

function parseEvents(t: Map<string, JVal>): TraceEvent[] {
    const out: TraceEvent[] = [];
    (asArray(t.get("events")) ?? []).forEach((e) => {
        const o = asObject(e);
        // Without an id the event is named by its content, never by its position in the file.
        const id = asString(o?.get("id")) ?? `ev-${sha256Hex(stable(e)).slice(0, 12)}`;
        const tv = o?.get("t");
        let tUs = 0; // recorders that do not time events: order falls back to the id
        if (tv?.t === "str") {
            const p = parseIsoUs(tv.v);
            if (p === null) throw new Error(`trace event ${id}: t is not ISO 8601`);
            tUs = p;
        } else if (tv?.t === "num") tUs = Math.round(Number(tv.raw) * 1000);
        const inputs = new Map<string, string>();
        flatten("", o?.get("inputs"), inputs);
        const requests: string[] = [];
        for (const r of asArray(o?.get("requests")) ?? []) {
            const s = asString(r);
            if (s !== null) requests.push(s);
        }
        out.push({
            key: sha256Hex(stable(withoutInputs(e))),
            id,
            tUs,
            kind: asString(o?.get("type")) ?? "",
            target: asString(o?.get("target")) ?? "",
            inputs,
            requests,
        });
    });
    return out;
}

/** Accepts a JSON array of cookies, or an object with a `cookies` array (Playwright storageState, DevTools exports). */
export function parseCookieJar(text: string): { name: string; value: string }[] {
    const v = parseJson(text);
    const list = v.t === "arr" ? v.items : asArray(asObject(v)?.get("cookies"));
    if (!list) throw new Error("cookie jar must be an array or an object with a cookies array");
    const out: { name: string; value: string }[] = [];
    for (const c of list) {
        const o = asObject(c);
        const name = asString(o?.get("name"));
        const value = asString(o?.get("value"));
        if (name !== null && value !== null) out.push({ name, value });
    }
    return out;
}

/** `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` to microseconds since the Unix epoch, or null. */
export function parseIsoUs(s: string): number | null {
    const m = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(Z|z|[+-]\d{2}:?\d{2})?$/.exec(s);
    if (!m) return null;
    const [, y, mo, d, h, mi, sec, frac, off] = m;
    if (Number(mo) < 1 || Number(mo) > 12 || Number(d) < 1 || Number(d) > 31 || Number(h) > 23 || Number(mi) > 59 || Number(sec) > 60) return null;
    let offsetS = 0;
    if (off && off.toLowerCase() !== "z") {
        const digits = off.slice(1).replace(":", "");
        offsetS = (off.startsWith("-") ? -1 : 1) * (Number(digits.slice(0, 2)) * 3600 + Number(digits.slice(2)) * 60);
    }
    const ms = Date.UTC(Number(y), Number(mo) - 1, Number(d), Number(h), Number(mi), Math.min(Number(sec), 59));
    if (Number.isNaN(ms) || new Date(ms).getUTCDate() !== Number(d)) return null;
    const fracUs = Number((frac ?? "").slice(0, 6).padEnd(6, "0"));
    return (ms / 1000 - offsetS) * 1_000_000 + fracUs;
}
