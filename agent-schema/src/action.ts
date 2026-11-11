// D5: an action is typed, classed and guarded. D1: its tier follows its kind.
import { z } from "zod";
import { registry } from "./registry";
import { PageText, SurfaceText } from "./origin";
import { ActionId, Code, InputName, Name, NodeId, Percent, RouteId, Sha256 } from "./primitives";
import { Effect, EvidenceSource, Tier } from "./policy";
import { UnsupportedReason } from "./result";
import { PaymentMethod, RequestLocation, RequestSpec, ResponseClass, DependencyEdge } from "./values";

/** One piece of evidence behind a claim (D1, section 3). `hash` pins the artifact it came from. */
export const Evidence = z
    .strictObject({
        source: EvidenceSource,
        /** What kind of artifact: `form-action`, `http-exchange`, `interaction-trace`, `handler-analysis`, `heuristic`... */
        type: Code,
        /** Pointer into the capture (an id or JSON Pointer). Authored by the emitter, not by the page. */
        ref: Name,
        hash: Sha256,
    })
    .register(registry, { id: "Evidence" });
export type Evidence = z.infer<typeof Evidence>;

/** Evidence trusted enough to ground an action: declared by the site, or observed (D5: >= observed). */
export const StrongEvidence = z
    .strictObject({
        source: z.enum(["declared", "observed"]),
        type: Code,
        ref: Name,
        hash: Sha256,
    })
    .register(registry, { id: "StrongEvidence" });
export type StrongEvidence = z.infer<typeof StrongEvidence>;

/** D7: names what is needed, never a value. */
export const AuthRequirement = z
    .discriminatedUnion("kind", [
        z.strictObject({ kind: z.literal("none") }),
        z.strictObject({ kind: z.literal("session-cookie") }),
        z.strictObject({ kind: z.literal("oauth"), scopes: z.array(Name).max(64) }),
        z.strictObject({ kind: z.literal("api-key"), header: Name }),
    ])
    .register(registry, { id: "AuthRequirement" });
export type AuthRequirement = z.infer<typeof AuthRequirement>;

/** How sensitive an input is. `credential` and `payment-token` are supplied at run time and redacted everywhere. */
export const Sensitivity = z.enum(["none", "personal", "credential", "payment-token"]).register(registry, { id: "Sensitivity" });

/** Types an input can take. There is deliberately no raw card-number type (D11): card data never passes through the surface. */
export const InputType = z
    .enum(["string", "number", "integer", "boolean", "enum", "email", "tel", "url", "date", "password", "file"])
    .register(registry, { id: "InputType" });

/** A typed input field with constraints taken from HTML validation, a schema or handler analysis. */
export const Input = z
    .strictObject({
        name: InputName,
        type: InputType,
        required: z.boolean(),
        sensitivity: Sensitivity,
        label: PageText.optional(),
        /** Allowed values for `enum`. */
        options: z.array(z.strictObject({ value: PageText, label: PageText.optional() })).max(512).optional(),
        min: z.int().optional(),
        max: z.int().optional(),
        min_length: z.int().min(0).optional(),
        max_length: z.int().min(0).optional(),
        pattern: PageText.optional(),
    })
    .register(registry, { id: "Input" });
export type Input = z.infer<typeof Input>;

/** What must hold before the call (D5). A missing CSRF token is fetched the way the site's own client does. */
export const Precondition = z
    .discriminatedUnion("type", [
        z.strictObject({ type: z.literal("authenticated") }),
        z.strictObject({ type: z.literal("prior_action"), action: ActionId }),
        z.strictObject({
            type: z.literal("token"),
            /** Where the token goes on the request. */
            into: RequestLocation,
            /** Where the site's own client obtains it. */
            source: DependencyEdge,
        }),
    ])
    .register(registry, { id: "Precondition" });
export type Precondition = z.infer<typeof Precondition>;

/** What the executor checks after the call to verify it had the intended effect. */
export const Postcondition = z
    .discriminatedUnion("type", [
        z.strictObject({ type: z.literal("response_class"), class: ResponseClass }),
        z.strictObject({ type: z.literal("node_present"), node: NodeId }),
        z.strictObject({ type: z.literal("node_text_changed"), node: NodeId }),
        z.strictObject({ type: z.literal("navigates_to"), route: RouteId }),
        z.strictObject({
            type: z.literal("response_field_equals_input"),
            at: Name,
            input: InputName,
        }),
    ])
    .register(registry, { id: "Postcondition" });
export type Postcondition = z.infer<typeof Postcondition>;

/** D5. A non-idempotent action is never retried automatically. */
export const Idempotency = z
    .strictObject({
        idempotent: z.boolean(),
        /** Where the action takes an idempotency key, if it does. */
        key: RequestLocation.optional(),
    })
    .register(registry, { id: "Idempotency" });
export type Idempotency = z.infer<typeof Idempotency>;

/** The action is enabled, or marked stale/disabled by drift detection (D3). */
export const ActionState = z.enum(["enabled", "stale", "disabled"]).register(registry, { id: "ActionState" });

const base = {
    id: ActionId,
    /** The control this action was derived from, when there is one. */
    node: NodeId.optional(),
    label: PageText.optional(),
    description: SurfaceText,
    inputs: z.array(Input).max(256),
    auth: AuthRequirement,
    preconditions: z.array(Precondition).max(64),
    postconditions: z.array(Postcondition).max(64),
    idempotency: Idempotency,
    state: ActionState,
};

/**
 * Evidence for anything that is not a plain navigation: the first element must be declared or
 * observed (D5), the rest may be any class. Emitters order evidence strongest first.
 */
const strongFirst = z.intersection(z.tuple([StrongEvidence], Evidence), z.array(Evidence).max(64));

/** T1. A read. May rest on inferred evidence, and is the only kind that carries `confidence`. */
export const NavigateAction = z
    .strictObject({
        ...base,
        kind: z.literal("navigate"),
        tier: z.literal("T1"),
        effect: z.literal("read"),
        target: PageText,
        route: RouteId.optional(),
        evidence: z.array(Evidence).min(1).max(64),
        confidence: Percent.optional(),
    })
    .register(registry, { id: "NavigateAction" });

/** T3. Local state change with no network (tabs, accordions, filters). */
export const StateAction = z
    .strictObject({
        ...base,
        kind: z.literal("state"),
        tier: z.literal("T3"),
        effect: z.literal("read"),
        /** Nodes whose content or visibility the state change affects. */
        affects: z.array(NodeId).max(256),
        evidence: strongFirst,
    })
    .register(registry, { id: "StateAction" });

const networkAction = <K extends "submit" | "fetch" | "download">(kind: K) =>
    z.strictObject({
        ...base,
        kind: z.literal(kind),
        tier: z.literal("T2"),
        effect: Effect,
        /** Present exactly when the effect is financial (checked in code). */
        payment: PaymentMethod.optional(),
        request: RequestSpec,
        evidence: strongFirst,
    });

/** T2. A form submission. */
export const SubmitAction = networkAction("submit").register(registry, { id: "SubmitAction" });
/** T2. A handler-driven request. */
export const FetchAction = networkAction("fetch").register(registry, { id: "FetchAction" });
/** T2. A file download. */
export const DownloadAction = networkAction("download").register(registry, { id: "DownloadAction" });

export const Action = z
    .discriminatedUnion("kind", [NavigateAction, StateAction, SubmitAction, FetchAction, DownloadAction])
    .register(registry, { id: "Action" });
export type Action = z.infer<typeof Action>;

/** D1 T4 and D10: an action we cannot model is listed, with a reason, and counted in coverage. */
export const UnsupportedItem = z
    .strictObject({
        id: ActionId,
        node: NodeId.optional(),
        label: PageText.optional(),
        /** The tier the control would have been at. Opaque reasons (canvas, webgl, drag, video) are T4. */
        tier: Tier,
        reason: UnsupportedReason,
        note: SurfaceText.optional(),
    })
    .register(registry, { id: "UnsupportedItem" });
export type UnsupportedItem = z.infer<typeof UnsupportedItem>;
