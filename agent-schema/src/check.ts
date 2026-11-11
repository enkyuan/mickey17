// Invariants the wire format cannot express (it has no refinements). They run in code
// after parsing: cross-references, uniqueness, hash, coverage, and rules tied to the mode.
// Each issue carries a stable `code` so fixtures can assert the reason, not the wording.
import { z } from "zod";
import type { Action } from "./action";
import { AgentSurface } from "./surface";
import type { Coverage } from "./surface";
import { Mandate, ConfirmationRequest } from "./authority";
import type { Flow } from "./flow";
import { computeHash } from "./hash";
import { EVIDENCE_ORDER, effectRank } from "./policy";
import type { Tier } from "./policy";
import type { DependencyEdge, RequestSpec, ValueClass } from "./values";

export type IssuePath = (string | number)[];

export interface CheckIssue {
    path: IssuePath;
    code: string;
    message: string;
}

export type CheckCode =
    | "hash_mismatch"
    | "duplicate_id"
    | "dangling_reference"
    | "hidden_not_opted_in"
    | "mode_forbids_tier"
    | "mode_forbids_flows"
    | "evidence_not_ordered"
    | "coverage_mismatch"
    | "flow_effect_below_step"
    | "dependency_not_earlier"
    | "undeclared_input"
    | "payment_effect_mismatch"
    | "time_order"
    | "cap_order"
    | "tier_duplicated";

const issues = () => {
    const list: CheckIssue[] = [];
    const add = (path: IssuePath, code: CheckCode, message: string) => list.push({ path, code, message });
    return { list, add };
};

type Add = ReturnType<typeof issues>["add"];

function duplicates<T>(items: readonly T[], key: (t: T) => string): number[] {
    const seen = new Set<string>();
    const dups: number[] = [];
    items.forEach((item, i) => {
        const k = key(item);
        if (seen.has(k)) dups.push(i);
        seen.add(k);
    });
    return dups;
}

function evidenceOrder(path: IssuePath, evidence: readonly { source: string }[], add: Add) {
    let last = -1;
    evidence.forEach((e, i) => {
        const rank = EVIDENCE_ORDER.indexOf(e.source as (typeof EVIDENCE_ORDER)[number]);
        if (rank < last) add([...path, i, "source"], "evidence_not_ordered", "evidence must be ordered declared, observed, derived, inferred");
        last = Math.max(last, rank);
    });
}

function edgesOf(value: ValueClass): DependencyEdge[] {
    if (value.class === "derived") return [value.edge];
    if (value.class === "client-computed" && value.resolution.kind === "lift") return value.resolution.args;
    return [];
}

function checkRequestValues(
    path: IssuePath,
    request: RequestSpec,
    declaredInputs: ReadonlySet<string>,
    add: Add,
    onEdge?: (edge: DependencyEdge, edgePath: IssuePath) => void,
) {
    request.values.forEach((rv, i) => {
        const vp = [...path, "values", i, "value"];
        if (rv.value.class === "input" && !declaredInputs.has(rv.value.input)) {
            add([...vp, "input"], "undeclared_input", `input ${rv.value.input} is not declared`);
        }
        if (onEdge) {
            edgesOf(rv.value).forEach((edge, j) =>
                onEdge(edge, rv.value.class === "derived" ? [...vp, "edge"] : [...vp, "resolution", "args", j]),
            );
        }
    });
}

function checkFlow(flow: Flow, fi: number, doc: AgentSurface, nodeIds: ReadonlySet<string>, actionIds: ReadonlySet<string>, add: Add) {
    const p: IssuePath = ["flows", fi];
    const declared = new Set(flow.inputs.map((i) => i.name));
    duplicates(flow.inputs, (i) => i.name).forEach((i) => add([...p, "inputs", i, "name"], "duplicate_id", "duplicate input name"));
    duplicates(flow.steps, (s) => s.id).forEach((i) => add([...p, "steps", i, "id"], "duplicate_id", "duplicate step id"));

    const stepIndex = new Map<string, number>();
    flow.steps.forEach((s, i) => {
        if (!stepIndex.has(s.id)) stepIndex.set(s.id, i);
    });
    const requestsOf = new Map<string, Set<string>>();
    flow.steps.forEach((s) => {
        const set = new Set<string>();
        s.requests.forEach((r) => set.add(r.id));
        requestsOf.set(s.id, set);
    });
    const allRequestIds = flow.steps.flatMap((s, si) => s.requests.map((r, ri) => ({ id: r.id, path: [...p, "steps", si, "requests", ri, "id"] as IssuePath })));
    duplicates(allRequestIds, (r) => r.id).forEach((i) => add(allRequestIds[i]!.path, "duplicate_id", "duplicate request id within flow"));

    let top: number = 0;
    flow.steps.forEach((step, si) => {
        const sp = [...p, "steps", si];
        top = Math.max(top, effectRank(step.effect));
        if (step.action !== undefined && !actionIds.has(step.action)) {
            add([...sp, "action"], "dangling_reference", `step refers to unknown action ${step.action}`);
        }
        if ((step.effect === "financial") !== (step.payment !== undefined)) {
            add(
                [...sp, step.payment !== undefined ? "payment" : "effect"],
                "payment_effect_mismatch",
                "exactly the financial step declares a payment method",
            );
        }
        step.requests.forEach((req, ri) => {
            checkRequestValues([...sp, "requests", ri], req, declared, add, (edge, ep) => {
                const src = edge.source;
                const srcStep = stepIndex.get(src.step);
                if (srcStep === undefined) {
                    add([...ep, "source", "step"], "dangling_reference", `dependency refers to unknown step ${src.step}`);
                    return;
                }
                // A request may depend on an earlier request of the same step, never a later step.
                if (srcStep > si) add([...ep, "source", "step"], "dependency_not_earlier", "a value can only be sourced from the same or an earlier step");
                if (src.kind === "response") {
                    if (!requestsOf.get(src.step)?.has(src.request)) {
                        add([...ep, "source", "request"], "dangling_reference", `dependency refers to unknown request ${src.request} in step ${src.step}`);
                    } else if (srcStep === si) {
                        const order = step.requests.findIndex((r) => r.id === src.request);
                        if (order >= ri) add([...ep, "source", "request"], "dependency_not_earlier", "a request can only depend on an earlier request");
                    }
                }
                if (src.kind === "dom" && !nodeIds.has(src.node)) add([...ep, "source", "node"], "dangling_reference", `dependency refers to unknown node ${src.node}`);
            });
        });
    });
    if (effectRank(flow.effect) < top) {
        add([...p, "effect"], "flow_effect_below_step", "flow effect must be at least the most consequential step effect");
    }
    evidenceOrder([...p, "evidence"], flow.evidence, add);
    if (flow.status === "blocked") {
        flow.unclassified.forEach((u, i) => {
            const sr = requestsOf.get(u.step);
            if (!sr) add([...p, "unclassified", i, "step"], "dangling_reference", `unknown step ${u.step}`);
            else if (!sr.has(u.request)) add([...p, "unclassified", i, "request"], "dangling_reference", `unknown request ${u.request}`);
        });
    }
}

function checkAction(action: Action, ai: number, nodeIds: ReadonlySet<string>, routeIds: ReadonlySet<string>, add: Add) {
    const p: IssuePath = ["actions", ai];
    if (action.node !== undefined && !nodeIds.has(action.node)) add([...p, "node"], "dangling_reference", `unknown node ${action.node}`);
    if (action.kind === "navigate" && action.route !== undefined && !routeIds.has(action.route)) {
        add([...p, "route"], "dangling_reference", `unknown route ${action.route}`);
    }
    if (action.kind === "state") action.affects.forEach((n, i) => { if (!nodeIds.has(n)) add([...p, "affects", i], "dangling_reference", `unknown node ${n}`); });
    duplicates(action.inputs, (i) => i.name).forEach((i) => add([...p, "inputs", i, "name"], "duplicate_id", "duplicate input name"));
    if ("request" in action) {
        checkRequestValues([...p, "request"], action.request, new Set(action.inputs.map((i) => i.name)), add);
    }
    action.postconditions.forEach((pc, i) => {
        if ((pc.type === "node_present" || pc.type === "node_text_changed") && !nodeIds.has(pc.node)) add([...p, "postconditions", i, "node"], "dangling_reference", `unknown node ${pc.node}`);
        if (pc.type === "navigates_to" && !routeIds.has(pc.route)) add([...p, "postconditions", i, "route"], "dangling_reference", `unknown route ${pc.route}`);
    });
    evidenceOrder([...p, "evidence"], action.evidence, add);
}

const TIER_ORDER: readonly Tier[] = ["T0", "T1", "T2", "T3", "T4"];

/** The coverage a document implies: tiers with any content, in tier order; reasons sorted by name. */
export function deriveCoverage(doc: Pick<AgentSurface, "routes" | "actions" | "unsupported">): Coverage {
    const count = new Map<Tier, { routes: number; actions: number; unsupported: number }>();
    const bump = (t: Tier, f: "routes" | "actions" | "unsupported") => {
        const e = count.get(t) ?? { routes: 0, actions: 0, unsupported: 0 };
        e[f]++;
        count.set(t, e);
    };
    doc.routes.forEach((r) => new Set(r.tiers).forEach((t) => bump(t, "routes")));
    doc.actions.forEach((a) => bump(a.tier, "actions"));
    doc.unsupported.forEach((u) => bump(u.tier, "unsupported"));
    const reasons = new Map<string, number>();
    doc.unsupported.forEach((u) => reasons.set(u.reason, (reasons.get(u.reason) ?? 0) + 1));
    return {
        tiers: TIER_ORDER.filter((t) => count.has(t)).map((t) => ({ tier: t, ...count.get(t)! })),
        unsupported_reasons: [...reasons.entries()]
            .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
            .map(([reason, n]) => ({ reason: reason as Coverage["unsupported_reasons"][number]["reason"], count: n })),
    };
}

/** Checks a structurally valid surface. Returns [] when every invariant holds. */
export function checkSurface(doc: AgentSurface): CheckIssue[] {
    const { list, add } = issues();

    if (computeHash(doc) !== doc.hash) add(["hash"], "hash_mismatch", "hash does not match the canonical content");

    duplicates(doc.routes, (r) => r.id).forEach((i) => add(["routes", i, "id"], "duplicate_id", "duplicate route id"));
    const allNodes = doc.routes.flatMap((r, ri) => r.nodes.map((n, ni) => ({ n, path: ["routes", ri, "nodes", ni] as IssuePath })));
    duplicates(allNodes, (x) => x.n.id).forEach((i) => add([...allNodes[i]!.path, "id"], "duplicate_id", "duplicate node id"));
    const ids = [...doc.actions.map((a, i) => ({ id: a.id, path: ["actions", i, "id"] as IssuePath })), ...doc.unsupported.map((a, i) => ({ id: a.id, path: ["unsupported", i, "id"] as IssuePath }))];
    duplicates(ids, (x) => x.id).forEach((i) => add(ids[i]!.path, "duplicate_id", "duplicate action id"));
    duplicates(doc.flows, (f) => f.id).forEach((i) => add(["flows", i, "id"], "duplicate_id", "duplicate flow id"));

    const nodeIds = new Set(allNodes.map((x) => x.n.id));
    const routeIds = new Set(doc.routes.map((r) => r.id));
    const actionIds = new Set(doc.actions.map((a) => a.id));

    doc.routes.forEach((route, ri) => {
        const local = new Set(route.nodes.map((n) => n.id));
        route.nodes.forEach((n, ni) => {
            if (n.parent !== undefined && !local.has(n.parent)) add(["routes", ri, "nodes", ni, "parent"], "dangling_reference", "parent must be a node of the same route");
            if (n.visibility !== "visible" && !doc.options.include_hidden) add(["routes", ri, "nodes", ni, "visibility"], "hidden_not_opted_in", "hidden or inert nodes require options.include_hidden");
        });
        if (new Set(route.tiers).size !== route.tiers.length) add(["routes", ri, "tiers"], "tier_duplicated", "tiers must be unique");
    });

    doc.actions.forEach((a, i) => checkAction(a, i, nodeIds, routeIds, add));
    doc.unsupported.forEach((u, i) => {
        if (u.node !== undefined && !nodeIds.has(u.node)) add(["unsupported", i, "node"], "dangling_reference", `unknown node ${u.node}`);
    });
    doc.flows.forEach((f, i) => checkFlow(f, i, doc, nodeIds, actionIds, add));

    // D2: a read-only surface carries T0/T1 only. Unsupported items (T4) are listed, not offered.
    if (doc.mode === "read-only") {
        const allowed: ReadonlySet<Tier> = new Set<Tier>(["T0", "T1"]);
        doc.actions.forEach((a, i) => { if (!allowed.has(a.tier)) add(["actions", i, "tier"], "mode_forbids_tier", "read-only surfaces carry T0/T1 only"); });
        doc.routes.forEach((r, ri) => r.tiers.forEach((t, ti) => { if (!allowed.has(t) && t !== "T4") add(["routes", ri, "tiers", ti], "mode_forbids_tier", "read-only surfaces carry T0/T1 only"); }));
        if (doc.flows.length > 0) add(["flows"], "mode_forbids_flows", "read-only surfaces carry no flows");
    }

    // Coverage must equal what the document contains.
    const expected = deriveCoverage(doc);
    const expectedTiers = new Map(expected.tiers.map((t) => [t.tier, t]));
    const seenTiers = new Set<Tier>();
    doc.coverage.tiers.forEach((c, i) => {
        const e = expectedTiers.get(c.tier) ?? { tier: c.tier, routes: 0, actions: 0, unsupported: 0 };
        if (seenTiers.has(c.tier)) add(["coverage", "tiers", i, "tier"], "tier_duplicated", "tier listed twice");
        seenTiers.add(c.tier);
        (["routes", "actions", "unsupported"] as const).forEach((f) => {
            if (c[f] !== e[f]) add(["coverage", "tiers", i, f], "coverage_mismatch", `coverage.${f} is ${c[f]} but the document has ${e[f]}`);
        });
    });
    expectedTiers.forEach((_, t) => {
        if (!seenTiers.has(t)) add(["coverage", "tiers"], "coverage_mismatch", `coverage is missing tier ${t}`);
    });
    const reasons = new Map(expected.unsupported_reasons.map((r) => [r.reason, r.count]));
    const listed = new Set<string>();
    doc.coverage.unsupported_reasons.forEach((r, i) => {
        listed.add(r.reason);
        if ((reasons.get(r.reason) ?? 0) !== r.count) add(["coverage", "unsupported_reasons", i, "count"], "coverage_mismatch", `count for ${r.reason} is ${r.count} but the document has ${reasons.get(r.reason) ?? 0}`);
    });
    reasons.forEach((_, r) => {
        if (!listed.has(r)) add(["coverage", "unsupported_reasons"], "coverage_mismatch", `coverage is missing unsupported reason ${r}`);
    });

    return list;
}

/** Cross-field rules for a mandate. */
export function checkMandate(m: z.infer<typeof Mandate>): CheckIssue[] {
    const { list, add } = issues();
    if (computeHash(m) !== m.hash) add(["hash"], "hash_mismatch", "hash does not match the canonical content");
    if (m.expires_at <= m.issued_at) add(["expires_at"], "time_order", "expires_at must be after issued_at");
    if (m.per_call_cap_minor !== undefined && m.per_call_cap_minor > m.spend_cap_minor) add(["per_call_cap_minor"], "cap_order", "per-call cap exceeds the spend cap");
    duplicates(m.scope, (s) => `${s.kind}:${s.id}`).forEach((i) => add(["scope", i], "duplicate_id", "duplicate scope entry"));
    return list;
}

/** Cross-field rules for a confirmation request. */
export function checkConfirmationRequest(c: z.infer<typeof ConfirmationRequest>): CheckIssue[] {
    const { list, add } = issues();
    if (c.expires_at <= c.issued_at) add(["expires_at"], "time_order", "expires_at must be after issued_at");
    if (c.effect === "financial") {
        c.items.forEach((it, i) => {
            if (it.amount !== undefined && it.amount.currency !== c.total.currency) add(["items", i, "amount", "currency"], "cap_order", "item currency differs from total currency");
        });
    }
    return list;
}

export type ValidationResult<T> = { ok: true; value: T } | { ok: false; phase: "schema" | "check"; issues: CheckIssue[] };

export const fromZodIssues = (error: z.ZodError): CheckIssue[] =>
    error.issues.map((i) => ({ path: i.path.map((s) => (typeof s === "symbol" ? String(s) : s)), code: i.code, message: i.message }));

/** Parse, then check. Consumers (executor, MCP server, verifier) refuse a surface that does not pass. */
export function validateSurface(input: unknown): ValidationResult<AgentSurface> {
    const parsed = AgentSurface.safeParse(input);
    if (!parsed.success) return { ok: false, phase: "schema", issues: fromZodIssues(parsed.error) };
    const found = checkSurface(parsed.data);
    return found.length === 0 ? { ok: true, value: parsed.data } : { ok: false, phase: "check", issues: found };
}
