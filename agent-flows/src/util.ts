import { createHash } from "node:crypto";

/** Code-unit string comparison: the one ordering used for every sort that reaches output. */
export function cmp(a: string, b: string): number {
    return a < b ? -1 : a > b ? 1 : 0;
}

export function cmpTuple(a: readonly (string | number | null)[], b: readonly (string | number | null)[]): number {
    for (let i = 0; i < Math.max(a.length, b.length); i++) {
        const x = a[i] ?? null;
        const y = b[i] ?? null;
        if (x === y) continue;
        if (x === null) return -1;
        if (y === null) return 1;
        if (typeof x === "number" && typeof y === "number") return x - y;
        return cmp(String(x), String(y));
    }
    return 0;
}

export function sha256Hex(s: string): string {
    return createHash("sha256").update(s, "utf8").digest("hex");
}

const utf8 = new TextDecoder("utf-8");
const utf8Strict = new TextDecoder("utf-8", { fatal: true });

function percentBytes(s: string): Uint8Array {
    const out: number[] = [];
    const enc = new TextEncoder();
    for (let i = 0; i < s.length; i++) {
        const c = s.charCodeAt(i);
        if (c === 0x25 && i + 2 < s.length && /^[0-9a-fA-F]{2}$/.test(s.slice(i + 1, i + 3))) {
            out.push(parseInt(s.slice(i + 1, i + 3), 16));
            i += 2;
        } else {
            // Copy the code point (handles surrogate pairs) as UTF-8.
            const cp = s.codePointAt(i)!;
            if (cp > 0xffff) i++;
            out.push(...enc.encode(String.fromCodePoint(cp)));
        }
    }
    return Uint8Array.from(out);
}

/** Percent-decoding with replacement characters for invalid UTF-8. */
export function decodeComponent(s: string): string {
    return utf8.decode(percentBytes(s));
}

/** Strict percent-decoding: `null` when the result is not valid UTF-8. */
export function decodeComponentStrict(s: string): string | null {
    try {
        return utf8Strict.decode(percentBytes(s));
    } catch {
        return null;
    }
}

export function codePointLength(s: string): number {
    let n = 0;
    for (const _ of s) n++;
    return n;
}
