// Alignment, cross-request matching and value classing.

import { isEvidence, isWeakEvidence, looksLikeCard, MIN_LEN, secretish, sensitiveLocation, shape, urlHasSecret } from "./evidence.ts";
import { bodyTokens, isFreeText, pathSegments, requestSlots, responseSlots, type Slot } from "./extract.ts";
import { endUs, type Entry, eventOf, type Recording } from "./har.ts";
import {
    apply,
    exactTransforms,
    plainOf,
    preimages,
    type Rel,
    relCmp,
    relIsExact,
    relKey,
    relVia,
    strongRelation,
    TRANSFORMS,
    type Transform,
} from "./matcher.ts";
import { SCHEMA } from "./types.ts";

/** Substring search skips targets longer than this (headers, ids and tokens are far shorter). */
const MAX_TARGET_LEN = 4096;
import type { Blocker, Edge, FlowGraph, InputParam, Locator, Select, SourceRef, Step, ValueClass, Warning } from "./types.ts";
import { cmp, cmpTuple } from "./util.ts";

export interface Options {
    /** Requests to these hosts are dropped before alignment (analytics, fonts). */
    ignoreHosts?: string[];
    /** Requests whose path starts with one of these are dropped (`/static/`). */
    ignorePathPrefixes?: string[];
    /** Keep only requests the interaction trace attributes to an event. */
    onlyTraced?: boolean;
}

/** Where a value can come from. Ordering is "earliest first": step, request before response, structured before free text, location. */
interface Sid {
    step: number;
    /** 0 initial cookie jar, 1 request, 2 response. */
    phase: number;
    /** 0 structured, 1 free-text body. */
    rank: number;
    loc: string;
}

const sidKey = (s: Sid): string => `${s.step}\u0000${s.phase}\u0000${s.rank}\u0000${s.loc}`;
const sidCmp = (a: Sid, b: Sid): number => a.step - b.step || a.phase - b.phase || a.rank - b.rank || cmp(a.loc, b.loc);

interface Aligned {
    step: number;
    loc: string;
    values: (string | null)[];
}

interface RunData {
    entries: Entry[];
    req: Slot[][];
    resp: Slot[][];
    reqMap: Map<string, string>[];
    reqSlots: Map<string, Slot>[];
    free: (Set<string> | null)[];
    jar: Slot[];
}

interface Hit {
    rels: Map<string, Rel>;
    /** True when some match in this run rests on a value that passes the strong evidence gate. */
    strong: boolean;
    token?: string;
    concrete?: string;
}

type RunHits = Map<number, Map<string, { sid: Sid; hit: Hit }>>;

type Cand =
    | { kind: "valid"; rel: Rel; locator?: Locator; select?: Select }
    | { kind: "partial"; why: string }
    /** A wildcard source whose concrete location is identical in every run: the concrete source is reported instead. */
    | { kind: "redundant" };

/** Infer the flow graph from two or more recorded runs. */
export function infer(recordings: Recording[], opts: Options = {}): FlowGraph {
    if (recordings.length < 2) {
        throw new Error(`flow inference needs at least two recorded runs (got ${recordings.length})`);
    }
    const recs = [...recordings].sort((a, b) => cmp(a.id, b.id));
    if (recs.some((r, i) => i > 0 && recs[i - 1]!.id === r.id)) throw new Error("run ids must be unique");
    const runIds = recs.map((r) => r.id);
    const blockers: Blocker[] = [];
    const warnings: Warning[] = [];

    const runs = recs.map((r) => prepare(r, opts));
    const alignErr = alignmentError(runs, recs);
    if (alignErr !== null) {
        blockers.push({ step: null, location: null, reason: alignErr });
        return { schema: SCHEMA, status: "blocked", runs: runIds, inputs: [], steps: [], edges: [], blockers, warnings };
    }
    const nRuns = runs.length;
    const nSteps = runs[0]!.entries.length;

    runs.forEach((r, ri) => {
        const starts = new Set(r.entries.map((e) => e.startedUs));
        if (starts.size !== r.entries.length) {
            warnings.push({
                step: null,
                location: null,
                kind: "ambiguous-order",
                detail: `run ${runIds[ri]} has requests with identical start times; their order is broken by content hash`,
            });
        }
    });

    // Aligned request slots (the targets to classify).
    const targets: Aligned[] = [];
    for (let step = 0; step < nSteps; step++) {
        const locs = [...new Set(runs.flatMap((r) => r.req[step]!.map((s) => s.loc)))].sort(cmp);
        for (const loc of locs) targets.push({ step, loc, values: runs.map((r) => r.reqMap[step]!.get(loc) ?? null) });
    }

    const hits = runs.map((r) => discover(r, targets));

    const classes: ValueClass[][] = Array.from({ length: nSteps }, () => []);
    const edges: Edge[] = [];
    const inputUses = new Map<string, { step: number; location: string }[]>();
    targets.forEach((t, ti) => {
        const { vc, edge } = classifyTarget(ti, t, runs, recs, hits, nRuns, warnings);
        if (vc.input !== undefined) {
            const list = inputUses.get(vc.input) ?? [];
            list.push({ step: t.step, location: t.loc });
            inputUses.set(vc.input, list);
        }
        if (edge) edges.push(edge);
        if (vc.class === "unclassified") blockers.push({ step: t.step, location: t.loc, reason: vc.reason ?? "" });
        classes[t.step]!.push(vc);
    });
    edges.sort((a, b) => a.to.step - b.to.step || cmp(a.to.location, b.to.location));

    const steps: Step[] = [];
    for (let i = 0; i < nSteps; i++) {
        const e0 = runs[0]!.entries[i]!;
        const values = classes[i]!;
        const provides = [
            ...new Set(edges.filter((e) => e.from.where === "response" && e.from.step === i).map((e) => e.from.location)),
        ].sort(cmp);
        const needs: string[] = [];
        if (values.some((v) => v.class === "client-computed")) needs.push("client-compute");
        if (values.some((v) => v.class === "unclassified")) needs.push("blocked");
        const statuses = [...new Set(runs.map((r) => r.entries[i]!.status))].sort((a, b) => a - b);
        const event = (r: number) => {
            const ev = eventOf(recs[r]!, runs[r]!.entries[i]!.id);
            return ev ? { type: ev.kind, target: ev.target } : undefined;
        };
        const ev0 = event(0);
        for (let r = 1; r < nRuns; r++) {
            const evr = event(r);
            if (JSON.stringify(evr) !== JSON.stringify(ev0)) {
                warnings.push({
                    step: i,
                    location: null,
                    kind: "event-mismatch",
                    detail: `trace attributes step ${i} to different events in runs ${runIds[0]} and ${runIds[r]}`,
                });
            }
        }
        steps.push({
            index: i,
            id: `s${i}`,
            ...(ev0 ? { event: ev0 } : {}),
            method: e0.method,
            origin: originOf(e0),
            path_template: pathTemplate(e0, values),
            statuses,
            values,
            provides,
            needs,
        });
    }

    const inputs: InputParam[] = [...inputUses.entries()]
        .sort((a, b) => cmp(a[0], b[0]))
        .map(([name, uses]) => ({ name, uses }));
    blockers.sort((a, b) => cmpTuple([a.step, a.location], [b.step, b.location]));
    warnings.sort((a, b) => cmpTuple([a.step, a.location, a.kind, a.detail], [b.step, b.location, b.kind, b.detail]));
    const uniqueWarnings = warnings.filter((w, i) => i === 0 || JSON.stringify(w) !== JSON.stringify(warnings[i - 1]));
    return {
        schema: SCHEMA,
        status: blockers.length === 0 ? "resolved" : "blocked",
        runs: runIds,
        inputs,
        steps,
        edges,
        blockers,
        warnings: uniqueWarnings,
    };
}

function originOf(e: Entry): string {
    return `${e.url.protocol}//${e.url.host}`;
}

function pathTemplate(e: Entry, values: ValueClass[]): string {
    let out = "";
    pathSegments(e.url).forEach((seg, i) => {
        const loc = `url.path[${i}]`;
        const v = values.find((x) => x.location === loc);
        out += v && (v.class !== "constant" || v.redacted) ? `/{${loc}}` : `/${seg}`;
    });
    return out === "" ? "/" : out;
}

function prepare(rec: Recording, opts: Options): RunData {
    const traced = new Set(rec.events.flatMap((e) => e.requests));
    const entries = rec.entries.filter((e) => {
        const host = e.url.hostname;
        return (
            !(opts.ignoreHosts ?? []).includes(host) &&
            !(opts.ignorePathPrefixes ?? []).some((p) => e.url.pathname.startsWith(p)) &&
            (!opts.onlyTraced || traced.has(e.id))
        );
    });
    const req = entries.map(requestSlots);
    return {
        entries,
        req,
        resp: entries.map(responseSlots),
        reqMap: req.map((slots) => new Map(slots.map((s) => [s.loc, s.value] as const))),
        reqSlots: req.map((slots) => new Map(slots.map((s) => [s.loc, s] as const))),
        free: entries.map((e) => (isFreeText(e) && e.respBody ? bodyTokens(e.respBody.text) : null)),
        jar: rec.initialCookies.map((c) => ({ loc: `cookie:${c.name}`, value: c.value })),
    };
}

/** Parameter values visible at a step in each run (event inputs and run-wide inputs). */
function paramValues(recs: Recording[], runs: RunData[], step: number): Set<string>[] {
    return recs.map((rec, r) => {
        const v = visible(rec, runs[r]!, step);
        return new Set([...(v.event?.values() ?? []), ...v.run.values()]);
    });
}

function alignmentError(runs: RunData[], recs: Recording[]): string | null {
    const n = runs[0]!.entries.length;
    for (const r of runs) {
        if (r.entries.length !== n) {
            return `runs are not alignable: they recorded ${n} and ${r.entries.length} requests (filter ambient traffic with ignoreHosts or onlyTraced, or re-record)`;
        }
    }
    for (let i = 0; i < n; i++) {
        const a = runs[0]!.entries[i]!;
        for (const r of runs.slice(1)) {
            const b = r.entries[i]!;
            if (a.method !== b.method || a.url.protocol !== b.url.protocol || a.url.host !== b.url.host) {
                return `runs are not alignable at step ${i}: method or host differs (${a.method} vs ${b.method})`;
            }
            if (pathSegments(a.url).length !== pathSegments(b.url).length) {
                return `runs are not alignable at step ${i}: the request paths have a different number of segments`;
            }
        }
        // A path segment that differs between runs must be an id (token-grade in every run) or
        // follow a run parameter; otherwise two different endpoints were paired by position.
        const segs = runs.map((r) => pathSegments(r.entries[i]!.url));
        const params = paramValues(recs, runs, i);
        for (let k = 0; k < segs[0]!.length; k++) {
            const vals = segs.map((s) => s[k]!);
            if (vals.every((v) => v === vals[0])) continue;
            const idLike = vals.every((v) => v.length >= 6 && /\d/.test(v) && /^[A-Za-z0-9_.~%-]+$/.test(v));
            const isParam = vals.every((v, r) => params[r]!.has(v));
            if (!idLike && !isParam) {
                return `runs are not alignable at step ${i}: path segment ${k} differs between runs and is neither an id nor a run parameter (the requests may have been reordered)`;
            }
        }
    }
    return null;
}

function eligible(run: RunData, sid: Sid, targetStep: number): boolean {
    if (sid.step >= targetStep) return false;
    if (sid.phase === 2) return endUs(run.entries[sid.step]!) <= run.entries[targetStep]!.startedUs;
    return true;
}

function addHit(map: RunHits, ti: number, sid: Sid, rel: Rel, strong: boolean, token?: string, concrete?: string): void {
    let byTarget = map.get(ti);
    if (!byTarget) map.set(ti, (byTarget = new Map()));
    const k = sidKey(sid);
    let e = byTarget.get(k);
    if (!e) byTarget.set(k, (e = { sid, hit: { rels: new Map(), strong: false } }));
    e.hit.rels.set(relKey(rel), rel);
    if (strong) e.hit.strong = true;
    if (token !== undefined) e.hit.token = token;
    if (e.hit.concrete === undefined && concrete !== undefined) e.hit.concrete = concrete;
}

function intersect<T>(sets: Set<T>[]): Set<T> {
    const [first, ...rest] = sets;
    return new Set([...(first ?? [])].filter((x) => rest.every((s) => s.has(x))));
}

const varies = (values: (string | null)[]): boolean => values.some((v, i) => i > 0 && v !== values[i - 1]);

/** `json:$.rates[3].id` -> `json:$.rates[*].id`; null when there is no array index. */
function wildcard(loc: string): string | null {
    if (!loc.startsWith("json:")) return null;
    const w = loc.replace(/\[\d+\]/g, "[*]");
    return w === loc ? null : w;
}

interface SrcEntry {
    sid: Sid;
    val: string;
    /** The concrete location behind a wildcard source. */
    concrete?: string;
}

function discover(run: RunData, targets: Aligned[]): RunHits {
    const structured = new Map<string, SrcEntry>();
    const wild: SrcEntry[] = [];
    const put = (sid: Sid, val: string) => structured.set(sidKey(sid), { sid, val });
    for (const s of run.jar) put({ step: -1, phase: 0, rank: 0, loc: s.loc }, s.value);
    run.req.forEach((slots, i) => slots.forEach((s) => put({ step: i, phase: 1, rank: 0, loc: s.loc }, s.value)));
    run.resp.forEach((slots, i) =>
        slots.forEach((s) => {
            put({ step: i, phase: 2, rank: 0, loc: s.loc }, s.value);
            const w = wildcard(s.loc);
            if (w !== null) wild.push({ sid: { step: i, phase: 2, rank: 0, loc: w }, val: s.value, concrete: s.loc });
        }),
    );
    const all = [...structured.values()].sort((a, b) => sidCmp(a.sid, b.sid)).concat(wild);

    // variant text -> [(source index, transform)]
    const index = new Map<string, [number, Transform][]>();
    all.forEach((e, si) => {
        for (const t of TRANSFORMS) {
            const v = apply(t, e.val);
            if (v !== null && isWeakEvidence(plainOf(t, e.val, v))) {
                const list = index.get(v) ?? [];
                list.push([si, t]);
                index.set(v, list);
            }
        }
    });

    const exact = (t: Transform): Rel => ({ transform: t, prefix: "", suffix: "" });
    // Substring search: every variant is at least MIN_LEN characters, so each occurrence starts
    // with a MIN_LEN-character window. Looking windows up costs O(target length) per target.
    const windows = new Map<string, string[]>();
    for (const variant of index.keys()) {
        if (variant.length < MIN_LEN) continue;
        const w = variant.slice(0, MIN_LEN);
        const list = windows.get(w);
        if (list) list.push(variant);
        else windows.set(w, [variant]);
    }
    const out: RunHits = new Map();
    targets.forEach((t, ti) => {
        const value = run.reqMap[t.step]!.get(t.loc);
        if (value === undefined) return;
        const isVarying = varies(t.values);
        for (const [si, tf] of index.get(value) ?? []) {
            const e = all[si]!;
            if (eligible(run, e.sid, t.step)) addHit(out, ti, e.sid, exact(tf), strongRelation(tf, e.val, value), undefined, e.concrete);
        }
        // Substring matches (Bearer tokens, embedded ids). Constants skip this: they never become
        // edges, so only the cheap exact lookups run for them (to raise a warning).
        if (isVarying && value.length > MIN_LEN && value.length <= MAX_TARGET_LEN) {
            const seen = new Set<string>();
            for (let i = 0; i + MIN_LEN <= value.length; i++) {
                for (const variant of windows.get(value.slice(i, i + MIN_LEN)) ?? []) {
                    if (variant.length >= value.length || !value.startsWith(variant, i) || seen.has(`${i}\u0000${variant}`)) continue;
                    seen.add(`${i}\u0000${variant}`);
                    // Only the first occurrence of a variant defines prefix and suffix.
                    if (value.indexOf(variant) !== i) continue;
                    for (const [si, tf] of index.get(variant) ?? []) {
                        const e = all[si]!;
                        if (!eligible(run, e.sid, t.step)) continue;
                        const rel: Rel = { transform: tf, prefix: value.slice(0, i), suffix: value.slice(i + variant.length) };
                        addHit(out, ti, e.sid, rel, strongRelation(tf, e.val, variant), undefined, e.concrete);
                    }
                }
            }
        }
        // Free-text bodies: the target (or a pre-image of it) occurs as a token.
        const pre = preimages(value);
        if (pre.length === 0) return;
        for (let j = 0; j < t.step; j++) {
            const tokens = run.free[j];
            if (!tokens) continue;
            for (const [tf, s] of pre) {
                if (!tokens.has(s)) continue;
                const sid: Sid = { step: j, phase: 2, rank: 1, loc: "body" };
                if (eligible(run, sid, t.step)) addHit(out, ti, sid, exact(tf), strongRelation(tf, s, value), s);
            }
        }
    });
    return out;
}

interface Visible {
    event: Map<string, string> | undefined;
    run: Map<string, string>;
}

/** Parameters visible at a step: the user input of the event that caused the request, then the run-wide inputs. */
function visible(rec: Recording, run: RunData, step: number): Visible {
    return { event: eventOf(rec, run.entries[step]!.id)?.inputs, run: rec.params };
}

interface InputMatch {
    /** Parameters equal to the target (through a common exact transform) in every run. */
    full: string[];
    /** Parameters equal to a token-grade target in some runs but not all. */
    partial: string[];
    /** The only matches came from run-wide parameters and the value is low-entropy. */
    weak: boolean;
}

/**
 * Which run parameters the target follows. Inputs of the trace event that caused the request are
 * tried first; run-wide parameters are the fallback. A low-entropy value that matches only a
 * run-wide parameter is reported as `weak` (it may be a coincidence with another parameter).
 */
function matchInputs(step: number, vals: string[], runs: RunData[], recs: Recording[]): InputMatch {
    const scope = (pick: (r: number) => Map<string, string> | undefined): { full: string[]; partial: string[] } => {
        const names = new Set<string>();
        for (let r = 0; r < vals.length; r++) for (const k of pick(r)?.keys() ?? []) names.add(k);
        const full: string[] = [];
        const partial: string[] = [];
        for (const name of [...names].sort(cmp)) {
            const sets: Set<Transform>[] = [];
            let matchedRuns = 0;
            for (let r = 0; r < vals.length; r++) {
                const pv = pick(r)?.get(name);
                if (pv === undefined) break;
                const ts = exactTransforms(vals[r]!, pv, false);
                if (ts.length > 0 && isEvidence(vals[r]!)) matchedRuns++;
                sets.push(new Set(ts));
            }
            if (sets.length === vals.length && intersect(sets).size > 0) full.push(name);
            else if (matchedRuns > 0) partial.push(name);
        }
        return { full, partial };
    };
    const event = scope((r) => visible(recs[r]!, runs[r]!, step).event);
    if (event.full.length > 0 || event.partial.length > 0) return { ...event, weak: false };
    const run = scope((r) => visible(recs[r]!, runs[r]!, step).run);
    return { ...run, weak: run.full.length > 0 && !vals.some((v) => isEvidence(v)) };
}

/**
 * A wildcard source picks one element of an array; the pick must follow a user input. The
 * element is identified by a sibling field that equals a parameter in every run, whose value is a
 * label (not a bare number) and unique among the array's elements.
 */
function explainSelection(
    runs: RunData[],
    recs: Recording[],
    srcStep: number,
    targetStep: number,
    concretes: string[],
): Select | string {
    const perRunPairs: Set<string>[] = [];
    for (let r = 0; r < concretes.length; r++) {
        const concrete = concretes[r]!;
        if ((concrete.match(/\[\d+\]/g) ?? []).length !== 1) return "nested array selection is not modelled";
        // `<parent>[n].<field>`: the element must be an object with named fields.
        const m = /^(.*\[\d+\])\.([A-Za-z_][A-Za-z0-9_-]*)$/.exec(concrete);
        if (!m) return "the selected element has no named fields to explain the choice";
        const prefix = `${m[1]}.`;
        const leaf = m[2]!;
        const arrayPrefix = prefix.slice(0, prefix.lastIndexOf("["));
        const vis = visible(recs[r]!, runs[r]!, targetStep);
        const slots = runs[r]!.resp[srcStep]!;
        const pairs = new Set<string>();
        for (const s of slots) {
            if (!s.loc.startsWith(prefix)) continue;
            const field = s.loc.slice(prefix.length);
            if (field.includes(".") || field.includes("[") || field === leaf) continue;
            // A bare number or boolean is a coincidence waiting to happen (`pos: "1"` vs quantity 1).
            if (/^\d{1,11}$/.test(s.value) || s.value === "true" || s.value === "false") continue;
            // The key must identify one element of the array.
            const siblings = slots.filter((o) => /\[\d+\]\./.test(o.loc.slice(arrayPrefix.length)) && o.loc.startsWith(`${arrayPrefix}[`) && o.loc.endsWith(`.${field}`) && o.value === s.value);
            if (siblings.length !== 1) continue;
            for (const map of [vis.event, vis.run]) {
                for (const [p, pv] of map ?? []) if (pv === s.value && pv !== "") pairs.add(`${field}\u0000${p}`);
            }
        }
        perRunPairs.push(pairs);
    }
    const list = [...intersect(perRunPairs)].sort(cmp);
    if (list.length === 1) {
        const [field, param] = list[0]!.split("\u0000") as [string, string];
        return { field, param };
    }
    return list.length === 0
        ? "the chosen array element differs between runs and no label field of the element follows a run parameter"
        : "several fields of the chosen element follow run parameters";
}

function sourceValue(run: RunData, sid: Sid): string | undefined {
    if (sid.phase === 0) return run.jar.find((s) => s.loc === sid.loc)?.value;
    if (sid.phase === 1) return run.reqMap[sid.step]?.get(sid.loc);
    return run.resp[sid.step]?.find((s) => s.loc === sid.loc)?.value;
}

const describe = (sid: Sid): string =>
    sid.phase === 0 ? `initial cookie \`${sid.loc}\`` : sid.phase === 1 ? `request ${sid.step} \`${sid.loc}\`` : `response ${sid.step} \`${sid.loc}\``;

function sourceRef(sid: Sid, locator?: Locator): SourceRef {
    if (sid.phase === 0) return { where: "initial-cookie", step: null, location: sid.loc };
    if (sid.phase === 1) return { where: "request", step: sid.step, location: sid.loc };
    return { where: "response", step: sid.step, location: sid.loc, ...(locator ? { locator } : {}) };
}

interface Valid {
    sid: Sid;
    rel: Rel;
    locator?: Locator;
    select?: Select;
}

function classifyTarget(
    ti: number,
    t: Aligned,
    runs: RunData[],
    recs: Recording[],
    hits: RunHits[],
    nRuns: number,
    warnings: Warning[],
): { vc: ValueClass; edge?: Edge } {
    const base = (cls: ValueClass["class"]): ValueClass => ({ location: t.loc, class: cls });
    const present = t.values.filter((v) => v !== null).length;
    if (present !== nRuns) {
        return {
            vc: { ...base("unclassified"), reason: `present in ${present} of ${nRuns} runs; optional values are not modelled` },
        };
    }
    const vals = t.values.map((v) => v ?? "");
    // JSON value types must agree: `2` and "2" are different on the wire.
    const types = runs.map((r) => r.reqSlots[t.step]!.get(t.loc)?.jt);
    if (types.some((x) => x !== types[0])) {
        return { vc: { ...base("unclassified"), reason: `JSON type differs between runs (${[...new Set(types)].join(", ")})` } };
    }
    const jsonType = types[0] && types[0] !== "string" ? types[0] : undefined;

    // Candidate sources across runs.
    const perSource = new Map<string, { sid: Sid; perRun: (Hit | undefined)[] }>();
    hits.forEach((h, ri) => {
        for (const [k, { sid, hit }] of h.get(ti) ?? []) {
            let e = perSource.get(k);
            if (!e) perSource.set(k, (e = { sid, perRun: new Array<Hit | undefined>(nRuns).fill(undefined) }));
            e.perRun[ri] = hit;
        }
    });
    // A match that exists in some runs is also counted in the others when the raw values are equal
    // there, even if a structural exclusion (a numeric-looking id) kept it out of the index: whether
    // an id is looked up must not depend on its characters.
    for (const { sid, perRun } of perSource.values()) {
        if (sid.rank !== 0 || sid.loc.includes("[*]")) continue;
        perRun.forEach((h, ri) => {
            if (h) return;
            const run = runs[ri]!;
            const own = run.reqMap[t.step]!.get(t.loc);
            const src = sourceValue(run, sid);
            if (own === undefined || src === undefined || !eligible(run, sid, t.step)) return;
            const tf = exactTransforms(own, src, false)[0];
            if (tf) perRun[ri] = { rels: new Map([[relKey({ transform: tf, prefix: "", suffix: "" }), { transform: tf, prefix: "", suffix: "" }]]), strong: false };
        });
    }
    const validExt: Valid[] = [];
    const validReq: Valid[] = [];
    const partialExt: string[] = [];
    const partialReq: string[] = [];
    for (const { sid, perRun } of [...perSource.values()].sort((a, b) => sidCmp(a.sid, b.sid))) {
        const cand = candidate(sid, perRun, runs, recs, t.step, nRuns);
        const [valid, partial] = sid.phase === 1 ? [validReq, partialReq] : [validExt, partialExt];
        if (cand.kind === "valid") {
            valid.push({
                sid,
                rel: cand.rel,
                ...(cand.locator ? { locator: cand.locator } : {}),
                ...(cand.select ? { select: cand.select } : {}),
            });
        } else if (cand.kind === "partial") partial.push(cand.why);
    }

    const withType = (vc: ValueClass): ValueClass => (jsonType ? { ...vc, json_type: jsonType } : vc);
    const derived = (v: Valid): { vc: ValueClass; edge: Edge } => {
        const src: SourceRef = { ...sourceRef(v.sid, v.locator), ...(v.select ? { select: v.select } : {}) };
        const via = relVia(v.rel);
        return {
            vc: withType({ ...base("derived"), source: src, via }),
            edge: {
                from: src,
                to: { step: t.step, location: t.loc },
                via,
                ...(v.rel.prefix ? { prefix: v.rel.prefix } : {}),
                ...(v.rel.suffix ? { suffix: v.rel.suffix } : {}),
            },
        };
    };
    const unclassified = (why: string) => ({ vc: { ...base("unclassified"), reason: `partial evidence: ${why}` } });

    if (varies(t.values)) {
        // Server-issued values (cookie jar, responses) first, then user inputs, then values reused
        // from earlier requests, then nothing: client-computed.
        if (validExt[0]) return derived(validExt[0]);
        if (partialExt[0] !== undefined) return unclassified(partialExt[0]);
        const inputs = matchInputs(t.step, vals, runs, recs);
        if (inputs.full.length === 1) {
            if (inputs.weak) {
                warnings.push({
                    step: t.step,
                    location: t.loc,
                    kind: "weak-input-match",
                    detail: `low-entropy value matched run-wide parameter \`${inputs.full[0]}\` only; a coincidence with another parameter is possible (attribute inputs to trace events to rule it out)`,
                });
            }
            return { vc: withType({ ...base("input"), input: inputs.full[0]! }) };
        }
        if (inputs.full.length > 1) {
            return { vc: { ...base("unclassified"), reason: `ambiguous: follows several run parameters (${inputs.full.join(", ")})` } };
        }
        if (inputs.partial.length > 0) {
            return unclassified(`follows run parameter \`${inputs.partial[0]}\` in some runs but not in all`);
        }
        if (validReq[0]) return derived(validReq[0]);
        if (partialReq[0] !== undefined) return unclassified(partialReq[0]);
        return { vc: withType({ ...base("client-computed"), shape: shape(vals[0]!) }) };
    }

    // Constant in every run.
    const value = vals[0]!;
    const vc = withType(base("constant"));
    if (sensitiveLocation(t.loc) || looksLikeCard(value) || urlHasSecret(value)) vc.redacted = true;
    else vc.value = value;
    const src = validExt[0] ?? validReq[0];
    if (isEvidence(value) && src) {
        warnings.push({
            step: t.step,
            location: t.loc,
            kind: "constant-matches-source",
            detail: `identical in every run and equal to ${describe(src.sid)}; if the server issues this value, record runs on fresh sessions so it varies`,
        });
    }
    // A declared user input that did not vary is baked in as a constant: say so.
    const declared = [...new Set(recs.flatMap((rec, r) => [...(visible(rec, runs[r]!, t.step).event ?? [])].filter(([, pv]) => pv === value).map(([k]) => k)))];
    const everyRun = declared.filter((name) => recs.every((rec, r) => visible(rec, runs[r]!, t.step).event?.get(name) === value));
    if (everyRun.length > 0) {
        warnings.push({
            step: t.step,
            location: t.loc,
            kind: "input-did-not-vary",
            detail: `equals user input \`${everyRun[0]}\` in every run, so it is embedded as a constant; record runs with different values to make it an input`,
        });
    }
    return { vc };
}

function candidate(sid: Sid, perRun: (Hit | undefined)[], runs: RunData[], recs: Recording[], targetStep: number, nRuns: number): Cand {
    const found = perRun.filter((h) => h !== undefined).length;
    if (found < nRuns) return { kind: "partial", why: `matches ${describe(sid)} in ${found} of ${nRuns} runs` };
    // The loose gate found the match; at least one run must pass the strong gate.
    if (!perRun.some((h) => h!.strong)) return { kind: "redundant" };
    const keys = intersect(perRun.map((h) => new Set(h!.rels.keys())));
    const rels = [...perRun[0]!.rels].filter(([k]) => keys.has(k)).map(([, r]) => r).sort((a, b) => Number(!relIsExact(a)) - Number(!relIsExact(b)) || relCmp(a, b));
    const rel = rels[0];
    if (!rel) {
        return { kind: "partial", why: `matches ${describe(sid)} with a different encoding or surrounding text in different runs` };
    }
    // Surrounding text is copied into the graph: it must not carry a secret.
    if (secretish(rel.prefix) || secretish(rel.suffix)) {
        return { kind: "partial", why: `matches ${describe(sid)} inside text that looks like a secret; it is not copied into the graph` };
    }
    if (sid.loc.includes("[*]")) {
        const concretes = perRun.map((h) => h!.concrete ?? "");
        if (concretes.every((c) => c === concretes[0])) return { kind: "redundant" };
        const sel = explainSelection(runs, recs, sid.step, targetStep, concretes);
        return typeof sel === "string"
            ? { kind: "partial", why: `matches an element of ${describe(sid)}: ${sel}` }
            : { kind: "valid", rel, select: sel };
    }
    if (sid.rank === 1) {
        const l = locator(runs, sid.step, perRun.map((h) => h!.token ?? ""));
        return l
            ? { kind: "valid", rel, locator: l }
            : { kind: "partial", why: `matches ${describe(sid)} but the surrounding text is not stable across runs or looks like a secret` };
    }
    return { kind: "valid", rel };
}

const TOKEN_CHAR = /[A-Za-z0-9_-]/;

/** First occurrence of `tok` that is a whole token (not inside a longer word), or -1. */
function findToken(text: string, tok: string): number {
    for (let i = text.indexOf(tok); i >= 0; i = text.indexOf(tok, i + 1)) {
        const before = i > 0 ? text[i - 1]! : "";
        const after = text[i + tok.length] ?? "";
        if (!(before && TOKEN_CHAR.test(before)) && !(after && TOKEN_CHAR.test(after))) return i;
    }
    return -1;
}

/**
 * Longest text before and after the token's first whole-token occurrence that is identical in
 * every run (at most 24 characters each side). At least four characters in total are required,
 * and neither side may contain a secret-looking constant (that text would be copied into the graph).
 */
function locator(runs: RunData[], step: number, tokens: string[]): Locator | null {
    const befores: string[][] = [];
    const afters: string[][] = [];
    for (let r = 0; r < runs.length; r++) {
        const text = runs[r]!.entries[step]!.respBody?.text;
        const tok = tokens[r]!;
        if (text === undefined) return null;
        const i = findToken(text, tok);
        if (i < 0) return null;
        befores.push([...text.slice(0, i)].reverse().slice(0, 24));
        afters.push([...text.slice(i + tok.length)].slice(0, 24));
    }
    const commonLen = (lists: string[][]): number => {
        let n = 0;
        while (lists[0]![n] !== undefined && lists.slice(1).every((l) => l[n] === lists[0]![n])) n++;
        return n;
    };
    const bl = commonLen(befores);
    const al = commonLen(afters);
    if (bl + al < 4) return null;
    // Cut each side where a word of 8 or more token characters begins: such a word may be a secret
    // (or part of one), and the context must stay structural (`var t="`, `";</script>`).
    const longWord = /[A-Za-z0-9_-]{8,}/;
    let before = befores[0]!.slice(0, bl).reverse().join("");
    let after = afters[0]!.slice(0, al).join("");
    const ib = [...before.matchAll(new RegExp(longWord, "g"))].pop();
    if (ib) before = before.slice(ib.index! + ib[0].length);
    const ia = longWord.exec(after);
    if (ia) after = after.slice(0, ia.index);
    if (before.length + after.length < 4 || secretish(before) || secretish(after)) return null;
    return { before, after, origin: "page" };
}
