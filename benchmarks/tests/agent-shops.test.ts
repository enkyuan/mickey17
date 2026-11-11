// The three fixture shops (React, server-rendered, Svelte) must behave identically over HTTP
// for every scenario, and their hand-labelled ground truth must hold. See
// benchmarks/fixtures/agent/shops/README.md and docs/agent-surface/log/agent-fixtures.md.
//
// Most tests call each shop's `fetch` handler in-process (no socket). The socket tests run only
// where loopback listening is allowed and are reported as skipped otherwise; a skip is not a pass.
import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { FIXED_NOW, SIGNATURE_TOLERANCE_MS, SEED_CATALOG, type Shop } from "../fixtures/agent/shops/shared/core";
import type { Handler } from "../fixtures/agent/shops/shared/http";
import { JsonClient, SsrClient, runScenario, type ShopClient, type Step } from "../fixtures/agent/shops/shared/clients";
import { SCENARIO_NAMES, SCENARIOS, STACKS, type ScenarioName, type Stack } from "../fixtures/agent/shops/shared/scenarios";
import { sign } from "../fixtures/agent/shops/shared/sign.js";
import { checkTruthAcrossRuns, checkTruthAgainstHar, graphSignature, requestsOf, validateTruth, type Truth } from "../fixtures/agent/shops/shared/truth";
import { createShop as createReact } from "../fixtures/agent/shops/react/server";
import { createShop as createSsr } from "../fixtures/agent/shops/server-rendered/server";
import { createShop as createSvelte } from "../fixtures/agent/shops/svelte/server";

const ROOT = join(import.meta.dir, "..", "fixtures", "agent", "shops");
const factories = { react: createReact, "server-rendered": createSsr, svelte: createSvelte };

interface Setup {
    shop: Shop;
    handler: Handler;
    client: ShopClient;
}
function setup(stack: Stack, seed: number): Setup {
    const { shop, handler } = factories[stack]({ seed, clock: () => FIXED_NOW });
    const client: ShopClient =
        stack === "server-rendered" ? new SsrClient(handler) : new JsonClient(stack, stack === "react" ? "meta" : "cookie", handler);
    return { shop, handler, client };
}

// What each scenario must produce, step by step (status codes are the observable contract).
const EXPECTED: Record<ScenarioName, [string, number][]> = {
    success: [["catalog", 200], ["create-cart", 201], ["add-item", 200], ["set-address", 200], ["shipping-rates", 200], ["select-shipping", 200], ["tokenize-payment", 201], ["place-order", 201], ["order-status", 200]],
    "validation-error": [["catalog", 200], ["create-cart", 201], ["add-item", 422], ["add-item", 200], ["set-address", 422], ["set-address", 200], ["shipping-rates", 200], ["select-shipping", 200], ["tokenize-payment", 201], ["place-order", 201], ["order-status", 200]],
    "out-of-stock": [["catalog", 200], ["create-cart", 201], ["add-item", 409]],
    declined: [["catalog", 200], ["create-cart", 201], ["add-item", 200], ["set-address", 200], ["shipping-rates", 200], ["select-shipping", 200], ["tokenize-payment", 201], ["place-order", 402]],
};

describe("shops behave identically over HTTP", () => {
    for (const scenario of SCENARIO_NAMES)
        SCENARIOS[scenario].runs.forEach((run, i) => {
            test(`${scenario} run ${i + 1}`, async () => {
                const results = new Map<Stack, { steps: Step[]; snapshot: unknown }>();
                for (const stack of STACKS) {
                    const s = setup(stack, run.seed);
                    results.set(stack, { steps: await runScenario(s.client, scenario, i), snapshot: s.shop.snapshot() });
                }
                const ref = results.get("react")!;
                // the scripted outcome is what the scenario promises, not merely what the stacks agree on
                expect(ref.steps.map((s) => [s.step, s.status])).toEqual(EXPECTED[scenario]);
                for (const stack of STACKS) {
                    const r = results.get(stack)!;
                    expect(r.steps.map((s) => [s.step, s.status])).toEqual(EXPECTED[scenario]);
                    // same response shapes and values (seeded ids line up because every stack draws in the same order)
                    r.steps.forEach((s, k) => expect({ stack, step: s.step, view: s.view }).toEqual({ stack, step: ref.steps[k].step, view: ref.steps[k].view }));
                    // same final shop state
                    expect(r.snapshot).toEqual(ref.snapshot);
                }
            });
        });

    test("final order state matches the quote in the success scenario", async () => {
        for (const stack of STACKS) {
            const s = setup(stack, 11);
            const steps = await runScenario(s.client, "success", 0);
            const snap = s.shop.snapshot();
            expect(snap.orders).toHaveLength(1);
            const o = snap.orders[0];
            const mug = SEED_CATALOG.find((p) => p.sku === "mug")!;
            expect(o.subtotalCents).toBe(2 * mug.priceCents);
            expect(o.shipping.label).toBe("Standard");
            expect(o.totalCents).toBe(o.subtotalCents + o.shipping.amountCents + o.taxCents);
            expect(snap.stock.mug).toBe(mug.stock - 2);
            expect(steps.at(-1)!.view.order).toEqual(o);
        }
    });

    test("failed scenarios leave no order and the stock untouched", async () => {
        for (const scenario of ["out-of-stock", "declined"] as const)
            for (const stack of STACKS) {
                const s = setup(stack, SCENARIOS[scenario].runs[0].seed);
                await runScenario(s.client, scenario, 0);
                const snap = s.shop.snapshot();
                expect(snap.orders).toHaveLength(0);
                expect(snap.stock).toEqual(Object.fromEntries(SEED_CATALOG.map((p) => [p.sku, p.stock]).sort()));
            }
    });

    test("different seeds give different ids; the same seed repeats", async () => {
        const ids = async (seed: number) => (await runScenario(setup("react", seed).client, "success", 0)).find((s) => s.step === "create-cart")!.view.cart.id;
        expect(await ids(1)).toBe(await ids(1));
        expect(await ids(1)).not.toBe(await ids(2));
    });
});

describe("request value classes behave as labelled", () => {
    test("signature vectors (the one client-computed value)", () => {
        expect(sign(1768478400000, "")).toBe("v1.1768478400000.090c6e05595146");
        expect(sign(1768478400000, '{"paymentToken":"tok_test_ok_deadbeef"}')).toBe("v1.1768478400000.081b3779048257");
        expect(sign(0, "a")).toBe("v1.0.032b7566ee92af");
    });

    for (const stack of STACKS)
        describe(stack, () => {
            const newClient = (handler: Handler): ShopClient =>
                stack === "server-rendered" ? new SsrClient(handler) : new JsonClient(stack, stack === "react" ? "meta" : "cookie", handler);
            /** A complete cart (item, address, rate) and a fresh token for `client`. */
            async function prepare(client: ShopClient) {
                await client.open();
                const cartId = (await client.createCart()).view.cart.id as string;
                await client.add(cartId, "mug", "1");
                await client.address(cartId, SCENARIOS.success.runs[0].address);
                const rates = (await client.rates(cartId)).view.rates as { id: string }[];
                await client.chooseRate(cartId, rates[0].id);
                const token = (await client.tokenize("test_ok")).view.token as string;
                return { cartId, token };
            }
            async function ready(seed = 5) {
                const s = setup(stack, seed);
                return { ...s, ...(await prepare(s.client)) };
            }
            const code = (o: { view: any }) => o.view.error?.code;

            test("CSRF token is required and comes from this stack's own source", async () => {
                const { client, cartId, token } = await ready();
                expect(code(await client.placeOrder(cartId, token, { csrf: null }))).toBe("csrf_invalid");
                expect((await client.placeOrder(cartId, token, { csrf: null })).status).toBe(403);
                expect((await client.placeOrder(cartId, token, { csrf: "csrf_wrong" })).status).toBe(403);
            });
            test("CSRF is enforced on every mutating endpoint, not just place-order", async () => {
                const { client, cartId } = await ready();
                const good = client.csrf;
                for (const bad of [null, "csrf_wrong"]) {
                    client.csrf = bad;
                    const results = [await client.createCart(), await client.add(cartId, "mug", "1"), await client.address(cartId, SCENARIOS.success.runs[0].address), await client.chooseRate(cartId, "rate_x")];
                    for (const r of results) expect([r.status, code(r)]).toEqual([403, "csrf_invalid"]);
                }
                client.csrf = good;
                expect((await client.add(cartId, "mug", "1")).status).toBe(200);
                // reads need no token
                client.csrf = null;
                expect((await client.rates(cartId)).status).toBe(200);
            });
            test("quantity text is parsed strictly and identically", async () => {
                const { client, cartId } = await ready();
                for (const q of ["2.0", "1e1", "0x2", "+2", " ", "", "11", "-1", "0", "abc", "１２"]) {
                    const r = await client.add(cartId, "tote", q);
                    expect([q, r.status, code(r)]).toEqual([q, 422, "validation_failed"]);
                }
                expect((await client.add(cartId, "tote", " 3 ")).status).toBe(200);
            });
            test("core edge cases: repeat quote, stale rate, name length, stock at order time, closed cart", async () => {
                const s = setup(stack, 12);
                await s.client.open();
                const c = s.client;
                const cartId = (await c.createCart()).view.cart.id as string;
                await c.add(cartId, "kettle", "2");
                expect(code(await c.address(cartId, { ...SCENARIOS.success.runs[0].address, name: "x".repeat(81) }))).toBe("validation_failed");
                await c.address(cartId, SCENARIOS.success.runs[0].address);
                const q1 = (await c.rates(cartId)).view.rates;
                expect((await c.rates(cartId)).view.rates).toEqual(q1); // a repeated read returns the same quote
                // changing the cart invalidates the quote
                await c.add(cartId, "mug", "1");
                expect(code(await c.chooseRate(cartId, q1[0].id))).toBe("validation_failed");
                const q2 = (await c.rates(cartId)).view.rates;
                expect(q2).not.toEqual(q1);
                await c.chooseRate(cartId, q2[0].id);
                // a second cart takes the same kettles: both fit at add time, only one can be ordered
                const other = (await c.createCart()).view.cart.id as string;
                expect((await c.add(other, "kettle", "2")).status).toBe(200);
                await c.address(other, SCENARIOS.success.runs[1].address);
                const q3 = (await c.rates(other)).view.rates;
                await c.chooseRate(other, q3[0].id);
                const t1 = (await c.tokenize("test_ok")).view.token as string;
                const t2 = (await c.tokenize("test_ok")).view.token as string;
                expect((await c.placeOrder(cartId, t1, { key: "s1" })).status).toBe(201);
                const late = await c.placeOrder(other, t2, { key: "s2" });
                expect([late.status, code(late)]).toEqual([409, "out_of_stock"]);
                // an ordered cart cannot change its rate either
                expect(code(await c.chooseRate(cartId, q2[0].id))).toBe("cart_closed");
            });
            test("idempotency keys are scoped to the session", async () => {
                const a = await ready(21);
                const bClient = newClient(a.handler); // a second session of the same shop
                const b = await prepare(bClient);
                expect((await a.client.placeOrder(a.cartId, a.token, { key: "same" })).status).toBe(201);
                const r = await bClient.placeOrder(b.cartId, b.token, { key: "same" });
                expect(r.status).toBe(201);
                expect(r.headers["idempotent-replayed"]).toBeUndefined();
            });
            test("tampered, stale and missing signatures are rejected", async () => {
                const { client, cartId, token } = await ready();
                const bad = await client.placeOrder(cartId, token, { signature: "v1.1768478400000.00000000000000" });
                expect([bad.status, code(bad)]).toEqual([401, "invalid_signature"]);
                const stale = await client.placeOrder(cartId, token, { now: FIXED_NOW - SIGNATURE_TOLERANCE_MS - 1 });
                expect([stale.status, code(stale)]).toEqual([401, "signature_expired"]);
                const missing = await client.placeOrder(cartId, token, { signature: null });
                expect([missing.status, code(missing)]).toEqual([401, "invalid_signature"]);
            });
            test("idempotency key: required, replayed, and bound to its request", async () => {
                const { client, cartId, token, shop } = await ready();
                const none = await client.placeOrder(cartId, token, { key: null });
                expect([none.status, code(none)]).toEqual([400, "idempotency_key_required"]);
                const first = await client.placeOrder(cartId, token, { key: "k1" });
                expect(first.status).toBe(201);
                const replay = await client.placeOrder(cartId, token, { key: "k1" });
                expect(replay.status).toBe(201);
                expect(replay.headers["idempotent-replayed"]).toBe("true");
                expect(replay.view.order.id).toBe(first.view.order.id);
                expect(shop.snapshot().orders).toHaveLength(1);
                const other = (await client.tokenize("test_ok")).view.token as string;
                const reuse = await client.placeOrder(cartId, other, { key: "k1" });
                expect([reuse.status, code(reuse)]).toEqual([422, "idempotency_key_reuse"]);
            });
            test("stale or foreign cart ids and reused tokens fail", async () => {
                const { client, cartId, token, handler } = await ready();
                const ghost = await client.add("cart_deadbeef", "mug", "1");
                expect([ghost.status, code(ghost)]).toEqual([404, "cart_not_found"]);
                expect((await client.placeOrder(cartId, token, { key: "a" })).status).toBe(201);
                const closed = await client.add(cartId, "mug", "1");
                expect([closed.status, code(closed)]).toEqual([409, "cart_closed"]);
                // the used token is dead: a second, complete cart cannot spend it
                const second = (await client.createCart()).view.cart.id as string;
                await client.add(second, "mug", "1");
                await client.address(second, SCENARIOS.success.runs[0].address);
                const quote = (await client.rates(second)).view.rates as { id: string }[];
                await client.chooseRate(second, quote[0].id);
                const reused = await client.placeOrder(second, token, { key: "b" });
                expect([reused.status, code(reused)]).toEqual([422, "invalid_payment_token"]);
                // a different session of the SAME shop cannot touch this cart or read its order
                const orderId = (await client.placeOrder(second, (await client.tokenize("test_ok")).view.token, { key: "c" })).view.order.id as string;
                const intruder = newClient(handler);
                await intruder.open();
                for (const o of [await intruder.add(second, "mug", "1"), await intruder.address(second, SCENARIOS.success.runs[0].address), await intruder.rates(second), await intruder.order(orderId)])
                    expect([o.status, code(o)]).toEqual([404, o.view.error.code === "order_not_found" ? "order_not_found" : "cart_not_found"]);
                expect((await client.order(orderId)).status).toBe(200);
            });
            test("rate must come from the latest quote; the cart must be complete to order", async () => {
                const s = setup(stack, 8);
                await s.client.open();
                const cartId = (await s.client.createCart()).view.cart.id as string;
                await s.client.add(cartId, "tote", "1");
                expect(code(await s.client.rates(cartId))).toBe("address_required");
                await s.client.address(cartId, SCENARIOS.success.runs[1].address);
                const bad = await s.client.chooseRate(cartId, "rate_nope");
                expect([bad.status, code(bad), Object.keys(bad.view.error.fields)]).toEqual([422, "validation_failed", ["rateId"]]);
                const token = (await s.client.tokenize("test_ok")).view.token as string;
                const early = await s.client.placeOrder(cartId, token, { key: "z" });
                expect([early.status, code(early), early.view.error.missing]).toEqual([409, "cart_incomplete", ["shippingRate"]]);
            });
            test("payment stub rejects anything but a test method id", async () => {
                const s = setup(stack, 9);
                const r = await s.client.tokenize("not_a_test_method");
                expect([r.status, code(r)]).toEqual([422, "validation_failed"]);
            });
        });

    test("each shell exposes the CSRF token only through its own source", async () => {
        const html = async (stack: Stack) => {
            const { handler } = setup(stack, 3);
            const res = await handler(new Request("http://shop.test/"));
            return { text: await res.text(), cookies: res.headers.getSetCookie() };
        };
        const react = await html("react");
        expect(react.text).toMatch(/<meta name="csrf-token" content="csrf_[0-9a-f]+"/);
        expect(react.cookies.some((c) => c.startsWith("csrf="))).toBe(false);
        const svelte = await html("svelte");
        expect(svelte.text).not.toMatch(/csrf_/);
        expect(svelte.cookies.some((c) => c.startsWith("csrf=") && !/HttpOnly/i.test(c))).toBe(true);
        const ssr = await html("server-rendered");
        expect(ssr.text).toMatch(/name="_csrf" value="csrf_[0-9a-f]+"/); // hidden form field in the page
        expect(ssr.cookies.some((c) => c.startsWith("csrf="))).toBe(false);
        // and without that field or the header there is no other source: the post is refused
        const { handler } = setup("server-rendered", 3);
        const open = await handler(new Request("http://shop.test/"));
        const sid = open.headers.getSetCookie()[0].split(";")[0];
        const cart = await handler(new Request("http://shop.test/carts", { method: "POST", headers: { cookie: sid } }));
        expect(cart.status).toBe(403);
    });
});

describe("ground truth", () => {
    const truths = Object.fromEntries(STACKS.map((s) => [s, JSON.parse(readFileSync(join(ROOT, s, "flow.truth.json"), "utf8")) as Truth])) as Record<Stack, Truth>;

    for (const stack of STACKS)
        test(`${stack}: well-formed (classes, edges, one client-computed value, one idempotency key)`, () => {
            expect(validateTruth(truths[stack])).toEqual([]);
            expect(truths[stack].stack).toBe(stack);
        });

    test("the three stacks have the same flow graph (steps, effects, value classes, dependency edges)", () => {
        const ref = graphSignature(truths.react);
        expect(graphSignature(truths["server-rendered"])).toEqual(ref);
        expect(graphSignature(truths.svelte)).toEqual(ref);
    });

    test("step coverage: every scenario step name used by the runner is a truth step", () => {
        const ids = new Set(truths.react.steps.map((s) => s.id));
        for (const sc of SCENARIO_NAMES) for (const [step] of EXPECTED[sc]) expect(ids.has(step)).toBe(true);
    });

    test("the validator can fail: mutated truth is rejected", () => {
        const t = structuredClone(truths.react);
        const edit = (f: (t: Truth) => void) => {
            const c = structuredClone(truths.react);
            f(c);
            return validateTruth(c);
        };
        expect(edit((c) => (requestsOf(c).find((r) => r.id === "place-order")!.values.find((v) => v.at === "header.x-signature")!.class = "input"))).not.toEqual([]);
        expect(edit((c) => (c.edges = c.edges.filter((e) => !e.from.startsWith("tokenize-payment"))))).not.toEqual([]);
        expect(edit((c) => (requestsOf(c).find((r) => r.id === "create-cart")!.values.find((v) => v.at === "header.x-csrf-token")!.class = "constant"))).not.toEqual([]);
        expect(validateTruth(t)).toEqual([]);
    });

    const recDir = join(ROOT, "recordings");
    const hasRecordings = existsSync(recDir);
    (hasRecordings ? describe : describe.skip)("against the committed recordings", () => {
        const har = (stack: Stack, sc: string, run: number) => JSON.parse(readFileSync(join(recDir, stack, `${sc}.run${run}.har`), "utf8"));
        const trace = (stack: Stack, sc: string, run: number) => JSON.parse(readFileSync(join(recDir, stack, `${sc}.run${run}.trace.json`), "utf8"));

        for (const stack of STACKS)
            for (const sc of SCENARIO_NAMES)
                SCENARIOS[sc].runs.forEach((_r, i) => {
                    test(`${stack} ${sc} run ${i + 1}: HAR 1.2, trace links, and every labelled value holds`, () => {
                        const h = har(stack, sc, i + 1);
                        expect(h.log.version).toBe("1.2");
                        expect(h.log.creator.name).toBeTruthy();
                        const ids = new Set<string>(h.log.entries.map((e: any) => e._requestId));
                        expect(ids.size).toBe(h.log.entries.length);
                        for (const e of h.log.entries) {
                            for (const k of ["method", "url", "httpVersion", "cookies", "headers", "queryString", "headersSize", "bodySize"]) expect(e.request).toHaveProperty(k);
                            for (const k of ["status", "statusText", "httpVersion", "cookies", "headers", "content", "redirectURL", "headersSize", "bodySize"]) expect(e.response).toHaveProperty(k);
                        }
                        const tr = trace(stack, sc, i + 1);
                        expect(tr.requests.map((r: any) => r.id)).toEqual([...ids]);
                        for (const ev of tr.events) for (const r of ev.requests) expect(ids.has(r)).toBe(true);
                        expect(tr.events.flatMap((e: any) => e.requests).sort()).toEqual([...ids].sort());
                        expect(checkTruthAgainstHar(truths[stack], h)).toEqual([]);
                    });
                });

        test("two runs per scenario differ in inputs and in server-issued ids", () => {
            for (const stack of STACKS)
                for (const sc of SCENARIO_NAMES) {
                    const a = trace(stack, sc, 1);
                    const b = trace(stack, sc, 2);
                    expect(a.seed).not.toBe(b.seed);
                    expect(JSON.stringify(a.inputs)).not.toBe(JSON.stringify(b.inputs));
                    const cartPath = (t: any) => t.requests.find((r: any) => r.method === "POST" && /\/carts\/[^/]+\/items$/.test(r.path)).path;
                    expect(cartPath(a)).not.toBe(cartPath(b));
                }
        });

        test("the recordings exercise each scenario's outcome", () => {
            const statuses = (stack: Stack, sc: string, run: number) => trace(stack, sc, run).requests.map((r: any) => r.status);
            for (const stack of STACKS)
                for (const run of [1, 2]) {
                    expect(statuses(stack, "success", run)).toContain(201);
                    expect(statuses(stack, "validation-error", run).filter((s: number) => s === 422)).toHaveLength(2);
                    expect(statuses(stack, "out-of-stock", run)).toContain(409);
                    expect(statuses(stack, "declined", run)).toContain(402);
                    expect(trace(stack, "success", run).finalDom.orderStatus).toBe("confirmed");
                    expect(trace(stack, "declined", run).finalDom.orderId).toBeNull();
                }
        });

        test("recordings of each stack agree across runs (constants constant, derived/computed fresh, every labelled status observed)", () => {
            for (const stack of STACKS) {
                const hars = SCENARIO_NAMES.flatMap((sc) => [1, 2].map((r) => har(stack, sc, r)));
                expect(checkTruthAcrossRuns(truths[stack], hars)).toEqual([]);
            }
        });

        test("wrong truth is rejected by the combined checks (every stack, every mutation)", () => {
            type Mut = [string, (t: Truth) => void];
            const val = (t: Truth, req: string, at: string) => requestsOf(t).find((r) => r.id === req)!.values.find((v) => v.at === at)!;
            const urlAt = (t: Truth, req: string) => requestsOf(t).find((r) => r.id === req)!.values.find((v) => v.at.startsWith("url.path"))!;
            const bodyAt = (t: Truth, req: string, name: string) => requestsOf(t).find((r) => r.id === req)!.values.find((v) => v.at.endsWith(`.${name}`))!;
            const muts: Mut[] = [
                ["paymentToken derived->input", (t) => { bodyAt(t, "place-order", "paymentToken").class = "input"; t.edges = t.edges.filter((e) => !e.from.startsWith("tokenize-payment")); }],
                ["rateId derived->input", (t) => { const v = requestsOf(t).find((r) => r.id === "select-shipping")!.values.find((x) => /rate/.test(x.at) && x.class === "derived")!; v.class = "input"; t.edges = t.edges.filter((e) => !e.from.startsWith("shipping-rates")); }],
                ["cart id segment derived->input", (t) => { urlAt(t, "place-order").class = "input"; t.edges = t.edges.map((e) => ({ ...e, to: (Array.isArray(e.to) ? e.to : [e.to]).filter((x) => !x.startsWith("place-order:url")) })).filter((e) => e.to.length); }],
                ["idempotency key -> constant", (t) => { const v = val(t, "place-order", "header.idempotency-key"); v.class = "constant"; v.value = "k"; delete v.role; }],
                ["signature -> input", (t) => { val(t, "place-order", "header.x-signature").class = "input"; t.clientComputedCount = 0; }],
                ["place-order 201 relabelled 200", (t) => { (requestsOf(t).find((r) => r.id === "place-order")!.responses as any[])[0].status = 200; }],
                ["add-item 409 dropped", (t) => { const r = requestsOf(t).find((r) => r.id === "add-item")!; r.responses = (r.responses as any[]).filter((x) => x.status !== 409); }],
                ["place-order effect -> read", (t) => { t.steps.find((s) => s.id === "place-order")!.effect = "read"; }],
                ["a constant relabelled with another value", (t) => { val(t, "create-cart", "header.content-type").value = "text/plain"; }],
                ["header label removed", (t) => { const r = requestsOf(t).find((r) => r.id === "place-order")!; r.values = r.values.filter((v) => v.at !== "header.x-signature"); t.clientComputedCount = 0; }],
            ];
            const problems = (t: Truth, stack: Stack) => [
                ...validateTruth(t),
                ...STACKS.length ? SCENARIO_NAMES.flatMap((sc) => [1, 2].flatMap((r) => checkTruthAgainstHar(t, har(stack, sc, r)))) : [],
                ...checkTruthAcrossRuns(t, SCENARIO_NAMES.flatMap((sc) => [1, 2].map((r) => har(stack, sc, r)))),
            ];
            for (const stack of STACKS)
                for (const [name, f] of muts) {
                    const t = structuredClone(truths[stack]);
                    f(t);
                    expect({ stack, name, rejected: problems(t, stack).length > 0 }).toEqual({ stack, name, rejected: true });
                }
        });

        test("a tampered recording is caught (the check can fail)", () => {
            const t = truths.react;
            const h = har("react", "success", 1);
            const mutate = (f: (h: any) => void) => {
                const c = structuredClone(h);
                f(c);
                return checkTruthAgainstHar(t, c);
            };
            const entry = (c: any, id: string) => c.log.entries.find((e: any) => e._requestId && e.request.url.includes(id));
            expect(checkTruthAgainstHar(t, h)).toEqual([]);
            // a CSRF header that matches nothing the page served
            expect(mutate((c) => (entry(c, "/api/carts").request.headers.find((x: any) => x.name === "x-csrf-token").value = "csrf_0000"))).not.toEqual([]);
            // a signature that is not sign(timestamp, body)
            expect(mutate((c) => (entry(c, "/orders").request.headers.find((x: any) => x.name === "x-signature").value = "v1.1.00000000000000"))).not.toEqual([]);
            // an unlabelled extra field in a body
            expect(mutate((c) => (entry(c, "/items").request.postData.text = JSON.stringify({ sku: "mug", quantity: 1, coupon: "X" })))).not.toEqual([]);
        });
    });
});

// The recorder drives the real client JS (React bundle, Svelte bundle, enhance.js) in happy-dom.
// It needs `happy-dom` (a benchmarks devDependency); without it these tests are skipped.
const hasHappyDom = (() => {
    try {
        Bun.resolveSync("happy-dom", ROOT);
        return true;
    } catch {
        return false;
    }
})();
(hasHappyDom ? describe : describe.skip)("browser-driven client code (happy-dom)", () => {
    for (const stack of STACKS)
        for (const sc of SCENARIO_NAMES)
            SCENARIOS[sc].runs.forEach((_r, i) => {
                test(`${stack} ${sc} run ${i + 1}: page JS ends in the same shop state as the HTTP client, and reproduces the committed recording`, async () => {
                    const { recordRun } = await import("../fixtures/agent/shops/record");
                    const rec = await recordRun(stack, sc, i);
                    const http = setup(stack, SCENARIOS[sc].runs[i].seed);
                    await runScenario(http.client, sc, i);
                    expect(rec.shop.snapshot()).toEqual(http.shop.snapshot());
                    const dir = join(ROOT, "recordings", stack);
                    if (existsSync(dir)) {
                        const file = (ext: string) => join(dir, `${sc}.run${i + 1}.${ext}`);
                        expect(JSON.stringify(rec.har, null, 1) + "\n").toBe(readFileSync(file("har"), "utf8"));
                        expect(JSON.stringify(rec.trace, null, 1) + "\n").toBe(readFileSync(file("trace.json"), "utf8"));
                    }
                }, 30_000);
            });
    test("recordings directory holds exactly two runs per stack and scenario", () => {
        const dir = join(ROOT, "recordings");
        if (!existsSync(dir)) return;
        for (const stack of STACKS) expect(readdirSync(join(dir, stack)).filter((f) => f.endsWith(".har"))).toHaveLength(SCENARIO_NAMES.length * 2);
    });
});

// Real sockets: only where loopback listening is allowed. Skipped (not passed) otherwise.
const canListen = (() => {
    try {
        Bun.serve({ port: 0, hostname: "127.0.0.1", fetch: () => new Response("ok") }).stop(true);
        return true;
    } catch {
        return false;
    }
})();
(canListen ? describe : describe.skip)("over real sockets (loopback)", () => {
    for (const stack of STACKS)
        test(`${stack}: success and declined runs over fetch equal the in-process runs`, async () => {
            for (const sc of ["success", "declined"] as const) {
                const run = SCENARIOS[sc].runs[0];
                const inproc = setup(stack, run.seed);
                const expected = await runScenario(inproc.client, sc, 0);
                const live = factories[stack]({ seed: run.seed, clock: () => FIXED_NOW });
                const server = Bun.serve({ port: 0, hostname: "127.0.0.1", fetch: live.handler });
                try {
                    const base = `http://127.0.0.1:${server.port}`;
                    const net = (req: Request) => fetch(req);
                    const client: ShopClient = stack === "server-rendered" ? new SsrClient(net, base) : new JsonClient(stack, stack === "react" ? "meta" : "cookie", net, base);
                    const got = await runScenario(client, sc, 0);
                    expect(got.map((s) => [s.step, s.status, s.view])).toEqual(expected.map((s) => [s.step, s.status, s.view]));
                    expect(live.shop.snapshot()).toEqual(inproc.shop.snapshot());
                } finally {
                    server.stop(true);
                }
            }
        });
});
