import { describe, expect, test } from "bun:test";
import { canonicalize, computeHash, sha256Hex, withHash } from "../src/index";
import { shop } from "../scripts/fixtures";

describe("canonical form and content hash", () => {
    test("keys are sorted and whitespace is absent", () => {
        expect(canonicalize({ b: 1, a: [true, null, "é"] })).toBe('{"a":[true,null,"é"],"b":1}');
    });
    test("key order does not change the hash input", () => {
        expect(canonicalize({ x: 1, y: { b: 2, a: 1 } })).toBe(canonicalize({ y: { a: 1, b: 2 }, x: 1 }));
    });
    test("undefined fields are omitted like absent fields", () => {
        expect(canonicalize({ a: 1, b: undefined })).toBe('{"a":1}');
    });
    test("floats are rejected: the wire format has none", () => {
        expect(() => canonicalize({ a: 0.5 })).toThrow();
        expect(() => canonicalize({ a: Number.NaN })).toThrow();
    });
    test("known vector: sha256 of {}", () => {
        expect(sha256Hex(canonicalize({}))).toBe("44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a");
    });
    test("the hash field is excluded from its own input", () => {
        const a = computeHash({ ...shop, hash: "sha256:" + "0".repeat(64) });
        const b = computeHash({ ...shop, hash: "sha256:" + "f".repeat(64) });
        expect(a).toBe(b);
        expect(withHash(shop).hash).toBe(a);
    });
    test("any content change changes the hash", () => {
        const changed = { ...shop, site: { ...shop.site, host: "other.example.test" } };
        expect(computeHash(changed)).not.toBe(shop.hash);
    });
});
