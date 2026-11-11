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

/** Luhn-valid 13 to 19 digit numbers (payment card numbers, possibly with spaces or dashes). */
export function looksLikeCard(v: string): boolean {
    if (!/^[0-9 -]+$/.test(v)) return false;
    const digits = [...v].filter((c) => c >= "0" && c <= "9").map(Number);
    if (digits.length < 13 || digits.length > 19) return false;
    let sum = 0;
    digits.reverse().forEach((d, i) => {
        if (i % 2 === 1) {
            const x = d * 2;
            sum += x > 9 ? x - 9 : x;
        } else sum += d;
    });
    return sum % 10 === 0;
}

/** Long names matched anywhere in a location. */
const SENSITIVE_SUBSTRINGS = [
    "authorization", "cookie", "token", "secret", "password", "passwd", "passphrase", "credential", "signature", "session",
    "csrf", "xsrf", "apikey", "api-key", "api_key", "privatekey", "private-key", "private_key", "bearer", "hmac", "passcode", "pincode",
    "passwort", "kennwort", "contrasena", "senha", "motdepasse", "mot_de_passe", "mot-de-passe",
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
    if (SENSITIVE_SUBSTRINGS.some((k) => l.includes(k))) return true;
    const words = loc
        .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
        .toLowerCase()
        .split(/[^a-z0-9]+/);
    return words.some((w) => SENSITIVE_WORDS.has(w));
}

/**
 * A card number anywhere in the text: any 13 to 19 digit window of a digit run (single spaces or
 * dashes between digits allowed) that passes the Luhn check, so an expiry date glued to the number
 * does not hide it.
 */
function cardIn(text: string): boolean {
    const head = text.length > 16_384 ? text.slice(0, 16_384) : text;
    let runs = 0;
    for (const m of head.matchAll(/\d(?:[ ._/-]{0,2}\d){12,63}/g)) {
        if (++runs > 64) break;
        const digits = m[0].replace(/\D/g, "").slice(0, 96);
        for (let s = 0; s + 13 <= digits.length; s++) {
            for (let l = 13; l <= 19 && s + l <= digits.length; l++) if (looksLikeCard(digits.slice(s, s + l))) return true;
        }
    }
    return false;
}

const distinctCount = (s: string): number => new Set(s).size;

/**
 * A token-grade word: a long unbroken run that is not prose. With a digit, 14+ characters; without
 * one, 20+. Words and short numbers joined by `-`/`_` (`blue-widget-2024`, `WIDGET-2024-XL`) are
 * exempt only where `slugOk` (a location that names a product, path or label): the same shape under any
 * other name may be a passphrase or a licence key.
 */
function tokenGrade(p: string, slugOk = false): boolean {
    if (!isEvidence(p)) return false;
    if (slugOk && slugLike(p)) return false;
    return /\d/.test(p) ? p.length >= 14 : p.length >= 20;
}

function slugLike(p: string): boolean {
    const parts = p.split(/[-_]/);
    return parts.length >= 2 && parts.every((w) => /^[A-Za-z]{1,12}$/.test(w) || /^\d{1,6}$/.test(w));
}

/** Locations that legitimately carry readable slugs and labels. */
const SLUG_OK_LOCATION = /sku|slug|handle|product|variant|category|collection|referer|href|url|location|path\[|title|label|name|route|page/i;


/** A long mixed blob (digits or both cases, high entropy) regardless of URL/path shape. */
function rawTokenish(p: string, slugOk = false): boolean {
    if (slugOk && slugLike(p)) return false;
    if (p.includes("://")) return false; // a URL: its pieces and parameters are checked separately
    const segs = p.split("/").filter(Boolean);
    if (slugOk && segs.length >= 2 && segs.filter((s) => /^[A-Za-z]{3,}$/.test(s) || slugLike(s)).length * 2 >= segs.length) return false; // a readable path
    return p.length >= 12 && distinctCount(p) >= 6 && entropyBitsPerChar(p) >= 3.0 && /\d/.test(p);
}

/** Text that is, or contains, something secret-looking: a token-grade piece, a card number or personal data. */
export function secretish(text: string, slugOk = false): boolean {
    if (cardIn(text) || piiish(text)) return true;
    const words = text.split(/[^A-Za-z0-9_-]+/);
    const blobs = text.split(/[\s"'<>&;,(){}[\]|\\`=:]+/);
    return words.some((p) => p.length > 0 && tokenGrade(p, slugOk)) || blobs.some((p) => p.length > 0 && (tokenGrade(p, slugOk) || rawTokenish(p, slugOk)));
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
const BENIGN_HEADER = /^header:(content-type|accept|accept-encoding|accept-language|cache-control|connection|dnt|upgrade-insecure-requests|pragma|sec-ch-[a-z-]+|sec-fetch-[a-z-]+)(#\d+)?$/;
/** A browser product token (`Mozilla/5.0 (...) ...`, `curl/8.4.0`) in a user agent. */
const PRODUCT_TOKEN = /^[A-Za-z][\w.-]*\/\d[\w.]*(?:\s|$)/;
const benignValue = (loc: string, value: string): boolean =>
    BENIGN_HEADER.test(loc) || (/^header:user-agent(#\d+)?$/.test(loc) && PRODUCT_TOKEN.test(value));

/** Whether a constant may not be written to the graph: by location name, or by what the value looks like. */
export function redactConstant(loc: string, value: string): boolean {
    if (sensitiveLocation(loc) || looksLikeCard(value) || cardIn(value) || piiish(value) || urlHasSecret(value)) return true;
    return !benignValue(loc, value) && secretish(value, SLUG_OK_LOCATION.test(loc));
}

/** A trace selector or label with attribute values removed (`[value="x"]` -> `[value=…]`), or withheld if it still looks secret. */
export function safeText(text: string): string {
    const cleaned = text
        .replace(/(\[?\bvalue\s*=\s*)(?:"[^"]*"|'[^']*'|[^\]\s]+)/gi, "$1…")
        .replace(/\b(fill|type|press|check|select_?option|sendKeys)\([^)]*\)/gi, "$1(…)")
        .replace(/\btext=\S+/gi, "text=…");
    return secretish(cleaned) ? "<redacted-text>" : cleaned;
}

/** A location that embeds a token-grade key (a session id used as a JSON key or query name). */
export function safeLocation(loc: string): string {
    return piiish(loc) || loc.split(/[^A-Za-z0-9_-]+/).some((p) => p.length > 0 && tokenGrade(p)) ? "<redacted-location>" : loc;
}
