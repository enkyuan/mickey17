// Generators for synthetic recordings: the truth is known by construction.

import { infer, type Options } from "../src/classify.ts";
import { parseRecording, type Recording } from "../src/har.ts";
import type { FlowGraph } from "../src/types.ts";

type Json = Record<string, any>;

/** xorshift64*, deterministic across platforms. */
export class Rng {
    private s: bigint;
    constructor(seed: number | bigint) {
        this.s = BigInt.asUintN(64, BigInt(seed)) || 1n;
    }
    next(): bigint {
        let x = this.s;
        x ^= x >> 12n;
        x ^= BigInt.asUintN(64, x << 25n);
        x ^= x >> 27n;
        this.s = x;
        return BigInt.asUintN(64, x * 0x2545f4914f6cdd1dn);
    }
    below(n: number): number {
        return Number(this.next() % BigInt(n));
    }
    hex(n: number): string {
        let out = "";
        for (let i = 0; i < n; i++) out += (Number(this.next() % 16n)).toString(16);
        return out;
    }
    shuffle<T>(v: T[]): void {
        for (let i = v.length - 1; i >= 1; i--) {
            const j = this.below(i + 1);
            [v[i], v[j]] = [v[j]!, v[i]!];
        }
    }
}

export interface Req {
    id: string;
    tMs: number;
    method: string;
    url: string;
    headers: [string, string][];
    body?: [string, string];
    status: number;
    respHeaders: [string, string][];
    respBody?: [string, string];
}

const iso = (ms: number): string => {
    const s = Math.floor(ms / 1000);
    return `2026-11-09T10:${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}.${String(ms % 1000).padStart(3, "0")}Z`;
};

export function harOf(reqs: Req[]): Json {
    const h = (v: [string, string][]) => v.map(([name, value]) => ({ name, value }));
    return {
        log: {
            version: "1.2",
            creator: { name: "test", version: "1" },
            entries: reqs.map((r) => {
                const request: Json = { method: r.method, url: r.url, headers: h(r.headers) };
                if (r.body) request.postData = { mimeType: r.body[0], text: r.body[1] };
                const response: Json = { status: r.status, headers: h(r.respHeaders), content: { mimeType: "text/plain", text: "" } };
                if (r.respBody) response.content = { mimeType: r.respBody[0], text: r.respBody[1] };
                return { _requestId: r.id, startedDateTime: iso(r.tMs), time: 40.0, request, response };
            }),
        },
    };
}

export function traceOf(run: string, params: [string, string][], reqs: Req[]): Json {
    return {
        version: 1,
        run,
        params: Object.fromEntries(params),
        events: reqs.map((r, i) => ({ id: `e${i}`, t: iso(r.tMs), type: "click", target: `#step${i}`, requests: [r.id] })),
    };
}

export interface RunDoc {
    har: Json;
    trace: Json;
    cookies: unknown;
}

export const parseDoc = (d: RunDoc): Recording => parseRecording(JSON.stringify(d.har), JSON.stringify(d.trace), JSON.stringify(d.cookies));

export const graphOf = (docs: RunDoc[], opts: Options = {}): FlowGraph => infer(docs.map(parseDoc), opts);

/** The fixture shop: CSRF cookie to header, cart id chaining, bearer prefix, idempotency key, signature, redirect, base64 header, form post. */
export function shop(run: string, seed: number, qty: number, zip: string, note: string): { reqs: Req[]; params: [string, string][] } {
    const g = new Rng(seed);
    const csrf = `csrf_${g.hex(20)}`;
    const sid = g.hex(24);
    const cart = `c_${g.hex(16)}`;
    const secret = `pi_${g.hex(12)}_secret_${g.hex(12)}`;
    const order = `ord_${g.hex(14)}`;
    const idem = `${g.hex(8)}-${g.hex(4)}-4${g.hex(3)}-a${g.hex(3)}-${g.hex(12)}`;
    const sig = g.hex(64);
    const ts = 1_794_218_400_000 + Number(g.next() % 1_000_000n);
    const line = run.endsWith("a") ? 1 : 2;
    const host = "https://shop.test";
    const ctJson: [string, string] = ["content-type", "application/json"];
    const ua: [string, string] = ["user-agent", "Mozilla/5.0 (test) Chrome/130"];
    const cookie = `csrf=${csrf}; sid=${sid}`;
    const mk = (i: number) => ({ id: `${run}-r${i}`, tMs: 1000 + i * 100 });
    const reqs: Req[] = [];
    reqs.push({
        ...mk(0),
        method: "GET",
        url: `${host}/`,
        headers: [ua],
        status: 200,
        respHeaders: [
            ["set-cookie", `csrf=${csrf}; Path=/; HttpOnly`],
            ["set-cookie", `sid=${sid}; Path=/`],
            ["date", `Mon, 09 Nov 2026 10:00:${String(seed % 60).padStart(2, "0")} GMT`],
        ],
        respBody: ["text/html", '<html><script>window.cfg={"pk":"pk\x5ftest_ABCDEFGH12345678"}</script></html>'],
    });
    reqs.push({
        ...mk(1),
        method: "POST",
        url: `${host}/api/cart`,
        headers: [ua, ctJson, ["x-csrf-token", csrf], ["cookie", cookie]],
        body: ["application/json", JSON.stringify({ currency: "USD" })],
        status: 201,
        respHeaders: [],
        respBody: ["application/json", JSON.stringify({ cartId: cart, created: "2026-11-09T10:00:00Z", active: true })],
    });
    reqs.push({
        ...mk(2),
        method: "POST",
        url: `${host}/api/cart/${cart}/items`,
        headers: [ua, ctJson, ["x-csrf-token", csrf], ["cookie", cookie]],
        body: ["application/json", JSON.stringify({ sku: "widget-001", quantity: qty })],
        status: 200,
        respHeaders: [],
        respBody: ["application/json", JSON.stringify({ ok: true, lineId: line, date: "2026-11-09" })],
    });
    reqs.push({
        ...mk(3),
        method: "POST",
        url: `${host}/api/checkout`,
        headers: [ua, ctJson, ["authorization", `Bearer ${sid}`], ["x-publishable-key", "pk\x5ftest_ABCDEFGH12345678"], ["cookie", cookie]],
        body: [
            "application/json",
            JSON.stringify({ cartId: cart, address: { zip }, idempotencyKey: idem, signature: sig, ts, line, currency: "USD" }),
        ],
        status: 200,
        respHeaders: [["location", `${host}/confirm?order=${order}&lang=en`]],
        respBody: ["application/json", JSON.stringify({ clientSecret: secret })],
    });
    reqs.push({
        ...mk(4),
        method: "GET",
        url: `${host}/confirm?order=${order}&lang=en`,
        headers: [ua, ["x-intent", Buffer.from(secret).toString("base64")]],
        status: 200,
        respHeaders: [],
        respBody: ["text/html", "<html>thanks</html>"],
    });
    const enc = new URLSearchParams([
        ["note", note],
        ["order", order],
    ]).toString();
    reqs.push({
        ...mk(5),
        method: "POST",
        url: `${host}/api/note`,
        headers: [ua, ["x-csrf-token", csrf]],
        body: ["application/x-www-form-urlencoded", enc],
        status: 204,
        respHeaders: [],
    });
    return { reqs, params: [["quantity", String(qty)], ["zip", zip], ["note", note]] };
}

export function shopDoc(run: string, seed: number, qty: number, zip: string, note: string): RunDoc {
    const { reqs, params } = shop(run, seed, qty, zip, note);
    return { har: harOf(reqs), trace: traceOf(run, params, reqs), cookies: { cookies: [] } };
}

export const twoRuns = (): RunDoc[] => [
    shopDoc("run-a", 11, 2, "94110", "leave at door"),
    shopDoc("run-b", 77, 3, "10001", "ring twice & wait"),
];

export type Truth =
    | { k: "constant" }
    | { k: "input"; param: string }
    | { k: "derived"; step: number; field: string }
    | { k: "fresh" };

/** A random flow with known truth: fields that are constant, run parameters, copies of earlier response fields, or fresh client-generated tokens. */
export function generated(seed: number, runs: number, steps?: number): { docs: RunDoc[]; truths: [string, Truth][][] } {
    const plan = new Rng(BigInt(seed) * 0x9e3779b97f4a7c15n);
    const nSteps = steps ?? 3 + plan.below(5);
    const truths: [string, Truth][][] = [];
    const respFields: string[][] = [];
    let paramN = 0;
    for (let i = 0; i < nSteps; i++) {
        const fields: [string, Truth][] = [];
        const nFields = 2 + plan.below(4);
        for (let k = 0; k < nFields; k++) {
            const loc = `json:$.f${k}`;
            const pick = plan.below(4);
            let t: Truth;
            if (pick === 0) t = { k: "constant" };
            else if (pick === 1) t = { k: "input", param: `p${++paramN}` };
            else if (pick === 2 && i > 0) {
                const j = plan.below(i);
                t = { k: "derived", step: j, field: respFields[j]![plan.below(respFields[j]!.length)]! };
            } else t = { k: "fresh" };
            fields.push([loc, t]);
        }
        const nResp = 2 + plan.below(3);
        respFields.push(Array.from({ length: nResp }, (_, k) => `r${k}`));
        truths.push(fields);
    }
    const docs: RunDoc[] = [];
    for (let r = 0; r < runs; r++) {
        const g = new Rng(seed * 1000 + r * 7919 + 3);
        const respVals = respFields.map((fs) => fs.map((f): [string, string] => [f, `${f}${g.hex(18)}`]));
        const params: [string, string][] = [];
        const reqs: Req[] = [];
        truths.forEach((fields, i) => {
            const body: Json = {};
            for (const [loc, t] of fields) {
                const key = loc.replace("json:$.", "");
                let v: string;
                if (t.k === "constant") v = `fixed-${key}-value`;
                else if (t.k === "input") {
                    v = `user-${t.param}-${g.hex(10)}`;
                    params.push([t.param, v]);
                } else if (t.k === "derived") v = respVals[t.step]!.find(([n]) => n === t.field)![1];
                else v = `gen${g.hex(20)}`;
                body[key] = v;
            }
            reqs.push({
                id: `g${r}-${i}`,
                tMs: 1000 + i * 100,
                method: "POST",
                url: `https://gen.test/step${i}`,
                headers: [["content-type", "application/json"]],
                body: ["application/json", JSON.stringify(body)],
                status: 200,
                respHeaders: [],
                respBody: ["application/json", JSON.stringify(Object.fromEntries(respVals[i]!))],
            });
        });
        docs.push({ har: harOf(reqs), trace: traceOf(`run-${r}`, params, reqs), cookies: [] });
    }
    return { docs, truths };
}
