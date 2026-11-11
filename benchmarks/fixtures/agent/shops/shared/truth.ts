// Checks on the hand-labelled ground truth (flow.truth.json). None of this is an extractor:
// it validates the truth files' structure, compares their normalized flow graphs across stacks,
// and checks each labelled value against what the recorded HARs actually contain.
import { sign } from "./sign.js";

export interface TruthValue {
    at: string;
    class: "constant" | "input" | "derived" | "client-computed";
    value?: unknown;
    name?: string;
    automatic?: boolean;
    role?: string;
    [k: string]: unknown;
}
export interface TruthRequest {
    id: string;
    method: string;
    path: string;
    values: TruthValue[];
    [k: string]: unknown;
}
export interface Truth {
    schema: string;
    stack: string;
    steps: { id: string; effect: string; requests: TruthRequest[]; [k: string]: unknown }[];
    edges: { from: string; to: string | string[]; via: string }[];
    clientComputedCount: number;
    [k: string]: unknown;
}

const CLASSES = ["constant", "input", "derived", "client-computed"];
const EFFECTS = ["read", "write-reversible", "write-irreversible", "financial", "account-control"];
const toList = (to: string | string[]) => (Array.isArray(to) ? to : [to]);

export const requestsOf = (t: Truth) => t.steps.flatMap((s) => s.requests);

/** Structural problems; empty when the truth is well-formed. */
export function validateTruth(t: Truth): string[] {
    const p: string[] = [];
    if (t.schema !== "mochi.agent.flow-truth/0.1") p.push(`unknown schema ${t.schema}`);
    const reqs = requestsOf(t);
    const byId = new Map(reqs.map((r) => [r.id, r]));
    if (byId.size !== reqs.length) p.push("duplicate request ids");
    for (const s of t.steps) if (!EFFECTS.includes(s.effect)) p.push(`step ${s.id}: bad effect ${s.effect}`);
    const inbound = new Set<string>();
    for (const e of t.edges) {
        const [fromReq] = e.from.split(":");
        if (!byId.has(fromReq)) p.push(`edge from unknown request ${e.from}`);
        for (const to of toList(e.to)) {
            const [toReq, ...rest] = to.split(":");
            const at = rest.join(":");
            const r = byId.get(toReq);
            if (!r) p.push(`edge to unknown request ${to}`);
            else if (!r.values.some((v) => v.at === at)) p.push(`edge to ${to}: no such value on ${toReq}`);
            inbound.add(to);
        }
    }
    for (const r of reqs)
        for (const v of r.values) {
            if (!CLASSES.includes(v.class)) p.push(`${r.id}:${v.at}: bad class ${v.class}`);
            if (v.class === "derived" && !inbound.has(`${r.id}:${v.at}`)) p.push(`${r.id}:${v.at}: derived but no inbound edge`);
            if (v.class === "constant" && !("value" in v) && !v.automatic) p.push(`${r.id}:${v.at}: constant without value`);
            if (v.class !== "derived" && inbound.has(`${r.id}:${v.at}`)) p.push(`${r.id}:${v.at}: ${v.class} value has an inbound edge`);
        }
    const cc = reqs.flatMap((r) => r.values.filter((v) => v.class === "client-computed"));
    if (cc.length !== 1) p.push(`expected exactly one client-computed value, found ${cc.length}`);
    if (t.clientComputedCount !== cc.length) p.push("clientComputedCount disagrees with the values");
    const idem = reqs.flatMap((r) => r.values.filter((v) => v.role === "idempotency-key"));
    if (idem.length !== 1) p.push(`expected exactly one idempotency-key value, found ${idem.length}`);
    if (!reqs.some((r) => r.values.some((v) => /csrf/.test(v.at) && v.class === "derived"))) p.push("no derived CSRF value");
    return p;
}

/** Stack-neutral view of the flow: the same for all stacks iff the flow graphs agree. */
export function graphSignature(t: Truth) {
    const norm = (at: string) =>
        at.replace(/^body\.(json|form)\./, "body.").replace(/^body\.(json|form)$/, "body").replace(/^url\.path\[\d+\]$/, "url.id");
    const key = (reqId: string, at: string, name?: string) => `${reqId}:${name ?? norm(at)}`;
    const neutral = (v: TruthValue) => !v.automatic && v.class !== "constant";
    const steps = t.steps.map((s) => ({
        id: s.id,
        effect: s.effect,
        requests: s.requests.map((r) => ({
            id: r.id,
            values: r.values.filter(neutral).map((v) => `${v.name ?? norm(v.at)}:${v.class}`).sort(),
        })),
    }));
    const byAt = new Map<string, TruthValue>();
    for (const r of requestsOf(t)) for (const v of r.values) byAt.set(`${r.id}:${v.at}`, v);
    const edges = t.edges
        .flatMap((e) => toList(e.to).map((to) => ({ from: e.from.split(":")[0], to })))
        .filter(({ to }) => neutral(byAt.get(to)!))
        .map(({ from, to }) => {
            const [reqId, ...rest] = to.split(":");
            const v = byAt.get(to)!;
            return `${from} -> ${key(reqId, rest.join(":"), v.name)}`;
        })
        .sort();
    return { steps, edges };
}

// ---------------------------------------------------------------- against recorded HARs

interface HarEntry {
    request: { method: string; url: string; headers: { name: string; value: string }[]; postData?: { text: string } };
    response: { status: number; headers: { name: string; value: string }[]; cookies: { name: string; value: string }[]; content: { text?: string } };
    _requestId: string;
}
const pathRegex = (tpl: string) => new RegExp("^" + tpl.replace(/[.*+?^$()|[\]\\]/g, "\\$&").replace(/\{[^}]+\}/g, "[^/]+") + "$");

export function matchRequest(t: Truth, e: HarEntry): TruthRequest | undefined {
    const url = new URL(e.request.url);
    return requestsOf(t).find((r) => r.method === e.request.method && pathRegex(r.path).test(url.pathname));
}

function readAt(e: HarEntry, at: string): string | undefined {
    const hdr = (n: string) => e.request.headers.find((h) => h.name.toLowerCase() === n)?.value;
    let m: RegExpExecArray | null;
    if ((m = /^header\.(.+)$/.exec(at))) return hdr(m[1]);
    if ((m = /^cookie\.(.+)$/.exec(at))) return (hdr("cookie") ?? "").split("; ").map((c) => c.split("=")).find(([k]) => k === m![1])?.[1];
    if ((m = /^url\.path\[(\d+)\]$/.exec(at))) return new URL(e.request.url).pathname.split("/").filter(Boolean)[Number(m[1])];
    const text = e.request.postData?.text ?? "";
    if (at === "body.json") return text;
    if (at === "body.form") return text;
    if ((m = /^body\.json\.(.+)$/.exec(at))) return m[1].split(".").reduce((o: any, k) => o?.[k], JSON.parse(text || "null")) as string | undefined;
    if ((m = /^body\.form\.(.+)$/.exec(at))) return new URLSearchParams(text).get(m[1]) ?? undefined;
    return undefined;
}
const responseText = (e: HarEntry) =>
    [e.response.content.text ?? "", ...e.response.cookies.map((c) => c.value)].join("\n");

/**
 * Checks every labelled value against the HAR's requests. Returns problems (empty when the truth
 * holds for this recording).
 */
export function checkTruthAgainstHar(t: Truth, har: { log: { entries: HarEntry[] } }): string[] {
    const p: string[] = [];
    const all = har.log.entries;
    const entries = all.filter((e) => !new URL(e.request.url).pathname.startsWith("/static/"));
    const ids = new Map<HarEntry, string>();
    for (const e of entries) {
        const r = matchRequest(t, e);
        if (!r) {
            p.push(`${e.request.method} ${new URL(e.request.url).pathname}: no truth request matches`);
            continue;
        }
        ids.set(e, r.id);
    }
    const idOf = (e: HarEntry) => ids.get(e);
    const byReq = (id: string) => entries.filter((e) => idOf(e) === id);
    const reqs = new Map(requestsOf(t).map((r) => [r.id, r]));

    for (const e of entries) {
        const r = reqs.get(idOf(e) ?? "");
        if (!r) continue;
        for (const v of r.values) {
            const got = readAt(e, v.at);
            if (got === undefined) {
                p.push(`${r.id}: value ${v.at} absent from the recorded request`);
                continue;
            }
            if (v.class === "constant" && v.value !== undefined && v.at !== "body.json" && JSON.stringify(got) !== JSON.stringify(v.value)) p.push(`${r.id}: ${v.at} is ${got}, truth says constant ${JSON.stringify(v.value)}`);
            if (v.class === "constant" && v.at === "body.json" && JSON.stringify(JSON.parse(got)) !== JSON.stringify(v.value)) p.push(`${r.id}: ${v.at} differs from the labelled constant`);
            if (v.class === "client-computed") {
                const m = /^v1\.(\d+)\.[0-9a-f]{14}$/.exec(got);
                const body = e.request.postData?.text ?? "";
                if (!m || sign(Number(m[1]), body) !== got) p.push(`${r.id}: ${v.at} does not equal sign(timestamp, body)`);
                if (all.some((o) => o !== e && responseText(o).includes(got))) p.push(`${r.id}: ${v.at} appears in a response, so it is not client-computed`);
            }
        }
        // no unlabelled request-side values: every header/body field/path segment must be labelled
        const labelled = new Set(r.values.map((v) => v.at));
        for (const h of e.request.headers) {
            const n = h.name.toLowerCase();
            if (["content-type", "x-csrf-token", "idempotency-key", "x-signature", "x-requested-with"].includes(n) && !labelled.has(`header.${n}`)) p.push(`${r.id}: header ${n} is not labelled`);
        }
        const text = e.request.postData?.text ?? "";
        const ct = e.request.headers.find((h) => h.name.toLowerCase() === "content-type")?.value ?? "";
        const fields = ct.includes("json") ? Object.keys(text && typeof JSON.parse(text) === "object" ? JSON.parse(text) : {}) : [...new URLSearchParams(text).keys()];
        for (const f of fields) if (!labelled.has(`body.json.${f}`) && !labelled.has(`body.form.${f}`)) p.push(`${r.id}: body field ${f} is not labelled`);
        for (const c of (e.request.headers.find((h) => h.name.toLowerCase() === "cookie")?.value ?? "").split("; ").filter(Boolean))
            if (!labelled.has(`cookie.${c.split("=")[0]}`)) p.push(`${r.id}: cookie ${c.split("=")[0]} is not labelled`);
    }
    // derived values: must occur in the declared source response, recorded earlier
    for (const edge of t.edges)
        for (const to of toList(edge.to)) {
            const [fromReq] = edge.from.split(":");
            const [toReq, ...rest] = to.split(":");
            for (const target of byReq(toReq)) {
                const got = readAt(target, rest.join(":"));
                if (!got) continue;
                const before = entries.slice(0, entries.indexOf(target)).filter((e) => idOf(e) === fromReq);
                if (!before.some((e) => responseText(e).includes(got))) p.push(`edge ${edge.from} -> ${to}: value ${got.slice(0, 24)} not found in an earlier ${fromReq} response`);
            }
        }
    return p;
}
