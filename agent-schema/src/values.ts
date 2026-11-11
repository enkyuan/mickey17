// D11: where a value lives in a request or response, how a request value is classed,
// and the dependency edge that sources a derived value.
import { z } from "zod";
import { MarkedValue, PageValue } from "./origin";
import { Code, InputName, LIMITS, NodeId, RequestId, Sha256, StepId } from "./primitives";

/** Position of a value inside a request. For `body-json` the name is a JSON Pointer (RFC 6901). */
export const RequestLocation = z
    .strictObject({
        in: z.enum(["header", "query", "path", "cookie", "body-json", "body-form"]),
        name: z.string().min(1).max(LIMITS.short),
    })
    .meta({ id: "RequestLocation" });
export type RequestLocation = z.infer<typeof RequestLocation>;

/** Position of a value inside a response. For `body-json` the name is a JSON Pointer. */
export const ResponseLocation = z
    .strictObject({
        in: z.enum(["header", "set-cookie", "body-json", "body-form", "redirect-location"]),
        name: z.string().min(1).max(LIMITS.short),
    })
    .meta({ id: "ResponseLocation" });
export type ResponseLocation = z.infer<typeof ResponseLocation>;

/** How the recorder matched the later value to its source: exact first, then structured. */
export const MatchKind = z
    .enum(["exact", "json-path", "form-field", "url-segment", "header"])
    .meta({ id: "MatchKind" });

/** Where a derived value comes from. A source step must precede the consuming step. */
export const DependencySource = z
    .discriminatedUnion("kind", [
        z.strictObject({
            kind: z.literal("response"),
            step: StepId,
            request: RequestId,
            at: ResponseLocation,
        }),
        z.strictObject({
            kind: z.literal("dom"),
            step: StepId,
            node: NodeId,
            /** Attribute name; absent means the node's text. */
            attribute: z.string().min(1).max(LIMITS.short).optional(),
        }),
        z.strictObject({
            kind: z.literal("cookie"),
            step: StepId,
            name: z.string().min(1).max(LIMITS.short),
        }),
        z.strictObject({
            kind: z.literal("storage"),
            step: StepId,
            area: z.enum(["local", "session"]),
            key: z.string().min(1).max(LIMITS.short),
        }),
    ])
    .meta({ id: "DependencySource" });
export type DependencySource = z.infer<typeof DependencySource>;

/** The dependency edge: a value produced at `source` reappears, matched by `match`. */
export const DependencyEdge = z
    .strictObject({ source: DependencySource, match: MatchKind })
    .meta({ id: "DependencyEdge" });
export type DependencyEdge = z.infer<typeof DependencyEdge>;

/** Why a client-computed value cannot be produced without a browser. */
export const BrowserReason = z
    .enum(["signature", "timestamp", "nonce", "fingerprint", "anti-bot", "tokenization", "unknown"])
    .meta({ id: "BrowserReason" });

/**
 * Value class (D11). Discriminated on `class`:
 *  - constant: same in every recorded run; embedded.
 *  - input: varies with caller intent; a typed parameter.
 *  - derived: taken from an earlier step; `edge` is the dependency that sources it.
 *  - client-computed: no source in the recording; either liftable (`lift`) or requires a browser.
 * There is no "unknown" class: an unclassifiable value blocks its flow (see Flow.status).
 */
export const ValueClass = z
    .discriminatedUnion("class", [
        z.strictObject({ class: z.literal("constant"), value: PageValue }),
        z.strictObject({ class: z.literal("input"), input: InputName }),
        z.strictObject({ class: z.literal("derived"), edge: DependencyEdge }),
        z.strictObject({
            class: z.literal("client-computed"),
            resolution: z.discriminatedUnion("kind", [
                z.strictObject({
                    kind: z.literal("lift"),
                    /** Hash of the reviewed `mochi lift` spec that computes the value. */
                    spec: Sha256,
                    /** Values the producing function reads. */
                    args: z.array(DependencyEdge).max(64),
                }),
                z.strictObject({ kind: z.literal("requires_browser"), reason: BrowserReason }),
            ]),
        }),
    ])
    .meta({ id: "ValueClass" });
export type ValueClass = z.infer<typeof ValueClass>;

/** One classed value in a request. */
export const RequestValue = z
    .strictObject({ at: RequestLocation, value: ValueClass })
    .meta({ id: "RequestValue" });
export type RequestValue = z.infer<typeof RequestValue>;

export const HttpMethod = z.enum(["GET", "POST", "PUT", "PATCH", "DELETE"]).meta({ id: "HttpMethod" });

/** Expected response class. A response outside it fails the step's postcondition. */
export const ResponseClass = z.enum(["2xx", "3xx", "4xx", "5xx"]).meta({ id: "ResponseClass" });

/** A URL or URL template recorded from page traffic. Page origin, bounded. */
export const PageUrl = z
    .strictObject({ origin: z.literal("page"), value: z.string().min(1).max(LIMITS.url) })
    .meta({ id: "PageUrl" });

/** A request as recorded, with every value classed. Secrets never appear: only classes and sources. */
export const RequestSpec = z
    .strictObject({
        id: RequestId,
        method: HttpMethod,
        /** URL template recorded from traffic; path variables are `{name}` with matching `path` values. */
        url: PageUrl,
        body: z.enum(["none", "json", "form", "multipart"]),
        values: z.array(RequestValue).max(512),
        expect: ResponseClass,
        /** Hash of the recorded exchange this was derived from. */
        recorded: Sha256,
    })
    .meta({ id: "RequestSpec" });
export type RequestSpec = z.infer<typeof RequestSpec>;

// Re-exported so consumers have one place for "a value with its origin".
export { MarkedValue, Code };
