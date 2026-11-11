const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

export function utf8(s: string): Uint8Array {
    return encoder.encode(s);
}

export function fromUtf8(b: Uint8Array): string | null {
    try {
        return decoder.decode(b);
    } catch {
        return null;
    }
}

const B64URL = /^[A-Za-z0-9_-]*$/;

export function b64urlEncode(bytes: Uint8Array): string {
    let bin = "";
    for (const b of bytes) bin += String.fromCharCode(b);
    return btoa(bin).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

/** Strict: URL-safe alphabet, no padding, and only the canonical encoding of the bytes. */
export function b64urlDecode(s: string): Uint8Array | null {
    if (!B64URL.test(s) || s.length % 4 === 1) return null;
    try {
        const std = s.replaceAll("-", "+").replaceAll("_", "/");
        const bin = atob(std + "=".repeat((4 - (std.length % 4)) % 4));
        const out = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
        return b64urlEncode(out) === s ? out : null;
    } catch {
        return null;
    }
}

/**
 * Compares two byte strings without an early exit on the first differing byte, and
 * folds the length difference into the result. JavaScript cannot promise hardware
 * constant time; this removes the data-dependent branch, which is what is in our
 * control. Use it for anything secret-derived (MACs, token digests, idempotency keys).
 */
export function constantTimeEqual(a: Uint8Array | string, b: Uint8Array | string): boolean {
    const x = typeof a === "string" ? utf8(a) : a;
    const y = typeof b === "string" ? utf8(b) : b;
    let diff = x.length ^ y.length;
    const n = Math.max(x.length, y.length);
    for (let i = 0; i < n; i++) diff |= (x[i] ?? 0) ^ (y[i] ?? 0);
    return diff === 0;
}

export async function sha256Hex(data: string | Uint8Array): Promise<string> {
    const bytes = typeof data === "string" ? utf8(data) : data;
    const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes as Uint8Array<ArrayBuffer>));
    let out = "";
    for (const b of digest) out += b.toString(16).padStart(2, "0");
    return out;
}

/** Deterministic JSON: sorted keys, finite numbers only, no undefined, plain data only. */
export function canonicalJson(v: unknown): string {
    if (v === null) return "null";
    switch (typeof v) {
        case "string":
        case "boolean":
            return JSON.stringify(v);
        case "number":
            if (!Number.isFinite(v)) throw new TypeError("non-finite number");
            return JSON.stringify(v);
        case "object": {
            if (Array.isArray(v)) return `[${v.map(canonicalJson).join(",")}]`;
            const proto = Object.getPrototypeOf(v);
            if (proto !== Object.prototype && proto !== null) throw new TypeError("non-plain object");
            const keys = Object.keys(v as object).sort();
            const parts: string[] = [];
            for (const k of keys) {
                const val = (v as Record<string, unknown>)[k];
                if (val === undefined) throw new TypeError("undefined value");
                parts.push(`${JSON.stringify(k)}:${canonicalJson(val)}`);
            }
            return `{${parts.join(",")}}`;
        }
        default:
            throw new TypeError(`unsupported type ${typeof v}`);
    }
}

export function isSafeNonNegInt(v: unknown): v is number {
    return typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
}
export function isSafePosInt(v: unknown): v is number {
    return typeof v === "number" && Number.isSafeInteger(v) && v > 0;
}
