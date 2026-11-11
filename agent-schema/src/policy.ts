// Modes (D2), tiers (D1), effect classes (D5) and the confirmation rule they imply.
// This file is vocabulary only. Enforcement (which mode permits which tier or effect)
// belongs to the executor/policy code, not to the wire format.
import { z } from "zod";

/** D2. Chosen by the operator, never inferred. Default for third-party sites is `read-only`. */
export const Mode = z.enum(["owner", "delegated", "read-only"]).meta({ id: "Mode" });
export type Mode = z.infer<typeof Mode>;

/** D1 fidelity tiers. T4 is opaque and only ever listed as unsupported. */
export const Tier = z.enum(["T0", "T1", "T2", "T3", "T4"]).meta({ id: "Tier" });
export type Tier = z.infer<typeof Tier>;

/** D5. Ordered from least to most consequential: see EFFECT_ORDER. */
export const Effect = z
    .enum(["read", "write-reversible", "write-irreversible", "financial", "account-control"])
    .meta({ id: "Effect" });
export type Effect = z.infer<typeof Effect>;

/** Least to most consequential. `account-control` ranks last: it can lock the caller out. */
export const EFFECT_ORDER: readonly Effect[] = [
    "read",
    "write-reversible",
    "write-irreversible",
    "financial",
    "account-control",
];

export const effectRank = (e: Effect): number => EFFECT_ORDER.indexOf(e);

/** The more consequential of two effects. */
export const maxEffect = (a: Effect, b: Effect): Effect => (effectRank(a) >= effectRank(b) ? a : b);

/** D5: these require an explicit per-call confirmation token. The surface never auto-confirms. */
export const CONFIRMATION_EFFECTS: readonly Effect[] = ["write-irreversible", "financial", "account-control"];

export const requiresConfirmation = (e: Effect): boolean => CONFIRMATION_EFFECTS.includes(e);

/** D1 evidence source classes, descending trust. */
export const EvidenceSource = z.enum(["declared", "observed", "derived", "inferred"]).meta({ id: "EvidenceSource" });
export type EvidenceSource = z.infer<typeof EvidenceSource>;

export const EVIDENCE_ORDER: readonly EvidenceSource[] = ["declared", "observed", "derived", "inferred"];
