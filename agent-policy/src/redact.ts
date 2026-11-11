import { b64urlEncode, utf8 } from "./bytes.ts";

export const REDACTED = "[REDACTED]";

const SENSITIVE_KEY = /(pass(word|wd)?|secret|token|api[-_]?key|authorization|auth|cookie|set-cookie|session|csrf|xsrf|credential|private[-_]?key|signature|card|cvv|cvc|pan|ssn|bearer)/i;

function luhn(digits: string): boolean {
    let sum = 0;
    let alt = false;
    for (let i = digits.length - 1; i >= 0; i--) {
        let d = digits.charCodeAt(i) - 48;
        if (alt) {
            d *= 2;
            if (d > 9) d -= 9;
        }
        sum += d;
        alt = !alt;
    }
    return sum % 10 === 0;
}

interface PatternRule {
    readonly re: RegExp;
    readonly replace?: (m: string, ...g: string[]) => string;
}

const PATTERNS: readonly PatternRule[] = [
    // Header-shaped credentials: everything to the end of the line (scheme words, several cookies).
    { re: /(\b(?:set-cookie|cookie|proxy-authorization|authorization)\s*[:=]\s*)(?!["'])[^\r\n]+/gi, replace: (_m, pre) => `${pre}${REDACTED}` },
    { re: /\b(Bearer|Basic|Digest)\s+[A-Za-z0-9._~+/=-]{6,}/gi, replace: (_m, scheme) => `${scheme} ${REDACTED}` },
    { re: /\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*/g },
    { re: /\b(?:sk|rk|pk_live|whsec|xox[abprs])[-_](?:live|test)?[-_]?[A-Za-z0-9]{12,}/g },
    { re: /\bsk_(?:live|test)_[A-Za-z0-9]{8,}/g },
    { re: /\bAKIA[0-9A-Z]{16}\b/g },
    { re: /\bgh[pousr]_[A-Za-z0-9]{30,}/g },
    { re: /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)/g },
    // key=value, key: value, "key":"value" for sensitive-looking keys
    {
        re: /(["']?\b[\w-]*(?:pass(?:word|wd)?|secret|token|api[-_]?key|authorization|cookie|session(?:id)?|csrf|xsrf|credential|signature)[\w-]*["']?\s*[:=]\s*)("(?:[^"\\]|\\.)*"|'[^']*'|[^\s&;,}"']+)/gi,
        replace: (_m, pre) => `${pre}${REDACTED}`,
    },
];

const CARD = /\b(?:\d[ -]?){12,18}\d\b/g;

/** Variants of one secret as it might appear after ordinary encoding. */
function variants(secret: string): string[] {
    const out = new Set<string>([secret]);
    const enc = encodeURIComponent(secret);
    out.add(enc);
    out.add(enc.replace(/%[0-9A-F]{2}/g, (m) => m.toLowerCase())); // lowercase %xx
    out.add(enc.replaceAll("%20", "+")); // application/x-www-form-urlencoded
    out.add(enc.replaceAll("%20", "+").replace(/%[0-9A-F]{2}/g, (m) => m.toLowerCase()));
    out.add(encodeURI(secret));
    out.add(JSON.stringify(secret).slice(1, -1));
    out.add(JSON.stringify(secret).slice(1, -1).replaceAll("/", "\\/"));
    const bytes = utf8(secret);
    out.add(Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(""));
    out.add(Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("").toUpperCase());
    // base64 at each of the three alignments the secret can have inside a larger string
    for (let k = 0; k < 3; k++) {
        const padded = new Uint8Array(k + bytes.length);
        padded.set(bytes, k);
        const std = btoa(String.fromCharCode(...padded)).replaceAll("=", "");
        const start = Math.ceil((8 * k) / 6);
        const end = Math.floor((8 * (k + bytes.length)) / 6);
        const core = std.slice(start, end);
        if (core.length >= 6) {
            out.add(core);
            out.add(core.replaceAll("+", "-").replaceAll("/", "_"));
        }
    }
    out.add(b64urlEncode(bytes));
    out.add(btoa(String.fromCharCode(...bytes)));
    return [...out].filter((v) => v.length >= 4).sort((a, b) => b.length - a.length);
}

/**
 * Holds the run-time secrets (credentials, tokens, canaries) and removes them, in their
 * common encodings, plus well-known secret shapes, from any string or data structure.
 * Registered secrets are never stored except inside this object.
 */
export class Redactor {
    #vars: string[] = [];

    register(secret: string): void {
        if (typeof secret !== "string" || secret.length < 4) throw new RangeError("secret too short to redact safely");
        this.#vars = [...new Set([...this.#vars, ...variants(secret)])].sort((a, b) => b.length - a.length);
    }

    redactString(input: string): string {
        let s = input;
        for (let pass = 0; pass < 2; pass++) {
            for (const v of this.#vars) if (s.includes(v)) s = s.split(v).join(REDACTED);
            for (const p of PATTERNS) s = s.replace(p.re, (...a: unknown[]) => (p.replace ? p.replace(...(a as [string, ...string[]])) : REDACTED));
            s = s.replace(CARD, (m) => {
                const d = m.replace(/[ -]/g, "");
                return d.length >= 13 && d.length <= 19 && luhn(d) ? REDACTED : m;
            });
        }
        return s;
    }

    /** Deep copy with every string redacted and sensitive-named keys blanked. Cycle-safe. */
    redactDeep(value: unknown, seen: WeakSet<object> = new WeakSet()): unknown {
        if (typeof value === "string") return this.redactString(value);
        if (typeof value === "number" || typeof value === "bigint") {
            // A secret or card number can travel as a number: judge its decimal text.
            const text = String(value);
            return this.redactString(text) === text ? value : REDACTED;
        }
        if (value === null || typeof value !== "object") {
            return typeof value === "function" || typeof value === "symbol" ? REDACTED : value;
        }
        // Raw bytes may hold a secret in any encoding: never emit them.
        if (ArrayBuffer.isView(value) || value instanceof ArrayBuffer) return REDACTED;
        if (seen.has(value)) return "[Circular]";
        seen.add(value);
        if (value instanceof Error) return this.errorToJson(value);
        if (Array.isArray(value)) return value.map((v) => this.redactDeep(v, seen));
        const out: Record<string, unknown> = {};
        for (const [k, v] of Object.entries(value)) {
            out[this.redactString(k)] = SENSITIVE_KEY.test(k) && v !== null && v !== undefined && v !== "" ? REDACTED : this.redactDeep(v, seen);
        }
        return out;
    }

    errorToJson(e: unknown): { name: string; message: string } {
        if (e instanceof Error) return { name: this.redactString(e.name), message: this.redactString(e.message).slice(0, 500) };
        return { name: "NonError", message: this.redactString(String(e)).slice(0, 500) };
    }
}

export interface Logger {
    log(level: "debug" | "info" | "warn" | "error", message: string, fields?: Record<string, unknown>): void;
}

/** Every line passes through the redactor; a sink that throws never leaks the input. */
export function createLogger(redactor: Redactor, sink: (line: string) => void): Logger {
    return {
        log(level, message, fields) {
            const rec = redactor.redactDeep({ level, message, ...(fields ?? {}) });
            try {
                sink(JSON.stringify(rec));
            } catch {
                /* a failing sink must not surface the original values */
            }
        },
    };
}
