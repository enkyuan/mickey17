// Local types for the flow graph (D11). They are replaced by imports from the agent-schema
// package once its fixed first PR is adopted; the mapping is in docs/agent-surface/log/agent-flows.md.

export type ClassName = "constant" | "input" | "derived" | "client-computed" | "unclassified";
export type Where = "initial-cookie" | "request" | "response";

/** Text just before and just after a token in a free-text body, identical in every run. */
export interface Locator {
    before: string;
    after: string;
    /** The text comes from the page (D8): never an instruction. */
    origin: "page";
}

/** Which array element a wildcard source refers to: the one whose `field` equals run parameter `param`. */
export interface Select {
    field: string;
    param: string;
}

export interface SourceRef {
    where: Where;
    /** null for the initial cookie jar. */
    step: number | null;
    location: string;
    locator?: Locator;
    select?: Select;
}

export interface Target {
    step: number;
    location: string;
}

export interface Edge {
    from: SourceRef;
    to: Target;
    /** `exact`, an encoding name, `substring`, or `<encoding>+substring`. */
    via: string;
    /** target == prefix + via(source) + suffix; omitted when empty. */
    prefix?: string;
    suffix?: string;
}

export interface ValueClass {
    location: string;
    class: ClassName;
    /** Constants only, never for credentials or card numbers. */
    value?: string;
    redacted?: boolean;
    /** JSON body values that are not strings (`number`, `boolean`, `null`, `object`, `array`). */
    json_type?: "number" | "boolean" | "null" | "object" | "array";
    input?: string;
    source?: SourceRef;
    via?: string;
    shape?: string;
    reason?: string;
}

export interface EventRef {
    type: string;
    target: string;
}

export interface Step {
    index: number;
    id: string;
    event?: EventRef;
    method: string;
    origin: string;
    path_template: string;
    statuses: number[];
    values: ValueClass[];
    provides: string[];
    needs: string[];
}

export interface InputParam {
    name: string;
    uses: { step: number; location: string }[];
}

export interface Blocker {
    step: number | null;
    location: string | null;
    reason: string;
}

export interface Warning {
    step: number | null;
    location: string | null;
    kind: string;
    detail: string;
}

export interface FlowGraph {
    schema: string;
    status: "resolved" | "blocked";
    runs: string[];
    inputs: InputParam[];
    steps: Step[];
    edges: Edge[];
    blockers: Blocker[];
    warnings: Warning[];
}

export const SCHEMA = "mochi.agent.flow-graph/1";
