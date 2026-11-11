// Minimal local types. To be replaced by the agent-schema package once its first
// PR merges; interface changes are agreed through the parent session.

export const MODES = ["owner", "delegated", "read-only"] as const;
export type Mode = (typeof MODES)[number];

export const TIERS = ["T0", "T1", "T2", "T3", "T4"] as const;
export type Tier = (typeof TIERS)[number];

export const EFFECTS = [
    "read",
    "write-reversible",
    "write-irreversible",
    "financial",
    "account-control",
] as const;
export type Effect = (typeof EFFECTS)[number];

/** Descending trust, as in AGENT_SURFACE.md section 3. */
export const EVIDENCE = ["declared", "observed", "derived", "inferred"] as const;
export type EvidenceKind = (typeof EVIDENCE)[number];

/** D10 failure semantics. */
export type Status =
    | "ok"
    | "stale"
    | "unsupported"
    | "requires_auth"
    | "requires_confirmation"
    | "requires_human"
    | "requires_browser"
    | "blocked"
    | "error";

export function isMode(v: unknown): v is Mode {
    return typeof v === "string" && (MODES as readonly string[]).includes(v);
}
export function isTier(v: unknown): v is Tier {
    return typeof v === "string" && (TIERS as readonly string[]).includes(v);
}
export function isEffect(v: unknown): v is Effect {
    return typeof v === "string" && (EFFECTS as readonly string[]).includes(v);
}
export function isEvidence(v: unknown): v is EvidenceKind {
    return typeof v === "string" && (EVIDENCE as readonly string[]).includes(v);
}

/**
 * A refusal always names the missing authorization or evidence (`missing`) and carries
 * a stable machine code. Messages never echo caller-supplied strings (only enumerated
 * values), so a refusal cannot carry a secret or injected page text.
 */
export interface Refusal {
    readonly ok: false;
    readonly status: Exclude<Status, "ok" | "stale" | "error">;
    readonly code: string;
    readonly missing: string;
    readonly message: string;
}

export function refuse(
    status: Refusal["status"],
    code: string,
    missing: string,
    message: string,
): Refusal {
    return Object.freeze({ ok: false, status, code, missing, message });
}

export type Result<T> = ({ readonly ok: true } & T) | Refusal;
