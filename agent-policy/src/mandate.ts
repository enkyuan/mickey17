import { isSafePosInt } from "./bytes.ts";
import { openToken, signToken, type Signer, type Verifier } from "./sign.ts";
import { refuse, type Refusal } from "./types.ts";

export const MANDATE_DOMAIN = "mochi-agent-policy/mandate/v1";
const DEFAULT_MAX_LIFETIME_MS = 7 * 24 * 3600 * 1000;

/**
 * A mandate is the unit of authority over money (D11): scoped to named actions, bound
 * to a merchant allowlist, one currency, a cumulative cap and an optional per-call cap,
 * valid for a window. Amounts are integers in the currency's minor unit.
 */
export interface MandatePayload {
    readonly v: 1;
    readonly id: string;
    readonly issuer: string;
    readonly subject: string;
    /** Exact lowercase ASCII hostnames. No wildcards: a mandate names who may be paid. */
    readonly merchants: readonly string[];
    /** Exact action ids. No wildcards. */
    readonly actions: readonly string[];
    readonly currency: string;
    readonly capMinor: number;
    readonly perTxCapMinor: number | null;
    readonly notBefore: number;
    readonly expiresAt: number;
}

const verifiedMandates = new WeakSet<object>();

/** Only verifyMandate can produce one; a hand-built object is not accepted by decide(). */
export type VerifiedMandate = Readonly<MandatePayload> & { readonly __verified: true };

export function isVerifiedMandate(x: unknown): x is VerifiedMandate {
    return typeof x === "object" && x !== null && verifiedMandates.has(x);
}

const ID = /^[A-Za-z0-9._:-]{1,128}$/;
const HOST = /^(?=.{1,253}$)([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)*$/;
const CURRENCY = /^[A-Z]{3}$/;
const KEYS = [
    "v", "id", "issuer", "subject", "merchants", "actions", "currency", "capMinor", "perTxCapMinor", "notBefore", "expiresAt",
] as const;

export function normalizeHost(h: string): string {
    return h.toLowerCase().replace(/\.$/, "");
}

function parsePayload(p: unknown): MandatePayload | null {
    if (typeof p !== "object" || p === null || Array.isArray(p)) return null;
    const o = p as Record<string, unknown>;
    const keys = Object.keys(o);
    if (keys.length !== KEYS.length || !KEYS.every((k) => k in o)) return null; // unknown or missing key: fail closed
    if (o.v !== 1) return null;
    if (typeof o.id !== "string" || !ID.test(o.id)) return null;
    if (typeof o.issuer !== "string" || !ID.test(o.issuer)) return null;
    if (typeof o.subject !== "string" || !ID.test(o.subject)) return null;
    const strList = (v: unknown, re: RegExp): string[] | null =>
        Array.isArray(v) && v.length > 0 && v.length <= 64 && v.every((x) => typeof x === "string" && re.test(x))
            ? (v as string[])
            : null;
    const merchants = strList(o.merchants, HOST);
    const actions = strList(o.actions, ID);
    if (!merchants || !actions) return null;
    if (typeof o.currency !== "string" || !CURRENCY.test(o.currency)) return null;
    if (!isSafePosInt(o.capMinor)) return null;
    if (o.perTxCapMinor !== null && !isSafePosInt(o.perTxCapMinor)) return null;
    if (o.perTxCapMinor !== null && (o.perTxCapMinor as number) > (o.capMinor as number)) return null;
    if (!isSafePosInt(o.notBefore) || !isSafePosInt(o.expiresAt) || (o.expiresAt as number) <= (o.notBefore as number)) {
        return null;
    }
    return {
        v: 1,
        id: o.id,
        issuer: o.issuer,
        subject: o.subject,
        merchants,
        actions,
        currency: o.currency,
        capMinor: o.capMinor as number,
        perTxCapMinor: o.perTxCapMinor as number | null,
        notBefore: o.notBefore as number,
        expiresAt: o.expiresAt as number,
    };
}

export async function issueMandate(payload: MandatePayload, signer: Signer): Promise<string> {
    return signToken(MANDATE_DOMAIN, payload, signer);
}

export interface MandateVerifyOptions {
    readonly now: number;
    readonly maxLifetimeMs?: number;
    readonly revoked?: ReadonlySet<string>;
}

export async function verifyMandate(
    token: unknown,
    verifier: Verifier,
    opts: MandateVerifyOptions,
): Promise<{ ok: true; mandate: VerifiedMandate } | Refusal> {
    const need = "a valid signed mandate";
    const opened = await openToken(MANDATE_DOMAIN, token, verifier);
    if (!opened.ok) {
        return refuse("requires_auth", opened.code === "malformed" ? "mandate-malformed" : "mandate-bad-signature", need,
            opened.code === "malformed" ? "Mandate is malformed." : "Mandate signature does not verify against the pinned key.");
    }
    const p = parsePayload(opened.payload);
    if (!p) return refuse("requires_auth", "mandate-malformed", need, "Mandate payload is malformed.");
    if (opts.now < p.notBefore) return refuse("requires_auth", "mandate-not-yet-valid", need, "Mandate is not yet valid.");
    if (opts.now >= p.expiresAt) return refuse("requires_auth", "mandate-expired", "an unexpired mandate", "Mandate has expired.");
    if (p.expiresAt - p.notBefore > (opts.maxLifetimeMs ?? DEFAULT_MAX_LIFETIME_MS)) {
        return refuse("requires_auth", "mandate-lifetime-too-long", need, "Mandate lifetime exceeds the allowed maximum.");
    }
    if (opts.revoked?.has(p.id)) return refuse("requires_auth", "mandate-revoked", need, "Mandate has been revoked.");
    const v = Object.freeze({ ...p, merchants: Object.freeze([...p.merchants]), actions: Object.freeze([...p.actions]), __verified: true as const });
    verifiedMandates.add(v);
    return { ok: true, mandate: v };
}

export interface SpendRequest {
    readonly actionId: string;
    readonly merchant: string;
    /** Who receives the money. Must also be on the allowlist (and equal the merchant, see policy.ts). */
    readonly payee: string;
    readonly currency: string;
    readonly amountMinor: number;
}

/** Pure per-call checks; cumulative spend is enforced atomically by the ledger. */
export function checkMandateScope(m: VerifiedMandate, s: SpendRequest, now: number): Refusal | null {
    if (!isVerifiedMandate(m)) return refuse("requires_auth", "mandate-unverified", "a verified mandate", "Mandate was not produced by verification.");
    if (now < m.notBefore) return refuse("requires_auth", "mandate-not-yet-valid", "a currently valid mandate", "Mandate is not yet valid.");
    if (now >= m.expiresAt) return refuse("requires_auth", "mandate-expired", "an unexpired mandate", "Mandate has expired.");
    if (!isSafePosInt(s.amountMinor)) return refuse("blocked", "mandate-amount-invalid", "a positive integer amount in minor units", "Amount is not a positive integer.");
    if (!m.actions.includes(s.actionId)) return refuse("requires_auth", "mandate-action-not-scoped", "a mandate scoped to this action", "Mandate does not cover this action.");
    if (!m.merchants.includes(normalizeHost(s.merchant))) return refuse("requires_auth", "mandate-merchant-not-allowed", "a mandate whose allowlist names this merchant", "Merchant is not on the mandate allowlist.");
    if (!m.merchants.includes(normalizeHost(s.payee))) return refuse("requires_auth", "mandate-payee-not-allowed", "a mandate whose allowlist names this payee", "Payee is not on the mandate allowlist.");
    if (s.currency !== m.currency) return refuse("requires_auth", "mandate-currency-mismatch", "a mandate in the call's currency", "Currency differs from the mandate currency.");
    if (m.perTxCapMinor !== null && s.amountMinor > m.perTxCapMinor) return refuse("requires_auth", "mandate-per-tx-cap-exceeded", "an amount within the mandate's per-call cap", "Amount exceeds the per-call cap.");
    if (s.amountMinor > m.capMinor) return refuse("requires_auth", "mandate-cap-exceeded", "an amount within the mandate cap", "Amount exceeds the mandate cap.");
    return null;
}

export type SettleOutcome = "committed" | "released";

/**
 * Cumulative spend, per mandate. `reserve` must be atomic with respect to other
 * reserves on the same mandate. An ambiguous outcome (request sent, result unknown)
 * stays reserved: over-counting is the safe direction.
 */
export interface SpendLedger {
    reserve(mandateId: string, ref: string, amountMinor: number, capMinor: number): Promise<{ ok: true; spentMinor: number } | { ok: false; spentMinor: number }>;
    settle(mandateId: string, ref: string, outcome: SettleOutcome): Promise<void>;
    spent(mandateId: string): Promise<number>;
}

export class InMemorySpendLedger implements SpendLedger {
    readonly #m = new Map<string, Map<string, { amount: number; state: "reserved" | "committed" }>>();

    #total(id: string): number {
        let t = 0;
        for (const e of this.#m.get(id)?.values() ?? []) t += e.amount;
        return t;
    }

    reserve(mandateId: string, ref: string, amountMinor: number, capMinor: number) {
        // No await between the check and the write: atomic on the single JS thread.
        const spent = this.#total(mandateId);
        const entries = this.#m.get(mandateId) ?? new Map();
        if (!isSafePosInt(amountMinor) || entries.has(ref) || spent + amountMinor > capMinor || !Number.isSafeInteger(spent + amountMinor)) {
            return Promise.resolve({ ok: false as const, spentMinor: spent });
        }
        entries.set(ref, { amount: amountMinor, state: "reserved" });
        this.#m.set(mandateId, entries);
        return Promise.resolve({ ok: true as const, spentMinor: spent + amountMinor });
    }

    settle(mandateId: string, ref: string, outcome: SettleOutcome): Promise<void> {
        const e = this.#m.get(mandateId)?.get(ref);
        // Only a reservation can be released; a committed spend is permanent.
        if (e && e.state === "reserved") {
            if (outcome === "released") this.#m.get(mandateId)?.delete(ref);
            else e.state = "committed";
        }
        return Promise.resolve();
    }

    spent(mandateId: string): Promise<number> {
        return Promise.resolve(this.#total(mandateId));
    }
}
