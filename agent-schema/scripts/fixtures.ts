// Source of truth for fixtures/. `bun scripts/build-fixtures.ts` writes the JSON files;
// a test fails if the committed files differ from this output. Valid fixtures are bare
// documents (the Rust side validates its emit against the same files). Every invalid
// fixture sits beside a `.expect.json` naming the phase, the issue path and the code.
import type { Action, AgentSurface, ConfirmationRequest, ConfirmationToken, Flow, Mandate, OperationResult, Receipt } from "../src/index";
import { deriveCoverage, sha256Hex, withHash } from "../src/index";

const h = (label: string) => `sha256:${sha256Hex(label)}` as const;
const nid = (label: string) => `n_${sha256Hex(label).slice(0, 16)}` as const;
const page = (value: string) => ({ origin: "page", value }) as const;
const surf = (value: string) => ({ origin: "surface", value }) as const;
const T = "2026-11-10T12:00:00Z";

const N = {
    main: nid("home/main"),
    h1: nid("home/h1"),
    link: nid("home/link-mug"),
    pName: nid("product/h1"),
    pAdd: nid("product/add"),
    pTab: nid("product/tab-details"),
    pPanel: nid("product/panel-details"),
    pCanvas: nid("product/canvas-viewer"),
    cart: nid("cart/h1"),
    co: nid("checkout/h1"),
    csrf: nid("product/csrf-meta"),
};

const nav: Action = {
    id: "nav-mug",
    kind: "navigate",
    tier: "T1",
    effect: "read",
    node: N.link,
    label: page("Ceramic mug"),
    description: surf("Open the product page."),
    inputs: [],
    auth: { kind: "none" },
    preconditions: [],
    postconditions: [{ type: "navigates_to", route: "product" }],
    idempotency: { idempotent: true },
    state: "enabled",
    target: page("/products/mug"),
    route: "product",
    evidence: [{ source: "declared", type: "anchor-href", ref: "route:home#link-mug", hash: h("ev:nav-mug") }],
};

const tab: Action = {
    id: "show-details",
    kind: "state",
    tier: "T3",
    effect: "read",
    node: N.pTab,
    label: page("Details"),
    description: surf("Show the details panel."),
    inputs: [],
    auth: { kind: "none" },
    preconditions: [],
    postconditions: [{ type: "node_present", node: N.pPanel }],
    idempotency: { idempotent: true },
    state: "enabled",
    affects: [N.pPanel],
    evidence: [{ source: "observed", type: "interaction-trace", ref: "trace:3", hash: h("ev:tab") }],
};

const addToCart: Action = {
    id: "add-to-cart",
    kind: "submit",
    tier: "T2",
    effect: "write-reversible",
    node: N.pAdd,
    label: page("Add to cart"),
    description: surf("Add the product to the cart."),
    inputs: [{ name: "quantity", type: "integer", required: true, sensitivity: "none", min: 1, max: 10 }],
    auth: { kind: "session-cookie" },
    preconditions: [
        { type: "authenticated" },
        {
            type: "token",
            into: { in: "header", name: "x-csrf-token" },
            source: { source: { kind: "dom", step: "add-item", node: N.csrf, attribute: "content" }, match: "exact" },
        },
    ],
    postconditions: [
        { type: "response_class", class: "2xx" },
        { type: "response_field_equals_input", at: "/items/0/quantity", input: "quantity" },
    ],
    idempotency: { idempotent: false },
    state: "enabled",
    request: {
        id: "add-item-req",
        method: "POST",
        url: page("/api/cart/items"),
        body: "json",
        values: [
            { at: { in: "body-json", name: "/sku" }, value: { class: "constant", value: page("mug-01") } },
            { at: { in: "body-json", name: "/quantity" }, value: { class: "input", input: "quantity" } },
        ],
        expect: "2xx",
        recorded: h("rec:add-item"),
    },
    evidence: [
        { source: "observed", type: "http-exchange", ref: "capture:net/12", hash: h("ev:add-item") },
        { source: "derived", type: "handler-analysis", ref: "js:onAddToCart", hash: h("ev:add-item-js") },
    ],
};

const flow: Flow = {
    id: "checkout",
    status: "ready",
    description: surf("Add a mug, set shipping, and pay with a caller-supplied payment token."),
    inputs: [
        { name: "quantity", type: "integer", required: true, sensitivity: "none", min: 1, max: 10 },
        { name: "address_line1", type: "string", required: true, sensitivity: "personal", max_length: 120 },
        { name: "payment_token", type: "string", required: true, sensitivity: "payment-token" },
        { name: "idempotency_key", type: "string", required: true, sensitivity: "none" },
    ],
    auth: { kind: "session-cookie" },
    effect: "financial",
    recorded: { environment: "payment-test-mode", hash: h("rec:checkout") },
    evidence: [
        { source: "observed", type: "http-exchange", ref: "capture:net/12-31", hash: h("ev:checkout") },
        { source: "derived", type: "handler-analysis", ref: "js:checkout", hash: h("ev:checkout-js") },
    ],
    steps: [
        {
            id: "add-item",
            action: "add-to-cart",
            effect: "write-reversible",
            human_gates: [],
            postconditions: [{ type: "response_class", class: "2xx" }],
            requests: [
                {
                    id: "add-item-req",
                    method: "POST",
                    url: page("/api/cart/items"),
                    body: "json",
                    values: [
                        {
                            at: { in: "header", name: "x-csrf-token" },
                            value: { class: "derived", edge: { source: { kind: "dom", step: "add-item", node: N.csrf, attribute: "content" }, match: "exact" } },
                        },
                        { at: { in: "body-json", name: "/sku" }, value: { class: "constant", value: page("mug-01") } },
                        { at: { in: "body-json", name: "/quantity" }, value: { class: "input", input: "quantity" } },
                    ],
                    expect: "2xx",
                    recorded: h("rec:add-item"),
                },
            ],
        },
        {
            id: "set-shipping",
            effect: "write-reversible",
            human_gates: [],
            postconditions: [{ type: "response_class", class: "2xx" }],
            requests: [
                {
                    id: "shipping-req",
                    method: "PUT",
                    url: page("/api/cart/{cart_id}/shipping"),
                    body: "json",
                    values: [
                        {
                            at: { in: "path", name: "cart_id" },
                            value: { class: "derived", edge: { source: { kind: "response", step: "add-item", request: "add-item-req", at: { in: "body-json", name: "/cart_id" } }, match: "json-path" } },
                        },
                        { at: { in: "body-json", name: "/line1" }, value: { class: "input", input: "address_line1" } },
                    ],
                    expect: "2xx",
                    recorded: h("rec:shipping"),
                },
            ],
        },
        {
            id: "create-intent",
            effect: "write-reversible",
            human_gates: [],
            postconditions: [{ type: "response_class", class: "2xx" }],
            requests: [
                {
                    id: "intent-req",
                    method: "POST",
                    url: page("/api/payment-intents"),
                    body: "json",
                    values: [
                        {
                            at: { in: "body-json", name: "/cart_id" },
                            value: { class: "derived", edge: { source: { kind: "response", step: "add-item", request: "add-item-req", at: { in: "body-json", name: "/cart_id" } }, match: "json-path" } },
                        },
                        {
                            at: { in: "header", name: "x-signature" },
                            value: {
                                class: "client-computed",
                                resolution: {
                                    kind: "lift",
                                    spec: h("lift:sign"),
                                    args: [{ source: { kind: "response", step: "set-shipping", request: "shipping-req", at: { in: "body-json", name: "/total" } }, match: "json-path" }],
                                },
                            },
                        },
                    ],
                    expect: "2xx",
                    recorded: h("rec:intent"),
                },
            ],
        },
        {
            id: "pay",
            effect: "financial",
            payment: "caller-token",
            human_gates: ["3ds", "fraud_challenge"],
            postconditions: [{ type: "response_class", class: "2xx" }],
            requests: [
                {
                    id: "confirm-req",
                    method: "POST",
                    url: page("/api/payment-intents/{intent_id}/confirm"),
                    body: "json",
                    values: [
                        {
                            at: { in: "path", name: "intent_id" },
                            value: { class: "derived", edge: { source: { kind: "response", step: "create-intent", request: "intent-req", at: { in: "body-json", name: "/id" } }, match: "json-path" } },
                        },
                        { at: { in: "body-json", name: "/payment_token" }, value: { class: "input", input: "payment_token" } },
                        { at: { in: "header", name: "idempotency-key" }, value: { class: "input", input: "idempotency_key" } },
                    ],
                    expect: "2xx",
                    recorded: h("rec:confirm"),
                },
            ],
        },
    ],
};

const shopBase: Omit<AgentSurface, "hash"> = {
    kind: "agent-surface",
    schema_version: "1.0.0",
    capture: { hash: h("capture:shop"), captured_at: T, tool: { name: "mochi", version: "0.1.0" } },
    mode: "owner",
    site: { host: "shop.example.test", scheme: "https" },
    gates: { robots_txt: "allowed", llms_txt: "absent", tos: "permitted" },
    options: { include_hidden: false },
    coverage: { tiers: [], unsupported_reasons: [] },
    routes: [
        {
            id: "home",
            url: page("/"),
            title: page("Mug shop"),
            tiers: ["T0", "T1"],
            nodes: [
                { id: N.main, role: "main", visibility: "visible" },
                { id: N.h1, parent: N.main, role: "heading", level: 1, name: page("Mug shop"), text: page("Mug shop"), visibility: "visible" },
                { id: N.link, parent: N.main, role: "link", name: page("Ceramic mug"), href: page("/products/mug"), visibility: "visible" },
            ],
        },
        {
            id: "product",
            url: page("/products/mug"),
            canonical: page("https://shop.example.test/products/mug"),
            tiers: ["T0", "T2", "T3"],
            nodes: [
                { id: N.pName, role: "heading", level: 1, name: page("Ceramic mug"), text: page("Ceramic mug"), visibility: "visible" },
                { id: N.pAdd, role: "button", name: page("Add to cart"), visibility: "visible" },
                { id: N.pTab, role: "tab", name: page("Details"), visibility: "visible" },
                { id: N.pPanel, role: "tabpanel", text: page("Dishwasher safe. 350 ml."), visibility: "visible" },
                { id: N.pCanvas, role: "img", name: page("3D viewer"), visibility: "visible", confidence: 40 },
                { id: N.csrf, role: "generic", visibility: "visible" },
            ],
        },
        { id: "cart", url: page("/cart"), tiers: ["T0", "T2"], nodes: [{ id: N.cart, role: "heading", level: 1, name: page("Cart"), text: page("Cart"), visibility: "visible" }] },
        { id: "checkout", url: page("/checkout"), tiers: ["T0"], nodes: [{ id: N.co, role: "heading", level: 1, name: page("Checkout"), text: page("Checkout"), visibility: "visible" }] },
    ],
    actions: [nav, addToCart, tab],
    flows: [flow],
    unsupported: [
        { id: "viewer-3d", node: N.pCanvas, label: page("3D viewer"), tier: "T4", reason: "opaque-canvas", note: surf("Canvas viewer is described, not operated.") },
    ],
};

const finish = (d: AgentSurface): AgentSurface => withHash({ ...d, coverage: deriveCoverage(d) });
export const shop: AgentSurface = finish(shopBase as AgentSurface);

export const readOnly: AgentSurface = finish({
    kind: "agent-surface",
    schema_version: "1.0.0",
    hash: h("placeholder"),
    capture: { hash: h("capture:docs"), captured_at: T, tool: { name: "mochi", version: "0.1.0" } },
    mode: "read-only",
    site: { host: "docs.example.test", scheme: "https" },
    gates: { robots_txt: "allowed", llms_txt: "present", tos: "not-checked" },
    options: { include_hidden: false },
    coverage: { tiers: [], unsupported_reasons: [] },
    routes: [
        {
            id: "index",
            url: page("/"),
            tiers: ["T0", "T1"],
            nodes: [
                { id: nid("docs/h1"), role: "heading", level: 1, name: page("Docs"), text: page("Docs"), visibility: "visible" },
                { id: nid("docs/link"), role: "link", name: page("Guide"), href: page("/guide"), visibility: "visible", confidence: 80 },
            ],
        },
    ],
    actions: [
        {
            id: "open-guide",
            kind: "navigate",
            tier: "T1",
            effect: "read",
            node: nid("docs/link"),
            description: surf("Open the guide."),
            inputs: [],
            auth: { kind: "none" },
            preconditions: [],
            postconditions: [],
            idempotency: { idempotent: true },
            state: "enabled",
            target: page("/guide"),
            confidence: 80,
            evidence: [{ source: "inferred", type: "heuristic", ref: "role:link", hash: h("ev:guide") }],
        },
    ],
    flows: [],
    unsupported: [],
});

const mandate: Mandate = withHash({
    id: "mandate-1",
    grantor: "grantor_7f3a",
    mode: "delegated",
    merchants: ["shop.example.test"],
    currency: "USD",
    spend_cap_minor: 5000,
    per_call_cap_minor: 2500,
    scope: [{ kind: "flow", id: "checkout" }],
    issued_at: T,
    expires_at: "2026-11-10T13:00:00Z",
    hash: h("placeholder"),
});

const confirmationFinancial: ConfirmationRequest = {
    id: "confirm-1",
    action: "add-to-cart",
    flow: "checkout",
    step: "pay",
    effect: "financial",
    request_digest: h("req:pay"),
    idempotency_key: "idem_9f1c2b7a4d5e6f80",
    issued_at: T,
    expires_at: "2026-11-10T12:05:00Z",
    items: [{ name: page("Ceramic mug"), quantity: 2, amount: { amount_minor: 1800, currency: "USD" } }],
    total: { amount_minor: 3600, currency: "USD" },
    payee: page("shop.example.test"),
    mandate: "mandate-1",
};

const confirmationIrreversible: ConfirmationRequest = {
    id: "confirm-2",
    action: "add-to-cart",
    effect: "account-control",
    request_digest: h("req:delete-account"),
    idempotency_key: "idem_0a1b2c3d4e5f6071",
    issued_at: T,
    expires_at: "2026-11-10T12:05:00Z",
    summary: surf("Permanently close the account."),
    page_context: page("Close my account"),
};

const token: ConfirmationToken = {
    confirmation: "confirm-1",
    request_digest: h("req:pay"),
    idempotency_key: "idem_9f1c2b7a4d5e6f80",
    granted_at: "2026-11-10T12:01:00Z",
};

const receipt: Receipt = {
    id: "receipt-1",
    at: "2026-11-10T12:02:00Z",
    mode: "delegated",
    tier: "T2",
    effect: "financial",
    action: "add-to-cart",
    flow: "checkout",
    step: "pay",
    evidence: [h("ev:checkout")],
    request: { method: "POST", host: "shop.example.test", path: page("/api/payment-intents/{intent_id}/confirm") },
    result: "ok",
    response_class: "2xx",
    duration_ms: 412,
    mandate: "mandate-1",
    confirmation: "confirm-1",
    idempotency_key: "idem_9f1c2b7a4d5e6f80",
    amount: { amount_minor: 3600, currency: "USD" },
    redactions: 3,
};

const results: Record<string, OperationResult> = {
    ok: { status: "ok", tier: "T2", fields: { order_id: { origin: "page", value: "A-1001" }, note: { origin: "surface", value: "Order placed." } }, receipt: "receipt-1" },
    stale: { status: "stale", action: "add-to-cart", reason: surf("Signature changed since capture; action disabled until re-verified.") },
    unsupported: { status: "unsupported", reason: "opaque-canvas", message: surf("Canvas viewer is not operable.") },
    "requires-auth": { status: "requires_auth", action: "add-to-cart", requirement: "session-cookie" },
    "requires-confirmation": { status: "requires_confirmation", action: "add-to-cart", effect: "financial", confirmation: "confirm-1" },
    "requires-human": { status: "requires_human", action: "add-to-cart", step: "pay", reason: "3ds", continue_at: page("https://pay.example.test/3ds/abc") },
    "requires-browser": { status: "requires_browser", step: "create-intent", reason: "signature" },
    blocked: { status: "blocked", reason: "mode_forbids_effect", mode: "read-only", message: surf("read-only mode refuses writes.") },
    error: { status: "error", class: "postcondition_failed", step: "add-item" },
};

export const validFixtures: Record<string, unknown> = {
    "surface.shop": shop,
    "surface.read-only": readOnly,
    mandate: mandate,
    "confirmation-request.financial": confirmationFinancial,
    "confirmation-request.irreversible": confirmationIrreversible,
    "confirmation-token": token,
    receipt: receipt,
    ...Object.fromEntries(Object.entries(results).map(([k, v]) => [`result.${k}`, v])),
};

// ---------------------------------------------------------------------------
// Invalid fixtures: a mutation of a valid document plus the exact expected issues.
// `phase: "schema"` fixtures fail Zod parsing; `phase: "check"` fixtures parse but
// fail `checkSurface` (hash fixed up first, so only the stated reason fails).
// ---------------------------------------------------------------------------

export interface Expect {
    schema: "surface" | "result" | "mandate" | "confirmation-request" | "confirmation-token" | "receipt";
    phase: "schema" | "check";
    issues: { path: (string | number)[]; code: string }[];
    /** Why this is invalid, in words. */
    why: string;
}

type Doc = any;
const clone = <T>(x: T): Doc => JSON.parse(JSON.stringify(x));
const rehash = (d: Doc): Doc => withHash(d);

export const invalidFixtures: Record<string, { doc: unknown; expect: Expect }> = (() => {
    const out: Record<string, { doc: unknown; expect: Expect }> = {};
    const add = (name: string, doc: unknown, expect: Expect) => (out[name] = { doc, expect });
    const s = () => clone(shop);
    const ro = () => clone(readOnly);

    // ---- schema phase -------------------------------------------------------
    {
        const d = s();
        d.actions[1].evidence = [{ source: "inferred", type: "heuristic", ref: "role:button", hash: h("x") }];
        add("action-submit-inferred-evidence-only", d, { schema: "surface", phase: "schema", issues: [{ path: ["actions", 1, "evidence", 0, "source"], code: "invalid_value" }], why: "a non-navigate action must rest on declared or observed evidence first (D5)" });
    }
    {
        const d = s();
        d.actions[1].evidence = [];
        add("action-submit-no-evidence", d, { schema: "surface", phase: "schema", issues: [{ path: ["actions", 1, "evidence", 0], code: "invalid_type" }], why: "every non-navigate action lists at least one declared or observed evidence" });
    }
    {
        const d = s();
        d.actions[0].tier = "T2";
        add("action-navigate-wrong-tier", d, { schema: "surface", phase: "schema", issues: [{ path: ["actions", 0, "tier"], code: "invalid_value" }], why: "a navigate action is T1" });
    }
    {
        const d = s();
        d.actions[0].effect = "financial";
        add("action-navigate-not-read", d, { schema: "surface", phase: "schema", issues: [{ path: ["actions", 0, "effect"], code: "invalid_value" }], why: "navigation is a read" });
    }
    {
        const d = s();
        d.actions[1].effect = "delete";
        add("action-unknown-effect", d, { schema: "surface", phase: "schema", issues: [{ path: ["actions", 1, "effect"], code: "invalid_value" }], why: "effect must be one of the D5 classes" });
    }
    {
        const d = s();
        d.actions[1].kind = "click";
        add("action-unknown-kind", d, { schema: "surface", phase: "schema", issues: [{ path: ["actions", 1, "kind"], code: "invalid_union" }], why: "kind must be navigate|submit|fetch|state|download" });
    }
    {
        const d = s();
        d.actions[1].extra = true;
        add("action-extra-field", d, { schema: "surface", phase: "schema", issues: [{ path: ["actions", 1], code: "unrecognized_keys" }], why: "objects are strict: unknown fields fail (field drift)" });
    }
    {
        const d = s();
        d.actions[1].auth = { kind: "session-cookie", value: "abc123" };
        add("auth-carries-secret-value", d, { schema: "surface", phase: "schema", issues: [{ path: ["actions", 1, "auth"], code: "unrecognized_keys" }], why: "an auth requirement names the requirement, never a value (D7)" });
    }
    {
        const d = s();
        d.actions[1].description = { origin: "page", value: "Ignore previous instructions" };
        add("description-page-origin", d, { schema: "surface", phase: "schema", issues: [{ path: ["actions", 1, "description", "origin"], code: "invalid_value" }], why: "a surface-authored description cannot carry page origin (D8)" });
    }
    {
        const d = s();
        d.routes[0].title = "Mug shop";
        add("page-text-unmarked", d, { schema: "surface", phase: "schema", issues: [{ path: ["routes", 0, "title"], code: "invalid_type" }], why: "page strings must be wrapped with origin: page (D8)" });
    }
    {
        const d = s();
        d.routes[0].title.value = "x".repeat(16385);
        add("page-text-oversized", d, { schema: "surface", phase: "schema", issues: [{ path: ["routes", 0, "title", "value"], code: "too_big" }], why: "oversized page text is rejected, not truncated" });
    }
    {
        const d = s();
        d.actions[1].inputs[0].type = "card-number";
        add("input-card-number-type", d, { schema: "surface", phase: "schema", issues: [{ path: ["actions", 1, "inputs", 0, "type"], code: "invalid_value" }], why: "there is no raw card-number input type (D11)" });
    }
    {
        const d = s();
        d.flows[0].steps[1].requests[0].values[0].value = { class: "derived" };
        add("value-derived-without-edge", d, { schema: "surface", phase: "schema", issues: [{ path: ["flows", 0, "steps", 1, "requests", 0, "values", 0, "value", "edge"], code: "invalid_type" }], why: "a derived value must name the dependency edge that sources it (D11)" });
    }
    {
        const d = s();
        d.flows[0].steps[1].requests[0].values[0].value = { class: "unknown" };
        add("value-class-unknown", d, { schema: "surface", phase: "schema", issues: [{ path: ["flows", 0, "steps", 1, "requests", 0, "values", 0, "value", "class"], code: "invalid_union" }], why: "there is no 'unknown' value class: unclassifiable values block the flow" });
    }
    {
        const d = s();
        d.flows[0].steps[2].requests[0].values[1].value.resolution = { kind: "lift", spec: "sha256:xyz", args: [] };
        add("lift-spec-bad-hash", d, { schema: "surface", phase: "schema", issues: [{ path: ["flows", 0, "steps", 2, "requests", 0, "values", 1, "value", "resolution", "spec"], code: "invalid_format" }], why: "a lift spec is pinned by a sha256 hash" });
    }
    {
        const d = s();
        d.flows[0].status = "blocked";
        add("flow-blocked-without-unclassified", d, { schema: "surface", phase: "schema", issues: [{ path: ["flows", 0, "unclassified"], code: "invalid_type" }], why: "a blocked flow lists the values that blocked it" });
    }
    {
        const d = s();
        d.flows[0].unclassified = [];
        add("flow-ready-with-unclassified", d, { schema: "surface", phase: "schema", issues: [{ path: ["flows", 0], code: "unrecognized_keys" }], why: "a ready flow has no unclassified values" });
    }
    {
        const d = s();
        d.flows[0].steps = [];
        add("flow-no-steps", d, { schema: "surface", phase: "schema", issues: [{ path: ["flows", 0, "steps"], code: "too_small" }], why: "a flow has at least one step" });
    }
    {
        const d = s();
        d.flows[0].steps[3].payment = "card-number";
        add("flow-payment-method-unknown", d, { schema: "surface", phase: "schema", issues: [{ path: ["flows", 0, "steps", 3, "payment"], code: "invalid_value" }], why: "payment is caller-token, hosted-session or agent-protocol (D11)" });
    }
    {
        const d = s();
        d.hash = "sha256:ABC";
        add("hash-malformed", d, { schema: "surface", phase: "schema", issues: [{ path: ["hash"], code: "invalid_format" }], why: "hash is sha256: plus 64 lowercase hex" });
    }
    {
        const d = s();
        d.mode = "admin";
        add("mode-unknown", d, { schema: "surface", phase: "schema", issues: [{ path: ["mode"], code: "invalid_value" }], why: "mode is owner, delegated or read-only (D2)" });
    }
    {
        const d = s();
        delete d.capture;
        add("capture-pin-missing", d, { schema: "surface", phase: "schema", issues: [{ path: ["capture"], code: "invalid_type" }], why: "the surface is pinned to the capture it came from" });
    }
    {
        const d = s();
        d.routes[0].nodes[0].id = "n_123";
        add("node-id-malformed", d, { schema: "surface", phase: "schema", issues: [{ path: ["routes", 0, "nodes", 0, "id"], code: "invalid_format" }], why: "node ids are n_ plus 16 lowercase hex" });
    }
    {
        const d = s();
        d.schema_version = "1.0";
        add("schema-version-malformed", d, { schema: "surface", phase: "schema", issues: [{ path: ["schema_version"], code: "invalid_format" }], why: "schema_version is MAJOR.MINOR.PATCH" });
    }
    {
        const d = s();
        d.routes[0].nodes[2].confidence = 1.5;
        add("confidence-float", d, { schema: "surface", phase: "schema", issues: [{ path: ["routes", 0, "nodes", 2, "confidence"], code: "invalid_type" }], why: "confidence is an integer percentage; the wire format has no floats" });
    }
    add("result-unknown-status", { status: "maybe" }, { schema: "result", phase: "schema", issues: [{ path: ["status"], code: "invalid_union" }], why: "status is one of the D10 statuses" });
    add("result-requires-human-no-reason", { status: "requires_human" }, { schema: "result", phase: "schema", issues: [{ path: ["reason"], code: "invalid_value" }], why: "requires_human names the challenge" });
    add("result-ok-unmarked-field", { status: "ok", tier: "T2", fields: { order_id: "A-1001" } }, { schema: "result", phase: "schema", issues: [{ path: ["fields", "order_id"], code: "invalid_type" }], why: "result values carry their origin (D8)" });
    {
        const d = clone(validFixtures["mandate"]);
        d.merchants = [];
        add("mandate-no-merchants", d, { schema: "mandate", phase: "schema", issues: [{ path: ["merchants"], code: "too_small" }], why: "a mandate is tied to a merchant allowlist" });
    }
    {
        const d = clone(validFixtures["mandate"]);
        d.spend_cap_minor = 50.5;
        add("mandate-float-cap", d, { schema: "mandate", phase: "schema", issues: [{ path: ["spend_cap_minor"], code: "invalid_type" }], why: "money is integer minor units" });
    }
    {
        const d = clone(validFixtures["mandate"]);
        d.card_number = "4242424242424242";
        add("mandate-carries-card-number", d, { schema: "mandate", phase: "schema", issues: [{ path: [], code: "unrecognized_keys" }], why: "no secret or instrument value anywhere in a mandate (D7, D11)" });
    }
    {
        const d = clone(validFixtures["confirmation-request.financial"]);
        delete d.total;
        add("confirmation-financial-no-total", d, { schema: "confirmation-request", phase: "schema", issues: [{ path: ["total"], code: "invalid_type" }], why: "a money confirmation shows the total (D11)" });
    }
    {
        const d = clone(validFixtures["confirmation-request.financial"]);
        delete d.payee;
        add("confirmation-financial-no-payee", d, { schema: "confirmation-request", phase: "schema", issues: [{ path: ["payee"], code: "invalid_type" }], why: "a money confirmation shows the payee (D11)" });
    }
    {
        const d = clone(validFixtures["confirmation-request.financial"]);
        d.effect = "read";
        add("confirmation-effect-read", d, { schema: "confirmation-request", phase: "schema", issues: [{ path: ["effect"], code: "invalid_union" }], why: "plain reads need no confirmation" });
    }
    {
        const d = clone(validFixtures["confirmation-token"]);
        d.secret = "hunter2";
        add("confirmation-token-extra-secret", d, { schema: "confirmation-token", phase: "schema", issues: [{ path: [], code: "unrecognized_keys" }], why: "a confirmation token carries no secret" });
    }
    {
        const d = clone(validFixtures["receipt"]);
        d.request.path = "/api/pay?card=4242";
        add("receipt-unmarked-path", d, { schema: "receipt", phase: "schema", issues: [{ path: ["request", "path"], code: "invalid_type" }], why: "receipt request class is marked and carries a path template only" });
    }
    {
        const d = clone(validFixtures["receipt"]);
        d.result = "success";
        add("receipt-unknown-result", d, { schema: "receipt", phase: "schema", issues: [{ path: ["result"], code: "invalid_value" }], why: "a receipt result is a D10 status" });
    }

    // ---- check phase (parses, then fails an invariant the wire cannot express) ----
    add("check-hash-mismatch", (() => { const d = s(); d.site.host = "other.example.test"; return d; })(), { schema: "surface", phase: "check", issues: [{ path: ["hash"], code: "hash_mismatch" }], why: "content changed without recomputing the hash" });
    {
        const d = s();
        d.routes[3].id = "cart";
        add("check-duplicate-route-id", rehash(d), { schema: "surface", phase: "check", issues: [{ path: ["routes", 3, "id"], code: "duplicate_id" }], why: "ids are unique within the document" });
    }
    {
        const d = s();
        d.actions[0].route = "nowhere";
        add("check-dangling-route", rehash(d), { schema: "surface", phase: "check", issues: [{ path: ["actions", 0, "route"], code: "dangling_reference" }], why: "a navigate action points at a route that exists" });
    }
    {
        const d = s();
        d.routes[0].nodes[2].visibility = "hidden";
        add("check-hidden-node-not-opted-in", rehash(d), { schema: "surface", phase: "check", issues: [{ path: ["routes", 0, "nodes", 2, "visibility"], code: "hidden_not_opted_in" }], why: "hidden and inert nodes are excluded unless the document opts in (D8)" });
    }
    {
        const d = s();
        d.coverage.tiers[1].actions = 7;
        add("check-coverage-mismatch", rehash(d), { schema: "surface", phase: "check", issues: [{ path: ["coverage", "tiers", 1, "actions"], code: "coverage_mismatch" }], why: "coverage equals what the document contains" });
    }
    {
        const d = s();
        d.flows[0].effect = "read";
        add("check-flow-effect-below-step", rehash(d), { schema: "surface", phase: "check", issues: [{ path: ["flows", 0, "effect"], code: "flow_effect_below_step" }], why: "a flow is at least as consequential as its most consequential step" });
    }
    {
        const d = s();
        d.flows[0].steps[3].payment = undefined;
        delete d.flows[0].steps[3].payment;
        add("check-financial-step-without-payment", rehash(d), { schema: "surface", phase: "check", issues: [{ path: ["flows", 0, "steps", 3, "effect"], code: "payment_effect_mismatch" }], why: "the step that moves money declares how it is paid (D11)" });
    }
    {
        const d = s();
        d.flows[0].steps[1].requests[0].values[0].value.edge.source.step = "pay";
        d.flows[0].steps[1].requests[0].values[0].value.edge.source.request = "confirm-req";
        add("check-dependency-from-later-step", rehash(d), { schema: "surface", phase: "check", issues: [{ path: ["flows", 0, "steps", 1, "requests", 0, "values", 0, "value", "edge", "source", "step"], code: "dependency_not_earlier" }], why: "a derived value is sourced from the same or an earlier step" });
    }
    {
        const d = s();
        d.flows[0].steps[1].requests[0].values[0].value.edge.source.request = "ghost-req";
        add("check-dependency-unknown-request", rehash(d), { schema: "surface", phase: "check", issues: [{ path: ["flows", 0, "steps", 1, "requests", 0, "values", 0, "value", "edge", "source", "request"], code: "dangling_reference" }], why: "the dependency names a request that exists in the source step" });
    }
    {
        const d = s();
        d.flows[0].steps[0].requests[0].values[2].value.input = "ghost_input";
        add("check-undeclared-input", rehash(d), { schema: "surface", phase: "check", issues: [{ path: ["flows", 0, "steps", 0, "requests", 0, "values", 2, "value", "input"], code: "undeclared_input" }], why: "an input value refers to a declared input" });
    }
    {
        const d = ro();
        d.mode = "read-only";
        d.actions[0] = { ...clone(shop.actions[2]), id: "open-guide" };
        d.actions[0].node = d.routes[0].nodes[1].id;
        d.actions[0].affects = [d.routes[0].nodes[0].id];
        d.actions[0].postconditions = [];
        d.coverage = deriveCoverage(d);
        add("check-read-only-carries-t3", rehash(d), { schema: "surface", phase: "check", issues: [{ path: ["actions", 0, "tier"], code: "mode_forbids_tier" }], why: "read-only surfaces carry T0/T1 only (D2)" });
    }
    {
        const d = clone(validFixtures["mandate"]);
        d.expires_at = d.issued_at;
        add("check-mandate-expiry-not-after-issue", rehash(d), { schema: "mandate", phase: "check", issues: [{ path: ["expires_at"], code: "time_order" }], why: "a mandate expires after it is issued" });
    }
    {
        const d = clone(validFixtures["mandate"]);
        d.per_call_cap_minor = 9999;
        add("check-mandate-per-call-over-cap", rehash(d), { schema: "mandate", phase: "check", issues: [{ path: ["per_call_cap_minor"], code: "cap_order" }], why: "the per-call cap cannot exceed the spend cap" });
    }
    return out;
})();
