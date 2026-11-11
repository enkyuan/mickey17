// Flow inference tests (D11, M2): synthetic HARs generated here, truth known by construction.
// Every assertion can fail: the mutation controls delete or alter a source and require the
// dependent value to stop being `derived`.

import { describe, expect, test } from "bun:test";
import { infer } from "../src/classify.ts";
import { toJson, valueOf, classOf } from "../src/graph.ts";
import { parseRecording, type Recording } from "../src/har.ts";
import type { ClassName, FlowGraph } from "../src/types.ts";
import { generated, graphOf, harOf, parseDoc, type Req, Rng, type RunDoc, traceOf, twoRuns } from "./support.ts";

const cls = (g: FlowGraph, step: number, loc: string): ClassName => {
    const c = classOf(g, step, loc);
    if (!c) throw new Error(`no value ${loc} at step ${step}`);
    return c;
};

function derivedFrom(g: FlowGraph, step: number, loc: string) {
    const v = valueOf(g, step, loc);
    expect(v?.class).toBe("derived");
    const s = v!.source!;
    return { where: s.where, step: s.step, location: s.location, via: v!.via! };
}

const entries = (d: RunDoc) => d.har.log.entries as any[];
const headers = (d: RunDoc, i: number) => entries(d)[i].request.headers as { name: string; value: string }[];

function removeStep(docs: RunDoc[], idx: number) {
    for (const d of docs) {
        entries(d).splice(idx, 1);
        (d.trace.events as any[]).splice(idx, 1);
    }
}

describe("the fixture flow", () => {
    test("is classed as constructed", () => {
        const g = graphOf(twoRuns());
        expect(g.blockers).toEqual([]);
        expect(g.status).toBe("resolved");
        expect(g.steps).toHaveLength(6);

        // CSRF: Set-Cookie at step 0 to header and cookie at step 1 and onward.
        expect(derivedFrom(g, 1, "header:x-csrf-token")).toEqual({ where: "response", step: 0, location: "setcookie:csrf", via: "exact" });
        expect(derivedFrom(g, 1, "cookie:csrf").location).toBe("setcookie:csrf");
        expect(derivedFrom(g, 5, "header:x-csrf-token").step).toBe(0);

        // Cart id: response JSON to a URL segment, then to a body field.
        expect(derivedFrom(g, 2, "url.path[2]")).toEqual({ where: "response", step: 1, location: "json:$.cartId", via: "exact" });
        expect(derivedFrom(g, 3, "json:$.cartId")).toEqual({ where: "response", step: 1, location: "json:$.cartId", via: "exact" });

        // Bearer: substring of a Set-Cookie value with a literal prefix.
        const auth = g.edges.find((e) => e.to.step === 3 && e.to.location === "header:authorization")!;
        expect([auth.via, auth.prefix]).toEqual(["substring", "Bearer "]);
        expect(auth.from.location).toBe("setcookie:sid");

        // Redirect query component; base64 of a JSON field.
        expect(derivedFrom(g, 4, "url.query:order")).toEqual({
            where: "response",
            step: 3,
            location: "header:location|query:order",
            via: "exact",
        });
        expect(derivedFrom(g, 4, "header:x-intent")).toEqual({ where: "response", step: 3, location: "json:$.clientSecret", via: "base64" });

        // Inputs follow run parameters; form encoding round-trips.
        for (const [step, loc, name] of [
            [2, "json:$.quantity", "quantity"],
            [3, "json:$.address.zip", "zip"],
            [5, "form:note", "note"],
        ] as const) {
            const v = valueOf(g, step, loc)!;
            expect([v.class, v.input]).toEqual(["input", name]);
        }
        expect(g.inputs.map((i) => i.name)).toEqual(["note", "quantity", "zip"]);

        // Client-computed: idempotency key, signature, timestamp.
        for (const [loc, shape] of [
            ["json:$.idempotencyKey", "uuid"],
            ["json:$.signature", "hex"],
            ["json:$.ts", "epoch-time"],
        ] as const) {
            const v = valueOf(g, 3, loc)!;
            expect(v.class).toBe("client-computed");
            expect(v.shape).toBe(shape);
        }
        expect(g.steps[3]!.needs).toContain("client-compute");

        const sku = valueOf(g, 2, "json:$.sku")!;
        expect([sku.class, sku.value]).toEqual(["constant", "widget-001"]);
        expect(g.steps[0]!.path_template).toBe("/");
        expect(g.steps[2]!.path_template).toBe("/api/cart/{url.path[2]}/items");
        expect(g.steps[3]!.event?.target).toBe("#step3");
        expect(g.steps[1]!.provides).toContain("json:$.cartId");
    });

    test("false edges are not created", () => {
        const g = graphOf(twoRuns());
        // `line` is 1 in run a and 2 in run b in both the response and the later request: a perfect
        // but low-entropy correlation. It must not become an edge.
        expect(cls(g, 3, "json:$.line")).toBe("client-computed");
        // `true`, `USD` and ISO dates recur across responses and requests but never link.
        expect(cls(g, 1, "json:$.currency")).toBe("constant");
        expect(g.edges.every((e) => !e.to.location.includes("currency"))).toBe(true);
    });

    test("a static value that also appears in a response stays constant, with a warning", () => {
        const g = graphOf(twoRuns());
        const v = valueOf(g, 3, "header:x-publishable-key")!;
        // A key-shaped constant is withheld even when it is public: the executor supplies it.
        expect([v.class, v.value, v.redacted]).toEqual(["constant", undefined, true]);
        expect(g.warnings.some((w) => w.kind === "constant-matches-source" && w.location === "header:x-publishable-key")).toBe(true);
    });

    test("credentials and card numbers are never written as constants", () => {
        const docs = twoRuns();
        for (const d of docs) {
            headers(d, 1).push({ name: "authorization", value: "Basic c3RhdGljLWtleTotc2VjcmV0LXZhbHVl" });
            headers(d, 1).push({ name: "x-card", value: "4242 4242 4242 4242" });
        }
        const g = graphOf(docs);
        const a = valueOf(g, 1, "header:authorization")!;
        expect([a.class, a.redacted, a.value]).toEqual(["constant", true, undefined]);
        const c = valueOf(g, 1, "header:x-card")!;
        expect(c.redacted).toBe(true);
        expect(c.value).toBeUndefined();
        const text = toJson(g);
        expect(text).not.toContain("c3RhdGljLWtleTotc2VjcmV0LXZhbHVl");
        expect(text).not.toContain("4242 4242");
    });
});

describe("mutation controls", () => {
    test("deleting the csrf source makes the header client-computed", () => {
        const docs = twoRuns();
        removeStep(docs, 0);
        const g = graphOf(docs);
        for (const loc of ["header:x-csrf-token", "cookie:csrf", "cookie:sid"]) expect(cls(g, 0, loc)).toBe("client-computed");
        expect(g.edges.every((e) => e.from.location !== "setcookie:csrf")).toBe(true);
    });

    test("deleting the cart request makes the cart id a root, not a derived value", () => {
        const docs = twoRuns();
        removeStep(docs, 1);
        const g = graphOf(docs);
        expect(cls(g, 1, "url.path[2]")).toBe("client-computed");
        // Later uses chain to the first use, never to the deleted response.
        const d = derivedFrom(g, 2, "json:$.cartId");
        expect([d.where, d.step, d.location]).toEqual(["request", 1, "url.path[2]"]);
    });

    test("deleting a source in one run only is unclassified, not derived", () => {
        const docs = twoRuns();
        entries(docs[1]!)[1].response.content.text = JSON.stringify({ created: "2026-11-09T10:00:00Z" });
        const g = graphOf(docs);
        expect(cls(g, 2, "url.path[2]")).toBe("unclassified");
        expect(g.status).toBe("blocked");
        expect(g.blockers.some((b) => b.reason.includes("1 of 2 runs") && b.step === 2)).toBe(true);
    });

    test("a coincidental match in a single run is not an edge", () => {
        const docs = twoRuns();
        const nonces = ["nonce-0123456789abcdef", "nonce-fedcba9876543210"];
        docs.forEach((d, i) => headers(d, 5).push({ name: "x-nonce", value: nonces[i]! }));
        entries(docs[0]!)[1].response.headers = [{ name: "x-trace", value: nonces[0]! }];
        const g = graphOf(docs);
        const v = valueOf(g, 5, "header:x-nonce")!;
        expect(v.class).toBe("unclassified");
        expect(v.reason).toContain("1 of 2 runs");
        expect(g.edges.every((e) => e.to.location !== "header:x-nonce")).toBe(true);
    });
});

describe("blocking instead of guessing", () => {
    test("unclassifiable values block the flow", () => {
        // Two parameters carry the same values in every run: the input is ambiguous.
        const docs = twoRuns();
        docs.forEach((d, i) => {
            const v = i === 0 ? "A-value-1" : "B-value-2";
            d.trace.params.alias_one = v;
            d.trace.params.alias_two = v;
            headers(d, 5).push({ name: "x-who", value: v });
        });
        const g = graphOf(docs);
        const v = valueOf(g, 5, "header:x-who")!;
        expect(v.class).toBe("unclassified");
        expect(v.reason).toContain("alias_one");
        expect(g.status).toBe("blocked");
        expect(g.steps[5]!.needs).toContain("blocked");
    });

    test("a value present in only some runs blocks", () => {
        const docs = twoRuns();
        headers(docs[0]!, 5).push({ name: "x-coupon", value: "SPRING-2026-X" });
        expect(cls(graphOf(docs), 5, "header:x-coupon")).toBe("unclassified");
    });

    test("misaligned runs block rather than guess", () => {
        const docs = twoRuns();
        removeStep([docs[1]!], 4);
        const g = graphOf(docs);
        expect(g.status).toBe("blocked");
        expect(g.steps).toEqual([]);
        expect(g.blockers[0]!.reason).toContain("not alignable");
    });

    test("one run is not enough", () => {
        expect(() => infer([parseDoc(twoRuns()[0]!)])).toThrow("at least two");
    });
});

describe("determinism", () => {
    function shuffled(docs: RunDoc[], seed: number): Recording[] {
        const g = new Rng(seed * 2 + 1);
        const recs = docs.map((d) => {
            const har = structuredClone(d.har);
            g.shuffle(har.log.entries);
            const trace = structuredClone(d.trace);
            g.shuffle(trace.events);
            for (const e of trace.events) g.shuffle(e.requests);
            const cookies = [
                { name: "pref", value: "dark-mode-enabled" },
                { name: "tracking", value: "t-9f8e7d6c5b4a" },
            ];
            g.shuffle(cookies);
            return parseRecording(JSON.stringify(har), JSON.stringify(trace), JSON.stringify(cookies));
        });
        g.shuffle(recs);
        return recs;
    }

    test("output is byte-identical for identical and shuffled input", () => {
        const docs = twoRuns();
        const base = toJson(graphOf(docs));
        expect(toJson(graphOf(docs))).toBe(base);
        const withJar = twoRuns();
        for (const d of withJar) {
            d.cookies = [
                { name: "pref", value: "dark-mode-enabled" },
                { name: "tracking", value: "t-9f8e7d6c5b4a" },
            ];
        }
        const jarBase = toJson(graphOf(withJar));
        for (let seed = 0; seed < 40; seed++) expect(toJson(infer(shuffled(docs, seed)))).toBe(jarBase);
        // The jar adds no request values, so the result also equals the jar-less run.
        expect(base).toBe(jarBase);
    });

    test("canonical JSON round-trips", () => {
        const g = graphOf(twoRuns());
        expect(JSON.parse(toJson(g))).toEqual(JSON.parse(JSON.stringify(g)));
        expect(toJson(g).endsWith("}\n")).toBe(true);
        expect(g.schema).toBe("mochi.agent.flow-graph/1");
    });

    test("works without request ids or trace events", () => {
        const docs = twoRuns();
        for (const d of docs) {
            for (const e of entries(d)) delete e._requestId;
            d.trace.events = [];
        }
        const g = graphOf(docs);
        expect(g.status).toBe("resolved");
        expect(derivedFrom(g, 2, "url.path[2]").location).toBe("json:$.cartId");
        expect(g.steps.every((s) => s.event === undefined)).toBe(true);
    });
});

describe("sources and encodings", () => {
    test("a cookie from the starting jar is a source", () => {
        const docs = twoRuns();
        const toks = ["sess_0123456789abcdefABCD", "sess_zyxwvutsrq9876543210"];
        docs.forEach((d, i) => {
            d.cookies = [{ name: "auth", value: toks[i] }];
            headers(d, 2).push({ name: "x-auth", value: toks[i]! });
        });
        const d = derivedFrom(graphOf(docs), 2, "header:x-auth");
        expect([d.where, d.step, d.location]).toEqual(["initial-cookie", null, "cookie:auth"]);
    });

    test("tokens in HTML bodies get a stable locator", () => {
        const docs = twoRuns();
        const toks = ["page-token-aabbccdd1122", "page-token-99887766ffee"];
        docs.forEach((d, i) => {
            entries(d)[0].response.content.text = `<html><script>window.__PT__="${toks[i]}";</script></html>`;
            headers(d, 2).push({ name: "x-page-token", value: toks[i]! });
        });
        const v = valueOf(graphOf(docs), 2, "header:x-page-token")!;
        expect(v.class).toBe("derived");
        expect([v.source!.step, v.source!.location]).toEqual([0, "body"]);
        expect(v.source!.locator!.before.endsWith('window.__PT__="')).toBe(true);
        expect(v.source!.locator!.after.startsWith('";</script>')).toBe(true);
    });

    test("hidden form fields and meta tags are structured sources", () => {
        const docs = twoRuns();
        const toks = ["hid-aaaabbbbccccdddd", "hid-1111222233334444"];
        docs.forEach((d, i) => {
            entries(d)[0].response.content.text =
                `<html><head><meta name="csrf-token" content="${toks[i]}"></head><body><!-- <input type="hidden" name="ghost" value="zzzzzzzz1234"> --><input type="hidden" name="authenticity" value="${toks[i]}"></body></html>`;
            headers(d, 2).push({ name: "x-meta", value: toks[i]! });
        });
        const d = derivedFrom(graphOf(docs), 2, "header:x-meta");
        expect([d.where, d.step, d.location]).toEqual(["response", 0, "html:input[name=authenticity]"]);
    });

    test("encoded variants: percent-encoded query and cookie against a JSON value", () => {
        const raws = ['tok "quoted"/slash+plus=eq 0123', 'tok "other"/slash+plus=eq 9876'];
        const enc = (s: string) => encodeURIComponent(s);
        const docs = twoRuns();
        docs.forEach((d, i) => {
            entries(d)[0].response.content = { mimeType: "application/json", text: JSON.stringify({ opaque: raws[i] }) };
            const e = entries(d)[2];
            e.request.url = `${e.request.url}?ref=${enc(raws[i]!)}`;
        });
        // `url.query` is already decoded by the extractor, so this is an exact match.
        const d1 = derivedFrom(graphOf(docs), 2, "url.query:ref");
        expect([d1.step, d1.location, d1.via]).toEqual([0, "json:$.opaque", "exact"]);

        const docs2 = twoRuns();
        docs2.forEach((d, i) => {
            entries(d)[0].response.content = { mimeType: "application/json", text: JSON.stringify({ opaque: raws[i] }) };
            headers(d, 2).push({ name: "cookie", value: `zz=${enc(raws[i]!)}` });
        });
        const d2 = derivedFrom(graphOf(docs2), 2, "cookie:zz");
        expect([d2.step, d2.location, d2.via]).toEqual([0, "json:$.opaque", "url-encoded"]);
    });

    test("number text is kept exactly (ids above 2^53, 1.0)", () => {
        const docs = twoRuns();
        const ids = ["9007199254740993123", "9007199254740993456"];
        docs.forEach((d, i) => {
            entries(d)[0].response.content = { mimeType: "application/json", text: `{"big": ${ids[i]}, "price": 1.0}` };
            headers(d, 2).push({ name: "x-big", value: ids[i]! });
        });
        const d = derivedFrom(graphOf(docs), 2, "header:x-big");
        expect(d.location).toBe("json:$.big");
    });
});

describe("generated ground truth", () => {
    test("random flows are classed exactly as constructed (and shuffle-stable)", () => {
        for (let seed = 1; seed <= 60; seed++) {
            const runs = 2 + (seed % 2);
            const { docs, truths } = generated(seed, runs);
            const g = graphOf(docs);
            expect(g.status, `seed ${seed}: ${JSON.stringify(g.blockers)}`).toBe("resolved");
            truths.forEach((fields, i) => {
                for (const [loc, t] of fields) {
                    const v = valueOf(g, i, loc);
                    if (!v) throw new Error(`seed ${seed}: missing ${loc}@${i}`);
                    const tag = `seed ${seed} ${loc}@${i}`;
                    if (t.k === "constant") expect(v.class, tag).toBe("constant");
                    else if (t.k === "fresh") expect(v.class, tag).toBe("client-computed");
                    else if (t.k === "input") {
                        expect(v.class, tag).toBe("input");
                        expect(v.input, tag).toBe(t.param);
                    } else {
                        expect(v.class, tag).toBe("derived");
                        const s = v.source!;
                        // The earliest source may be an earlier request that already carried this
                        // value; the root must still be step j.
                        const root = (s.where === "response" && s.step === t.step && s.location === `json:$.${t.field}`) || s.where === "request";
                        expect(root, `${tag}: ${JSON.stringify(s)}`).toBe(true);
                    }
                }
            });
            const base = toJson(g);
            const rng = new Rng(seed + 99);
            const recs = docs.map((d) => {
                const har = structuredClone(d.har);
                rng.shuffle(har.log.entries);
                return parseRecording(JSON.stringify(har), JSON.stringify(d.trace), null);
            });
            rng.shuffle(recs);
            expect(toJson(infer(recs)), `seed ${seed}`).toBe(base);
        }
    });

    test("a deleted source leaves its first consumer a root; later consumers chain to it", () => {
        let checked = 0;
        for (let seed = 1; seed <= 60; seed++) {
            const { docs, truths } = generated(seed, 2);
            // Consumers of each (source step, field), in request order.
            const consumers = new Map<string, [number, string][]>();
            truths.forEach((fields, i) => {
                for (const [loc, t] of fields) {
                    if (t.k !== "derived") continue;
                    const key = `${t.step}/${t.field}`;
                    consumers.set(key, [...(consumers.get(key) ?? []), [i, loc]]);
                }
            });
            const first = [...consumers.entries()][0];
            if (!first) continue;
            const [key, list] = first;
            const j = Number(key.split("/")[0]);
            // The recorded response no longer carries any value at all.
            for (const d of docs) entries(d)[j].response.content.text = "{}";
            const g = graphOf(docs);
            const [i0, loc0] = list[0]!;
            expect(cls(g, i0, loc0), `seed ${seed}: first consumer`).toBe("client-computed");
            for (const [i, loc] of list.slice(1)) {
                const v = valueOf(g, i, loc)!;
                if (i === i0) {
                    // Same request: a request value cannot source another value of the same request.
                    expect(v.class, `seed ${seed}: same-step consumer`).toBe("client-computed");
                } else {
                    expect(v.class, `seed ${seed}: later consumer`).toBe("derived");
                    expect(v.source!.where, `seed ${seed}: later consumer`).toBe("request");
                }
            }
            checked++;
        }
        expect(checked).toBeGreaterThanOrEqual(20);
    });

    test("a large recording is classed correctly and quickly", { timeout: 60_000 }, () => {
        const { docs, truths } = generated(7, 3, 200);
        const started = performance.now();
        const g = graphOf(docs);
        const took = performance.now() - started;
        expect(g.blockers.slice(0, 3)).toEqual([]);
        let derived = 0;
        truths.forEach((fields, i) => {
            for (const [loc, t] of fields) {
                if (t.k === "derived") {
                    expect(cls(g, i, loc)).toBe("derived");
                    derived++;
                }
            }
        });
        expect(derived).toBeGreaterThan(100);
        expect(took).toBeLessThan(30_000);
    });
});

test("harOf / traceOf produce a parseable recording", () => {
    const reqs: Req[] = [{ id: "a", tMs: 1000, method: "GET", url: "https://x.test/", headers: [], status: 200, respHeaders: [] }];
    const rec = parseRecording(JSON.stringify(harOf(reqs)), JSON.stringify(traceOf("r", [], reqs)));
    expect(rec.entries).toHaveLength(1);
});
