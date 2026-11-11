import { describe, expect, test } from "bun:test";
import { isEvidence, looksLikeCard } from "../src/evidence.ts";
import { parseIsoUs, parseMultipart } from "../src/har.ts";
import { parseJson } from "../src/json.ts";
import { apply, preimages, relations, relVia } from "../src/matcher.ts";

describe("evidence rule", () => {
    test("rejects common and short values", () => {
        for (const v of [
            "1", "0", "true", "false", "null", "USD", "en-US", "42", "1042", "2026-11-09", "2026-11-09T10:00:00.000Z",
            "application/json", "https://shop.test/cart", "/api/cart", "checkout", "aaaaaaaaaaaa", "12345678", "1794218400",
            "1794218400250", "Mon, 09 Nov 2026 10:00:00 GMT",
        ]) {
            expect(isEvidence(v), `\`${v}\` must be non-evidence`).toBe(false);
        }
    });
    test("accepts tokens", () => {
        for (const v of ["c_8f2Kq9ZxLm41ab", "3f9a1c77b2d44e0a8c5d", "550e8400-e29b-41d4-a716-446655440000", "pi_3Hxyz123_secret_ABCdef456", "123456789012345"]) {
            expect(isEvidence(v), `\`${v}\` must be evidence`).toBe(true);
        }
    });
    test("luhn", () => {
        expect(looksLikeCard("4242 4242 4242 4242")).toBe(true);
        expect(looksLikeCard("4242 4242 4242 4241")).toBe(false);
        expect(looksLikeCard("not a card")).toBe(false);
    });
});

describe("matcher", () => {
    test("exact and encoded", () => {
        const s = "tok/with+special=chars&more";
        expect(relations(apply("UrlEncode", s)!, s).some((r) => r.transform === "UrlEncode" && !r.prefix && !r.suffix)).toBe(true);
        expect(relations(Buffer.from(s).toString("base64"), s).some((r) => r.transform === "Base64")).toBe(true);
    });
    test("form encoding uses + and escapes !'()~", () => {
        expect(apply("FormEncode", "a b!c~")).toBe("a+b%21c%7E");
        expect(apply("UrlEncode", "a b!c~")).toBe("a%20b!c~");
    });
    test("base64 decoding is strict", () => {
        expect(apply("Base64Decode", Buffer.from("hello world token").toString("base64"))).toBe("hello world token");
        expect(apply("Base64Decode", "3f9a1c77b2d44e0a8c5d")).toBeNull();
    });
    test("bearer prefix is a substring relation", () => {
        const r = relations("Bearer 3f9a1c77b2d44e0a8c5d", "3f9a1c77b2d44e0a8c5d");
        expect(r).toHaveLength(1);
        expect([r[0]!.prefix, relVia(r[0]!)]).toEqual(["Bearer ", "substring"]);
    });
    test("short values are never related", () => {
        expect(relations("Bearer 1", "1")).toEqual([]);
        expect(relations("true", "true")).toEqual([]);
    });
    test("preimages invert encoders", () => {
        const s = "a token/with+specials=ok!";
        expect(preimages(apply("UrlEncode", s)!).some(([, p]) => p === s)).toBe(true);
    });
});

describe("har helpers", () => {
    test("iso parses known instants", () => {
        expect(parseIsoUs("1970-01-01T00:00:00Z")).toBe(0);
        expect(parseIsoUs("2026-11-09T10:00:00.250Z")).toBe(1_794_218_400_250_000);
        expect(parseIsoUs("2026-11-09T11:00:00.250+01:00")).toBe(parseIsoUs("2026-11-09T10:00:00.250Z"));
        expect(parseIsoUs("not a date")).toBeNull();
    });
    test("multipart names are not confused with filenames", () => {
        const text =
            '--b\r\nContent-Disposition: form-data; name="f"; filename="a.txt"\r\n\r\nhello\r\n--b\r\nContent-Disposition: form-data; name="k"\r\n\r\nv\r\n--b--\r\n';
        const parts = parseMultipart("multipart/form-data; boundary=b", text);
        expect(parts.map((p) => [p.name, p.filename, p.value])).toEqual([
            ["f", "a.txt", "hello"],
            ["k", null, "v"],
        ]);
    });
});

describe("json parser", () => {
    test("keeps number text and rejects hostile nesting", () => {
        const v = parseJson('{"a": 12345678901234567890, "b": 1.0}');
        expect(v.t === "obj" && v.entries.get("a")).toEqual({ t: "num", raw: "12345678901234567890" });
        expect(() => parseJson("[".repeat(10_000) + "]".repeat(10_000))).toThrow();
        expect(() => parseJson('{"a": }')).toThrow();
    });
});
