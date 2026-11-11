// Regression tests for the findings of the adversarial review of the first port: every case
// here was a verified failure (wrong class, leaked secret, panic, non-determinism, perf).

import { describe, expect, test } from "bun:test";
import { infer } from "../src/classify.ts";
import { toJson, valueOf } from "../src/graph.ts";
import { parseRecording } from "../src/har.ts";
import { graphOf, harOf, Rng, type Req, type RunDoc, traceOf } from "./support.ts";

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
    test("parallel identical requests, no ids: swapping them changes nothing", () => {
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
            const ids = [`id_${run}AAAA1111`, `id_${run}BBBB2222`];
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
