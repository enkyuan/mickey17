import { isSafePosInt } from "./bytes.ts";
import { isIssuedEgress, type EgressAllow } from "./egress.ts";
import {
    isConfirmationExpiredOrMismatched,
    type ConfirmationBinding,
    type ConsumedStore,
    verifyConfirmation,
    type VerifiedConfirmation,
    isVerifiedConfirmation,
} from "./confirm.ts";
import {
    checkMandateScope,
    isVerifiedMandate,
    normalizeHost,
    verifyMandate,
    type SpendLedger,
    type VerifiedMandate,
} from "./mandate.ts";
import type { Verifier } from "./sign.ts";
import {
    isEffect,
    isEvidence,
    isMode,
    isTier,
    refuse,
    type Effect,
    type EvidenceKind,
    type Mode,
    type Refusal,
    type Tier,
} from "./types.ts";

export interface Credential {
    readonly kind: "service" | "user-session" | "oauth";
    /** Effect classes the credential is scoped to. */
    readonly scopes: readonly Effect[];
}

export interface ThirdPartyChecks {
    readonly robots: "allowed" | "disallowed" | "unknown";
    readonly tos: "ok" | "violates" | "unchecked";
}

export interface MoneyDetails {
    /**
     * The egress decision the request will be sent under (from evaluateEgress or
     * evaluateRedirect). Only a decision issued by this library is accepted, and its host
     * must equal `merchant`.
     */
    readonly egress: EgressAllow;
    readonly merchant: string;
    readonly item: string;
    readonly totalMinor: number;
    readonly currency: string;
    readonly payee: string;
}

export interface ActionRequest {
    readonly actionId: string;
    readonly mode: Mode;
    readonly tier: Tier;
    readonly effect: Effect;
    /** Strongest evidence backing the action (declared > observed > derived > inferred). */
    readonly evidence: EvidenceKind;
    readonly idempotent: boolean;
    readonly idempotencyKey?: string;
    /** Whether the endpoint needs credentials even for a read. */
    readonly requiresAuth?: boolean;
    readonly credential?: Credential;
    readonly thirdParty?: ThirdPartyChecks;
    /** Target of an irreversible / account-control action (bound into the confirmation). */
    readonly item?: string;
    /** Required for `financial`. */
    readonly money?: MoneyDetails;
    readonly now: number;
}

export interface Proofs {
    readonly mandate?: VerifiedMandate;
    readonly confirmation?: VerifiedConfirmation;
}

export interface Allow {
    readonly ok: true;
    /** The library never retries this action automatically. */
    readonly neverRetry: boolean;
}

const ID = /^[A-Za-z0-9._:-]{1,128}$/;
const KEY = /^[\x21-\x7e]{8,128}$/;

const NEVER_RETRY_EFFECTS: readonly Effect[] = ["write-irreversible", "financial", "account-control"];

/**
 * Mode x tier x effect, before any proof. Default deny: a cell is allowed only if it is
 * named here.
 *   read-only: T0/T1 reads only.
 *   owner/delegated: T0/T1/T3 are reads (T3 is local state, no network); T2 may carry
 *   any effect; T4 is never operable.
 */
export function modeCell(mode: Mode, tier: Tier, effect: Effect): Refusal | null {
    if (tier === "T4") return refuse("unsupported", "tier-opaque", "an operable tier (T4 is described, never operated)", "T4 (opaque) actions are listed as unsupported and never executed.");
    if (mode === "read-only") {
        if (tier !== "T0" && tier !== "T1") return refuse("blocked", "mode-read-only-tier", "owner or delegated mode (read-only permits T0/T1 only)", `Mode read-only does not permit tier ${tier}.`);
        if (effect !== "read") return refuse("blocked", "mode-read-only-effect", "owner or delegated mode (read-only permits reads only)", `Mode read-only does not permit effect ${effect}.`);
        return null;
    }
    if (tier === "T2") return null;
    if (effect !== "read") return refuse("blocked", "tier-effect-incoherent", "tier T2 (only T2 carries side effects)", `Tier ${tier} cannot carry effect ${effect}.`);
    return null;
}

function validate(req: ActionRequest): Refusal | null {
    const bad = (field: string) => refuse("blocked", "invalid-request", `a well-formed ${field}`, `Request field ${field} is missing or malformed.`);
    if (!isMode(req.mode)) return bad("mode");
    if (!isTier(req.tier)) return bad("tier");
    if (!isEffect(req.effect)) return bad("effect");
    if (!isEvidence(req.evidence)) return bad("evidence");
    if (typeof req.actionId !== "string" || !ID.test(req.actionId)) return bad("actionId");
    if (typeof req.idempotent !== "boolean") return bad("idempotent");
    if (typeof req.now !== "number" || !Number.isFinite(req.now)) return bad("now");
    if (req.requiresAuth !== undefined && typeof req.requiresAuth !== "boolean") return bad("requiresAuth");
    if (req.idempotencyKey !== undefined && (typeof req.idempotencyKey !== "string" || !KEY.test(req.idempotencyKey))) return bad("idempotencyKey");
    return null;
}

/** Everything that does not need a cryptographic proof. */
export function checkStatic(req: ActionRequest): Refusal | null {
    const invalid = validate(req);
    if (invalid) return invalid;
    const cell = modeCell(req.mode, req.tier, req.effect);
    if (cell) return cell;

    if (req.tier === "T2" || req.tier === "T3") {
        if (req.evidence !== "declared" && req.evidence !== "observed") {
            return refuse("unsupported", "evidence-insufficient", "declared or observed evidence (derived or inferred alone never suffices for an action)", `Evidence level ${req.evidence} is below observed.`);
        }
    }

    if (req.mode === "read-only") {
        const tp = req.thirdParty;
        if (!tp || tp.robots !== "allowed") return refuse("blocked", "robots-not-allowed", "a robots.txt evaluation that allows this path", "robots.txt does not allow this request, or was not evaluated.");
        if (tp.tos !== "ok") return refuse("blocked", "tos-not-cleared", "a recorded terms-of-service check that permits access", "The terms-of-service check is missing or negative.");
    }

    // read-only must positively assert that the endpoint is public; silence is not enough
    if (req.mode === "read-only" && (req.requiresAuth !== false || req.credential !== undefined)) {
        return refuse("blocked", "read-only-private-endpoint", "owner or delegated mode (read-only never replays private endpoints or attaches credentials)", "Mode read-only does not permit authenticated requests.");
    }
    const needsCredential = req.effect !== "read" || req.requiresAuth === true;
    if (needsCredential && req.mode !== "read-only") {
        const c = req.credential;
        const want = req.mode === "owner" ? ["service"] : ["user-session", "oauth"];
        if (!c || !want.includes(c.kind)) {
            return refuse("requires_auth", "credential-missing", req.mode === "owner" ? "a service credential (owner mode)" : "the user's own session or OAuth grant (delegated mode)", "No credential of the kind this mode requires was supplied.");
        }
        if (!Array.isArray(c.scopes) || !c.scopes.includes(req.effect)) {
            return refuse("requires_auth", "credential-scope", `a credential scoped to effect ${req.effect}`, "The credential is not scoped to this effect class.");
        }
    }

    if (req.effect === "financial") {
        const m = req.money;
        if (!m || typeof m.merchant !== "string" || typeof m.item !== "string" || typeof m.payee !== "string" || typeof m.currency !== "string" || !isSafePosInt(m.totalMinor)) {
            return refuse("blocked", "invalid-request", "a well-formed money object (merchant, item, totalMinor, currency, payee)", "Financial action lacks well-formed money details.");
        }
        if (!isIssuedEgress(m.egress)) {
            return refuse("blocked", "egress-decision-missing", "an egress decision issued by evaluateEgress for this request", "Financial actions must carry the egress decision they will be sent under.");
        }
        if (normalizeHost(m.egress.host) !== normalizeHost(m.merchant)) {
            return refuse("blocked", "egress-host-mismatch", "a request whose egress host is the authorized merchant", "The host the request goes to is not the merchant being authorized.");
        }
        if (normalizeHost(m.payee) !== normalizeHost(m.merchant)) {
            return refuse("blocked", "payee-merchant-mismatch", "a payee equal to the merchant the mandate allows", "The payee differs from the merchant being called.");
        }
        if (req.idempotencyKey === undefined) return refuse("blocked", "idempotency-key-missing", "an idempotency key (money-moving calls must carry one)", "Financial action has no idempotency key.");
    }
    if (req.effect === "write-irreversible" || req.effect === "account-control") {
        if (req.item === undefined) return refuse("blocked", "item-missing", "the item or target the confirmation is bound to", "Irreversible and account-control actions must name their target.");
        if (typeof req.item !== "string" || req.item.length === 0 || req.item.length > 256) {
            return refuse("blocked", "invalid-request", "a well-formed item", "Request field item is malformed.");
        }
    }
    return null;
}

export function bindingFor(req: ActionRequest): ConfirmationBinding {
    if (req.effect === "financial" && req.money) {
        return { actionId: req.actionId, item: req.money.item, totalMinor: req.money.totalMinor, currency: req.money.currency, payee: req.money.payee };
    }
    return { actionId: req.actionId, item: req.item ?? null, totalMinor: null, currency: null, payee: null };
}

/** Proof checks on already-verified proofs. Pure. */
export function checkProofs(req: ActionRequest, proofs: Proofs): Refusal | null {
    if (req.effect === "financial" && req.money) {
        const m = proofs.mandate;
        if (!m) return refuse("requires_auth", "mandate-missing", "a verified mandate (scoped, expiring, merchant allowlist, spend cap, currency)", "No mandate was supplied for a money-moving action.");
        const scope = checkMandateScope(m, { actionId: req.actionId, merchant: req.money.merchant, payee: req.money.payee, currency: req.money.currency, amountMinor: req.money.totalMinor }, req.now);
        if (scope) return scope;
        if (!isVerifiedMandate(m)) return refuse("requires_auth", "mandate-unverified", "a verified mandate", "Mandate was not produced by verification.");
    }
    if (NEVER_RETRY_EFFECTS.includes(req.effect)) {
        const c = proofs.confirmation;
        if (!c) {
            return refuse("requires_confirmation", "confirmation-missing", req.effect === "financial" ? "a per-call confirmation of item, total, currency and payee" : "a per-call confirmation token", "No confirmation token was supplied.");
        }
        if (!isVerifiedConfirmation(c)) return refuse("requires_confirmation", "confirmation-unverified", "a verified confirmation", "Confirmation was not produced by verification.");
        const bad = isConfirmationExpiredOrMismatched(c, bindingFor(req), req.now);
        if (bad) return bad;
    }
    return null;
}

/**
 * Synchronous decision over already-verified proofs. Default deny. It does NOT reserve
 * spend or consume a confirmation: executors must call `authorize`, never this alone.
 */
export function decide(req: ActionRequest, proofs: Proofs = {}): Allow | Refusal {
    const s = checkStatic(req);
    if (s) return s;
    const p = checkProofs(req, proofs);
    if (p) return p;
    return Object.freeze({ ok: true as const, neverRetry: !req.idempotent || NEVER_RETRY_EFFECTS.includes(req.effect) });
}

/**
 * Copies every field the policy reads exactly once into a frozen plain object, so a getter,
 * proxy or later mutation of the caller's object cannot show the checks one value and the
 * ledger or executor another. Unknown fields are dropped. The egress decision is kept by
 * reference (it is frozen and its brand is its identity).
 */
export function snapshotRequest(req: ActionRequest): ActionRequest {
    const cred = req.credential;
    const tp = req.thirdParty;
    const m = req.money;
    const snap: Record<string, unknown> = {
        actionId: req.actionId, mode: req.mode, tier: req.tier, effect: req.effect, evidence: req.evidence, idempotent: req.idempotent, now: req.now,
    };
    const optional = { idempotencyKey: req.idempotencyKey, requiresAuth: req.requiresAuth, item: req.item };
    for (const [k, v] of Object.entries(optional)) if (v !== undefined) snap[k] = v;
    // null or non-object values are passed through unchanged so checkStatic refuses them (never throw)
    const isObj = (x: unknown): x is object => typeof x === "object" && x !== null;
    if (cred !== undefined) {
        if (isObj(cred)) {
            const scopes = cred.scopes;
            snap.credential = Object.freeze({ kind: cred.kind, scopes: Array.isArray(scopes) ? Object.freeze([...scopes]) : scopes });
        } else snap.credential = cred;
    }
    if (tp !== undefined) snap.thirdParty = isObj(tp) ? Object.freeze({ robots: tp.robots, tos: tp.tos }) : tp;
    if (m !== undefined) {
        snap.money = isObj(m) ? Object.freeze({ egress: m.egress, merchant: m.merchant, item: m.item, totalMinor: m.totalMinor, currency: m.currency, payee: m.payee }) : m;
    }
    return Object.freeze(snap) as unknown as ActionRequest;
}

export interface AuthorizeDeps {
    readonly mandateVerifier: Verifier;
    readonly confirmationVerifier: Verifier;
    readonly ledger: SpendLedger;
    readonly consumed: ConsumedStore;
    readonly revokedMandates?: ReadonlySet<string>;
    readonly maxMandateLifetimeMs?: number;
    readonly maxConfirmationTtlMs?: number;
    /** Unique id for the spend reservation (the idempotency key is the natural choice). */
    readonly reservationRef?: string;
}

export interface Tokens {
    readonly mandate?: string;
    readonly confirmation?: string;
}

export interface Authorization {
    readonly ok: true;
    /** The frozen request that was authorized. Send exactly this, not the caller's original object. */
    readonly request: ActionRequest;
    readonly neverRetry: boolean;
    readonly mandateId: string | null;
    readonly confirmationId: string | null;
    /** Call once the outcome is known. Unknown outcome: do NOT release (stays reserved). */
    settle(outcome: "committed" | "released"): Promise<void>;
}

/**
 * The entry point an executor calls before a request leaves the process. Order matters:
 * static policy, then token verification, then proof binding, and only then side
 * effects (reserve spend, consume the confirmation). If consuming fails the
 * reservation is released, so a replayed confirmation neither executes nor spends.
 */
export async function authorize(input: ActionRequest, tokens: Tokens, deps: AuthorizeDeps): Promise<Authorization | Refusal> {
    if (typeof input !== "object" || input === null) return refuse("blocked", "invalid-request", "a well-formed request object", "The request is not an object.");
    if (typeof tokens !== "object" || tokens === null) return refuse("blocked", "invalid-request", "a well-formed tokens object", "Tokens must be an object (possibly empty).");
    const req = snapshotRequest(input);
    // deps are read once as well: a swapped ledger or store between steps must not split one authorization
    const { mandateVerifier, confirmationVerifier, ledger, consumed, revokedMandates, maxMandateLifetimeMs, maxConfirmationTtlMs } = deps;
    const reservationRef = deps.reservationRef;
    const s = checkStatic(req);
    if (s) return s;

    const mandateToken = tokens.mandate;
    const confirmationToken = tokens.confirmation;
    const proofs: { mandate?: VerifiedMandate; confirmation?: VerifiedConfirmation } = {};
    if (req.effect === "financial" && mandateToken !== undefined) {
        const v = await verifyMandate(mandateToken, mandateVerifier, {
            now: req.now,
            ...(maxMandateLifetimeMs !== undefined ? { maxLifetimeMs: maxMandateLifetimeMs } : {}),
            ...(revokedMandates ? { revoked: revokedMandates } : {}),
        });
        if (!v.ok) return v;
        proofs.mandate = v.mandate;
    }
    if (NEVER_RETRY_EFFECTS.includes(req.effect) && confirmationToken !== undefined) {
        const v = await verifyConfirmation(confirmationToken, confirmationVerifier, bindingFor(req), {
            now: req.now,
            ...(maxConfirmationTtlMs !== undefined ? { maxTtlMs: maxConfirmationTtlMs } : {}),
        });
        if (!v.ok) return v;
        proofs.confirmation = v.confirmation;
    }
    const d = decide(req, proofs);
    if (!d.ok) return d;

    let reserved: { id: string; ref: string } | null = null;
    if (req.effect === "financial" && proofs.mandate && req.money) {
        const ref = reservationRef ?? req.idempotencyKey ?? "";
        const r = await ledger.reserve(proofs.mandate.id, ref, req.money.totalMinor, proofs.mandate.capMinor);
        if (!r.ok) {
            return refuse("requires_auth", "mandate-cap-exceeded", "remaining mandate budget covering this amount", "Cumulative spend would exceed the mandate cap, or this reservation already exists.");
        }
        reserved = { id: proofs.mandate.id, ref };
    }
    if (proofs.confirmation) {
        const fresh = await consumed.consume(proofs.confirmation.jti, proofs.confirmation.exp, req.now);
        if (!fresh) {
            if (reserved) await ledger.settle(reserved.id, reserved.ref, "released");
            return refuse("requires_confirmation", "confirmation-reused", "a fresh single-use confirmation", "This confirmation was already used.");
        }
    }
    const res = reserved;
    let settled = false;
    return {
        ok: true,
        request: req,
        neverRetry: d.neverRetry,
        mandateId: proofs.mandate?.id ?? null,
        confirmationId: proofs.confirmation?.jti ?? null,
        async settle(outcome) {
            if (settled) return; // one-shot: a second settle can never undo the first
            settled = true;
            if (res) await ledger.settle(res.id, res.ref, outcome);
        },
    };
}

/**
 * Executor obligation: the host a financial request is actually sent to must be the
 * merchant that was authorized. Call this with the egress decision's host.
 */
export function hostMatchesMerchant(egressHost: string, merchant: string): boolean {
    return normalizeHost(egressHost) === normalizeHost(merchant);
}

export { normalizeHost };
