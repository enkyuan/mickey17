import { isSafePosInt } from "./bytes.ts";
import { openToken, signToken, type Signer, type Verifier } from "./sign.ts";
import { refuse, type Refusal } from "./types.ts";

export const CONFIRMATION_DOMAIN = "mochi-agent-policy/confirmation/v1";
const DEFAULT_MAX_TTL_MS = 15 * 60 * 1000;

/**
 * What the human saw and approved (D5, D11): the action, the item, the total, the
 * currency and the payee. Fields that do not apply (no money) are explicit nulls and
 * must be null on both sides, so a token cannot be widened by omission.
 */
export interface ConfirmationBinding {
    readonly actionId: string;
    readonly item: string | null;
    readonly totalMinor: number | null;
    readonly currency: string | null;
    readonly payee: string | null;
}

export interface ConfirmationPayload extends ConfirmationBinding {
    readonly v: 1;
    readonly jti: string;
    readonly iat: number;
    readonly exp: number;
}

const verifiedConfirmations = new WeakSet<object>();
export type VerifiedConfirmation = Readonly<ConfirmationPayload> & { readonly __verified: true };
export function isVerifiedConfirmation(x: unknown): x is VerifiedConfirmation {
    return typeof x === "object" && x !== null && verifiedConfirmations.has(x);
}

const ID = /^[A-Za-z0-9._:-]{1,128}$/;
const KEYS = ["v", "jti", "actionId", "item", "totalMinor", "currency", "payee", "iat", "exp"] as const;
const BOUND = ["actionId", "item", "totalMinor", "currency", "payee"] as const;

function bindingOk(b: ConfirmationBinding): boolean {
    const strOrNull = (v: unknown, max: number) => v === null || (typeof v === "string" && v.length > 0 && v.length <= max);
    return (
        typeof b.actionId === "string" && ID.test(b.actionId) &&
        strOrNull(b.item, 256) &&
        (b.totalMinor === null || isSafePosInt(b.totalMinor)) &&
        (b.currency === null || (typeof b.currency === "string" && /^[A-Z]{3}$/.test(b.currency))) &&
        strOrNull(b.payee, 253) &&
        // money fields travel together
        (b.totalMinor === null) === (b.currency === null) &&
        (b.totalMinor === null) === (b.payee === null)
    );
}

export async function issueConfirmation(
    signer: Signer,
    binding: ConfirmationBinding,
    opts: { jti: string; now: number; ttlMs: number },
): Promise<string> {
    if (!bindingOk(binding) || !ID.test(opts.jti)) throw new RangeError("invalid confirmation binding");
    const payload: ConfirmationPayload = { v: 1, jti: opts.jti, iat: opts.now, exp: opts.now + opts.ttlMs, ...binding };
    return signToken(CONFIRMATION_DOMAIN, payload, signer);
}

export async function verifyConfirmation(
    token: unknown,
    verifier: Verifier,
    expected: ConfirmationBinding,
    opts: { now: number; maxTtlMs?: number },
): Promise<{ ok: true; confirmation: VerifiedConfirmation } | Refusal> {
    const need = "a confirmation for exactly this action, item, total, currency and payee";
    const opened = await openToken(CONFIRMATION_DOMAIN, token, verifier);
    if (!opened.ok) {
        return refuse("requires_confirmation", opened.code === "malformed" ? "confirmation-malformed" : "confirmation-bad-signature", need,
            opened.code === "malformed" ? "Confirmation token is malformed." : "Confirmation token signature does not verify.");
    }
    const o = opened.payload;
    if (typeof o !== "object" || o === null || Array.isArray(o)) return refuse("requires_confirmation", "confirmation-malformed", need, "Confirmation payload is malformed.");
    const r = o as Record<string, unknown>;
    if (Object.keys(r).length !== KEYS.length || !KEYS.every((k) => k in r) || r.v !== 1) {
        return refuse("requires_confirmation", "confirmation-malformed", need, "Confirmation payload is malformed.");
    }
    const p = r as unknown as ConfirmationPayload;
    if (typeof p.jti !== "string" || !ID.test(p.jti) || !isSafePosInt(p.iat) || !isSafePosInt(p.exp) || p.exp <= p.iat || !bindingOk(p)) {
        return refuse("requires_confirmation", "confirmation-malformed", need, "Confirmation payload is malformed.");
    }
    if (p.exp - p.iat > (opts.maxTtlMs ?? DEFAULT_MAX_TTL_MS)) return refuse("requires_confirmation", "confirmation-ttl-too-long", need, "Confirmation lifetime exceeds the allowed maximum.");
    if (opts.now < p.iat) return refuse("requires_confirmation", "confirmation-not-yet-valid", need, "Confirmation is not yet valid.");
    if (opts.now >= p.exp) return refuse("requires_confirmation", "confirmation-expired", "an unexpired confirmation", "Confirmation has expired.");
    for (const k of BOUND) {
        if (p[k] !== expected[k]) {
            return refuse("requires_confirmation", "confirmation-binding-mismatch", `a confirmation bound to this call's ${k}`, `Confirmation was issued for a different ${k}.`);
        }
    }
    const v = Object.freeze({ ...p, __verified: true as const });
    verifiedConfirmations.add(v);
    return { ok: true, confirmation: v };
}

/** Single use. `consume` must be atomic: true for the first caller only. */
export interface ConsumedStore {
    consume(jti: string, expiresAt: number, now: number): Promise<boolean>;
}

export class InMemoryConsumedStore implements ConsumedStore {
    readonly #seen = new Map<string, number>();
    consume(jti: string, expiresAt: number, now: number): Promise<boolean> {
        for (const [k, exp] of this.#seen) if (exp <= now) this.#seen.delete(k);
        if (this.#seen.has(jti)) return Promise.resolve(false);
        this.#seen.set(jti, expiresAt);
        return Promise.resolve(true);
    }
}

/** Re-check of a verified confirmation against the call about to run. Pure. */
export function isConfirmationExpiredOrMismatched(c: VerifiedConfirmation, expected: ConfirmationBinding, now: number): Refusal | null {
    if (now < c.iat) return refuse("requires_confirmation", "confirmation-not-yet-valid", "a currently valid confirmation", "Confirmation is not yet valid.");
    if (now >= c.exp) return refuse("requires_confirmation", "confirmation-expired", "an unexpired confirmation", "Confirmation has expired.");
    for (const k of BOUND) {
        if (c[k] !== expected[k]) return refuse("requires_confirmation", "confirmation-binding-mismatch", `a confirmation bound to this call's ${k}`, `Confirmation was issued for a different ${k}.`);
    }
    return null;
}
