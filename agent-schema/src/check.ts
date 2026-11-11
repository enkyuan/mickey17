// Invariants the wire format cannot express (it has no refinements). They run in code
// after parsing: cross-references, uniqueness, hash, coverage, money and secret rules,
// and rules tied to the mode. Each issue carries a stable `code` so fixtures assert the
// reason, not the wording.
import { z } from "zod";
import type { Action, Input } from "./action";
import type { ConfirmationRequest, ConfirmationToken, Mandate, Receipt } from "./authority";
import type { OperationResult } from "./result";
import type { Flow, FlowStep } from "./flow";
import { canonicalize, computeHash } from "./hash";
import { EFFECT_ORDER, EVIDENCE_ORDER, effectRank, requiresConfirmation } from "./policy";
import type { Effect, Tier } from "./policy";
import { SCHEMA_MAJOR, SCHEMA_MINOR } from "./primitives";
import { AgentSurface } from "./surface";
import type { Coverage } from "./surface";
import type { DependencyEdge, RequestSpec, ValueClass } from "./values";

export type IssuePath = (string | number)[];

export interface CheckIssue {
    path: IssuePath;
    code: string;
    message: string;
}

export const CHECK_CODES = [
    "hash_mismatch",
    "duplicate_id",
    "dangling_reference",
    "hidden_not_opted_in",
    "mode_forbids_tier",
    "mode_forbids_effect",
    "mode_forbids_flows",
    "gates_forbid_read_only",
    "read_only_requires_auth",
    "evidence_not_ordered",
    "coverage_mismatch",
    "coverage_not_canonical",
    "flow_effect_mismatch",
    "step_effect_mismatch",
    "dependency_not_earlier",
    "dependency_missing_step",
    "edge_needs_flow",
    "undeclared_input",
    "payment_effect_mismatch",
    "idempotency_key_missing",
    "idempotency_key_not_in_request",
    "time_order",
    "invalid_timestamp",
    "cap_order",
    "currency_mismatch",
    "total_mismatch",
    "tier_duplicated",
    "secret_constant",
    "card_number_constant",
    "password_not_credential",
    "card_input_forbidden",
    "sensitive_in_url",
    "page_text_in_surface",
    "ill_formed_string",
    "parent_cycle",
    "unclassified_has_value",
    "constraint_order",
    "enum_without_options",
    "schema_version_newer",
    "unsupported_tier_mismatch",
    "receipt_incomplete",
    "receipt_tier_effect",
    "amount_not_financial",
    "payment_token_not_financial",
    "human_gate_not_financial",
    "secret_in_url",
    "gates_unchecked",
] as const;
export type CheckCode = (typeof CHECK_CODES)[number];

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

// ---------------------------------------------------------------------------
// Schema version
// ---------------------------------------------------------------------------

export type VersionCheck = { ok: true } | { ok: false; reason: "malformed" | "major" | "minor" };

/** Reader rule: accept the same major and a minor no newer than this package's. */
export function checkSchemaVersion(version: string): VersionCheck {
    const m = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.exec(version);
    if (!m) return { ok: false, reason: "malformed" };
    if (Number(m[1]) !== SCHEMA_MAJOR) return { ok: false, reason: "major" };
    if (Number(m[2]) > SCHEMA_MINOR) return { ok: false, reason: "minor" };
    return { ok: true };
}

// ---------------------------------------------------------------------------
// Generic scan: ill-formed strings, timestamps, page text copied into surface text
// ---------------------------------------------------------------------------

/** A page string must be at least this long (normalised) and have at least three words to count as copyable text. */
const MIN_COPY_CHARS = 20;
const SHINGLE = 3;

const normalise = (t: string): string => t.normalize("NFKC").toLowerCase().replace(/[\u200b-\u200d\u2060\ufeff]/g, "").replace(/\s+/g, " ").trim();

/**
 * Scans any document. Flags (1) ill-formed UTF-16 (a lone surrogate cannot round-trip through
 * UTF-8 JSON), (2) timestamps that are not real instants (keys ending `_at`, and a top-level `at`),
 * and (3) surface-authored text that repeats a page string verbatim after case, whitespace and
 * zero-width normalisation (D8 property: page text never appears in a surface-origin field).
 * Only page strings of 20+ characters and 3+ words count, so ordinary headings do not trip it, and
 * matching is by three-word shingle, so cost is linear in the text. It catches copying, not
 * paraphrase or a copy that starts mid-word: a tripwire, not a proof.
 */
function scan(doc: unknown, add: Add) {
    const pageByKey = new Map<string, string[]>();
    const surface: { path: IssuePath; value: string }[] = [];
    const walk = (n: unknown, path: IssuePath) => {
        if (typeof n === "string") {
            if (!n.isWellFormed()) add(path, "ill_formed_string", "string contains a lone surrogate");
            return;
        }
        if (Array.isArray(n)) return n.forEach((x, i) => walk(x, [...path, i]));
        if (!n || typeof n !== "object") return;
        const o = n as Record<string, unknown>;
        if (typeof o.value === "string" && (o.origin === "page" || o.origin === "surface")) {
            if (o.origin === "page") {
                const t = normalise(o.value);
                const words = t.split(" ");
                if (t.length >= MIN_COPY_CHARS && words.length >= SHINGLE) {
                    const key = words.slice(0, SHINGLE).join(" ");
                    const list = pageByKey.get(key);
                    if (list) list.push(t);
                    else pageByKey.set(key, [t]);
                }
            } else surface.push({ path: [...path, "value"], value: o.value });
        }
        for (const [k, v] of Object.entries(o)) {
            if ((k.endsWith("_at") || (k === "at" && path.length === 0)) && typeof v === "string" && /^[0-9]{4}-/.test(v)) {
                const t = new Date(v);
                if (Number.isNaN(t.getTime()) || `${t.toISOString().slice(0, 19)}Z` !== v) {
                    add([...path, k], "invalid_timestamp", `${v} is not a real instant`);
                }
            }
            walk(v, [...path, k]);
        }
    };
    walk(doc, []);
    if (pageByKey.size === 0) return;
    for (const s of surface) {
        const t = normalise(s.value);
        const words = t.split(" ");
        let hit = false;
        for (let i = 0; i + SHINGLE <= words.length && !hit; i++) {
            const cands = pageByKey.get(words.slice(i, i + SHINGLE).join(" "));
            if (cands?.some((c) => t.includes(c))) hit = true;
        }
        if (hit) add(s.path, "page_text_in_surface", "surface-authored text repeats page text");
    }
}

// ---------------------------------------------------------------------------
// Secrets and instruments (D7, D11)
// ---------------------------------------------------------------------------

/** Names that carry credentials. Applied to the last segment of a header, query, form, cookie or JSON-pointer name. */
const SECRET_NAME = /(passw(or)?d|passcode|secret|token|api[-_]?key|access[-_]?key|auth|session|sid$|credential|bearer|jwt|private[-_]?key)/i;
const SECRET_SHAPE =
    /^(bearer|basic)\s+\S+|\bsk_(live|test)_[A-Za-z0-9]+|^eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*$|^AKIA[0-9A-Z]{16}$|\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}|\bxox[abprs]-[A-Za-z0-9-]{10,}|\bAIza[0-9A-Za-z_-]{20,}|-----BEGIN [A-Z ]*PRIVATE KEY-----/i;
/** Card-ish input names, after splitting camelCase: card number/num/no, cc number, credit card, cvv/cvc, security code, expiry fields. */
const CARD_INPUT =
    /(^|_)(card_?(number|num|no|pan)|cardnumber|cc_?(num|number|no)|ccnum|credit_?card(_?(number|num))?|cvv2?|cvc2?|security_?code|expiry|expiration|exp_(month|year|date)|card_?exp[a-z_]*|primary_?account_?number)($|_)/;
const PASSWORD_NAME = /(passw(or)?d|passcode|secret|api[-_]?key)/i;

const lastSegment = (name: string): string => name.split("/").filter(Boolean).pop() ?? name;
const snake = (name: string): string => name.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase().replace(/[-.]/g, "_");

/** Issuer prefixes and lengths that make a Luhn-valid digit run plausibly a payment card (13-digit GTINs excluded). */
function looksLikeCard(digits: string): boolean {
    if (!/^[0-9]{15,19}$/.test(digits) || !luhn(digits)) return false;
    return /^(4|5[1-5]|2[2-7]|3[47]|3[0689]|6011|65|35|62|81|50|56|57|58|6[0-9])/.test(digits);
}

function luhn(digits: string): boolean {
    let sum = 0;
    let alt = false;
    for (let i = digits.length - 1; i >= 0; i--) {
        let d = digits.charCodeAt(i) - 48;
        if (alt) {
            d *= 2;
            if (d > 9) d -= 9;
        }
        sum += d;
        alt = !alt;
    }
    return sum % 10 === 0;
}

function checkInputs(path: IssuePath, inputs: readonly Input[], add: Add) {
    duplicates(inputs, (i) => i.name).forEach((i) => add([...path, i, "name"], "duplicate_id", "duplicate input name"));
    inputs.forEach((inp, i) => {
        const p = [...path, i];
        if (CARD_INPUT.test(snake(inp.name))) add([...p, "name"], "card_input_forbidden", "card data never passes through the surface; use a payment-token input");
        if ((inp.type === "password" || PASSWORD_NAME.test(inp.name)) && inp.sensitivity !== "credential") add([...p, "sensitivity"], "password_not_credential", "a password or secret input must be sensitivity credential");
        if (inp.min !== undefined && inp.max !== undefined && inp.min > inp.max) add([...p, "min"], "constraint_order", "min exceeds max");
        if (inp.min_length !== undefined && inp.max_length !== undefined && inp.min_length > inp.max_length) add([...p, "min_length"], "constraint_order", "min_length exceeds max_length");
        if (inp.type === "enum" && (!inp.options || inp.options.length === 0)) add([...p, "options"], "enum_without_options", "an enum input lists its options");
    });
}

function edgesOf(value: ValueClass): { edge: DependencyEdge; sub: IssuePath }[] {
    if (value.class === "derived") return [{ edge: value.edge, sub: ["edge"] }];
    if (value.class === "client-computed" && value.resolution.kind === "lift") {
        return value.resolution.args.map((edge, j) => ({ edge, sub: ["resolution", "args", j] }));
    }
    return [];
}

/** Rules every request obeys, in or out of a flow. `effect` is the effect of the action or step that sends it. */
function checkRequest(path: IssuePath, request: RequestSpec, inputs: readonly Input[], effect: Effect, add: Add, onEdge: (edge: DependencyEdge, p: IssuePath) => void) {
    const byName = new Map(inputs.map((i) => [i.name, i]));
    const url = request.url.value;
    if (SECRET_SHAPE.test(url) || /[?&;][^=&]*(passw(or)?d|secret|token|api[-_]?key|access[-_]?key|auth|session)[^=&]*=/i.test(url)) {
        add([...path, "url", "value"], "secret_in_url", "a credential must not appear in a URL");
    }
    for (const run of url.replace(/[ .-]/g, "").match(/[0-9]{13,19}/g) ?? []) {
        if (looksLikeCard(run)) {
            add([...path, "url", "value"], "card_number_constant", "URL contains what looks like a payment card number");
            break;
        }
    }
    request.values.forEach((rv, i) => {
        const vp = [...path, "values", i, "value"];
        const v = rv.value;
        if (v.class === "input") {
            const inp = byName.get(v.input);
            if (!inp) add([...vp, "input"], "undeclared_input", `input ${v.input} is not declared`);
            else {
                if ((inp.sensitivity === "credential" || inp.sensitivity === "payment-token") && (rv.at.in === "query" || rv.at.in === "path")) {
                    add([...path, "values", i, "at", "in"], "sensitive_in_url", "a credential or payment token must not be placed in the URL");
                }
                if (inp.sensitivity === "payment-token" && effect !== "financial") {
                    add([...vp, "input"], "payment_token_not_financial", "a request that carries a payment token is financial");
                }
            }
        }
        if (v.class === "constant") {
            const raw = String(v.value.value);
            const name = lastSegment(rv.at.name);
            if (SECRET_NAME.test(name) || SECRET_SHAPE.test(raw)) {
                add([...vp, "value"], "secret_constant", "a credential or session value must be supplied at run time, not embedded");
            }
            if (looksLikeCard(raw.replace(/[ .-]/g, ""))) add([...vp, "value"], "card_number_constant", "constant looks like a payment card number");
        }
        edgesOf(v).forEach(({ edge, sub }) => onEdge(edge, [...vp, ...sub]));
    });
}

// ---------------------------------------------------------------------------
// Evidence and money rules shared by actions and flow steps
// ---------------------------------------------------------------------------

function evidenceRules(path: IssuePath, evidence: readonly { source: string }[], add: Add) {
    let last = -1;
    evidence.forEach((e, i) => {
        const rank = EVIDENCE_ORDER.indexOf(e.source as (typeof EVIDENCE_ORDER)[number]);
        if (rank < last) add([...path, i, "source"], "evidence_not_ordered", "evidence must be ordered declared, observed, derived, inferred");
        last = Math.max(last, rank);
    });
}

function idempotencyRules(
    path: IssuePath,
    effect: Effect,
    idem: { idempotent: boolean; key?: { in: string; name: string } | undefined },
    requests: readonly RequestSpec[],
    add: Add,
) {
    if (effect === "financial" && idem.key === undefined) {
        add([...path, "idempotency"], "idempotency_key_missing", "a money-moving call carries an idempotency key (D11)");
    }
    if (idem.key !== undefined) {
        const key = idem.key;
        const found = requests.some((r) => r.values.some((rv) => rv.at.in === key.in && rv.at.name === key.name && rv.value.class === "input"));
        if (!found) add([...path, "idempotency", "key"], "idempotency_key_not_in_request", "the key location must be a caller-supplied input in the request");
    }
}

// ---------------------------------------------------------------------------
// Flows and actions
// ---------------------------------------------------------------------------

interface DocIndex {
    nodeIds: ReadonlySet<string>;
    routeIds: ReadonlySet<string>;
    actions: ReadonlyMap<string, Action>;
}

function postconditionRefs(path: IssuePath, pc: Action["postconditions"][number], idx: DocIndex, add: Add, inputs: readonly Input[]) {
    if (pc.type === "response_field_equals_input" && !inputs.some((i) => i.name === pc.input)) add([...path, "input"], "undeclared_input", `input ${pc.input} is not declared`);
    if ((pc.type === "node_present" || pc.type === "node_text_changed") && !idx.nodeIds.has(pc.node)) add([...path, "node"], "dangling_reference", `unknown node ${pc.node}`);
    if (pc.type === "navigates_to" && !idx.routeIds.has(pc.route)) add([...path, "route"], "dangling_reference", `unknown route ${pc.route}`);
}

function checkFlow(flow: Flow, fi: number, idx: DocIndex, add: Add) {
    const p: IssuePath = ["flows", fi];
    checkInputs([...p, "inputs"], flow.inputs, add);
    duplicates(flow.steps, (s) => s.id).forEach((i) => add([...p, "steps", i, "id"], "duplicate_id", "duplicate step id"));

    const stepIndex = new Map<string, number>();
    flow.steps.forEach((s, i) => {
        if (!stepIndex.has(s.id)) stepIndex.set(s.id, i);
    });
    const requestsOf = new Map<string, FlowStep["requests"]>();
    flow.steps.forEach((s) => { if (!requestsOf.has(s.id)) requestsOf.set(s.id, s.requests); });
    const allRequestIds = flow.steps.flatMap((s, si) => s.requests.map((r, ri) => ({ id: r.id, path: [...p, "steps", si, "requests", ri, "id"] as IssuePath })));
    duplicates(allRequestIds, (r) => r.id).forEach((i) => add(allRequestIds[i]!.path, "duplicate_id", "duplicate request id within flow"));

    const edgeRule = (si: number, ri: number | null) => (edge: DependencyEdge, ep: IssuePath) => {
        const src = edge.source;
        if (src.step === undefined) {
            add([...ep, "source"], "dependency_missing_step", "in a flow every dependency names the step it was observed at");
            return;
        }
        const srcStep = stepIndex.get(src.step);
        if (srcStep === undefined) {
            add([...ep, "source", "step"], "dangling_reference", `dependency refers to unknown step ${src.step}`);
            return;
        }
        if (srcStep > si) add([...ep, "source", "step"], "dependency_not_earlier", "a value can only be sourced from the same or an earlier step");
        if (src.kind === "response") {
            const reqs = requestsOf.get(src.step) ?? [];
            const target = reqs.findIndex((r) => r.id === src.request);
            if (target < 0) add([...ep, "source", "request"], "dangling_reference", `dependency refers to unknown request ${src.request} in step ${src.step}`);
            else if (srcStep === si && (ri === null || target >= ri)) add([...ep, "source", "request"], "dependency_not_earlier", "a request or precondition can only depend on an earlier request");
        }
        if (src.kind === "dom" && !idx.nodeIds.has(src.node)) add([...ep, "source", "node"], "dangling_reference", `dependency refers to unknown node ${src.node}`);
    };

    let top = 0;
    flow.steps.forEach((step, si) => {
        const sp = [...p, "steps", si];
        top = Math.max(top, effectRank(step.effect));
        if (step.action !== undefined) {
            const a = idx.actions.get(step.action);
            if (!a) add([...sp, "action"], "dangling_reference", `step refers to unknown action ${step.action}`);
            else if (step.effect !== a.effect) add([...sp, "effect"], "step_effect_mismatch", `step effect differs from its action's effect (${a.effect})`);
        }
        if ((step.effect === "financial") !== (step.payment !== undefined)) {
            add([...sp, step.payment !== undefined ? "payment" : "effect"], "payment_effect_mismatch", "exactly the financial step declares a payment method");
        }
        idempotencyRules(sp, step.effect, step.idempotency, step.requests, add);
        if (step.effect !== "financial" && step.human_gates.some((g) => g === "3ds" || g === "sca")) {
            add([...sp, "human_gates"], "human_gate_not_financial", "a 3-D Secure or SCA challenge belongs to a financial step");
        }
        step.preconditions.forEach((pc, i) => {
            if (pc.type === "prior_action" && !idx.actions.has(pc.action)) add([...sp, "preconditions", i, "action"], "dangling_reference", `unknown action ${pc.action}`);
            if (pc.type === "token") edgeRule(si, null)(pc.source, [...sp, "preconditions", i, "source"]);
        });
        step.postconditions.forEach((pc, i) => postconditionRefs([...sp, "postconditions", i], pc, idx, add, flow.inputs));
        step.requests.forEach((req, ri) => checkRequest([...sp, "requests", ri], req, flow.inputs, step.effect, add, edgeRule(si, ri)));
    });
    if (flow.effect !== EFFECT_ORDER[top]) add([...p, "effect"], "flow_effect_mismatch", "flow effect must equal the most consequential step effect");
    evidenceRules([...p, "evidence"], flow.evidence, add);
    if (flow.status === "blocked") {
        flow.unclassified.forEach((u, i) => {
            const reqs = requestsOf.get(u.step);
            const req = reqs?.find((r) => r.id === u.request);
            if (!reqs) add([...p, "unclassified", i, "step"], "dangling_reference", `unknown step ${u.step}`);
            else if (!req) add([...p, "unclassified", i, "request"], "dangling_reference", `unknown request ${u.request}`);
            else if (req.values.some((rv) => rv.at.in === u.at.in && rv.at.name === u.at.name)) {
                add([...p, "unclassified", i, "at"], "unclassified_has_value", "a value listed as unclassified is also classed in its request");
            }
        });
    }
}

function checkAction(action: Action, ai: number, idx: DocIndex, add: Add) {
    const p: IssuePath = ["actions", ai];
    if (action.node !== undefined && !idx.nodeIds.has(action.node)) add([...p, "node"], "dangling_reference", `unknown node ${action.node}`);
    if (action.kind === "navigate" && action.route !== undefined && !idx.routeIds.has(action.route)) add([...p, "route"], "dangling_reference", `unknown route ${action.route}`);
    if (action.kind === "state") action.affects.forEach((n, i) => { if (!idx.nodeIds.has(n)) add([...p, "affects", i], "dangling_reference", `unknown node ${n}`); });
    checkInputs([...p, "inputs"], action.inputs, add);

    // A standalone action has no steps: only dom, cookie and storage sources, with no step.
    const edgeRule = (edge: DependencyEdge, ep: IssuePath) => {
        const src = edge.source;
        if (src.kind === "response") add([...ep, "source", "kind"], "edge_needs_flow", "a response source needs a flow step; model it in a flow");
        else if (src.step !== undefined) add([...ep, "source", "step"], "dangling_reference", "a standalone action has no steps");
        if (src.kind === "dom" && !idx.nodeIds.has(src.node)) add([...ep, "source", "node"], "dangling_reference", `dependency refers to unknown node ${src.node}`);
    };
    action.preconditions.forEach((pc, i) => {
        if (pc.type === "prior_action" && !idx.actions.has(pc.action)) add([...p, "preconditions", i, "action"], "dangling_reference", `unknown action ${pc.action}`);
        if (pc.type === "token") edgeRule(pc.source, [...p, "preconditions", i, "source"]);
    });
    action.postconditions.forEach((pc, i) => postconditionRefs([...p, "postconditions", i], pc, idx, add, action.inputs));
    if ("request" in action) {
        checkRequest([...p, "request"], action.request, action.inputs, action.effect, add, edgeRule);
        if ((action.effect === "financial") !== (action.payment !== undefined)) {
            add([...p, action.payment !== undefined ? "payment" : "effect"], "payment_effect_mismatch", "exactly a financial action declares a payment method");
        }
        idempotencyRules(p, action.effect, action.idempotency, [action.request], add);
    } else if (action.idempotency.key !== undefined) {
        add([...p, "idempotency", "key"], "idempotency_key_not_in_request", "an action without a request has no key location");
    }
    evidenceRules([...p, "evidence"], action.evidence, add);
}

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

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

const OPAQUE_REASONS: ReadonlySet<string> = new Set(["opaque-canvas", "opaque-webgl", "drag-only", "video"]);

/** Hash of a document, or undefined when it cannot be canonicalised (the scan reports why). */
function safeHash(doc: { hash: string }): string | undefined {
    try {
        return computeHash(doc);
    } catch {
        return undefined;
    }
}

/** Checks a structurally valid surface. Returns [] when every invariant holds. */
export function checkSurface(doc: AgentSurface): CheckIssue[] {
    const { list, add } = issues();

    const hash = safeHash(doc);
    if (hash !== undefined && hash !== doc.hash) add(["hash"], "hash_mismatch", "hash does not match the canonical content");
    if (!checkSchemaVersion(doc.schema_version).ok) add(["schema_version"], "schema_version_newer", `this reader supports schema ${SCHEMA_MAJOR}.0 to ${SCHEMA_MAJOR}.${SCHEMA_MINOR}`);
    scan(doc, add);

    duplicates(doc.routes, (r) => r.id).forEach((i) => add(["routes", i, "id"], "duplicate_id", "duplicate route id"));
    const allNodes = doc.routes.flatMap((r, ri) => r.nodes.map((n, ni) => ({ n, path: ["routes", ri, "nodes", ni] as IssuePath })));
    duplicates(allNodes, (x) => x.n.id).forEach((i) => add([...allNodes[i]!.path, "id"], "duplicate_id", "duplicate node id"));
    const ids = [
        ...doc.actions.map((a, i) => ({ id: a.id, path: ["actions", i, "id"] as IssuePath })),
        ...doc.unsupported.map((a, i) => ({ id: a.id, path: ["unsupported", i, "id"] as IssuePath })),
    ];
    duplicates(ids, (x) => x.id).forEach((i) => add(ids[i]!.path, "duplicate_id", "duplicate action id"));
    duplicates(doc.flows, (f) => f.id).forEach((i) => add(["flows", i, "id"], "duplicate_id", "duplicate flow id"));

    const idx: DocIndex = {
        nodeIds: new Set(allNodes.map((x) => x.n.id)),
        routeIds: new Set(doc.routes.map((r) => r.id)),
        actions: new Map(doc.actions.map((a) => [a.id, a])),
    };

    doc.routes.forEach((route, ri) => {
        const parentOf = new Map(route.nodes.map((n) => [n.id, n.parent]));
        route.nodes.forEach((n, ni) => {
            const np = ["routes", ri, "nodes", ni];
            if (n.parent !== undefined && !parentOf.has(n.parent)) add([...np, "parent"], "dangling_reference", "parent must be a node of the same route");
            else if (n.parent !== undefined) {
                const seen = new Set<string>([n.id]);
                for (let cur: string | undefined = n.parent; cur !== undefined; cur = parentOf.get(cur)) {
                    if (seen.has(cur)) {
                        add([...np, "parent"], "parent_cycle", "a node cannot be its own ancestor");
                        break;
                    }
                    seen.add(cur);
                }
            }
            if (n.visibility !== "visible" && !doc.options.include_hidden) add([...np, "visibility"], "hidden_not_opted_in", "hidden or inert nodes require options.include_hidden");
        });
        if (new Set(route.tiers).size !== route.tiers.length) add(["routes", ri, "tiers"], "tier_duplicated", "tiers must be unique");
    });

    doc.actions.forEach((a, i) => checkAction(a, i, idx, add));
    doc.unsupported.forEach((u, i) => {
        if (u.node !== undefined && !idx.nodeIds.has(u.node)) add(["unsupported", i, "node"], "dangling_reference", `unknown node ${u.node}`);
        if (OPAQUE_REASONS.has(u.reason) && u.tier !== "T4") add(["unsupported", i, "tier"], "unsupported_tier_mismatch", `${u.reason} is opaque: tier T4`);
    });
    doc.flows.forEach((f, i) => checkFlow(f, i, idx, add));

    // D2: a read-only surface carries T0/T1 only, needs no auth, and is not built where robots or ToS forbid it.
    if (doc.mode === "read-only") {
        const allowed: ReadonlySet<Tier> = new Set<Tier>(["T0", "T1"]);
        doc.actions.forEach((a, i) => {
            if (!allowed.has(a.tier)) add(["actions", i, "tier"], "mode_forbids_tier", "read-only surfaces carry T0/T1 only");
            else if (a.auth.kind !== "none") add(["actions", i, "auth"], "read_only_requires_auth", "read-only surfaces carry no authenticated actions");
            a.preconditions.forEach((pc, pi) => { if (pc.type === "authenticated") add(["actions", i, "preconditions", pi], "read_only_requires_auth", "read-only surfaces carry no authenticated actions"); });
        });
        doc.routes.forEach((r, ri) => r.tiers.forEach((t, ti) => { if (!allowed.has(t)) add(["routes", ri, "tiers", ti], "mode_forbids_tier", "read-only surfaces carry T0/T1 only"); }));
        if (doc.flows.length > 0) add(["flows"], "mode_forbids_flows", "read-only surfaces carry no flows");
        if (doc.gates.robots_txt === "not-checked") add(["gates", "robots_txt"], "gates_unchecked", "a read-only surface requires robots.txt to have been checked");
        if (doc.gates.tos === "not-checked") add(["gates", "tos"], "gates_unchecked", "a read-only surface requires the terms of service to have been checked");
        if (doc.gates.robots_txt === "disallowed") add(["gates", "robots_txt"], "gates_forbid_read_only", "robots.txt disallows access");
        if (doc.gates.tos === "restricted") add(["gates", "tos"], "gates_forbid_read_only", "terms of service restrict automated access");
    }

    // Coverage must equal what the document contains, in exactly one encoding.
    const expected = deriveCoverage(doc);
    const before = list.length;
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
        if (listed.has(r.reason)) add(["coverage", "unsupported_reasons", i, "reason"], "duplicate_id", "reason listed twice");
        listed.add(r.reason);
        if ((reasons.get(r.reason) ?? 0) !== r.count) add(["coverage", "unsupported_reasons", i, "count"], "coverage_mismatch", `count for ${r.reason} is ${r.count} but the document has ${reasons.get(r.reason) ?? 0}`);
    });
    reasons.forEach((_, r) => {
        if (!listed.has(r)) add(["coverage", "unsupported_reasons"], "coverage_mismatch", `coverage is missing unsupported reason ${r}`);
    });
    if (list.length === before && canonicalize(doc.coverage) !== canonicalize(expected)) {
        add(["coverage"], "coverage_not_canonical", "coverage must be exactly deriveCoverage(document): tier order, no empty rows");
    }

    return list;
}

// ---------------------------------------------------------------------------
// Authority documents
// ---------------------------------------------------------------------------

/** Cross-field rules for a mandate. */
export function checkMandate(m: Mandate): CheckIssue[] {
    const { list, add } = issues();
    scan(m, add);
    const hash = safeHash(m);
    if (hash !== undefined && hash !== m.hash) add(["hash"], "hash_mismatch", "hash does not match the canonical content");
    if (m.expires_at <= m.issued_at) add(["expires_at"], "time_order", "expires_at must be after issued_at");
    if (m.per_call_cap_minor !== undefined && m.per_call_cap_minor > m.spend_cap_minor) add(["per_call_cap_minor"], "cap_order", "per-call cap exceeds the spend cap");
    duplicates(m.scope, (s) => `${s.kind}:${s.id}`).forEach((i) => add(["scope", i], "duplicate_id", "duplicate scope entry"));
    duplicates(m.merchants, (h) => h).forEach((i) => add(["merchants", i], "duplicate_id", "duplicate merchant"));
    return list;
}

/** Cross-field rules for a confirmation request. */
export function checkConfirmationRequest(c: ConfirmationRequest): CheckIssue[] {
    const { list, add } = issues();
    scan(c, add);
    if (c.expires_at <= c.issued_at) add(["expires_at"], "time_order", "expires_at must be after issued_at");
    if (c.effect === "financial") {
        let sum = 0;
        c.items.forEach((it, i) => {
            if (it.amount.currency !== c.total.currency) add(["items", i, "amount", "currency"], "currency_mismatch", "item currency differs from total currency");
            sum += it.amount.amount_minor * it.quantity;
        });
        if (sum !== c.total.amount_minor) add(["total", "amount_minor"], "total_mismatch", `items sum to ${sum} but the total is ${c.total.amount_minor}`);
    }
    return list;
}

/** Cross-field rules for a confirmation token (timestamps and string well-formedness). */
export function checkConfirmationToken(t: ConfirmationToken): CheckIssue[] {
    const { list, add } = issues();
    scan(t, add);
    return list;
}

/** Cross-field rules for an operation result (string well-formedness, page text in surface fields). */
export function checkOperationResult(r: OperationResult): CheckIssue[] {
    const { list, add } = issues();
    scan(r, add);
    return list;
}

/** Cross-field rules for a receipt. */
export function checkReceipt(r: Receipt): CheckIssue[] {
    const { list, add } = issues();
    scan(r, add);
    if (r.mode === "read-only") {
        if (r.effect !== "read") add(["effect"], "mode_forbids_effect", "read-only mode records reads only");
        if (r.tier !== "T0" && r.tier !== "T1") add(["tier"], "mode_forbids_tier", "read-only mode records T0/T1 only");
    }
    if (r.effect !== "read" && r.tier !== "T2") add(["tier"], "receipt_tier_effect", "a write is a T2 network call");
    if (r.amount !== undefined && r.effect !== "financial") add(["amount"], "amount_not_financial", "only a financial call records an amount");
    // A call that ran (ok) or failed after being sent (error) must be reconcilable: a timed-out payment is exactly
    // the case that needs its key, mandate and confirmation. Refusals (blocked, requires_*) never reached the site.
    if (r.result === "ok" || r.result === "error") {
        const need: ("amount" | "confirmation" | "mandate" | "idempotency_key")[] = [];
        if (r.effect === "financial") need.push("confirmation", "mandate", "idempotency_key", ...(r.result === "ok" ? (["amount"] as const) : []));
        else if (requiresConfirmation(r.effect)) need.push("confirmation");
        for (const f of need) if (r[f] === undefined) add([f], "receipt_incomplete", `a ${r.result} ${r.effect} call records ${f}`);
    }
    return list;
}

// ---------------------------------------------------------------------------
// Validation entry point
// ---------------------------------------------------------------------------

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
