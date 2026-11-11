import fc from "fast-check";
import { describe, expect, test } from "bun:test";
import { b64urlDecode, b64urlEncode, canonicalJson } from "../src/bytes.ts";
import { issueConfirmation, verifyConfirmation, type ConfirmationBinding } from "../src/confirm.ts";
import { compileEgressPolicy, evaluateEgress } from "../src/egress.ts";
import { classifyAddress } from "../src/ip.ts";
import { InMemorySpendLedger } from "../src/mandate.ts";
import { Redactor } from "../src/redact.ts";
import { patternMatches } from "../src/robots.ts";
import { sanitizePageText } from "../src/sanitize.ts";
import { describeTool, findPageLeaks, KINDS, PARAM_TYPES, surfaceFields, surfaceId, type ToolSpec } from "../src/tooldesc.ts";
import { EFFECTS, TIERS } from "../src/types.ts";
import { pageText, unwrapPageText, wrapPageText } from "../src/wrap.ts";
import { assertWrapInvariants, INVISIBLE_RE } from "./invisible.ts";
import { NOW, world } from "./helpers.ts";

const seed = process.env.FC_SEED ? Number(process.env.FC_SEED) : undefined;
const cfg = (numRuns = 300): fc.Parameters<unknown> => ({ numRuns, ...(seed !== undefined ? { seed } : {}) });

/** Hostile strings: arbitrary code points mixed with the characters that matter. */
const SPECIALS = ["​", "‮", " ", "\r", "\0", "[[/PAGE-CONTENT sha=0]]", "[[PAGE-CONTENT", "```", "<script>", "![a](b)", "\u{e0041}", "ﬁ", "а", "\n| ", "&amp;", "&#91;"];
const hostile = fc.array(fc.oneof(fc.string({ unit: "binary", maxLength: 12 }), fc.constantFrom(...SPECIALS)), { maxLength: 12 }).map((a) => a.join(""));

describe("property: wrapping page text", () => {
    test("any text yields a block whose structure cannot be forged and which round-trips", () => {
        fc.assert(
            fc.property(hostile, (raw) => {
                const p = pageText(raw);
                const block = wrapPageText(p, "node_0123abcd");
                assertWrapInvariants(block);
                expect(unwrapPageText(block)?.text).toBe(p.text);
            }),
            cfg(500),
        );
    });
});

describe("property: sanitising page text", () => {
    test("output never holds invisible characters", () => {
        fc.assert(
            fc.property(hostile, (raw) => {
                expect(sanitizePageText(raw).text.replace(/[\n\t]/g, "")).not.toMatch(INVISIBLE_RE);
            }),
            cfg(500),
        );
    });
    test("idempotent", () => {
        fc.assert(
            fc.property(hostile, (raw) => {
                const once = sanitizePageText(raw).text;
                expect(sanitizePageText(once).text).toBe(once);
            }),
            cfg(500),
        );
    });
    test("respects maxChars", () => {
        fc.assert(
            fc.property(hostile, fc.integer({ min: 0, max: 40 }), (raw, max) => {
                expect(Array.from(sanitizePageText(raw, { maxChars: max }).text).length).toBeLessThanOrEqual(max);
            }),
            cfg(300),
        );
    });
    test("plain ASCII text without markup is unchanged", () => {
        fc.assert(
            fc.property(fc.stringMatching(/^[A-Za-z0-9 .,;:!?'\n-]{0,80}$/), (s) => {
                const r = sanitizePageText(s);
                expect(r.text).toBe(s);
                expect(r.flags).toEqual([]);
            }),
            cfg(200),
        );
    });
});

describe("property: tool descriptions never interpolate page text", () => {
    const sentinel = fc.stringMatching(/^[0-9a-f]{16}$/).map((h) => `PG${h}`);
    const hex = fc.stringMatching(/^[0-9a-f]{8,32}$/);

    test("valid specs with arbitrary page labels: no page string reaches name, description or params", () => {
        fc.assert(
            fc.property(
                sentinel, sentinel, sentinel, hex, fc.constantFrom(...KINDS), fc.constantFrom(...EFFECTS), fc.constantFrom(...TIERS), fc.boolean(),
                fc.array(fc.tuple(hex, fc.constantFrom(...PARAM_TYPES), fc.boolean(), sentinel), { maxLength: 4 }),
                (label, extra, extra2, idHex, kind, effect, tier, idempotent, params) => {
                    const spec: ToolSpec = {
                        id: surfaceId("act", idHex), kind, effect, tier, idempotent,
                        params: params.map(([h, type, required]) => ({ id: surfaceId("param", h), type, required })),
                        pageLabel: pageText(`${label} ${extra}`),
                        paramLabels: Object.fromEntries(params.map(([h, , , s]) => [surfaceId("param", h), pageText(`${s} ${extra2}`)])),
                    };
                    const d = describeTool(spec);
                    const pages = [label, extra, extra2, ...params.map((p) => p[3])];
                    expect(findPageLeaks(surfaceFields(d), pages)).toEqual([]);
                    expect(JSON.stringify(d.pageLabels)).toContain(label); // it is carried, in the marked field only
                },
            ),
            cfg(300),
        );
    });

    test("arbitrary strings in any surface field are rejected, and the error never echoes them", () => {
        fc.assert(
            fc.property(
                sentinel, fc.constantFrom("id", "kind", "effect", "tier", "idempotent", "paramId", "paramType"),
                (s, field) => {
                    const ok: Record<string, unknown> = {
                        id: surfaceId("act", "0123456789abcdef"), kind: "submit", effect: "read", tier: "T2", idempotent: true,
                        params: [{ id: surfaceId("param", "0123456789abcdef"), type: "string", required: true }],
                    };
                    const bad = { ...ok };
                    if (field === "paramId") bad.params = [{ id: s, type: "string", required: true }];
                    else if (field === "paramType") bad.params = [{ id: surfaceId("param", "0123456789abcdef"), type: s, required: true }];
                    else bad[field] = s;
                    try {
                        const d = describeTool(bad as unknown as ToolSpec);
                        // `idempotent` is boolean-checked: a string must have been refused
                        throw new Error(`accepted ${field}: ${JSON.stringify(d).slice(0, 80)}`);
                    } catch (e) {
                        expect(e).toBeInstanceOf(TypeError);
                        expect((e as Error).message.includes(s)).toBe(false);
                    }
                },
            ),
            cfg(300),
        );
    });

    test("prototype-pollution style keys are not vocabulary", () => {
        for (const k of ["__proto__", "constructor", "toString", "hasOwnProperty"]) {
            expect(() => describeTool({ id: surfaceId("act", "0123456789abcdef"), kind: k as never, effect: "read", tier: "T2", idempotent: true, params: [] })).toThrow();
            expect(() => describeTool({ id: surfaceId("act", "0123456789abcdef"), kind: "submit", effect: k as never, tier: "T2", idempotent: true, params: [] })).toThrow();
        }
    });
});

describe("property: SSRF classification", () => {
    // Reference written independently, with explicit octet conditions.
    const refPublicV4 = ([a, b, c, d]: readonly [number, number, number, number]): boolean => {
        if (a === 0 || a === 10 || a === 127 || a >= 224) return false;
        if (a === 100 && b >= 64 && b <= 127) return false;
        if (a === 169 && b === 254) return false;
        if (a === 172 && b >= 16 && b <= 31) return false;
        if (a === 192 && b === 0 && (c === 0 || c === 2)) return false;
        if (a === 192 && b === 88 && c === 99) return false;
        if (a === 192 && b === 168) return false;
        if (a === 198 && (b === 18 || b === 19)) return false;
        if (a === 198 && b === 51 && c === 100) return false;
        if (a === 203 && b === 0 && c === 113) return false;
        if (a === 168 && b === 63 && c === 129 && d === 16) return false;
        return true;
    };
    const octet = fc.integer({ min: 0, max: 255 });
    const v4 = fc.tuple(octet, octet, octet, octet);

    test("IPv4 matches the reference over random and boundary-biased addresses", () => {
        const boundary = fc.tuple(fc.constantFrom(0, 9, 10, 11, 99, 100, 126, 127, 128, 168, 169, 171, 172, 173, 191, 192, 193, 197, 198, 199, 202, 203, 204, 223, 224, 239, 240, 255), fc.constantFrom(0, 15, 16, 31, 32, 63, 64, 88, 127, 128, 168, 169, 253, 254, 255, 18, 19, 20, 51, 17), octet, octet);
        fc.assert(fc.property(fc.oneof(v4, boundary), (o) => expect(classifyAddress(o.join(".")).public).toBe(refPublicV4(o))), cfg(5000));
    });

    test("IPv4-mapped and NAT64 IPv6 forms inherit the IPv4 verdict", () => {
        fc.assert(
            fc.property(v4, (o) => {
                const hex = (x: number, y: number) => ((x << 8) | y).toString(16);
                const verdict = refPublicV4(o);
                expect(classifyAddress(`::ffff:${o.join(".")}`).public).toBe(verdict);
                expect(classifyAddress(`::ffff:${hex(o[0]!, o[1]!)}:${hex(o[2]!, o[3]!)}`).public).toBe(verdict);
                expect(classifyAddress(`64:ff9b::${o.join(".")}`).public).toBe(verdict);
                expect(classifyAddress(`::${o.join(".")}`).public).toBe(false); // IPv4-compatible: never
            }),
            cfg(1000),
        );
    });

    test("only 2000::/3 can be public IPv6", () => {
        fc.assert(
            fc.property(fc.array(fc.integer({ min: 0, max: 0xffff }), { minLength: 8, maxLength: 8 }), (g) => {
                const addr = g.map((x) => x.toString(16)).join(":");
                if (classifyAddress(addr).public) expect(g[0]! >> 13).toBe(1);
            }),
            cfg(2000),
        );
    });

    test("whatever the resolver says, a non-public answer anywhere in the set refuses the request", () => {
        const policy = compileEgressPolicy({ allowHosts: ["shop.example.com"] });
        const privateV4 = v4.filter((o) => !refPublicV4(o)).map((o) => o.join("."));
        const publicV4 = v4.filter(refPublicV4).map((o) => o.join("."));
        fc.assert(
            fc.property(fc.array(publicV4, { maxLength: 3 }), privateV4, fc.array(publicV4, { maxLength: 3 }), (before, bad, after) => {
                const d = evaluateEgress("https://shop.example.com/", policy, [...before, bad, ...after]);
                expect(d.allow).toBe(false);
            }),
            cfg(500),
        );
    });

    test("arbitrary URLs against a fixed allowlist: allow implies an allowlisted https host", () => {
        const policy = compileEgressPolicy({ allowHosts: ["shop.example.com", "*.cdn.example.com"] });
        fc.assert(
            fc.property(fc.oneof(fc.webUrl(), fc.string({ maxLength: 40 }), fc.constantFrom("https://shop.example.com@evil.net/", "https://evil.net/shop.example.com", "https://shop.example.com:99999/")), (u) => {
                const d = evaluateEgress(u, policy, ["93.184.216.34"]);
                if (d.allow) {
                    const host = new URL(d.url).hostname;
                    expect(new URL(d.url).protocol).toBe("https:");
                    expect(host === "shop.example.com" || host.endsWith(".cdn.example.com")).toBe(true);
                }
            }),
            cfg(1000),
        );
    });
});

describe("property: confirmation binding", () => {
    const field = fc.record({
        actionId: fc.stringMatching(/^[a-z_]{3,12}$/),
        item: fc.oneof(fc.constant(null), fc.stringMatching(/^[a-z0-9-]{1,12}$/)),
    });
    test("a token verifies for exactly the binding it was issued for", async () => {
        const w = await world();
        const money = fc.record({ totalMinor: fc.integer({ min: 1, max: 1_000_000 }), currency: fc.constantFrom("USD", "EUR", "GBP"), payee: fc.constantFrom("a.example.com", "b.example.com") });
        const binding = fc.tuple(field, fc.option(money, { nil: null })).map(([f, m]): ConfirmationBinding => ({ ...f, totalMinor: m?.totalMinor ?? null, currency: m?.currency ?? null, payee: m?.payee ?? null }));
        await fc.assert(
            fc.asyncProperty(binding, binding, async (a, b) => {
                const tok = await issueConfirmation(w.confirmSigner, a, { jti: "j1", now: NOW, ttlMs: 60_000 });
                const r = await verifyConfirmation(tok, w.deps.confirmationVerifier, b, { now: NOW + 1 });
                expect(r.ok).toBe(JSON.stringify(a) === JSON.stringify(b));
            }),
            cfg(300),
        );
    });
});

describe("property: spend ledger", () => {
    test("accepted spend never exceeds the cap, and a refusal means it would have", async () => {
        await fc.assert(
            fc.asyncProperty(fc.integer({ min: 1, max: 5000 }), fc.array(fc.integer({ min: 1, max: 2000 }), { minLength: 1, maxLength: 40 }), async (cap, amounts) => {
                const l = new InMemorySpendLedger();
                let sum = 0;
                for (const [i, a] of amounts.entries()) {
                    const r = await l.reserve("m", `r${i}`, a, cap);
                    expect(r.ok).toBe(sum + a <= cap);
                    if (r.ok) sum += a;
                    expect(await l.spent("m")).toBe(sum);
                    expect(sum).toBeLessThanOrEqual(cap);
                }
            }),
            cfg(300),
        );
    });
});

describe("property: redaction", () => {
    test("a registered secret never survives, wherever it sits in surrounding text", () => {
        fc.assert(
            fc.property(fc.stringMatching(/^[A-Za-z0-9]{8,32}$/), fc.stringMatching(/^[A-Za-z0-9 =&?:,\n]{0,40}$/), fc.stringMatching(/^[A-Za-z0-9 =&?:,\n]{0,40}$/), (secret, pre, post) => {
                const r = new Redactor();
                r.register(secret);
                for (const enc of [secret, encodeURIComponent(secret), btoa(secret), JSON.stringify(secret).slice(1, -1)]) {
                    expect(r.redactString(`${pre}${enc}${post}`).includes(enc)).toBe(false);
                }
                expect(JSON.stringify(r.redactDeep({ a: [`${pre}${secret}`], b: { c: post + secret } })).includes(secret)).toBe(false);
            }),
            cfg(500),
        );
    });
});

describe("property: codecs", () => {
    test("base64url round-trips and only the canonical form decodes", () => {
        fc.assert(
            fc.property(fc.uint8Array({ maxLength: 64 }), (b) => {
                const e = b64urlEncode(b);
                expect(b64urlDecode(e)).toEqual(b);
                if (e.length % 4 !== 0) expect(b64urlDecode(e + "=")).toBeNull();
            }),
            cfg(300),
        );
    });
    test("non-canonical base64url (stray trailing bits) is refused", () => {
        expect(b64urlDecode("QQ")).toEqual(new Uint8Array([65]));
        expect(b64urlDecode("QR")).toBeNull();
        expect(b64urlDecode("Q")).toBeNull();
        expect(b64urlDecode("a+b/")).toBeNull();
    });
    test("canonical JSON is independent of key order", () => {
        fc.assert(
            fc.property(fc.dictionary(fc.string({ maxLength: 6 }), fc.oneof(fc.integer(), fc.string({ maxLength: 6 }), fc.boolean(), fc.constant(null))), (o) => {
                const rev = Object.fromEntries(Object.entries(o).reverse());
                expect(canonicalJson(rev)).toBe(canonicalJson(o));
            }),
            cfg(300),
        );
    });
    test("a literal robots pattern matches itself and anything it prefixes; `$` anchors", () => {
        fc.assert(
            fc.property(fc.stringMatching(/^\/[a-z0-9/_.-]{0,20}$/), fc.stringMatching(/^[a-z0-9/_.-]{0,10}$/), (p, suffix) => {
                expect(patternMatches(p, p)).toBe(true);
                expect(patternMatches(p, p + suffix)).toBe(true);
                const merge = (x: string) => x.replace(/\/{2,}/g, "/"); // slash runs are merged on both sides
                expect(patternMatches(`${p}$`, `${p}${suffix}`)).toBe(merge(p + suffix) === merge(p));
            }),
            cfg(500),
        );
    });
});
