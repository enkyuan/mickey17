// Comparison with the hand-labelled ground truth of the fixture shops (agent-fixtures,
// `flow.truth.json`): three stacks, four scenarios, two runs each. The fixtures live on another
// branch until it merges: point `MOCHI_AGENT_SHOPS` at `benchmarks/fixtures/agent/shops`, and set
// `MOCHI_REQUIRE_AGENT_FIXTURES=1` to turn "not found" into a failure.

import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { infer, type Options } from "../src/classify.ts";
import { valueOf } from "../src/graph.ts";
import { parseRecording, type Recording } from "../src/har.ts";
import type { ClassName, FlowGraph } from "../src/types.ts";

const STACKS = ["react", "server-rendered", "svelte"] as const;
const SCENARIOS = ["success", "validation-error", "out-of-stock", "declined"] as const;

function shopsDir(): string | null {
    const dir = process.env.MOCHI_AGENT_SHOPS ?? resolve(import.meta.dir, "../../benchmarks/fixtures/agent/shops");
    const found = existsSync(join(dir, "recordings"));
    if (!found && process.env.MOCHI_REQUIRE_AGENT_FIXTURES) throw new Error(`fixture shops not found at ${dir}`);
    return found ? dir : null;
}

const dir = shopsDir();
const maybe = dir ? describe : describe.skip;
if (!dir) console.warn("fixture shops not found (set MOCHI_AGENT_SHOPS); skipping ground-truth tests");

const readJson = (p: string) => JSON.parse(readFileSync(p, "utf8"));
const OPTS: Options = { ignorePathPrefixes: ["/static/"] };

function load(stack: string, scenario: string, mutate?: (entries: any[]) => void): Recording[] {
    return [1, 2].map((n) => {
        const base = join(dir!, "recordings", stack);
        const har = readJson(join(base, `${scenario}.run${n}.har`));
        mutate?.(har.log.entries);
        return parseRecording(JSON.stringify(har), readFileSync(join(base, `${scenario}.run${n}.trace.json`), "utf8"));
    });
}

const graphFor = (stack: string, scenario: string, mutate?: (entries: any[]) => void): FlowGraph => infer(load(stack, scenario, mutate), OPTS);

/** `header.x-csrf-token` -> `header:x-csrf-token`, `body.json.a.b` -> `json:$.a.b`. */
function mine(at: string): string | null {
    if (at.startsWith("header.")) return `header:${at.slice(7)}`;
    if (at.startsWith("cookie.")) return `cookie:${at.slice(7)}`;
    if (at.startsWith("url.query.")) return `url.query:${at.slice(10)}`;
    if (at.startsWith("body.json.")) return `json:$.${at.slice(10)}`;
    if (at.startsWith("body.form.")) return `form:${at.slice(10)}`;
    if (at.startsWith("url.path[")) return at;
    return null;
}

const normPath = (p: string): string => p.replace(/\{[^}]*\}/g, "{}");

/** A truth `res.*` locator against the location of ours it may be reported as. */
function sourceMatches(fromAt: string, loc: string): boolean {
    if (fromAt.startsWith("res.cookie.")) return loc === `setcookie:${fromAt.slice(11)}`;
    if (fromAt.startsWith("res.json.")) return loc.replace(/\[(\d+|\*)\]/g, "[*]") === `json:$.${fromAt.slice(9)}`.replace(/\[\]/g, "[*]");
    if (fromAt.startsWith("res.html.")) {
        // Hidden inputs and meta tags are structured sources and must name the same field; anything
        // else in the truth is an attribute of an element, found as a located token in the body.
        const m = /^res\.html\.(meta|input)\[name=([^\]]+)\]@/.exec(fromAt);
        if (m?.[1] === "meta") return loc === `html:meta[name=${m[2]}]`;
        // `input[name=x]` is a hidden field (structured) or a radio button (a located token).
        if (m) return loc === `html:input[name=${m[2]}]` || loc === "body";
        return loc === "body";
    }
    return false;
}

/** Divergences that are not extraction errors; anything else that differs from the truth fails. */
function allowedDivergence(scenario: string, loc: string, truth: any, g: FlowGraph, step: number): string | null {
    const v = valueOf(g, step, loc)!;
    if (truth.class === "input" && v.class === "constant" && (loc === "json:$.country" || loc === "form:country" || loc === "json:$.method")) {
        return "input that did not vary between the two recorded runs";
    }
    if (truth.class === "input" && v.class === "client-computed" && truth.role === "idempotency-key" && v.shape === "uuid") {
        return "caller-minted uuid: no source in the recording";
    }
    if (truth.class === "input" && v.class === "client-computed" && scenario === "validation-error" && loc === "json:$.quantity") {
        return "input coerced by site code (Number('abc') is null): not an encoding of the input";
    }
    return null;
}

maybe("ground truth (agent-fixtures)", () => {
    test("matches the hand-labelled truth on all three stacks", () => {
        const allowed = new Map<string, number>();
        let checkedEdges = 0;
        for (const stack of STACKS) {
            const truth = readJson(join(dir!, stack, "flow.truth.json"));
            const reqs: { id: string; method: string; path: string; values: any[] }[] = truth.steps.flatMap((s: any) =>
                s.requests.map((r: any) => ({ id: r.id, method: r.method, path: normPath(r.path), values: r.values })),
            );
            for (const scenario of SCENARIOS) {
                const tag = `${stack}/${scenario}`;
                const g = graphFor(stack, scenario);
                expect(g.blockers, tag).toEqual([]);
                expect(g.status, tag).toBe("resolved");
                // Replayable constants must survive redaction: every content type is kept.
                for (const st of g.steps) {
                    for (const v of st.values) if (v.location === "header:content-type") expect(v.value, `${tag}: step ${st.index}`).toBeDefined();
                }
                const reqOf = (i: number) => {
                    const s = g.steps[i]!;
                    const r = reqs.find((q) => q.method === s.method && q.path === normPath(s.path_template));
                    if (!r) throw new Error(`${tag}: step ${i} ${s.method} ${s.path_template} has no labelled request`);
                    return r;
                };
                for (const st of g.steps) {
                    const tvals = reqOf(st.index).values;
                    for (const v of st.values) {
                        const tv = tvals.find((x) => mine(x.at) === v.location);
                        if (!tv) {
                            // The truth omits constant path segments; everything else must be labelled.
                            if (!((v.location.startsWith("url.path[") || v.location === "json:$") && v.class === "constant")) {
                                throw new Error(`${tag}: step ${st.index} value ${v.location} (${v.class}) is not labelled in the truth`);
                            }
                            continue;
                        }
                        if (tv.class !== v.class) {
                            const why = allowedDivergence(scenario, v.location, tv, g, st.index);
                            if (!why) throw new Error(`${tag}: step ${st.index} ${v.location}: truth=${tv.class} mine=${v.class} ${JSON.stringify(v.source)}`);
                            allowed.set(why, (allowed.get(why) ?? 0) + 1);
                        }
                    }
                    for (const tv of tvals) {
                        const loc = mine(tv.at);
                        if (loc) expect(st.values.some((v) => v.location === loc), `${tag}: step ${st.index} lacks labelled value ${loc}`).toBe(true);
                    }
                }
                // Dependency edges: every labelled edge that applies in this scenario is found with the labelled source request and location.
                for (const e of truth.edges) {
                    const [fromReq, fromAt] = (e.from as string).split(/:(.*)/s) as [string, string];
                    const tos: string[] = Array.isArray(e.to) ? e.to : [e.to];
                    for (const to of tos) {
                        const [toReq, toAt] = to.split(/:(.*)/s) as [string, string];
                        const loc = mine(toAt);
                        if (!loc) continue;
                        for (const st of g.steps.filter((s) => reqOf(s.index).id === toReq)) {
                            const v = valueOf(g, st.index, loc)!;
                            if (v.class !== "derived") continue; // class mismatches were handled above
                            const src = v.source!;
                            expect(src.where, `${tag}: ${to}`).toBe("response");
                            expect(reqOf(src.step!).id, `${tag}: ${to} comes from the wrong request`).toBe(fromReq);
                            expect(sourceMatches(fromAt, src.location), `${tag}: ${to}: source ${src.location} vs truth ${fromAt}`).toBe(true);
                            checkedEdges++;
                        }
                    }
                }
            }
        }
        // The selection edge names the run parameter that picks the shipping rate.
        const rate = graphFor("react", "success").edges.find((e) => e.to.location === "json:$.rateId")!;
        expect(rate.from.select).toEqual({ field: expect.any(String), param: "rate" });
        expect(rate.from.location).toBe("json:$.rates[*].id");
        expect(checkedEdges).toBeGreaterThan(100);
        console.log(`edges compared against the truth: ${checkedEdges}`, Object.fromEntries(allowed));
    });

    test("the three stacks yield the same flow graph", () => {
        // Per step: the (leaf name, class) of every value that carries information, with transport
        // differences (JSON vs form bodies, cookie vs meta CSRF, path prefixes) removed.
        const signature = (g: FlowGraph) =>
            g.steps.map((s) =>
                s.values
                    .filter((v) => !v.location.startsWith("cookie:") && !v.location.startsWith("url.path[") && v.class !== "constant")
                    .map((v): [string, ClassName, number | null] => {
                        let leaf = v.location.split(/[.:]/).pop()!;
                        // The same field is `rateId` in JSON and `rate` in a form: a wire name.
                        if (leaf === "rate") leaf = "rateId";
                        // The SPAs coerce a text field with Number(): "abc" is posted as JSON null, which
                        // is not an encoding of the input. The server-rendered shop posts the text itself.
                        const cls: ClassName = leaf === "quantity" && v.class === "client-computed" ? "input" : v.class;
                        // The step that produced a derived value is stack-independent.
                        return [leaf, cls, v.class === "derived" ? v.source!.step : null];
                    })
                    .sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)),
            );
        for (const scenario of SCENARIOS) {
            const base = signature(graphFor("react", scenario));
            for (const stack of ["server-rendered", "svelte"]) expect(signature(graphFor(stack, scenario)), `${stack}/${scenario}`).toEqual(base);
        }
    });

    test("deleting bootstrap from the real recordings unwires session and CSRF", () => {
        for (const stack of STACKS) {
            const intact = graphFor(stack, "success");
            expect(intact.edges.filter((e) => e.to.location === "header:x-csrf-token").length, stack).toBeGreaterThanOrEqual(4);
            const mutated = graphFor(stack, "success", (entries) => {
                const i = entries.findIndex((e) => e._requestId === "r1");
                entries.splice(i, 1);
            });
            // Following request-to-request reuse back from the first CSRF header ends at a value with
            // no source: a root the recording cannot explain.
            const first = mutated.steps.find((s) => s.values.some((v) => v.location === "header:x-csrf-token"))!;
            let [step, loc] = [first.index, "header:x-csrf-token"];
            let root: ClassName;
            for (;;) {
                const v = valueOf(mutated, step, loc)!;
                if (v.source?.where === "request") {
                    step = v.source.step!;
                    loc = v.source.location;
                } else {
                    root = v.class;
                    break;
                }
            }
            expect(root, `${stack}: the first CSRF use must become a root`).toBe("client-computed");
        }
    });
});
