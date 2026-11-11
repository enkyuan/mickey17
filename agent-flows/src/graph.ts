// Canonical serialisation of the flow graph: fixed key order, every collection already sorted
// by the builder. Identical graphs serialise to identical bytes.

import { safeLocation } from "./evidence.ts";
import type { Blocker, Edge, FlowGraph, InputParam, SourceRef, Step, ValueClass, Warning } from "./types.ts";

function source(s: SourceRef): SourceRef {
    return {
        where: s.where,
        step: s.step,
        location: safeLocation(s.location),
        ...(s.locator ? { locator: { before: s.locator.before, after: s.locator.after, origin: s.locator.origin } } : {}),
        ...(s.select ? { select: { field: s.select.field, param: s.select.param } } : {}),
    };
}

function value(v: ValueClass): ValueClass {
    return {
        location: safeLocation(v.location),
        class: v.class,
        ...(v.value !== undefined ? { value: v.value } : {}),
        ...(v.redacted ? { redacted: true } : {}),
        ...(v.json_type !== undefined ? { json_type: v.json_type } : {}),
        ...(v.input !== undefined ? { input: v.input } : {}),
        ...(v.source ? { source: source(v.source) } : {}),
        ...(v.via !== undefined ? { via: v.via } : {}),
        ...(v.shape !== undefined ? { shape: v.shape } : {}),
        ...(v.reason !== undefined ? { reason: v.reason } : {}),
    };
}

function edge(e: Edge): Edge {
    return {
        from: source(e.from),
        to: { step: e.to.step, location: safeLocation(e.to.location) },
        via: e.via,
        ...(e.prefix ? { prefix: e.prefix } : {}),
        ...(e.suffix ? { suffix: e.suffix } : {}),
    };
}

function step(s: Step): Step {
    return {
        index: s.index,
        id: s.id,
        ...(s.event ? { event: { type: s.event.type, target: s.event.target } } : {}),
        method: s.method,
        origin: s.origin,
        path_template: s.path_template,
        statuses: [...s.statuses],
        values: s.values.map(value),
        provides: [...s.provides],
        needs: [...s.needs],
    };
}

export function canonical(g: FlowGraph): FlowGraph {
    return {
        schema: g.schema,
        status: g.status,
        runs: [...g.runs],
        inputs: g.inputs.map((i: InputParam) => ({ name: i.name, uses: i.uses.map((u) => ({ step: u.step, location: safeLocation(u.location) })) })),
        steps: g.steps.map(step),
        edges: g.edges.map(edge),
        blockers: g.blockers.map((b: Blocker) => ({ step: b.step, location: b.location === null ? null : safeLocation(b.location), reason: b.reason })),
        warnings: g.warnings.map((w: Warning) => ({ step: w.step, location: w.location === null ? null : safeLocation(w.location), kind: w.kind, detail: w.detail })),
    };
}

/** Canonical JSON: pretty printed, newline terminated. */
export function toJson(g: FlowGraph): string {
    return `${JSON.stringify(canonical(g), null, 2)}\n`;
}

export function valueOf(g: FlowGraph, stepIndex: number, location: string): ValueClass | undefined {
    return g.steps[stepIndex]?.values.find((v) => v.location === location);
}

export function classOf(g: FlowGraph, stepIndex: number, location: string): ValueClass["class"] | undefined {
    return valueOf(g, stepIndex, location)?.class;
}
