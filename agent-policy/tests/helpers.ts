import { InMemoryConsumedStore } from "../src/confirm.ts";
import { generateEd25519, hmacSigner, type Signer, type Verifier } from "../src/sign.ts";
import { InMemorySpendLedger, issueMandate, type MandatePayload } from "../src/mandate.ts";
import { issueConfirmation, type ConfirmationBinding } from "../src/confirm.ts";
import type { ActionRequest, AuthorizeDeps } from "../src/policy.ts";

export const NOW = 1_800_000_000_000;
export const HOUR = 3_600_000;

export async function keys() {
    const ed = await generateEd25519();
    const hmac = await hmacSigner(crypto.getRandomValues(new Uint8Array(32)));
    return { ed, hmac };
}

export interface World {
    deps: AuthorizeDeps;
    mandateSigner: Signer;
    mandateVerifier: Verifier;
    confirmSigner: Signer;
    ledger: InMemorySpendLedger;
    consumed: InMemoryConsumedStore;
}

export async function world(): Promise<World> {
    const { ed, hmac } = await keys();
    const ledger = new InMemorySpendLedger();
    const consumed = new InMemoryConsumedStore();
    return {
        mandateSigner: ed.signer,
        mandateVerifier: ed.verifier,
        confirmSigner: hmac,
        ledger,
        consumed,
        deps: { mandateVerifier: ed.verifier, confirmationVerifier: hmac, ledger, consumed },
    };
}

export const baseMandate: MandatePayload = {
    v: 1,
    id: "mandate-1",
    issuer: "user-1",
    subject: "agent-1",
    merchants: ["shop.example.com"],
    actions: ["act_pay"],
    currency: "USD",
    capMinor: 10_000,
    perTxCapMinor: 6_000,
    notBefore: NOW - HOUR,
    expiresAt: NOW + HOUR,
};

export const mandateToken = (w: World, over: Partial<MandatePayload> = {}) => issueMandate({ ...baseMandate, ...over }, w.mandateSigner);

export const moneyReq = (over: Partial<ActionRequest> = {}): ActionRequest => ({
    actionId: "act_pay",
    mode: "delegated",
    tier: "T2",
    effect: "financial",
    evidence: "observed",
    idempotent: true,
    idempotencyKey: "idem-key-0001",
    credential: { kind: "oauth", scopes: ["financial"] },
    money: { egressHost: "shop.example.com", merchant: "shop.example.com", item: "sku-42", totalMinor: 4_999, currency: "USD", payee: "shop.example.com" },
    now: NOW,
    ...over,
});

export const bindingOf = (r: ActionRequest): ConfirmationBinding =>
    r.money
        ? { actionId: r.actionId, item: r.money.item, totalMinor: r.money.totalMinor, currency: r.money.currency, payee: r.money.payee }
        : { actionId: r.actionId, item: r.item ?? null, totalMinor: null, currency: null, payee: null };

export const confirmToken = (w: World, r: ActionRequest, over: Partial<ConfirmationBinding> = {}, jti = `c-${crypto.randomUUID()}`) =>
    issueConfirmation(w.confirmSigner, { ...bindingOf(r), ...over }, { jti, now: NOW - 1000, ttlMs: 5 * 60_000 });
