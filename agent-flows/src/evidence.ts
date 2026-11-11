// What counts as evidence that two values are the same thing.
//
// A cross-request edge is only trustworthy when the matched value is unlikely to occur twice
// by accident. Short, low-entropy and common values (`1`, `true`, `USD`, an ISO date, a MIME
// type) match unrelated slots constantly, so they never create an edge. The thresholds are
// exported so tests pin them.

import { codePointLength } from "./util.ts";

/** Minimum length in characters. */
export const MIN_LEN = 8;
/** Values longer than this are blobs, not tokens. */
export const MAX_LEN = 2048;
/** Minimum number of distinct characters. */
export const MIN_DISTINCT = 6;
/** Minimum Shannon entropy in bits per character. */
export const MIN_BITS_PER_CHAR = 2.5;
/** Purely numeric values need at least this many digits (ids like `1042` are not evidence). */
export const MIN_NUMERIC_DIGITS = 12;
/** Purely alphabetic values (words) need at least this many characters. */
export const MIN_ALPHA_LEN = 16;

export function entropyBitsPerChar(s: string): number {
    const counts = new Map<string, number>();
    let n = 0;
    for (const c of s) {
        counts.set(c, (counts.get(c) ?? 0) + 1);
        n++;
    }
    if (n === 0) return 0;
    let h = 0;
    for (const c of counts.values()) {
        const p = c / n;
        h -= p * Math.log2(p);
    }
    return h;
}

/**
 * Weak gate, used to find candidate relations: only the structural exclusions (length, URLs,
 * MIME types, dates, short numbers and words). The per-value entropy of a short random id (8 hex
 * characters) fails the strong gate about a quarter of the time, so which values are looked up
 * must not depend on it; the strong gate is applied across runs instead (a relation needs the
 * strong gate in at least one run; see `strong` in the classifier).
 */
export const WEAK_MIN_DISTINCT = 1;
export const WEAK_MIN_BITS_PER_CHAR = 0;

/** True when `v` is distinctive enough that an equal value elsewhere is evidence of a relationship. */
export function isEvidence(v: string): boolean {
    return evidenceWith(v, MIN_DISTINCT, MIN_BITS_PER_CHAR);
}

/** The looser gate that decides which values are looked up at all. */
export function isWeakEvidence(v: string): boolean {
    return evidenceWith(v, WEAK_MIN_DISTINCT, WEAK_MIN_BITS_PER_CHAR);
}

function evidenceWith(v: string, minDistinct: number, minBits: number): boolean {
    const len = codePointLength(v);
    if (len < MIN_LEN || len > MAX_LEN) return false;
    if (new Set(v).size < minDistinct) return false;
    if (entropyBitsPerChar(v) < minBits) return false;
    if (looksUrlOrPath(v) || looksMime(v) || looksIsoDatetime(v) || looksHttpDate(v)) return false;
    if (/^[0-9]+$/.test(v)) return len >= MIN_NUMERIC_DIGITS && !looksEpoch(v);
    if (/^[A-Za-z]+$/.test(v) && len < MIN_ALPHA_LEN) return false;
    return true;
}

function looksUrlOrPath(v: string): boolean {
    return v.includes("://") || v.startsWith("//") || (v.startsWith("/") && !v.includes(" "));
}

function looksMime(v: string): boolean {
    const base = v.split(";")[0] ?? v;
    return /^[a-z-]+\/[a-z0-9+.-]+$/.test(base);
}

/** `2026-11-09`, `2026-11-09T10:00:00Z`, `2026-11-09 10:00:00.123+01:00`. */
export function looksIsoDatetime(v: string): boolean {
    return /^\d{4}-\d{2}-\d{2}(?:$|[T ])/.test(v);
}

function looksHttpDate(v: string): boolean {
    return v.endsWith(" GMT") && v.includes(", ");
}

/** Epoch seconds, milliseconds or microseconds between 2001 and 2100. */
export function looksEpoch(v: string): boolean {
    if (!/^[0-9]+$/.test(v)) return false;
    const n = Number(v);
    switch (v.length) {
        case 10:
            return n >= 946_684_800 && n <= 4_102_444_800;
        case 13:
            return n >= 946_684_800_000 && n <= 4_102_444_800_000;
        case 16:
            return n >= 946_684_800_000_000 && n <= 4_102_444_800_000_000;
        default:
            return false;
    }
}

/** A coarse description of a value that has no source. A label for humans, never used to decide a class. */
export function shape(v: string): string {
    const hex = (s: string) => s.length > 0 && /^[0-9a-fA-F]+$/.test(s);
    const parts = v.split("-");
    if (v.length === 36 && parts.map((p) => p.length).join() === "8,4,4,4,12" && parts.every(hex)) return "uuid";
    if (looksEpoch(v)) return "epoch-time";
    if (looksIsoDatetime(v)) return "iso-datetime";
    if (v.length >= 16 && hex(v)) return "hex";
    if (v.length >= 16 && /^[A-Za-z0-9+/=_-]+$/.test(v)) return "base64";
    return "opaque";
}

/** A card number: 13 to 19 digits (spaces and dashes allowed) with a valid Luhn checksum. */
export function looksLikeCard(v: string): boolean {
    if (!/^[0-9 -]+$/.test(v)) return false;
    const str = v.replace(/\D/g, "");
    if (str.length < 13 || str.length > 19) return false;
    return luhn([...str].map(Number), 0, str.length);
}

/** Long names matched anywhere in a location. */
const SENSITIVE_SUBSTRINGS = [
    "authorization", "cookie", "token", "secret", "password", "passwd", "passphrase", "credential", "signature", "session",
    "csrf", "xsrf", "apikey", "api-key", "api_key", "privatekey", "private-key", "private_key", "bearer", "hmac", "passcode", "pincode",
    "passwort", "kennwort", "contrasena", "contraseña", "senha", "motdepasse", "clave", "wachtwoord", "parola", "haslo", "lösenord", "пароль",
];
/** Short names matched only as whole words (`pin` must not redact `shipping`). */
const SENSITIVE_WORDS = new Set([
    "pwd", "pw", "pass", "pin", "otp", "jwt", "auth", "card", "cvc", "cvv", "cvv2", "iban", "ssn", "social", "email", "sig", "body", "phone", "key", "totp",
]);

/**
 * Locations whose values are credentials or secrets by name. Their constant values are never
 * written to the graph. Opaque request bodies (`body`) are always treated as sensitive.
 */
export function sensitiveLocation(loc: string): boolean {
    const l = loc.toLowerCase();
    const squashed = l.replace(/[\s_-]+/g, "");
    if (SENSITIVE_SUBSTRINGS.some((k) => l.includes(k) || squashed.includes(k))) return true;
    const words = loc
        .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
        .toLowerCase()
        .split(/[^a-z0-9]+/);
    return words.some((w) => SENSITIVE_WORDS.has(w));
}

/**
 * A card number anywhere in the text. Digits form groups separated by single separators (space, comma, `.`, `_`, `/`, `-`, NBSP;
 * full-width digits are normalised; a separator may be two characters, `, `). A window is one group, or the whole groups of a span
 * of groups, 13 to 19 digits long and Luhn-valid, or the same with the first group cut to its last 4 or 6 digits and the last group
 * cut to its first 3, 4 or 5, so digits glued in front of the number or an expiry date behind it do not hide it. A span must look like
 * card grouping: every interior group has 3 or more digits, or the whole run has groups of one length (`40 12 88 88 ...`); this keeps version strings,
 * dates, IPs and price lists (1 to 2 digit groups) from reading as cards. A single unseparated run is tested whole. Any card network
 * qualifies; values that are not cards but pass the checksum by chance are withheld, the safe direction. Work is bounded and the
 * bounds fail closed.
 */
function cardIn(text: string): boolean {
    if (text.length > 1_048_576) return true;
    const t = text.normalize("NFKC");
    let runs = 0;
    let work = 0;
    for (const m of t.matchAll(/\d(?:[ \u00a0,._/-]{0,2}\d)*/g)) {
        if (m[0].length < 13) continue;
        if (++runs > 4096) return true;
        const groups = m[0].split(/[^0-9]+/).filter(Boolean);
        const total = groups.reduce((n, g) => n + g.length, 0);
        if (total > 4096) return true;
        const test = (digits: string): boolean => {
            work += digits.length;
            return digits.length >= 13 && digits.length <= 19 && luhnStr(digits);
        };
        if (groups.length === 1) {
            if (test(groups[0]!)) return true;
            continue;
        }
        // Digit-by-digit or pair-by-pair spacing (`4 0 1 2 8 ...`, `40 12 88 ...`): every group of the run has the same length.
        const runUniform = groups.every((g) => g.length === groups[0]!.length);
        for (let i = 0; i < groups.length; i++) {
            const first = groups[i]!;
            if (test(first)) return true;
            // A card's first group has 3 or more digits (or the window is uniform, or the first group is cut from a longer glued run).
            const firsts = [...(first.length >= 3 ? [first] : []), ...[4, 6].filter((k) => k < first.length).map((k) => first.slice(-k))];
            let body = "";
            let interiorOk = true;
            for (let j = i + 1; j < groups.length && j < i + 24; j++) {
                const last = groups[j]!;
                if (first.length + body.length > 31) break;
                if (j > i + 1) {
                    // groups i+1 .. j-1 are interior
                    const g = groups[j - 1]!;
                    if (g.length < 3) interiorOk = false;
                }
                if (interiorOk || runUniform) {
                    const lasts = [...(last.length >= 3 ? [last] : []), ...[3, 4, 5].filter((k) => k < last.length).map((k) => last.slice(0, k))];
                    for (const f of firsts) for (const l of lasts) if (test(f + body + l)) return true;
                    if (runUniform && test(first + body + last)) return true;
                }
                body += last;
            }
            if (work > 20_000_000) return true;
        }
    }
    return false;
}

function luhnStr(d: string): boolean {
    let sum = 0;
    for (let i = 0; i < d.length; i++) {
        let x = d.charCodeAt(d.length - 1 - i) - 48;
        if (i % 2 === 1) {
            x *= 2;
            if (x > 9) x -= 9;
        }
        sum += x;
    }
    return sum % 10 === 0;
}

function luhn(d: number[], start: number, len: number): boolean {
    let sum = 0;
    for (let i = 0; i < len; i++) {
        let x = d[start + len - 1 - i]!;
        if (i % 2 === 1) {
            x *= 2;
            if (x > 9) x -= 9;
        }
        sum += x;
    }
    return sum % 10 === 0;
}

const distinctCount = (s: string): number => new Set(s).size;

/**
 * A token-grade word: a long unbroken run that is not prose. With a digit, 14+ characters; without
 * one, 20+. Words and short numbers joined by `-`/`_` (`blue-widget-2024`, `WIDGET-2024-XL`) are
 * exempt only where `slugOk` (a location that names a product, path or label): the same shape under any
 * other name may be a passphrase or a licence key.
 */
export function tokenGrade(p: string, slugOk = false): boolean {
    if (!isEvidence(p)) return false;
    if (slugOk && slugLike(p)) return false;
    return /\d/.test(p) ? p.length >= 14 : p.length >= 20;
}

function slugLike(p: string): boolean {
    const parts = p.split(/[-_]/);
    if (parts.length < 2 || !parts.every((w) => /^[A-Za-z]{1,12}$/.test(w) || /^\d{1,6}$/.test(w))) return false;
    // Readable: at least one real word, and not a key in equal-sized groups (`ABCDE-FGHIJ-KLMNO-PQRST`, `4821-9937-1102-5583`).
    const word = (w: string) => /^(?:[a-z]{3,}|[A-Z][a-z]{2,}|[A-Z]{3,})$/.test(w) && /[aeiouAEIOU]/.test(w);
    if (!parts.some(word)) return false;
    // Every alphabetic part is a short word (1-2 letters) or looks like one: random-case runs (`KqZxLm`) are not words.
    if (!parts.every((w) => /^\d+$/.test(w) || w.length <= 2 || word(w))) return false;
    const sizes = new Map<number, number>();
    for (const w of parts) sizes.set(w.length, (sizes.get(w.length) ?? 0) + 1);
    return Math.max(...sizes.values()) < 3;
}

/** Leaf names that legitimately carry readable slugs and labels (whole words of the field name, never the whole location). */
const SLUG_WORDS = new Set([
    "sku", "slug", "handle", "product", "variant", "category", "collection", "referer", "href", "title", "label", "route",
    "method", "rate", "color", "colour", "coupon", "option", "size", "carrier", "shipping",
]);

/** Whether a location may carry a readable slug: a path segment, or a field whose own name says so. */
export function slugOkLocation(loc: string): boolean {
    if (loc.startsWith("url.path[")) return true;
    const leaf = loc.split("|")[0]!.replace(/\[\d+\]/g, "").split(/[.:]/).pop() ?? "";
    return leaf
        .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
        .toLowerCase()
        .split(/[^a-z0-9]+/)
        .some((w) => SLUG_WORDS.has(w));
}

/** A path segment that is plainly not a secret: a word, a slug, a short number, a date or version, or a short prefixed id (`st_12345`, `a1b2c3`). */
const readableSegment = (s: string): boolean =>
    /^[A-Za-z]{3,}$/.test(s) || slugLike(s) || /^\d{1,4}$/.test(s) || /^\d{4}-\d{2}(?:-\d{2})?$/.test(s) || /^v?\d{1,4}(?:\.\d{1,4})+$/.test(s) || /^[a-z]{1,6}_?\d{1,5}$/i.test(s);

/** A host name (`www.shop24.de`): letters, digits and dots ending in a real TLD. */
const HOST = /^(?:[a-z0-9-]+(?:\.[a-z0-9-]+)*\.[a-z]{2,10}|localhost|\d{1,3}(?:\.\d{1,3}){3})$/i;


/** A long mixed blob (digits or both cases, high entropy) regardless of URL/path shape. */
function rawTokenish(p: string, slugOk = false, pathOk = false): boolean {
    if (slugOk && slugLike(p)) return false;
    if (HOST.test(p)) return false; // a bare host name
    if (p.includes("://") || p.startsWith("//")) {
        // A URL: the scheme and a real host are not secrets; the rest (its path, or a host that is not one) is checked, once.
        const m = /^(?:[A-Za-z][A-Za-z0-9+.-]*:)?\/\/([^/?#]*)/.exec(p);
        if (m) p = HOST.test(m[1]!) ? p.slice(m[0].length) : p.slice(m[0].length - m[1]!.length);
        if (p.length < 2) return false;
    }
    const segs = p.split("/").filter(Boolean);
    // A readable path (`/api/v2/carts/`): every segment is a word, a slug, a number or a version token.
    if ((slugOk || pathOk) && segs.length >= 2 && segs.every(readableSegment)) return false;
    return p.length >= 12 && distinctCount(p) >= 6 && entropyBitsPerChar(p) >= 3.0 && /\d/.test(p);
}

/** Whether a value contains a token-grade word (an id or nonce), even inside a path or URL. */
export function hasTokenPiece(value: string): boolean {
    return value.split(/[^A-Za-z0-9_-]+/).some((w) => w.length > 0 && tokenGrade(w));
}

/** Text that is, or contains, something secret-looking: a token-grade piece, a card number or personal data. */
export function secretish(text: string, slugOk = false, pathOk = false): boolean {
    if (cardIn(text) || piiish(text)) return true;
    const words = text.split(/[^A-Za-z0-9_-]+/);
    const blobs = text.split(/[\s"'<>&;,(){}[\]|\\`=:]+/);
    return words.some((p) => p.length > 0 && tokenGrade(p, slugOk)) || blobs.some((p) => p.length > 0 && (tokenGrade(p, slugOk) || rawTokenish(p, slugOk, pathOk)));
}

/** Personal data by shape: an email address (linear scan, no backtracking) or a US social security number. */
export function piiish(value: string): boolean {
    let from = 0;
    for (let n = 0; n < 64; n++) {
        const at = value.indexOf("@", from);
        if (at < 0) break;
        from = at + 1;
        let l = at;
        while (l > 0 && at - l < 256 && !/\s/.test(value[l - 1]!) && value[l - 1] !== "@") l--;
        let r = at + 1;
        while (r < value.length && r - at < 256 && !/\s/.test(value[r]!) && value[r] !== "@") r++;
        const domain = value.slice(at + 1, r);
        if (l < at && /[^.]\.[^.]/.test(domain)) return true;
    }
    return /(?:^|\D)\d{3}-\d{2}-\d{4}(?:\D|$)/.test(value);
}

const decode = (s: string): string => {
    try {
        return decodeURIComponent(s);
    } catch {
        return s;
    }
};

/**
 * `name=value` pairs (URL query, fragment, `;` or form body) and `"name":"value"` pairs (JSON text)
 * anywhere in a value, with a sensitive name or a secret-looking value. Names are percent-decoded.
 */
export function urlHasSecret(value: string): boolean {
    if (value.startsWith("http://") || value.startsWith("https://")) {
        try {
            const u = new URL(value);
            if (u.username || u.password) return true;
            if (u.pathname.split("/").some((seg) => looksLikeCard(seg))) return true;
        } catch {
            /* fall through to the textual scan */
        }
    }
    if (value.length > 8192) return false;
    for (const m of value.matchAll(/(?:^|[?&#;])([^=&#;\s]+)=([^&#;\s]*)/g)) {
        if (sensitiveLocation(decode(m[1]!)) || secretish(decode(m[2]!))) return true;
    }
    for (const m of value.matchAll(/["']([^"'\\]{1,64})["']\s*:\s*(?:"((?:[^"\\]|\\.){0,256})"|'((?:[^'\\]|\\.){0,256})'|(\d{1,32}))/g)) {
        if (sensitiveLocation(m[1]!) || secretish(m[2] ?? m[3] ?? m[4] ?? "")) return true;
    }
    return false;
}

/** Headers whose values are descriptive, not credentials: only URL/card/PII checks apply to them. */
const BENIGN_HEADER = /^header:content-type(#\d+)?$/;
/** Descriptive headers whose values are MIME types, language tags and directives. */
const DESCRIPTIVE_HEADER = /^header:(accept|accept-encoding|accept-language|cache-control|sec-ch-[a-z-]+|sec-fetch-[a-z-]+)(#\d+)?$/;
/** A browser product token (`Mozilla/5.0 (...) ...`, `curl/8.4.0`) in a user agent. */
const PRODUCT_TOKEN = /^[A-Za-z][\w.-]*\/\d[\w.]*(?:\s|$)/;
/** A piece of a descriptive header value: short, a MIME type, a `product/1.2` token or `name=number`. */
const descriptivePiece = (w: string): boolean =>
    w.length < 16 || /^[a-z-]+\/[a-z0-9+.*-]+$/i.test(w) || /^[A-Za-z][\w.-]*\/[\d.]+$/.test(w) || /^[a-z-]+=\d+(?:\.\d+)?$/i.test(w);
const noTokenWords = (value: string): boolean =>
    value.split(/[\s,;]+/).every(descriptivePiece) &&
    !value.split(/[^A-Za-z0-9_-]+/).some((w) => w.length > 0 && tokenGrade(w)) &&
    !/[A-Za-z0-9]{3,}_[A-Za-z0-9_]{6,}/.test(value);
const benignValue = (loc: string, value: string): boolean =>
    BENIGN_HEADER.test(loc) ||
    (DESCRIPTIVE_HEADER.test(loc) && noTokenWords(value) && !cardIn(value) && !piiish(value)) ||
    (/^header:user-agent(#\d+)?$/.test(loc) && PRODUCT_TOKEN.test(value) && !value.split(/[^A-Za-z0-9_-]+/).some((w) => w.length > 0 && tokenGrade(w)) && !/[A-Za-z0-9]{3,}_[A-Za-z0-9_]{6,}/.test(value));

/** Whether a constant may not be written to the graph: by location name, or by what the value looks like. */
export function redactConstant(loc: string, value: string): boolean {
    if (sensitiveLocation(loc) || looksLikeCard(value) || cardIn(value) || piiish(value) || urlHasSecret(value)) return true;
    const urlish = /^(?:https?:)?\/\//.test(value);
    return !benignValue(loc, value) && secretish(value, slugOkLocation(loc), slugOkLocation(loc) || urlish);
}

const CALLS = /(?<![\w-])[A-Za-z_][\w-]*\s*\(/g;
/** CSS structural pseudo-classes whose short argument is kept (`:nth-child(2)`). */
const KEEP_PSEUDO = /:(?:nth-child|nth-of-type|nth-last-child|nth-last-of-type|not|is|where)$/;

/**
 * Replace the arguments of calls in a selector or action text (`fill("x")`, `t.typeText('#pw', 'x')`) with `…`, in one
 * pass; only CSS pseudo-classes with a short plain argument (`:nth-child(2)`) keep theirs.
 * An unclosed call drops the rest.
 */
function scrubCalls(text: string): string {
    let out = "";
    let last = 0;
    CALLS.lastIndex = 0;
    for (let m = CALLS.exec(text); m !== null; m = CALLS.exec(text)) {
        const open = m.index + m[0].length;
        let depth = 1;
        let quote = "";
        let quoted = false;
        let i = open;
        for (; i < text.length && depth > 0; i++) {
            const c = text[i]!;
            if (quote) {
                if (c === "\\") i++;
                else if (c === quote) quote = "";
            } else if (c === '"' || c === "'" || c === "`") {
                quote = c;
                quoted = true;
            } else if (c === "(") depth++;
            else if (c === ")") depth--;
        }
        const body = text.slice(open, depth === 0 ? i - 1 : i);
        if (depth > 0) return `${out}${text.slice(last, open)}…`;
        // Only a CSS pseudo-class with a short plain argument (`:nth-child(2n+1)`) is kept.
        const pseudo = KEEP_PSEUDO.test(text.slice(Math.max(0, m.index - 20), m.index + m[0].replace(/\s*\($/, "").length)) && !quoted && body.length < 16 && !/[A-Za-z]{4,}/.test(body);
        if (!pseudo) {
            out += `${text.slice(last, open)}…)`;
            last = i;
        }
        CALLS.lastIndex = Math.max(i, open);
    }
    return out + text.slice(last);
}

/** A trace selector or label with attribute values and typed text removed, or withheld if it still looks secret. */
export function safeText(text: string): string {
    const cleaned = scrubCalls(text.slice(0, 8192))
        .replace(/(\[?\bvalue\s*=\s*)(?:"[^"]{0,256}"|'[^']{0,256}'|[^\]\s]{0,256})/gi, "$1…")
        .replace(/\b(text|type|value|label)=\S{0,256}/gi, "$1=…");
    return secretish(cleaned) ? "<redacted-text>" : cleaned.slice(0, 1024);
}

/** A location that embeds a token-grade key (a session id used as a JSON key or query name). */
export function safeLocation(loc: string): string {
    return piiish(loc) || cardIn(loc.replace(/\d+/g, (d) => (d.startsWith("1") && looksEpoch(d) ? "x" : d))) || loc.split(/[^A-Za-z0-9_-]+/).some((p) => p.length > 0 && tokenGrade(p)) ? "<redacted-location>" : loc;
}
