// Recorder: drives each scenario through the real client JS of each shop (React bundle, Svelte
// bundle, enhance.js) in a happy-dom window, with `fetch` routed to the shop's in-process handler
// (no sockets), and writes per run
//     recordings/<stack>/<scenario>.run<N>.har          HAR 1.2: every request the page made
//     recordings/<stack>/<scenario>.run<N>.trace.json   interaction trace: event -> request ids + DOM changes
// Deterministic: seeded server ids, pinned Date.now and crypto.randomUUID, virtual HAR times.
// This is a stand-in for the real-browser recorder named in docs/AGENT_SURFACE.md D11 (a browser is
// the oracle there); the HAR and trace formats are the same, so a Chromium recorder can replace it.
//
//   bun record.ts [--stack react|server-rendered|svelte] [--scenario success|...] [--out DIR]
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { FIXED_NOW, type Shop } from "./shared/core";
import type { Handler } from "./shared/http";
import { SCENARIOS, SCENARIO_NAMES, STACKS, type RunInputs, type ScenarioName, type Stack } from "./shared/scenarios";

const ORIGIN = "http://shop.test";
const here = import.meta.dir;

async function shopFor(stack: Stack, seed: number): Promise<{ shop: Shop; handler: Handler }> {
    const mod = await import(`./${stack}/server`);
    return mod.createShop({ seed, clock: () => FIXED_NOW });
}

interface Entry {
    id: string;
    event: string;
    started: number;
    request: Request;
    reqBody: string;
    status: number;
    statusText: string;
    resHeaders: [string, string][];
    resBody: string | null; // null: body not stored (static assets)
    resSize: number;
    mime: string;
    setCookies: string[];
}
interface EventRec {
    id: string;
    type: string;
    target: string;
    inputs: Record<string, string>;
    requests: string[];
    dom: { added: string[]; removed: string[] };
}

export interface Recording {
    stack: Stack;
    scenario: ScenarioName;
    run: number;
    seed: number;
    shop: Shop;
    har: unknown;
    trace: unknown;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function recordRun(stack: Stack, scenario: ScenarioName, runIndex: number): Promise<Recording> {
    const { Window } = await import("happy-dom");
    const run: RunInputs = SCENARIOS[scenario].runs[runIndex];
    const { shop, handler } = await shopFor(stack, run.seed);

    // ---- virtual time and ids so recordings are reproducible ----
    let virtual = FIXED_NOW + run.seed * 1000;
    let uuidN = 0;
    const pad = (n: number, w: number) => String(n).padStart(w, "0");
    const nextUuid = () => `00000000-0000-4000-8000-${pad(run.seed, 4)}${pad(++uuidN, 8)}`;

    const window = new Window({
        url: `${ORIGIN}/`,
        settings: { disableJavaScriptFileLoading: true, disableCSSFileLoading: true, handleDisabledFileLoadingAsSuccess: true },
    });
    const document = window.document;
    const w = window as any;
    w.Date.now = () => (virtual += 10);
    Object.defineProperty(w, "crypto", { value: { randomUUID: nextUuid }, configurable: true });

    // ---- network: page fetch -> in-process handler, with a cookie jar and HAR bookkeeping ----
    const jar = new Map<string, string>();
    const entries: Entry[] = [];
    const events: EventRec[] = [];
    let current: EventRec = { id: "e0", type: "load", target: "document", inputs: {}, requests: [], dom: { added: [], removed: [] } };
    events.push(current);
    let inflight = 0;
    let activity = 0;

    async function transport(input: string | URL | Request, init?: RequestInit, storeBody = true): Promise<Response> {
        const req = new Request(new URL(String(input), `${ORIGIN}/`), init);
        const reqBody = init?.body == null ? "" : String(init.body);
        if (jar.size) req.headers.set("cookie", [...jar].map(([k, v]) => `${k}=${v}`).join("; "));
        const entry: Entry = {
            id: `r${entries.length + 1}`, event: current.id, started: FIXED_NOW + run.seed * 1000 + entries.length * 50, request: req, reqBody,
            status: 0, statusText: "", resHeaders: [], resBody: null, resSize: 0, mime: "", setCookies: [],
        };
        entries.push(entry);
        current.requests.push(entry.id);
        inflight++;
        activity++;
        try {
            const res = await handler(req);
            const text = await res.clone().text();
            entry.status = res.status;
            entry.statusText = res.statusText || (res.status < 400 ? "OK" : "Error");
            res.headers.forEach((v, k) => k !== "set-cookie" && entry.resHeaders.push([k, v]));
            entry.setCookies = res.headers.getSetCookie();
            entry.resBody = storeBody ? text : null;
            entry.resSize = text.length;
            entry.mime = (res.headers.get("content-type") ?? "").split(";")[0];
            for (const c of entry.setCookies) {
                const [pair] = c.split(";");
                const i = pair.indexOf("=");
                jar.set(pair.slice(0, i), pair.slice(i + 1));
                if (!/;\s*HttpOnly/i.test(c)) document.cookie = pair;
            }
            return res;
        } finally {
            inflight--;
            activity++;
        }
    }
    w.fetch = (input: any, init?: RequestInit) => transport(input, init);

    const mutationLog = () => current.dom;
    const seen = (list: string[], v: string) => list.includes(v) || list.push(v);
    const label = (n: any) => (n.nodeType === 1 ? (n.id ? `#${n.id}` : n.getAttribute?.("data-fragment") ? `[data-fragment=${n.getAttribute("data-fragment")}]` : null) : null);
    new w.MutationObserver((recs: any[]) => {
        activity += recs.length;
        for (const r of recs) {
            for (const n of r.addedNodes) (l => l && seen(mutationLog().added, l))(label(n));
            for (const n of r.removedNodes) (l => l && seen(mutationLog().removed, l))(label(n));
        }
    }).observe(document, { childList: true, subtree: true });

    // ---- load the page, then run its scripts (happy-dom has no module loader) ----
    const page = await transport("/", { method: "GET" });
    document.write(await page.text());
    const signSrc = await Bun.file(join(here, "shared", "sign.js")).text();
    for (const s of [...document.querySelectorAll("script[src]")] as any[]) {
        const src = s.getAttribute("src") as string;
        const res = await transport(src, { method: "GET" }, false);
        let code = await res.text();
        if (src.endsWith("enhance.js")) {
            await transport("/static/sign.js", { method: "GET" }, false);
            code = code.replace(/import \{ sign \} from "\/static\/sign\.js";/, `const sign = (() => { ${signSrc.replace(/^export /m, "")}; return sign; })();`);
        }
        w.eval(code);
    }

    // ---- driver ----
    const q = (sel: string) => document.querySelector(sel) as any;
    async function settle() {
        let quiet = 0;
        let last = -1;
        while (quiet < 4) {
            await sleep(4);
            if (inflight === 0 && activity === last) quiet++;
            else quiet = 0;
            last = activity;
        }
    }
    async function waitFor(sel: string, ms = 4000) {
        for (let t = 0; t < ms; t += 4) {
            if (q(sel)) return q(sel);
            await sleep(4);
        }
        throw new Error(`[${stack}/${scenario}/run${runIndex}] timed out waiting for ${sel}\n${document.body.innerHTML.slice(0, 1500)}`);
    }
    function beginEvent(type: string, target: string, inputs: Record<string, string> = {}) {
        current = { id: `e${events.length}`, type, target, inputs, requests: [], dom: { added: [], removed: [] } };
        events.push(current);
    }
    const fire = (el: any, type: string) => el.dispatchEvent(new w.Event(type, { bubbles: true }));
    function setValue(el: any, v: string) {
        el.value = v;
        fire(el, "input");
        fire(el, "change");
    }
    async function click(sel: string, inputs: Record<string, string> = {}) {
        const el = await waitFor(sel);
        beginEvent("click", sel, inputs);
        el.click();
        await settle();
    }
    async function expectAlert(code: string) {
        const a = await waitFor("[role=alert]");
        if (a.getAttribute("data-error") !== code) throw new Error(`expected alert ${code}, got ${a.getAttribute("data-error")}`);
    }
    async function addItem(sku: string, quantity: string) {
        const input = await waitFor(`li[data-sku="${sku}"] input[name=quantity]`);
        setValue(input, quantity);
        await click(`li[data-sku="${sku}"] [data-action=add]`, { quantity });
    }
    async function fillAddress(values: Record<string, string>) {
        const form = await waitFor("#address-form");
        for (const [k, v] of Object.entries(values)) setValue(form.querySelector(`[name=${k}]`), v);
        await click("#save-address", values);
    }
    async function chooseRate(labelText: string) {
        const radio = await waitFor(`#rates input[name=rate][data-rate-label="${labelText}"]`);
        radio.checked = true;
        fire(radio, "change");
        await click("#save-rate", { rate: labelText });
    }
    async function pay(method: string) {
        const sel = await waitFor("#pay-method");
        setValue(sel, method);
        await click("#pay", { method });
    }

    await settle();
    await waitFor("#catalog li[data-sku]");
    await waitFor("#start-cart");
    await click("#start-cart");
    await waitFor("#cart-summary");
    if (scenario === "validation-error") {
        await addItem(run.sku, run.badQuantity!);
        await expectAlert("validation_failed");
    }
    await addItem(run.sku, run.quantity);
    if (scenario === "out-of-stock") {
        await expectAlert("out_of_stock");
    } else {
        await waitFor(`#cart-summary [data-line-sku="${run.sku}"]`);
        if (scenario === "validation-error") {
            await fillAddress({ ...run.address, ...run.badAddress } as Record<string, string>);
            await expectAlert("validation_failed");
        }
        await fillAddress(run.address as unknown as Record<string, string>);
        await waitFor("#rates input[name=rate]");
        await chooseRate(run.rate);
        await waitFor("#payment");
        await pay(scenario === "declined" ? "test_declined" : "test_ok");
        if (scenario === "declined") await expectAlert("card_declined");
        else await waitFor("#order-status[data-status=confirmed]");
    }
    await settle();

    const finalDom = {
        orderId: q("#confirmation")?.getAttribute("data-order-id") ?? null,
        orderStatus: q("#order-status")?.getAttribute("data-status") ?? null,
        alert: q("[role=alert]")?.getAttribute("data-error") ?? null,
    };
    window.close();
    return { stack, scenario, run: runIndex, seed: run.seed, shop, har: buildHar(entries, stack, scenario, runIndex), trace: buildTrace(entries, events, { stack, scenario, runIndex, run, finalDom }) };
}

const headerList = (h: Headers) => {
    const out: { name: string; value: string }[] = [];
    h.forEach((value, name) => out.push({ name, value }));
    return out;
};

function buildHar(entries: Entry[], stack: Stack, scenario: ScenarioName, run: number) {
    const started = new Date(entries[0].started).toISOString();
    return {
        log: {
            version: "1.2",
            creator: { name: "mochi-agent-fixture-recorder", version: "1" },
            comment: `${stack} / ${scenario} / run ${run + 1}; in-process, virtual time; static asset bodies omitted`,
            pages: [{ startedDateTime: started, id: "page_1", title: `${stack} shop`, pageTimings: {} }],
            entries: entries.map((e) => {
                const url = new URL(e.request.url);
                const reqHeaders = headerList(e.request.headers);
                const mime = e.request.headers.get("content-type") ?? "";
                return {
                    pageref: "page_1",
                    startedDateTime: new Date(e.started).toISOString(),
                    time: 0,
                    request: {
                        method: e.request.method,
                        url: e.request.url,
                        httpVersion: "HTTP/1.1",
                        cookies: (e.request.headers.get("cookie") ?? "").split("; ").filter(Boolean).map((c) => ({ name: c.split("=")[0], value: c.slice(c.indexOf("=") + 1) })),
                        headers: reqHeaders,
                        queryString: [...url.searchParams].map(([name, value]) => ({ name, value })),
                        ...(e.reqBody ? { postData: { mimeType: mime, text: e.reqBody } } : {}),
                        headersSize: -1,
                        bodySize: e.reqBody.length,
                    },
                    response: {
                        status: e.status,
                        statusText: e.statusText,
                        httpVersion: "HTTP/1.1",
                        cookies: e.setCookies.map((c) => {
                            const [pair, ...attrs] = c.split(";").map((s) => s.trim());
                            const i = pair.indexOf("=");
                            return { name: pair.slice(0, i), value: pair.slice(i + 1), path: "/", httpOnly: attrs.some((a) => /^httponly$/i.test(a)) };
                        }),
                        headers: e.resHeaders.map(([name, value]) => ({ name, value })),
                        content: { size: e.resSize, mimeType: e.mime, ...(e.resBody !== null ? { text: e.resBody } : { comment: "body omitted (static asset)" }) },
                        redirectURL: "",
                        headersSize: -1,
                        bodySize: e.resSize,
                    },
                    cache: {},
                    timings: { send: 0, wait: 0, receive: 0 },
                    _requestId: e.id,
                    _eventId: e.event,
                };
            }),
        },
    };
}

function buildTrace(entries: Entry[], events: EventRec[], meta: { stack: Stack; scenario: ScenarioName; runIndex: number; run: RunInputs; finalDom: unknown }) {
    return {
        version: 1,
        stack: meta.stack,
        scenario: meta.scenario,
        run: meta.runIndex + 1,
        seed: meta.run.seed,
        inputs: { sku: meta.run.sku, quantity: meta.run.quantity, rate: meta.run.rate, address: meta.run.address, badQuantity: meta.run.badQuantity, badAddress: meta.run.badAddress },
        events: events.map((e) => ({ id: e.id, type: e.type, target: e.target, inputs: e.inputs, requests: e.requests, domChanges: e.dom })),
        requests: entries.map((e) => ({ id: e.id, method: e.request.method, path: new URL(e.request.url).pathname, status: e.status, initiator: e.event })),
        finalDom: meta.finalDom,
        note: "events are scripted user actions; `load` (e0) covers the document, script loads and requests issued on page load; DOM-level listener inventory is not available in this recorder",
    };
}

if (import.meta.main) {
    const arg = (n: string) => {
        const i = Bun.argv.indexOf(`--${n}`);
        return i > 0 ? Bun.argv[i + 1] : undefined;
    };
    const stacks = (arg("stack") ? [arg("stack")!] : [...STACKS]) as Stack[];
    const scenarios = (arg("scenario") ? [arg("scenario")!] : [...SCENARIO_NAMES]) as ScenarioName[];
    const out = arg("out") ?? join(here, "recordings");
    for (const stack of stacks)
        for (const scenario of scenarios)
            for (let i = 0; i < SCENARIOS[scenario].runs.length; i++) {
                const rec = await recordRun(stack, scenario, i);
                const dir = join(out, stack);
                await mkdir(dir, { recursive: true });
                const base = join(dir, `${scenario}.run${i + 1}`);
                await writeFile(`${base}.har`, JSON.stringify(rec.har, null, 1) + "\n");
                await writeFile(`${base}.trace.json`, JSON.stringify(rec.trace, null, 1) + "\n");
                console.log(`${stack}/${scenario}.run${i + 1}: ${(rec.har as any).log.entries.length} requests`);
            }
}
