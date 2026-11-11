// Regression tests for the findings of the adversarial review of the first port: every case
// here was a verified failure (wrong class, leaked secret, panic, non-determinism, perf).

import { describe, expect, test } from "bun:test";
import { infer } from "../src/classify.ts";
import { toJson, valueOf } from "../src/graph.ts";
import { parseRecording } from "../src/har.ts";
import { graphOf, harOf, Rng, type Req, type RunDoc, traceOf, twoRuns as twoRunsLocal } from "./support.ts";

const JSON_CT = ["content-type", "application/json"] as [string, string];

function req(i: number, method: string, url: string, over: Partial<Req> = {}): Req {
    return { id: `q${i}`, tMs: 1000 + i * 100, method, url, headers: [], status: 200, respHeaders: [], ...over };
}

const jsonBody = (o: unknown): [string, string] => ["application/json", JSON.stringify(o)];

function doc(run: string, reqs: Req[], params: [string, string][] = []): RunDoc {
    return { har: harOf(reqs), trace: traceOf(run, params, reqs), cookies: [] };
}

const B = "https://s.test";

describe("alignment (H1)", () => {
    test("reordered requests with the same path depth block instead of pairing up", () => {
        const a = doc("a", [req(0, "GET", `${B}/api/cart`), req(1, "GET", `${B}/api/user`)]);
        const b = doc("b", [req(0, "GET", `${B}/api/user`), req(1, "GET", `${B}/api/cart`)]);
        const g = graphOf([a, b]);
        expect(g.status).toBe("blocked");
        expect(g.blockers[0]!.reason).toContain("not alignable");
        expect(g.blockers[0]!.reason).toContain("reordered");
    });

    test("an id or a parameter in a path segment is still alignable", () => {
        const mk = (run: string, id: string, sku: string) =>
            doc(run, [req(0, "GET", `${B}/items/${id}`), req(1, "GET", `${B}/products/${sku}`)], [["sku", sku]]);
        const g = graphOf([mk("a", "item_8f2Kq9ZxLm41", "mug"), mk("b", "item_Zq81mXk9L2f8", "kettle")]);
        expect(g.status).toBe("resolved");
        expect(valueOf(g, 1, "url.path[1]")!.input).toBe("sku");
    });

    test("blocker text carries no path", () => {
        const a = doc("a", [req(0, "GET", `${B}/reset/SECRETSEGMENT123`)]);
        const b = doc("b", [req(0, "GET", `${B}/reset`)]);
        const g = graphOf([a, b]);
        expect(JSON.stringify(g.blockers)).not.toContain("SECRETSEGMENT123");
    });
});

describe("secrets stay out of the output (H2)", () => {
    const tokens = ["tok_8f2Kq9ZxLm41AbCd", "tok_Zq81mXk9L2f8QwEr"];

    test("edge prefix and suffix that look like a secret are not copied", () => {
        const mk = (run: string, t: string) =>
            doc(run, [
                req(0, "GET", `${B}/`, { respHeaders: [["set-cookie", `sid=${t}; Path=/`]] }),
                req(1, "POST", `${B}/x`, { headers: [["authorization", `Token key=sk\x5flive_STATICSECRET99, session=${t}`]] }),
            ]);
        const g = graphOf([mk("a", tokens[0]!), mk("b", tokens[1]!)]);
        expect(toJson(g)).not.toContain("sk\x5flive_STATICSECRET99");
        expect(valueOf(g, 1, "header:authorization")!.class).not.toBe("derived");
    });

    test("a locator never copies a secret-looking neighbour", () => {
        const mk = (run: string, t: string) =>
            doc(run, [
                req(0, "GET", `${B}/`, { respBody: ["text/html", `<script>var t="${t}";var k="sk\x5flive_ADJSECRET77";</script>`] }),
                req(1, "POST", `${B}/x`, { headers: [["x-page", t]] }),
            ]);
        const g = graphOf([mk("a", tokens[0]!), mk("b", tokens[1]!)]);
        const text = toJson(g);
        expect(text).not.toContain("sk\x5flive_ADJSECRET77");
        expect(text).not.toContain("sk\x5flive_ADJ");
        // The token is still found; its locator stops before the neighbouring word.
        const v = valueOf(g, 1, "header:x-page")!;
        expect(v.class).toBe("derived");
        expect(v.source!.locator).toEqual({ before: '<script>var t="', after: '";var k="', origin: "page" });
    });

    test("path template, parent URLs, other names and bodies are redacted", () => {
        const mk = (run: string) =>
            doc(run, [
                req(0, "POST", `${B}/pay/4242424242424242/go`, {
                    headers: [["referer", `${B}/reset?token=RESETtok_1234567890abcdef&pan=4242424242424242`]],
                    body: jsonBody({ pwd: "Hunter2!Hunter2", shipping: "standard-ground", pin: "12345678" }),
                }),
                req(1, "POST", `${B}/raw`, { body: ["text/plain", "password=Hunter2!Hunter2"] }),
            ]);
        const g = graphOf([mk("a"), mk("b")]);
        const text = toJson(g);
        for (const leaked of ["4242424242424242", "RESETtok_1234567890abcdef", "Hunter2!Hunter2"]) expect(text).not.toContain(leaked);
        expect(g.steps[0]!.path_template).toBe("/pay/{url.path[1]}/go");
        // Word-bounded names: `shipping` is not `pin`.
        expect(valueOf(g, 0, "json:$.shipping")!.value).toBe("standard-ground");
        expect(valueOf(g, 0, "json:$.pin")!.redacted).toBe(true);
        expect(valueOf(g, 1, "body")!.redacted).toBe(true);
        expect(text).not.toContain("sha256");
    });
});

describe("hostile input (M1)", () => {
    test("an absurd response time does not make the response a source", () => {
        const mk = (run: string, cart: string) =>
            doc(run, [
                req(0, "POST", `${B}/cart`, { respBody: jsonBody({ cartId: cart }) }),
                req(1, "POST", `${B}/cart/${cart}/items`),
            ]);
        const docs = [mk("a", "cart_8f2Kq9ZxLm41"), mk("b", "cart_Zq81mXk9L2f8")];
        for (const d of docs) d.har.log.entries[0].time = 1e300;
        const g = graphOf(docs);
        expect(valueOf(g, 1, "url.path[2]")!.class).not.toBe("derived");
        // And it does not throw on odd statuses or dates.
        for (const d of docs) d.har.log.entries[0].response.status = 1e300;
        expect(() => graphOf(docs)).not.toThrow();
        expect(() => parseRecording(JSON.stringify(docs[0]!.har).replace("2026-11-09T10:00:01.000Z", "2026-13-45T99:99:99Z"), JSON.stringify(docs[0]!.trace))).toThrow();
    });
});

describe("determinism under ties (M2)", () => {
    test("parallel identical requests with no ids cannot be ordered: the flow blocks, identically for any file order", () => {
        const mk = (run: string, a: string, b: string, swap: boolean) => {
            const parallel = [
                req(0, "GET", `${B}/api/status`, { tMs: 1000, respBody: jsonBody({ v: a }) }),
                req(1, "GET", `${B}/api/status`, { tMs: 1000, respBody: jsonBody({ v: b }) }),
            ];
            const reqs = [...(swap ? parallel.reverse() : parallel), req(2, "POST", `${B}/x`, { tMs: 1500, headers: [["x-v", b]] })];
            const d = doc(run, reqs);
            for (const e of d.har.log.entries) delete e._requestId;
            d.trace.events = [];
            return d;
        };
        const base = (swapA: boolean) => toJson(graphOf([mk("a", "valueA_8f2Kq9ZxLm41", "valueB_8f2Kq9ZxLm42", swapA), mk("b", "valueA_Zq81mXk9L2f8", "valueB_Zq81mXk9L2f9", false)]));
        expect(base(true)).toBe(base(false));
        const g = JSON.parse(base(true));
        expect(g.status).toBe("blocked");
        expect(g.blockers[0].reason).toContain("identical start times");
    });

    test("with explicit request ids the same ties are ordered by id and stay resolved", () => {
        const mk = (run: string, swap: boolean) => {
            const parallel = [
                req(0, "GET", `${B}/api/status`, { id: "p1", tMs: 1000, respBody: jsonBody({ v: `valueA_${run}8f2Kq9ZxLm41` }) }),
                req(1, "GET", `${B}/api/status`, { id: "p2", tMs: 1000, respBody: jsonBody({ v: `valueB_${run}8f2Kq9ZxLm42` }) }),
            ];
            const d = doc(run, [...(swap ? parallel.reverse() : parallel), req(2, "POST", `${B}/x`, { tMs: 1500 })]);
            d.trace.events = [];
            return d;
        };
        const a = toJson(graphOf([mk("a", true), mk("b", false)]));
        expect(toJson(graphOf([mk("a", false), mk("b", true)]))).toBe(a);
        expect(JSON.parse(a).status).toBe("resolved");
    });

    test("key order inside the HAR and trace never changes the output", () => {
        const docs = twoRunsLocal();
        const base = toJson(graphOf(docs));
        const reverseKeys = (v: any): any =>
            Array.isArray(v) ? v.map(reverseKeys) : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).reverse().map(([k, x]) => [k, reverseKeys(x)])) : v;
        const flipped = docs.map((d) => ({ ...d, har: reverseKeys(d.har), trace: reverseKeys(d.trace) }));
        expect(toJson(graphOf(flipped))).toBe(base);
    });

    test("events without ids are named by content, not position", () => {
        const reqs = [req(0, "GET", `${B}/a`), req(1, "GET", `${B}/b`)];
        const t1 = traceOf("a", [], reqs);
        for (const e of t1.events) delete e.id;
        const t2 = structuredClone(t1);
        t2.events.reverse();
        const r1 = parseRecording(JSON.stringify(harOf(reqs)), JSON.stringify(t1));
        const r2 = parseRecording(JSON.stringify(harOf(reqs)), JSON.stringify(t2));
        expect(r1.events.map((e) => e.id)).toEqual(r2.events.map((e) => e.id));
    });
});

describe("array selection (M3)", () => {
    const mk = (run: string, pick: number, labelsFirst: boolean) => {
        const rates = [
            { id: `rate_${run}AAAA1111`, pos: "1", label: labelsFirst ? "Standard" : "Express" },
            { id: `rate_${run}BBBB2222`, pos: "2", label: labelsFirst ? "Express" : "Standard" },
        ];
        return doc(
            run,
            [
                req(0, "GET", `${B}/rates`, { respBody: jsonBody({ rates, query: "red shoes", ids: rates.map((r) => r.id) }) }),
                req(1, "POST", `${B}/rate`, { body: jsonBody({ rateId: rates[pick]!.id }) }),
            ],
            [["quantity", String(pick + 1)], ["q", "red shoes"], ["rate", pick === 0 ? "Standard" : "Express"]],
        );
    };

    test("a numeric sibling that happens to equal a parameter is not a selection rule; a label is", () => {
        // `pos` is 1/2 and so is `quantity`; `label` follows `rate`. Only the label may explain it.
        const g = graphOf([mk("a", 0, true), mk("b", 1, true)]);
        const v = valueOf(g, 1, "json:$.rateId")!;
        expect(v.class).toBe("derived");
        expect(v.source!.select).toEqual({ field: "label", param: "rate" });
        expect(v.source!.location).toBe("json:$.rates[*].id");
    });

    test("with no label field the pick is unexplained and blocks", () => {
        const noLabel = (d: RunDoc) => {
            for (const e of d.har.log.entries) {
                if (e.response.content.text.includes("rates")) {
                    const o = JSON.parse(e.response.content.text);
                    o.rates = o.rates.map(({ id, pos }: any) => ({ id, pos }));
                    delete o.ids;
                    e.response.content.text = JSON.stringify(o);
                }
            }
            return d;
        };
        const g = graphOf([noLabel(mk("a", 0, true)), noLabel(mk("b", 1, true))]);
        const v = valueOf(g, 1, "json:$.rateId")!;
        expect(v.class).toBe("unclassified");
        expect(v.reason).toContain("no label field");
    });

    test("an array of scalars has no sibling fields: top-level fields are not siblings", () => {
        const mkIds = (run: string, pick: number) => {
            const ids = [`id_${run}Zq81mXk9L2f8`, `id_${run}Wp47nRj3T6c5`];
            return doc(
                run,
                [
                    req(0, "GET", `${B}/s`, { respBody: jsonBody({ query: "red shoes", ids }) }),
                    req(1, "POST", `${B}/pick`, { body: jsonBody({ id: ids[pick] }) }),
                ],
                [["q", "red shoes"]],
            );
        };
        const g = graphOf([mkIds("a", 0), mkIds("b", 1)]);
        const v = valueOf(g, 1, "json:$.id")!;
        expect(v.class).toBe("unclassified");
        expect(v.source).toBeUndefined();
    });
});

describe("inputs (M4, M5)", () => {
    test("a low-entropy match against a run-wide parameter is flagged as weak", () => {
        const mk = (run: string, page: number) =>
            doc(run, [req(0, "POST", `${B}/list`, { body: jsonBody({ page }) })], [["quantity", String(page)]]);
        const g = graphOf([mk("a", 1), mk("b", 2)]);
        expect(valueOf(g, 0, "json:$.page")!.input).toBe("quantity");
        expect(g.warnings.some((w) => w.kind === "weak-input-match" && w.location === "json:$.page")).toBe(true);
    });

    test("an input that matches in one run only blocks instead of becoming client-computed", () => {
        const mk = (run: string, zip: string, param: string) => doc(run, [req(0, "POST", `${B}/addr`, { body: jsonBody({ zip }) })], [["zip", param]]);
        // Token-grade values: run a equals the parameter, run b is a normalised form of it.
        const g = graphOf([mk("a", "ZIP-A7q9-K3m2x", "ZIP-A7q9-K3m2x"), mk("b", "ZIP-Zr58-Pw4Lb", "ZIP-Zr58")]);
        const v = valueOf(g, 0, "json:$.zip")!;
        expect(v.class).toBe("unclassified");
        expect(v.reason).toContain("some runs but not in all");
    });
});

describe("evidence is consistent across runs (M6)", () => {
    test("8-character hex ids are derived whatever their characters", () => {
        for (let seed = 1; seed <= 80; seed++) {
            const mk = (run: string, s: number) => {
                const id = new Rng(s).hex(8);
                return doc(run, [req(0, "POST", `${B}/cart`, { respBody: jsonBody({ id }) }), req(1, "GET", `${B}/cart/${id}`)]);
            };
            const g = graphOf([mk("a", seed * 2), mk("b", seed * 2 + 1)]);
            // Either derived, or (when no run passes the strong gate) never wrongly blocked.
            const v = valueOf(g, 1, "url.path[1]")!;
            expect(["derived", "client-computed"], `seed ${seed}: ${JSON.stringify(v)}`).toContain(v.class);
            expect(g.status === "resolved", `seed ${seed}: ${JSON.stringify(g.blockers)}`).toBe(true);
        }
    });
});

describe("request values keep their shape (M8)", () => {
    test("duplicate cookie names are both classed", () => {
        const mk = (run: string, a: string, b: string) =>
            doc(run, [req(0, "GET", `${B}/`, { headers: [["cookie", `sid=${a}; sid=${b}`]] })]);
        const g = graphOf([mk("a", "AAAA1111bbbb", "CCCC2222dddd"), mk("b", "EEEE3333ffff", "GGGG4444hhhh")]);
        expect(valueOf(g, 0, "cookie:sid")).toBeDefined();
        expect(valueOf(g, 0, "cookie:sid#1")).toBeDefined();
    });

    test("JSON types survive, empty containers are values, type changes block", () => {
        const mk = (run: string, q: unknown) =>
            doc(run, [req(0, "POST", `${B}/x`, { body: jsonBody({ n: 5, flag: true, none: null, empty: {}, list: [], q }) })]);
        const g = graphOf([mk("a", 2), mk("b", "2")]);
        expect(valueOf(g, 0, "json:$.n")).toMatchObject({ class: "constant", value: "5", json_type: "number" });
        expect(valueOf(g, 0, "json:$.flag")!.json_type).toBe("boolean");
        expect(valueOf(g, 0, "json:$.none")!.json_type).toBe("null");
        expect(valueOf(g, 0, "json:$.empty")).toMatchObject({ value: "{}", json_type: "object" });
        expect(valueOf(g, 0, "json:$.list")).toMatchObject({ value: "[]", json_type: "array" });
        expect(valueOf(g, 0, "json:$.q")!.class).toBe("unclassified");
        expect(valueOf(g, 0, "json:$.q")!.reason).toContain("JSON type differs");
    });
});

describe("declared inputs that did not vary (M9)", () => {
    test("embedded as constants with a warning that names the input", () => {
        const mk = (run: string, zip: string) => {
            const reqs = [req(0, "POST", `${B}/addr`, { body: jsonBody({ country: "US", zip }) })];
            const t = traceOf(run, [], reqs);
            t.events[0].inputs = { country: "US", zip };
            return { har: harOf(reqs), trace: t, cookies: [] } as RunDoc;
        };
        const g = graphOf([mk("a", "94110"), mk("b", "10001")]);
        expect(valueOf(g, 0, "json:$.country")!.class).toBe("constant");
        expect(g.warnings.some((w) => w.kind === "input-did-not-vary" && w.location === "json:$.country" && w.detail.includes("`country`"))).toBe(true);
        expect(valueOf(g, 0, "json:$.zip")!.input).toBe("zip");
    });
});

describe("cost", () => {
    test("many leaves and many varying fields stay fast", () => {
        const mk = (run: string, seed: number) => {
            const g = new Rng(seed);
            const reqs: Req[] = [];
            for (let i = 0; i < 40; i++) {
                const leaves: Record<string, string> = {};
                for (let k = 0; k < 20; k++) leaves[`f${k}`] = `v${g.hex(16)}`;
                const body: Record<string, string> = {};
                for (let k = 0; k < 20; k++) body[`g${k}`] = `w${g.hex(16)}`;
                reqs.push(req(i, "POST", `${B}/s${i}`, { body: jsonBody(body), respBody: jsonBody(leaves) }));
            }
            return doc(run, reqs);
        };
        const started = performance.now();
        const g = infer([mk("a", 1), mk("b", 2)].map((d) => parseRecording(JSON.stringify(d.har), JSON.stringify(d.trace))));
        expect(g.steps).toHaveLength(40);
        expect(performance.now() - started).toBeLessThan(10_000);
        void JSON_CT;
    });
});

// ---------------------------------------------------------------------------------------------
// Round 2 (review of the TypeScript port)

describe("alignment by operation (round 2, H-a)", () => {
    test("two GraphQL operations that swap order block", () => {
        const op = (name: string, id: string) => jsonBody({ operationName: name, query: `query ${name} { x }`, variables: { id } });
        const a = doc("a", [req(0, "POST", `${B}/graphql`, { body: op("GetCart", "c_8f2Kq9ZxLm41") }), req(1, "POST", `${B}/graphql`, { body: op("GetUser", "u_Zq81mXk9L2f8") })]);
        const b = doc("b", [req(0, "POST", `${B}/graphql`, { body: op("GetUser", "u_Wp47nRj3T6c5") }), req(1, "POST", `${B}/graphql`, { body: op("GetCart", "c_Lm41Zx9qK2f8") })]);
        const g = graphOf([a, b]);
        expect(g.status).toBe("blocked");
        expect(g.blockers[0]!.reason).toContain("operation");
    });
    test("an operation that follows a run parameter, and slug path segments, still align", () => {
        const mk = (run: string, slug: string) => doc(run, [req(0, "POST", `${B}/rpc`, { body: jsonBody({ method: slug }) }), req(1, "GET", `${B}/shop/${slug}-collection`)], [["pick", slug]]);
        expect(graphOf([mk("a", "red-shoes"), mk("b", "blue-hats")]).status).toBe("resolved");
    });
});

describe("constants are redacted by what they look like (round 2, H-b..H-d)", () => {
    const secrets = [
        "AIza\x53yA1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6", "sk\x5flive_QWERTYUIOPASDFGHJKLZXCV", "RESETtok_1234567890abcdef", "alice.smith@example.com",
        "123-45-6789", "4242424242424242|12/30|123", "access_token=ya\x329.A0ARrdaM-abcdef", "AbC123xyz789QwE", "737399",
    ];
    test("query, header, body, relative URLs, fragments, emails, SSNs and payment strings never reach the output", () => {
        const mk = (run: string) =>
            doc(run, [
                req(0, "POST", `${B}/reset/${secrets[2]}/confirm?key=${secrets[0]}`, {
                    headers: [["x-hmac", secrets[1]!], ["referer", `${B}/cb?code=${secrets[7]}`]],
                    body: jsonBody({ next: `/reset?token=${secrets[2]}`, returnUrl: `${B}/done#${secrets[6]}`, cvv2: "737", email: secrets[3], social: secrets[4], payment_info: secrets[5], shippingNote: "leave at door" }),
                }),
            ]);
        const g = graphOf([mk("a"), mk("b")]);
        const text = toJson(g);
        for (const s of secrets.slice(0, 8)) expect(text, s).not.toContain(s);
        expect(text).not.toContain("737");
        // Ordinary text is kept.
        expect(valueOf(g, 0, "json:$.shippingNote")!.value).toBe("leave at door");
        // Components of a redacted URL are redacted with it.
        expect(valueOf(g, 0, "header:referer|query:code")!.redacted).toBe(true);
        expect(g.steps[0]!.path_template).toBe("/reset/{url.path[1]}/confirm");
    });

    test("edge prefixes: digitless keys, spaced card numbers and anything before a sensitive target's credential", () => {
        const toks = ["tok_8f2Kq9ZxLm41AbCd", "tok_Zq81mXk9L2f8QwEr"];
        for (const prefix of ["apikey=sk\x5flive_QWERTYUIOPASDFGHJKLZXCV;pan=4242 4242 4242 4242;session=", "user=alice; "]) {
            const mk = (run: string, t: string) =>
                doc(run, [
                    req(0, "GET", `${B}/`, { respHeaders: [["set-cookie", `sid=${t}; Path=/`]] }),
                    req(1, "POST", `${B}/x`, { headers: [["x-auth", `${prefix}${t}`]] }),
                ]);
            const g = graphOf([mk("a", toks[0]!), mk("b", toks[1]!)]);
            const text = toJson(g);
            expect(text).not.toContain("QWERTYUIOPASDFGHJKLZXCV");
            expect(text).not.toContain("4242 4242");
            expect(valueOf(g, 1, "header:x-auth")!.class).not.toBe("derived");
        }
    });

    test("locators: path-like neighbours are cut or refused", () => {
        const mk = (run: string, t: string) =>
            doc(run, [
                req(0, "GET", `${B}/`, { respBody: ["text/html", `<script>var t="${t}";var k="/Xy7q/Zr9w/Lm3p/Ab5c";</script>`] }),
                req(1, "POST", `${B}/x`, { headers: [["x-page", t]] }),
            ]);
        const g = graphOf([mk("a", "tok_8f2Kq9ZxLm41AbCd"), mk("b", "tok_Zq81mXk9L2f8QwEr")]);
        expect(toJson(g)).not.toContain("Xy7q");
    });

    test("page-derived keys that are tokens are not echoed in locations", () => {
        const tok = (run: string) => `sess_${run === "a" ? "8f2Kq9ZxLm41AbCd" : "Zq81mXk9L2f8QwEr"}`;
        const mk = (run: string) => doc(run, [req(0, "POST", `${B}/x?${tok(run)}=1`, { body: jsonBody({ sessions: { [tok(run)]: true } }) })]);
        const g = graphOf([mk("a"), mk("b")]);
        const text = toJson(g);
        expect(text).not.toContain("8f2Kq9ZxLm41AbCd");
        expect(text).not.toContain("Zq81mXk9L2f8QwEr");
        expect(g.status).toBe("blocked");
    });

    test("a return URL with an account id is refused as a source, never emitted", () => {
        const mk = (run: string, cart: string) =>
            doc(run, [
                req(0, "POST", `${B}/cart`, { respBody: jsonBody({ cartId: cart }) }),
                req(1, "POST", `${B}/x`, { body: jsonBody({ return_url: `${B}/t/acct_1Hh1ZK2eZvKYlo2C/done?cart=${cart}` }) }),
            ]);
        const g = graphOf([mk("a", "cart_8f2Kq9ZxLm41"), mk("b", "cart_Zq81mXk9L2f8")]);
        expect(toJson(g)).not.toContain("acct_1Hh1ZK2eZvKYlo2C");
        expect(valueOf(g, 1, "json:$.return_url")!.class).toBe("unclassified");
        expect(valueOf(g, 1, "json:$.return_url|path[1]")!.redacted).toBe(true);
    });
});

describe("matching (round 2, M-a..M-e)", () => {
    test("a token that needs URL-encoding in one run and not in the other is still derived", () => {
        const mk = (run: string, raw: string) =>
            doc(run, [
                req(0, "GET", `${B}/`, { respHeaders: [["set-cookie", `XSRF-TOKEN=${encodeURIComponent(raw)}; Path=/`]] }),
                req(1, "POST", `${B}/x`, { headers: [["x-xsrf", raw]] }),
            ]);
        const g = graphOf([mk("a", "ab/cd+ef0123456789Zq81mXk9"), mk("b", "abcdef0123456789Zq81mXk9L2f8")]);
        expect(valueOf(g, 1, "header:x-xsrf")!.class).toBe("derived");
        expect(g.status).toBe("resolved");
    });

    test("weak-only matches neither link nor block, and say so", () => {
        const mk = (run: string, sep: string) =>
            doc(run, [req(0, "GET", `${B}/a`, { respBody: jsonBody({ sep: "--------", defaultCity: "San Francisco" }) }), req(1, "POST", `${B}/b`, { body: jsonBody({ rule: sep, city: run === "a" ? "San Francisco" : "Oakland Heights" }) })], [["rule", sep], ["city", run === "a" ? "San Francisco" : "Oakland Heights"]]);
        const g = graphOf([mk("a", "--------"), mk("b", "========")]);
        expect(g.status).toBe("resolved");
        expect(valueOf(g, 1, "json:$.rule")!.input).toBe("rule");
        // A user input that the server echoed in only one run is still an input.
        expect(valueOf(g, 1, "json:$.city")!.input).toBe("city");
    });

    test("a consistent low-entropy match that is ignored is reported", () => {
        const mk = (run: string, id: string) => doc(run, [req(0, "GET", `${B}/a`, { respBody: jsonBody({ id }) }), req(1, "POST", `${B}/b`, { body: jsonBody({ ref: id }) })]);
        const g = graphOf([mk("a", "a1a1b2b2"), mk("b", "c3c3d4d4")]);
        expect(valueOf(g, 1, "json:$.ref")!.class).toBe("client-computed");
        expect(g.warnings.some((w) => w.kind === "weak-source-ignored" && w.location === "json:$.ref")).toBe(true);
    });

    test("choosing an array element by its own value is an input, not a block", () => {
        const mk = (run: string, name: string) =>
            doc(run, [req(0, "GET", `${B}/rates`, { respBody: jsonBody({ rates: [{ code: "A", name: "Standard Delivery" }, { code: "B", name: "Express Delivery" }] }) }), req(1, "POST", `${B}/rate`, { body: jsonBody({ method: name }) })], [["shipping", name]]);
        const g = graphOf([mk("a", "Standard Delivery"), mk("b", "Express Delivery")]);
        expect(valueOf(g, 1, "json:$.method")!.class).toBe("input");
        expect(g.status).toBe("resolved");
    });

    test("a low-entropy partial input match is reported", () => {
        const mk = (run: string, zip: string, p: string) => doc(run, [req(0, "POST", `${B}/a`, { body: jsonBody({ zip }) })], [["zip", p]]);
        const g = graphOf([mk("a", "94110", "94110"), mk("b", "10001", "10002")]);
        expect(valueOf(g, 0, "json:$.zip")!.class).toBe("client-computed");
        expect(g.warnings.some((w) => w.kind === "weak-input-partial")).toBe(true);
    });
});

describe("robustness (round 2, M-f, low)", () => {
    test("pathological HTML is scanned in linear time", () => {
        const html = (unit: string) => `<html>${unit.repeat(20_000)}<meta name="csrf-token" content="tok_8f2Kq9ZxLm41AbCd"></html>`;
        const started = performance.now();
        for (const unit of ["<!--a", "<script>", "<input ", "<meta "]) {
            const mk = (run: string) => doc(run, [req(0, "GET", `${B}/`, { respBody: ["text/html", html(unit)] }), req(1, "POST", `${B}/x`)]);
            graphOf([mk("a"), mk("b")]);
        }
        expect(performance.now() - started).toBeLessThan(8_000);
    });

    test("many variants that share a prefix do not make search quadratic", () => {
        const mk = (run: string, seed: number) => {
            const g = new Rng(seed);
            const leaves = Object.fromEntries(Array.from({ length: 3000 }, (_, i) => [`k${i}`, `eyJhbGci${g.hex(24)}`]));
            return doc(run, [req(0, "GET", `${B}/a`, { respBody: jsonBody(leaves) }), req(1, "POST", `${B}/b`, { headers: Array.from({ length: 5 }, (_, i) => [`x-h${i}`, `Bearer ${"x".repeat(2000)}${g.hex(16)}`] as [string, string]) })]);
        };
        const started = performance.now();
        graphOf([mk("a", 1), mk("b", 2)]);
        expect(performance.now() - started).toBeLessThan(8_000);
    });

    test("duplicate jar cookies, repeated Set-Cookie names and prototype-named entities", () => {
        const mk = (run: string, a: string, b: string) => {
            const d = doc(run, [req(0, "GET", `${B}/`, { respHeaders: [["set-cookie", `c=${a}`], ["set-cookie", `c=${b}`]], respBody: ["text/html", `<meta name="m" content="&constructor;&toString;x">`] }), req(1, "POST", `${B}/x`, { headers: [["x-a", a], ["x-b", b]] })]);
            d.cookies = [{ name: "j", value: a }, { name: "j", value: b }];
            return d;
        };
        const g = graphOf([mk("a", "k8Q2m9ZxL41aBcD7", "p3W7nRj6T5cEfG2h"), mk("b", "Zq81mXk9L2f8QwEr", "Hy47Vt3Bn6Ds0KuJ")]);
        expect(g.edges.map((e) => e.from.location).sort()).toEqual(["setcookie:c", "setcookie:c#1"]);
        expect(JSON.stringify(g)).not.toContain("native code");
    });

    test("an invalid URL entry is reported, not silently dropped", () => {
        const d = doc("a", [req(0, "GET", `${B}/a`)]);
        d.har.log.entries.push({ ...structuredClone(d.har.log.entries[0]), _requestId: "bad", request: { method: "GET", url: "not a url", headers: [] } });
        const g = graphOf([d, doc("b", [req(0, "GET", `${B}/a`)])]);
        expect(g.warnings.some((w) => w.kind === "entry-skipped")).toBe(true);
    });

    test("input-did-not-vary names the alphabetically first input, and covers run-wide parameters", () => {
        const mk = (run: string, order: string[]) => {
            const reqs = [req(0, "POST", `${B}/a`, { body: jsonBody({ zip: "94110", other: run }) })];
            const t = traceOf(run, [], reqs);
            t.params = Object.fromEntries(order.map((k) => [k, "94110"]));
            return { har: harOf(reqs), trace: t, cookies: [] } as RunDoc;
        };
        const w = (order: string[]) => graphOf([mk("a", order), mk("b", order)]).warnings.filter((x) => x.kind === "input-did-not-vary");
        expect(w(["billingZip", "shippingZip"])).toEqual(w(["shippingZip", "billingZip"]));
        expect(w(["shippingZip", "billingZip"])[0]!.detail).toContain("`billingZip`");
    });
});

// ---------------------------------------------------------------------------------------------
// Round 3 (re-review of the round-2 fixes)

describe("round 3: redaction leaks", () => {
    const onlyConstants = (value: Record<string, unknown>, headers: [string, string][] = [], url = `${B}/x`) => {
        const mk = (run: string) => doc(run, [req(0, "POST", url, { headers, body: jsonBody(value) })]);
        return graphOf([mk("a"), mk("b")]);
    };

    test("a card number with its expiry glued on is found", () => {
        const g = onlyConstants({ a: "4539 1488 0343 6467 12/30", b: "4242 4242 4242 4242 1230", c: "card 4242-4242-4242-4242-123", d: "4242424242424242|12/30|123" });
        const text = toJson(g);
        for (const s of ["4539 1488", "4242 4242", "4242-4242", "4242424242424242"]) expect(text).not.toContain(s);
    });

    test("short secrets, form and JSON text, encoded names, emails and event text stay out", () => {
        const g = onlyConstants(
            { pw: "Hunter2xyz", passcode: "Tr0ub4dor", pincode: "4821", totp_seed: "JBSWY3DP", key: "k-abc", form: "user=bob&password=hunter2pw", text: '{"password":"hunter3pw"}', rel: "/cb?t%6Fken=hunter2", note: "mail alice@example.com pw=hunter2 ok", prefs: { "alice.smith@example.com": true } },
        );
        const text = toJson(g);
        for (const s of ["Hunter2xyz", "Tr0ub4dor", "4821", "JBSWY3DP", "hunter2pw", "hunter3pw", "hunter2", "alice@example.com", "alice.smith@example.com"]) expect(text, s).not.toContain(s);
    });

    test("edge prefixes and locators carrying emails or passwords are refused", () => {
        const toks = ["tok_8f2Kq9ZxLm41AbCd", "tok_Zq81mXk9L2f8QwEr"];
        const mk = (run: string, t: string) =>
            doc(run, [
                req(0, "GET", `${B}/`, { respHeaders: [["set-cookie", `sid=${t}; Path=/`]], respBody: ["text/html", `<b>bo@x.io</b><i>${t}</i>`] }),
                req(1, "POST", `${B}/x`, { body: jsonBody({ note: `mail alice@example.com pw=hunter2 ref ${t}` }), headers: [["x-page", t]] }),
            ]);
        const g = graphOf([mk("a", toks[0]!), mk("b", toks[1]!)]);
        const text = toJson(g);
        expect(text).not.toContain("alice@example.com");
        expect(text).not.toContain("bo@x.io");
        expect(text).not.toContain("hunter2");
    });

    test("trace selectors with attribute values are cleaned", () => {
        const reqs = [req(0, "GET", `${B}/a`)];
        const mk = (run: string) => {
            const t = traceOf(run, [], reqs);
            t.events[0].target = 'input[name=password][value="Hunter2xyz"]';
            return { har: harOf(reqs), trace: t, cookies: [] } as RunDoc;
        };
        const text = toJson(graphOf([mk("a"), mk("b")]));
        expect(text).not.toContain("Hunter2xyz");
        expect(text).toContain("input[name=password]");
    });
});

describe("round 3: over-redaction of replayable constants", () => {
    test("content types, user agents, slugs, SKUs and country codes are kept", () => {
        const mk = (run: string) =>
            doc(run, [
                req(0, "POST", `${B}/x`, {
                    headers: [["content-type", "application/x-www-form-urlencoded"], ["user-agent", "Mozilla/5.0 (Macintosh) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"], ["referer", `${B}/shop/blue-widget-2024`]],
                    body: jsonBody({ sku: "WIDGET-2024-BLUE-XL", currencyCode: "USD", postal_code_label: "ZIP" }),
                }),
            ]);
        const g = graphOf([mk("a"), mk("b")]);
        for (const loc of ["header:content-type", "header:user-agent", "header:referer", "json:$.sku", "json:$.currencyCode"]) {
            expect(valueOf(g, 0, loc)!.redacted, loc).toBeUndefined();
            expect(valueOf(g, 0, loc)!.value, loc).toBeDefined();
        }
    });
});

describe("round 3: matching", () => {
    test("a substring match (Bearer) survives a token that needs encoding in only one run", () => {
        const mk = (run: string, raw: string) =>
            doc(run, [
                req(0, "GET", `${B}/`, { respHeaders: [["set-cookie", `tk=${encodeURIComponent(raw)}; Path=/`]] }),
                req(1, "POST", `${B}/x`, { headers: [["authorization", `Bearer ${raw}`]] }),
            ]);
        const g = graphOf([mk("a", "abcdef0123456789Zq81mXk9L2f8"), mk("b", "ab/cd+ef0123456789Zq81mXk9")]);
        expect(g.status).toBe("resolved");
        expect(valueOf(g, 1, "header:authorization")!.class).toBe("derived");
    });

    test("operation names are compared for GET, batched and query-only GraphQL", () => {
        const swap = (mk: (_run: string, first: boolean) => Req[]) => graphOf([doc("a", mk("a", true)), doc("b", mk("b", false))]);
        const get = (name: string) => req(0, "GET", `${B}/graphql?operationName=${name}`);
        const g1 = swap((_run, first) => (first ? [get("GetCart"), { ...get("GetUser"), id: "q1", tMs: 1100 }] : [get("GetUser"), { ...get("GetCart"), id: "q1", tMs: 1100 }]));
        expect(g1.status).toBe("blocked");
        const batch = (n: string) => req(0, "POST", `${B}/graphql`, { body: jsonBody([{ operationName: n, variables: { id: "x" } }]) });
        const g2 = swap((_run, first) => (first ? [batch("GetCart"), { ...batch("GetUser"), id: "q1", tMs: 1100 }] : [batch("GetUser"), { ...batch("GetCart"), id: "q1", tMs: 1100 }]));
        expect(g2.status).toBe("blocked");
        const q = (n: string) => req(0, "POST", `${B}/graphql`, { body: jsonBody({ query: `query ${n} { x }` }) });
        const g3 = swap((_run, first) => (first ? [q("GetCart"), { ...q("GetUser"), id: "q1", tMs: 1100 }] : [q("GetUser"), { ...q("GetCart"), id: "q1", tMs: 1100 }]));
        expect(g3.status).toBe("blocked");
    });

    test("duplicate jar cookie names are not sources, and say so", () => {
        const mk = (run: string, a: string, b: string) => {
            const d = doc(run, [req(0, "POST", `${B}/x`, { headers: [["x-a", a], ["x-b", b]] })]);
            d.cookies = [{ name: "j", value: a }, { name: "j", value: b }];
            return d;
        };
        const g = graphOf([mk("a", "k8Q2m9ZxL41aBcD7", "p3W7nRj6T5cEfG2h"), mk("b", "Zq81mXk9L2f8QwEr", "Hy47Vt3Bn6Ds0KuJ")]);
        expect(g.edges.filter((e) => e.from.where === "initial-cookie")).toEqual([]);
        expect(g.warnings.some((w) => w.kind === "jar-duplicate-names")).toBe(true);
    });

    test("entries tied on start time with numeric-looking ids pair by natural order", () => {
        const mk = (run: string, ids: string[]) =>
            doc(run, [
                req(0, "POST", `${B}/api/items`, { id: ids[0]!, tMs: 1000, body: jsonBody({ sku: "mug-001" }) }),
                req(1, "POST", `${B}/api/items`, { id: ids[1]!, tMs: 1000, body: jsonBody({ sku: "kettle-002" }) }),
            ]);
        const g = graphOf([mk("a", ["100.9", "100.10"]), mk("b", ["200.7", "200.8"])]);
        expect(valueOf(g, 0, "json:$.sku")).toMatchObject({ class: "constant", value: "mug-001" });
        expect(valueOf(g, 1, "json:$.sku")).toMatchObject({ class: "constant", value: "kettle-002" });
    });

    test("entry-skipped output does not depend on entry order", () => {
        const a = doc("a", [req(0, "GET", `${B}/a`)]);
        a.har.log.entries.push({ ...structuredClone(a.har.log.entries[0]), _requestId: "bad", request: { method: "GET", url: "not a url", headers: [] } });
        const flipped = structuredClone(a);
        flipped.har.log.entries.reverse();
        const b = doc("b", [req(0, "GET", `${B}/a`)]);
        expect(toJson(graphOf([a, b]))).toBe(toJson(graphOf([flipped, b])));
    });
});

describe("round 3: scanners and cost", () => {
    const csrf = (html: string) => {
        const mk = (run: string, t: string) => doc(run, [req(0, "GET", `${B}/`, { respBody: ["text/html", html.replace("TOK", t)] }), req(1, "POST", `${B}/x`, { headers: [["x-csrf", t]] })]);
        return graphOf([mk("a", "tok_8f2Kq9ZxLm41AbCd"), mk("b", "tok_Zq81mXk9L2f8QwEr")]);
    };
    const loc = (g: ReturnType<typeof graphOf>) => valueOf(g, 1, "header:x-csrf")!.source?.location;

    test("non-ASCII before the tag, empty comments and markup inside attribute values", () => {
        expect(loc(csrf('<p>İstanbul</p><input type="hidden" name="csrf" value="TOK">'))).toBe("html:input[name=csrf]");
        expect(loc(csrf('<!--><input type="hidden" name="csrf" value="TOK">'))).toBe("html:input[name=csrf]");
        expect(loc(csrf(`<div title='<input type=hidden name=csrf value=FAKEFAKEFAKE1234>'></div><input type="hidden" name="csrf" value="TOK">`))).toBe("html:input[name=csrf]");
    });

    test("pathological HTML still finds the token where HTML would", () => {
        for (const unit of ["<!--a", "<script>"]) {
            const g = csrf(`${unit.repeat(2000)}<meta name="csrf" content="TOK">`);
            expect(g.status).toBe("resolved");
        }
    });

    test("a huge whitespace-free constant is not quadratic", () => {
        const started = performance.now();
        const mk = (run: string) => doc(run, [req(0, "POST", `${B}/x`, { body: jsonBody({ blob: "a".repeat(200_000) }) })]);
        graphOf([mk("a"), mk("b")]);
        expect(performance.now() - started).toBeLessThan(5_000);
    });

    test("crowded windows with many distinct lengths stay bounded", () => {
        const mk = (run: string) => {
            const leaves = Object.fromEntries(Array.from({ length: 2000 }, (_, i) => [`k${i}`, `${"a".repeat(8 + (i % 1500))}Z9`]));
            const headers = Array.from({ length: 10 }, (_, i) => [`x-h${i}`, `${"a".repeat(4000)}${run}${i}`] as [string, string]);
            return doc(run, [req(0, "GET", `${B}/a`, { respBody: jsonBody(leaves) }), req(1, "POST", `${B}/b`, { headers })]);
        };
        const started = performance.now();
        graphOf([mk("a"), mk("b")]);
        expect(performance.now() - started).toBeLessThan(10_000);
    });
});

// ---------------------------------------------------------------------------------------------
// Round 4

describe("round 4", () => {
    const constants = (body: Record<string, unknown>, headers: [string, string][] = []) => {
        const mk = (run: string) => doc(run, [req(0, "POST", `${B}/x`, { headers, body: jsonBody(body) })]);
        return graphOf([mk("a"), mk("b")]);
    };

    test("slug- and word-shaped secrets under ordinary names stay redacted; slugs under product names stay", () => {
        const g = constants({
            recoveryPhrase: "correct-horse-battery-staple",
            mnemonic: "abandon_ability_able_about_above_absent",
            licence: "ABCDE-FGHIJ-KLMNO-PQRST",
            token2: "sk\x5flive_abcdefghijkl_123456",
            sk: "sk\x5flive_abcdefghijklmnopqrstuvwx",
            sku: "WIDGET-2024-BLUE-XL",
            product_handle: "blue-widget-deluxe-edition",
        });
        const text = toJson(g);
        for (const s of ["correct-horse", "abandon_ability", "ABCDE-FGHIJ", "sk\x5flive_abcdefghijkl", "abcdefghijklmnopqrstuvwx"]) expect(text, s).not.toContain(s);
        expect(valueOf(g, 0, "json:$.sku")!.value).toBe("WIDGET-2024-BLUE-XL");
        expect(valueOf(g, 0, "json:$.product_handle")!.value).toBe("blue-widget-deluxe-edition");
    });

    test("a credential in a descriptive header is not exempt, a real user agent is", () => {
        const g = constants({}, [["x-requested-with", "sk\x5flive_ABCdef1234567890xyz"], ["origin", "sk\x5flive_ABCdef1234567890xyz"], ["user-agent", "sk\x5flive_ABCdef1234567890xyz"]]);
        expect(toJson(g)).not.toContain("sk\x5flive_ABCdef1234567890xyz");
    });

    test("non-English password names, numeric and single-quoted JSON text, odd card separators, automation selectors", () => {
        const g = constants({ passwort: "hunter2024", mot_de_passe: "Tr0ub4dor-3", senha: "abc12345", text: "{'pin': 482913}", text2: '{"password": "a\\"b"}', card: "4242.4242.4242.4242", card2: "4242_4242_4242_4242" });
        const text = toJson(g);
        for (const s of ["hunter2024", "Tr0ub4dor-3", "abc12345", "482913", "4242.4242", "4242_4242"]) expect(text, s).not.toContain(s);
        const reqs = [req(0, "GET", `${B}/a`)];
        const mk = (run: string) => {
            const t = traceOf(run, [], reqs);
            t.events[0].target = '#password >> fill("hunter2024") >> text=hunter2024';
            return { har: harOf(reqs), trace: t, cookies: [] } as RunDoc;
        };
        expect(toJson(graphOf([mk("a"), mk("b")]))).not.toContain("hunter2024");
    });

    test("nested method/op fields and free text starting with 'query' do not block alignment", () => {
        const mk = (run: string, pm: string, method: string, message: string) =>
            doc(run, [req(0, "POST", `${B}/pay`, { body: jsonBody({ payment: { method: pm }, shipping: { method }, message }) })], [["message", message]]);
        const g = graphOf([mk("a", "pm_8f2Kq9ZxLm41", "express", "query about my order"), mk("b", "pm_Zq81mXk9L2f8", "standard", "query regarding sizes")]);
        expect(g.status).toBe("resolved");
        // A top-level server-issued action value is derived, not an operation name.
        const act = (run: string, n: string) => doc(run, [req(0, "POST", `${B}/a`, { respBody: jsonBody({ n }) }), req(1, "POST", `${B}/b?action=${n}`)]);
        expect(graphOf([act("a", "nonce_8f2Kq9ZxLm41"), act("b", "nonce_Zq81mXk9L2f8")]).status).toBe("resolved");
    });

    test("a truncated variant index is reported and keeps the longest sources", () => {
        const mk = (run: string, seed: number) => {
            const g = new Rng(seed);
            const token = `eyJhbGci${g.hex(300)}`;
            const images = Object.fromEntries(Array.from({ length: 80 }, (_, k) => [`i${k}`, `eyJhbGci${g.hex(60 + k)}`]));
            return doc(run, [req(0, "POST", `${B}/a`, { respBody: jsonBody({ accessToken: token, images }) }), req(1, "POST", `${B}/b`, { headers: [["authorization", `Bearer ${token}`]] })]);
        };
        const g = graphOf([mk("a", 1), mk("b", 2)]);
        expect(valueOf(g, 1, "header:authorization")!.class).toBe("derived");
        expect(g.warnings.some((w) => w.kind === "variant-index-truncated")).toBe(true);
    });

    test("HTML: inert containers, bogus comments and --!> comment ends", () => {
        const mk = (run: string, t: string) => doc(run, [req(0, "GET", `${B}/`, { respBody: ["text/html", `<textarea><input type=hidden name=csrf value=DECOYDECOY123></textarea><![CDATA[<input type=hidden name=csrf value=DECOYDECOY456>]]><template><input type=hidden name=csrf value=DECOYDECOY789></template><!-- x --!><input type=hidden name=csrf value=${t}>`] }), req(1, "POST", `${B}/x`, { headers: [["x-csrf", t]] })]);
        const g = graphOf([mk("a", "tok_8f2Kq9ZxLm41AbCd"), mk("b", "tok_Zq81mXk9L2f8QwEr")]);
        expect(valueOf(g, 1, "header:x-csrf")!.source!.location).toBe("html:input[name=csrf]");
    });

    test("digit floods and huge numeric ids stay fast", () => {
        const started = performance.now();
        const mk = (run: string) => doc(run, [req(0, "POST", `${B}/x`, { body: jsonBody({ blob: "1".repeat(1_000_000) }) })]);
        graphOf([mk("a"), mk("b")]);
        const tied = (run: string, a: string, b: string) => doc(run, [req(0, "GET", `${B}/p`, { id: a, tMs: 1000 }), req(1, "GET", `${B}/q`, { id: b, tMs: 1000 })]);
        graphOf([tied("a", "9".repeat(200_000), "9".repeat(200_001)), tied("b", "8".repeat(200_000), "8".repeat(200_001))]);
        expect(performance.now() - started).toBeLessThan(10_000);
    });
});

// ---------------------------------------------------------------------------------------------
// Round 5

describe("round 5", () => {
    test("reordered RPC requests block for digit-bearing and capitalised operation names", () => {
        const pair = (mk: (n: string) => Req, a: string, b: string) => graphOf([doc("a", [mk(a), { ...mk(b), id: "q1", tMs: 1100 }]), doc("b", [mk(b), { ...mk(a), id: "q1", tMs: 1100 }])]);
        const persisted = (n: string) => req(0, "POST", `${B}/graphql`, { body: jsonBody({ operationName: n }) });
        expect(pair(persisted, "GetProductV2", "GetReviewsV2").status).toBe("blocked");
        const get = (n: string) => req(0, "GET", `${B}/graphql?operationName=${n}`);
        expect(pair(get, "cartQuery1", "userQuery1").status).toBe("blocked");
        const rpc = (n: string) => req(0, "POST", `${B}/rpc`, { body: jsonBody({ method: n }) });
        expect(pair(rpc, "cart.v1.get", "order.v1.create").status).toBe("blocked");
        const form = (n: string) => req(0, "POST", `${B}/`, { body: ["application/x-www-form-urlencoded", `Action=${n}`] });
        expect(pair(form, "DescribeInstances", "TerminateInstances").status).toBe("blocked");
    });

    test("versioned paths and readable labels in prefixes and locators still derive", () => {
        const mk = (run: string, id: string) =>
            doc(run, [
                req(0, "GET", `${B}/`, { respBody: ["text/html", `<a href="/api/v1/carts/${id}">cart</a>`] }),
                req(1, "POST", `${B}/cart`, { respBody: jsonBody({ cartId: id }) }),
                req(2, "POST", `${B}/x`, { headers: [["x-ref", `/shop/v2/carts/${id}/items`], ["x-cart", id]] }),
            ]);
        const g = graphOf([mk("a", "cart_8f2Kq9ZxLm41"), mk("b", "cart_Zq81mXk9L2f8")]);
        expect(g.status).toBe("resolved");
        expect(valueOf(g, 2, "header:x-ref")!.class).toBe("derived");
    });

    test("slug- and key-shaped secrets in query strings and paths, and legitimate slugs, hosts and labels", () => {
        const mk = (run: string) =>
            doc(run, [
                req(0, "GET", `${B}/activate/ABCDE-FGHIJ-KLMNO-PQRST/4821-9937-1102-5583?licence=ABCDE-FGHIJ-KLMNO-PQRSX&recovery=correct-horse-battery-staple&voucher=4821-9937-1102-5583`, {
                    headers: [["origin", "https://www.shop24.de"]],
                }),
                req(1, "POST", `${B}/shop/blue-widget-deluxe-edition`, { body: jsonBody({ username: "correct-horse-battery-staple", shippingMethod: "express-next-day-delivery", coupon: "SUMMER-SALE-2026", color: "midnight-blue-metallic" }) }),
            ]);
        const g = graphOf([mk("a"), mk("b")]);
        const text = toJson(g);
        for (const s of ["ABCDE-FGHIJ", "4821-9937", "correct-horse"]) expect(text, s).not.toContain(s);
        expect(valueOf(g, 0, "header:origin")!.value).toBe("https://www.shop24.de");
        expect(g.steps[1]!.path_template).toBe("/shop/blue-widget-deluxe-edition");
        for (const loc of ["json:$.shippingMethod", "json:$.coupon", "json:$.color"]) expect(valueOf(g, 1, loc)!.value, loc).toBeDefined();
    });

    test("a card past 16 KB or after many digit runs is still found", () => {
        const g = (note: string) => graphOf([doc("a", [req(0, "POST", `${B}/x`, { body: jsonBody({ note }) })]), doc("b", [req(0, "POST", `${B}/x`, { body: jsonBody({ note }) })])]);
        expect(toJson(g("lorem ipsum ".repeat(1400) + "card 4242 4242 4242 4242"))).not.toContain("4242 4242");
        expect(toJson(g("order 1000000000001; ".repeat(65) + "4242424242424242"))).not.toContain("4242424242424242");
    });

    test("comment-heavy HTML and unclosed fill( targets stay fast", () => {
        const started = performance.now();
        const mk = (run: string) => doc(run, [req(0, "GET", `${B}/`, { respBody: ["text/html", "<!--x-->".repeat(80_000)] })]);
        graphOf([mk("a"), mk("b")]);
        const reqs = [req(0, "GET", `${B}/a`)];
        const mkT = (run: string) => {
            const t = traceOf(run, [], reqs);
            t.events[0].target = "fill(".repeat(80_000);
            return { har: harOf(reqs), trace: t, cookies: [] } as RunDoc;
        };
        graphOf([mkT("a"), mkT("b")]);
        expect(performance.now() - started).toBeLessThan(8_000);
    });

    test("user agents with embedded keys, other-language password names and quoted selector arguments", () => {
        const mk = (run: string) =>
            doc(run, [req(0, "POST", `${B}/x`, { headers: [["user-agent", "MyApp/4.2.1 (iOS; install sk\x5flive_ABCdef1234567890xyz)"], ["accept", "application/json, sk\x5flive_ABCdef1234567890xyz"]], body: jsonBody({ contraseña: "hunter2024", clave: "hunter2025", wachtwoord: "hunter2026", "mot de passe": "hunter2027" }) })]);
        const text = toJson(graphOf([mk("a"), mk("b")]));
        for (const s of ["sk\x5flive_ABCdef", "hunter202"]) expect(text, s).not.toContain(s);
        const reqs = [req(0, "GET", `${B}/a`)];
        const mkT = (run: string) => {
            const t = traceOf(run, [], reqs);
            t.events[0].target = "#pw >> fill('a)hunter2024') >> :has-text(\"hunter2025\") >> type=hunter2026";
            return { har: harOf(reqs), trace: t, cookies: [] } as RunDoc;
        };
        const t2 = toJson(graphOf([mkT("a"), mkT("b")]));
        for (const s of ["hunter2024", "hunter2025", "hunter2026"]) expect(t2, s).not.toContain(s);
    });

    test("self-closing raw-text tags follow HTML (the slash is ignored): no phantom structured source, the token is still found", () => {
        const mk = (run: string, t: string) => doc(run, [req(0, "GET", `${B}/`, { respBody: ["text/html", `<textarea/><input type=hidden name=csrf value=${t}>`] }), req(1, "POST", `${B}/x`, { headers: [["x-csrf", t]] })]);
        const g = graphOf([mk("a", "tok_8f2Kq9ZxLm41AbCd"), mk("b", "tok_Zq81mXk9L2f8QwEr")]);
        expect(valueOf(g, 1, "header:x-csrf")!.class).toBe("derived");
        expect(valueOf(g, 1, "header:x-csrf")!.source!.location).toBe("body");
    });
});

// ---------------------------------------------------------------------------------------------
// Round 6

describe("round 6", () => {
    const pair = (mk: (n: string) => Req, a: string, b: string, before: Req[] = []) =>
        graphOf([doc("a", [...before, mk(a), { ...mk(b), id: "q9", tMs: 5100 }]), doc("b", [...before, mk(b), { ...mk(a), id: "q9", tMs: 5100 }])]);

    test("an operation name that an earlier response merely lists does not make swapped requests alignable", () => {
        const features = req(0, "GET", `${B}/config`, { respBody: jsonBody({ features: [{ name: "GetCart" }, { name: "GetUser" }], perms: ["update", "delete"] }) });
        const gql = (n: string) => req(5, "POST", `${B}/graphql`, { body: jsonBody({ operationName: n, variables: { id: "42" } }) });
        expect(pair(gql, "GetCart", "GetUser", [features]).status).toBe("blocked");
        const rpc = (n: string) => req(5, "POST", `${B}/rpc`, { body: jsonBody({ action: n }) });
        expect(pair(rpc, "update", "delete", [features]).status).toBe("blocked");
    });

    test("a server nonce in an operation field from a cookie jar or inline HTML still aligns", () => {
        const nonce = (run: string) => (run === "a" ? "act_8f2Kq9ZxLm41" : "act_Zq81mXk9L2f8");
        const mk = (run: string) => {
            const d = doc(run, [req(0, "GET", `${B}/`, { respBody: ["text/html", `<script>var n="${nonce(run)}"</script>`] }), req(1, "POST", `${B}/x`, { body: jsonBody({ action: nonce(run) }) })]);
            return d;
        };
        expect(graphOf([mk("a"), mk("b")]).status).toBe("resolved");
        const jar = (run: string) => {
            const d = doc(run, [req(0, "POST", `${B}/x`, { body: jsonBody({ action: nonce(run) }) })]);
            d.cookies = [{ name: "n", value: nonce(run) }];
            return d;
        };
        expect(graphOf([jar("a"), jar("b")]).status).toBe("resolved");
    });

    test("card scanning is cheap and fails closed", () => {
        const started = performance.now();
        const big = (run: string) => doc(run, [req(0, "POST", `${B}/x`, { body: jsonBody({ blob: "1".repeat(1_000_000), more: "9".repeat(500_000) }) })]);
        graphOf([big("a"), big("b")]);
        expect(performance.now() - started).toBeLessThan(5_000);
        const hidden = "1234567890123x".repeat(8300) + " 4242 4242 4242 4242";
        const g = graphOf([doc("a", [req(0, "POST", `${B}/x`, { body: jsonBody({ note: hidden }) })]), doc("b", [req(0, "POST", `${B}/x`, { body: jsonBody({ note: hidden }) })])]);
        expect(toJson(g)).not.toContain("4242 4242");
    });

    test("provides, select.field and card-shaped keys go through the location filter", () => {
        const mk = (run: string, tok: string) =>
            doc(run, [
                req(0, "POST", `${B}/a`, { respBody: jsonBody({ grants: { [`sess_${tok}`]: tok }, users: { "ada.quill@example.com": tok }, "4111 1111 1111 1111": "x" }) }),
                req(1, "POST", `${B}/b`, { headers: [["x-t", tok]] }),
            ]);
        const text = toJson(graphOf([mk("a", "tok_8f2Kq9ZxLm41AbCd"), mk("b", "tok_Zq81mXk9L2f8QwEr")]));
        for (const s of ["sess_tok_8f2Kq9ZxLm41AbCd", "ada.quill@example.com", "4111 1111"]) expect(text, s).not.toContain(s);
    });

    test("events that reuse an id and have no time are ordered by content", () => {
        const mk = (run: string, swap: boolean) => {
            const reqs = [req(0, "POST", `${B}/x`, { body: jsonBody({ qty: run === "a" ? 2 : 3 }) })];
            const t = traceOf(run, [], reqs);
            const evs = [
                { id: "e1", type: "click", target: "#a", inputs: { qty: String(run === "a" ? 2 : 3) }, requests: ["q0"] },
                { id: "e1", type: "fill", target: "#b", requests: ["q0"] },
            ];
            t.events = swap ? evs.reverse() : evs;
            return { har: harOf(reqs), trace: t, cookies: [] } as RunDoc;
        };
        expect(toJson(graphOf([mk("a", true), mk("b", false)]))).toBe(toJson(graphOf([mk("a", false), mk("b", true)])));
    });

    test("random-case slugs and scheme-relative blobs are not exempt; typed text beyond 256 chars is scrubbed", () => {
        const mk = (run: string) =>
            doc(run, [req(0, "POST", `${B}/x`, { headers: [["accept", "application/vnd.github.v3+json"]], body: jsonBody({ shippingMethod: "KqZxLmAbCd-QwErTyUi-482917", title: "Xyzzy-Plugh-9921-Frobozz", ref: "//a1B2.c3D4+e5F6g", serviceType: "express-next-day-delivery" }) })]);
        const g = graphOf([mk("a"), mk("b")]);
        const text = toJson(g);
        for (const s of ["KqZxLmAbCd", "Xyzzy-Plugh", "a1B2.c3D4"]) expect(text, s).not.toContain(s);
        expect(valueOf(g, 0, "header:accept")!.value).toBe("application/vnd.github.v3+json");
        const reqs = [req(0, "GET", `${B}/a`)];
        const mkT = (run: string) => {
            const t = traceOf(run, [], reqs);
            t.events[0].target = `#pw >> fill('${"x".repeat(400)}hunter2024') >> pressSequentially("hunter2025") >> setValue(hunter2026)`;
            return { har: harOf(reqs), trace: t, cookies: [] } as RunDoc;
        };
        const t2 = toJson(graphOf([mkT("a"), mkT("b")]));
        for (const s of ["hunter2024", "hunter2025", "hunter2026"]) expect(t2, s).not.toContain(s);
    });
});

// ---------------------------------------------------------------------------------------------
// Round 7

describe("round 7", () => {
    const swapped = (mk: (n: string) => Req, a: string, b: string, before: Req[]) =>
        graphOf([doc("a", [...before, mk(a), { ...mk(b), id: "q9", tMs: 5100 }]), doc("b", [...before, mk(b), { ...mk(a), id: "q9", tMs: 5100 }])]);
    const gql = (n: string) => req(5, "POST", `${B}/graphql`, { body: jsonBody({ operationName: n }) });

    test("operation names listed in a script bundle do not make swapped requests alignable", () => {
        for (const bundle of [
            'function a(){return c({operationName:"GetCartV2",query:A})}function b(){return c({operationName:"GetUserV2",query:U})}',
            'q("GetCartV2");\nq("GetUserV2");\n',
            '[\n  "GetCartV2",\n  "GetUserV2",\n]',
        ]) {
            const js = req(0, "GET", `${B}/static/app.js`, { respBody: ["application/javascript", bundle] });
            expect(swapped(gql, "GetCartV2", "GetUserV2", [js]).status).toBe("blocked");
        }
    });

    test("an operation field carrying a server-issued URL or a digitless nonce still aligns", () => {
        const mk = (run: string, id: string) => {
            const next = `/cart/update/${id}`;
            return doc(run, [req(0, "GET", `${B}/c`, { respBody: jsonBody({ next }) }), req(1, "POST", `${B}/x`, { body: ["application/x-www-form-urlencoded", `action=${encodeURIComponent(next)}&qty=1`] })]);
        };
        expect(graphOf([mk("a", "c_8f2Kq9ZxLm41"), mk("b", "c_Zq81mXk9L2f8")]).status).toBe("resolved");
        const nonce = (run: string) => (run === "a" ? "abcdefghijklmnopqrstuvwxy" : "zyxwvutsrqponmlkjihgfedcb");
        const mk2 = (run: string) => {
            const d = doc(run, [req(0, "POST", `${B}/x`, { body: jsonBody({ action: nonce(run) }) })]);
            d.cookies = [{ name: "n", value: nonce(run) }];
            return d;
        };
        expect(graphOf([mk2("a"), mk2("b")]).status).toBe("resolved");
    });

    test("cards glued to other digits, straddling long runs, with odd separators or full-width digits are found", () => {
        const note = (n: string) => graphOf([doc("a", [req(0, "POST", `${B}/x`, { body: jsonBody({ note: n }) })]), doc("b", [req(0, "POST", `${B}/x`, { body: jsonBody({ note: n }) })])]);
        for (const n of [
            "14242 4242 4242 4242", "94111 1111 1111 1111", "15555 5555 5555 4444", "55512344242 4242 4242 4242",
            `${"1".repeat(52)} 4111 1111 1111 1111`, "4242 4242 4242 4242", "4242,4242,4242,4242", "４２４２ ４２４２ ４２４２ ４２４２",
        ]) {
            const text = toJson(note(n));
            expect(text, n).not.toMatch(/4242 4242|4242,4242|4242 4242|1111 1111 1111 1111|４２４２|55512344242/);
            expect(text, n).not.toContain("4444");
        }
    });

    test("automation calls with spaces, snake_case names and typed text are scrubbed; a card straddling the old cut is withheld", () => {
        const text = (t: string) => {
            const reqs = [req(0, "GET", `${B}/a`)];
            const mk = (run: string) => {
                const tr = traceOf(run, [], reqs);
                tr.events[0].target = t;
                return { har: harOf(reqs), trace: tr, cookies: [] } as RunDoc;
            };
            return toJson(graphOf([mk("a"), mk("b")]));
        };
        for (const t of [`fill ("hunter2")`, `send_keys("hunter2")`, `t.typeText('#pw','hunter2')`, `setInputFiles('/Users/ada/passport.pdf')`]) {
            expect(text(t), t).not.toMatch(/hunter2|passport/);
        }
        expect(text(`#x${"x".repeat(1008)} 4111 1111 1111 1111`)).not.toContain("4111 1111 111");
        expect(text("li:nth-child(2) > a")).toContain("nth-child(2)");
    });

    test("blob secrets in descriptive headers and dotted secrets in URL paths are redacted; epoch keys are not", () => {
        const mk = (run: string) =>
            doc(run, [req(0, "POST", `${B}/x`, { headers: [["cache-control", "max-age=0, ext=Ab3/Xy9+Qw7/Lm2+Zt5="], ["accept", "application/json; key=a1B2.c3D4+e5F6g7"]], body: jsonBody({ cdn: "//cdn.x.com/a1B2.c3D4+e5F6g", series: { "1699999999000": 1, "1699999999001": 2, "1699999999002": 3 } }) })]);
        const g = graphOf([mk("a"), mk("b")]);
        const text = toJson(g);
        for (const s of ["Ab3/Xy9", "a1B2.c3D4"]) expect(text, s).not.toContain(s);
        expect(g.steps[0]!.values.filter((v) => v.location.startsWith("json:$.series")).length).toBe(3);
    });
});
