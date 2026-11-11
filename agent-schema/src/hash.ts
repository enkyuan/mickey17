// Content addressing (D4). Hash = sha256 over the RFC 8785 (JCS) canonical JSON of the
// document with its `hash` field omitted, written `sha256:<hex>`. The wire format has no
// floats, so JCS reduces to: UTF-8, keys sorted by UTF-16 code unit, no whitespace,
// integers in shortest decimal form. The Rust emitter must produce the same bytes.
import { createHash } from "node:crypto";

export type Json = null | boolean | number | string | Json[] | { [k: string]: Json };

export function canonicalize(value: unknown): string {
    if (value === null) return "null";
    switch (typeof value) {
        case "boolean":
            return value ? "true" : "false";
        case "number":
            if (!Number.isFinite(value)) throw new Error("non-finite number in canonical JSON");
            if (!Number.isInteger(value)) throw new Error("non-integer number in canonical JSON: the wire format has no floats");
            return JSON.stringify(value);
        case "string":
            if (!value.isWellFormed()) throw new Error("ill-formed string (lone surrogate) in canonical JSON");
            return JSON.stringify(value);
        case "object": {
            if (Array.isArray(value)) return `[${value.map(canonicalize).join(",")}]`;
            const obj = value as Record<string, unknown>;
            const keys = Object.keys(obj)
                .filter((k) => obj[k] !== undefined)
                .sort();
            return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalize(obj[k])}`).join(",")}}`;
        }
        default:
            throw new Error(`unsupported value in canonical JSON: ${typeof value}`);
    }
}

export const sha256Hex = (text: string): string => createHash("sha256").update(text, "utf8").digest("hex");

/** Hash of a document with its `hash` field omitted. */
export function computeHash(doc: { hash: string }): string {
    const { hash: _omitted, ...rest } = doc;
    return `sha256:${sha256Hex(canonicalize(rest))}`;
}

/** Return the document with a correct `hash`. */
export function withHash<T extends { hash: string }>(doc: T): T {
    return { ...doc, hash: computeHash(doc) };
}
