// Primitive wire types shared by every schema in this package.
//
// Wire-format rules (docs/AGENT_SURFACE.md D4, section 6):
//  - only constructs that convert cleanly to JSON Schema: no transforms,
//    refinements, defaults, coercion, dates or bigints;
//  - objects are strict (`additionalProperties: false`), so field drift fails;
//  - optional fields are absent, never `null`;
//  - numbers on the wire are integers (no floats), so canonical hashing cannot
//    diverge between JS and Rust number formatting.
import { z } from "zod";

/** The schema major.minor.patch this package emits. Minor bumps are additive only. */
export const SCHEMA_VERSION = "1.0.0";

/** Hard caps. Oversized values are rejected at the wire, not truncated silently. */
export const LIMITS = {
    /** Longest single string a page may contribute to one field (UTF-16 code units). */
    pageText: 16384,
    /** Longest short identifier, name or code. */
    short: 256,
    /** Longest URL or URL template. */
    url: 4096,
    maxItems: 10000,
} as const;

const HEX64 = /^[0-9a-f]{64}$/;

/** `sha256:` followed by 64 lowercase hex digits. */
export const Sha256 = z
    .string()
    .regex(/^sha256:[0-9a-f]{64}$/)
    .meta({ id: "Sha256", description: "sha256:<64 lowercase hex>" });
export type Sha256 = z.infer<typeof Sha256>;

export const isHex64 = (s: string): boolean => HEX64.test(s);

/** Semantic version, no prerelease or build metadata. */
export const SemVer = z
    .string()
    .regex(/^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/)
    .meta({ id: "SemVer" });

/** RFC 3339 UTC timestamp, second precision: `2026-11-10T12:00:00Z`. */
export const Timestamp = z
    .string()
    .regex(/^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$/)
    .meta({ id: "Timestamp", description: "RFC 3339 UTC, second precision, e.g. 2026-11-10T12:00:00Z" });

/** Short lowercase slug, safe as an MCP tool name. */
const slug = (what: string, id: string) =>
    z
        .string()
        .regex(/^[a-z][a-z0-9_-]{0,63}$/)
        .meta({
            id,
            description: `${what}: lowercase slug, unique within the document. Derive it from structure (kind, hash), never from page text.`,
        });

/**
 * Stable node id: `n_` + first 64 bits of a SHA-256 over role, accessible name,
 * structural position and route (D4). The derivation lives with the emitter; this
 * package fixes only the shape.
 */
export const NodeId = z
    .string()
    .regex(/^n_[0-9a-f]{16}$/)
    .meta({ id: "NodeId", description: "n_ + 16 lowercase hex" });
export type NodeId = z.infer<typeof NodeId>;

export const RouteId = slug("Route id", "RouteId");
export const ActionId = slug("Action id", "ActionId");
export const FlowId = slug("Flow id", "FlowId");
/** Unique within its flow. */
export const StepId = slug("Step id", "StepId");
/** Unique within its flow. */
export const RequestId = slug("Request id", "RequestId");
export const MandateId = slug("Mandate id", "MandateId");
export const ConfirmationId = slug("Confirmation id", "ConfirmationId");
export const ReceiptId = slug("Receipt id", "ReceiptId");
export type RouteId = z.infer<typeof RouteId>;
export type ActionId = z.infer<typeof ActionId>;
export type FlowId = z.infer<typeof FlowId>;
export type StepId = z.infer<typeof StepId>;
export type RequestId = z.infer<typeof RequestId>;
export type MandateId = z.infer<typeof MandateId>;
export type ConfirmationId = z.infer<typeof ConfirmationId>;
export type ReceiptId = z.infer<typeof ReceiptId>;

/** Name of a typed input / parameter. */
export const InputName = z
    .string()
    .regex(/^[A-Za-z_][A-Za-z0-9_]{0,63}$/)
    .meta({ id: "InputName" });
export type InputName = z.infer<typeof InputName>;

/** Machine-readable reason or class code. Open vocabulary within the pattern. */
export const Code = z
    .string()
    .regex(/^[a-z][a-z0-9_.-]{0,63}$/)
    .meta({ id: "Code" });
export type Code = z.infer<typeof Code>;

/** Hostname or host:port, lowercase, no scheme or path. */
export const Host = z
    .string()
    .regex(/^[a-z0-9]([a-z0-9.-]{0,251}[a-z0-9])?(:[0-9]{1,5})?$/)
    .meta({ id: "Host" });

/** ISO 4217 currency code. */
export const Currency = z
    .string()
    .regex(/^[A-Z]{3}$/)
    .meta({ id: "Currency", description: "ISO 4217 alphabetic code" });

/** Money in minor units (cents). Integer only: no floats on the wire. */
export const Money = z
    .strictObject({
        amount_minor: z.int().min(0).max(Number.MAX_SAFE_INTEGER),
        currency: Currency,
    })
    .meta({ id: "Money" });
export type Money = z.infer<typeof Money>;

/** Integer percentage 0..100, used for read-only inference confidence. */
export const Percent = z.int().min(0).max(100).meta({ id: "Percent" });
