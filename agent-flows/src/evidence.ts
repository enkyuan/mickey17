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
    "csrf", "xsrf", "apikey", "api-key", "api_key", "privatekey", "private-key", "private_key", "bearer",
];
/** Short names matched only as whole words (`pin` must not redact `shipping`). */
const SENSITIVE_WORDS = new Set(["pwd", "pass", "pin", "otp", "jwt", "auth", "card", "cvc", "cvv", "iban", "ssn", "sig", "body"]);

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

/** Text that is, or contains, something secret-looking: a token-grade piece or a card number. */
export function secretish(text: string): boolean {
    if (looksLikeCard(text)) return true;
    const tokenGrade = (p: string) => isEvidence(p) && /\d/.test(p);
    const words = text.split(/[^A-Za-z0-9_-]+/);
    const blobs = text.split(/[\s"'<>&;,(){}[\]|\\`=:]+/);
    return [...words, ...blobs].some((p) => p.length > 0 && (tokenGrade(p) || looksLikeCard(p)));
}

/** An absolute URL whose query or path carries a sensitive parameter name or a secret-looking value. */
export function urlHasSecret(value: string): boolean {
    if (!value.startsWith("http://") && !value.startsWith("https://")) return false;
    let u: URL;
    try {
        u = new URL(value);
    } catch {
        return false;
    }
    if (u.username || u.password) return true;
    for (const [k, v] of u.searchParams) if (sensitiveLocation(k) || secretish(v)) return true;
    return u.pathname.split("/").some((seg) => looksLikeCard(seg));
}
