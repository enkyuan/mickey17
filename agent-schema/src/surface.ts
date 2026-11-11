// The surface document (D4): one versioned, content-addressed JSON artifact, pinned to the
// capture it came from. Everything else (Markdown, MCP, OpenAPI) is projected from it.
import { z } from "zod";
import { PageText } from "./origin";
import { Action, UnsupportedItem } from "./action";
import { Flow } from "./flow";
import { Mode, Tier } from "./policy";
import { Code, Host, NodeId, Percent, RouteId, SemVer, Sha256, Timestamp } from "./primitives";
import { UnsupportedReason } from "./result";

export const Visibility = z.enum(["visible", "hidden", "inert"]).meta({ id: "Visibility" });

/** A T0 node. Flat: `parent` links nodes within a route, so documents diff and stream cleanly. */
export const SurfaceNode = z
    .strictObject({
        id: NodeId,
        parent: NodeId.optional(),
        /** ARIA role or HTML-AAM mapped role. */
        role: z.string().regex(/^[a-z][a-z-]{0,31}$/),
        name: PageText.optional(),
        text: PageText.optional(),
        /** Heading level. */
        level: z.int().min(1).max(6).optional(),
        href: PageText.optional(),
        /** Hidden or inert nodes appear only when `options.include_hidden` is set (D8). */
        visibility: Visibility,
        /** Present only when the role or name was inferred. */
        confidence: Percent.optional(),
    })
    .meta({ id: "SurfaceNode" });
export type SurfaceNode = z.infer<typeof SurfaceNode>;

export const Route = z
    .strictObject({
        id: RouteId,
        /** Path and query template as the site serves it. */
        url: PageText,
        canonical: PageText.optional(),
        title: PageText.optional(),
        /** Tiers this route is covered at (D1). */
        tiers: z.array(Tier).min(1).max(5),
        nodes: z.array(SurfaceNode).max(100000),
    })
    .meta({ id: "Route" });
export type Route = z.infer<typeof Route>;

/** Coverage per tier. Derivable from the document; `checkSurface` verifies it matches. */
export const TierCoverage = z
    .strictObject({
        tier: Tier,
        routes: z.int().min(0),
        actions: z.int().min(0),
        unsupported: z.int().min(0),
    })
    .meta({ id: "TierCoverage" });

export const Coverage = z
    .strictObject({
        tiers: z.array(TierCoverage).max(5),
        unsupported_reasons: z.array(z.strictObject({ reason: UnsupportedReason, count: z.int().min(1) })).max(32),
    })
    .meta({ id: "Coverage" });
export type Coverage = z.infer<typeof Coverage>;

/** Legal and ethical gate results recorded in the manifest (section 6). */
export const Gates = z
    .strictObject({
        robots_txt: z.enum(["allowed", "disallowed", "absent", "not-checked"]),
        llms_txt: z.enum(["present", "absent", "not-checked"]),
        tos: z.enum(["permitted", "restricted", "not-checked"]),
    })
    .meta({ id: "Gates" });

export const Capture = z
    .strictObject({
        /** Hash of the capture this surface was derived from. */
        hash: Sha256,
        captured_at: Timestamp,
        tool: z.strictObject({ name: Code, version: SemVer }),
    })
    .meta({ id: "Capture" });

export const AgentSurface = z
    .strictObject({
        kind: z.literal("agent-surface"),
        /** Schema version the document was written against. Readers accept equal major, minor <= theirs. */
        schema_version: SemVer,
        /** sha256 of the RFC 8785 canonical form of this document with `hash` omitted (see hash.ts). */
        hash: Sha256,
        capture: Capture,
        /** Chosen by the operator, never inferred (D2). */
        mode: Mode,
        site: z.strictObject({ host: Host, scheme: z.enum(["https", "http"]) }),
        gates: Gates,
        options: z.strictObject({ include_hidden: z.boolean() }),
        coverage: Coverage,
        routes: z.array(Route).max(100000),
        actions: z.array(Action).max(100000),
        flows: z.array(Flow).max(10000),
        unsupported: z.array(UnsupportedItem).max(100000),
    })
    .meta({ id: "AgentSurface", description: "Agent surface document, schema v1." });
export type AgentSurface = z.infer<typeof AgentSurface>;
