import { sha256Hex } from "./bytes.ts";
import type { Redactor } from "./redact.ts";
import type { Effect, Mode, Status, Tier } from "./types.ts";

export interface ReceiptInput {
    readonly id: string;
    readonly at: number;
    readonly mode: Mode;
    readonly tier: Tier;
    readonly effect: Effect;
    readonly actionId: string;
    readonly evidenceIds: readonly string[];
    readonly requestClass: string;
    readonly engine: "http" | "js-runtime";
    readonly host: string;
    readonly result: Status;
    readonly resultCode?: string;
    readonly durationMs: number;
    readonly mandateId?: string | null;
    readonly confirmationId?: string | null;
    readonly idempotencyKey?: string;
    readonly money?: { readonly totalMinor: number; readonly currency: string; readonly payee: string };
    readonly error?: unknown;
}

export interface Receipt {
    readonly v: 1;
    readonly id: string;
    readonly at: number;
    readonly mode: Mode;
    readonly tier: Tier;
    readonly effect: Effect;
    readonly actionId: string;
    readonly evidenceIds: readonly string[];
    readonly requestClass: string;
    readonly engine: string;
    readonly host: string;
    readonly result: Status;
    readonly resultCode: string | null;
    readonly durationMs: number;
    readonly mandateId: string | null;
    readonly confirmationId: string | null;
    /** Digest only: the key itself may be guessable or sensitive. */
    readonly idempotencyKeyDigest: string | null;
    readonly money: { readonly totalMinor: number; readonly currency: string; readonly payee: string } | null;
    readonly error: { readonly name: string; readonly message: string } | null;
}

/**
 * Built field by field from an allowlist: nothing the caller attaches that is not named
 * here (headers, bodies, cookies, query strings) can reach a receipt. Every string then
 * passes the redactor as a second line of defence.
 */
export async function buildReceipt(i: ReceiptInput, redactor: Redactor): Promise<Receipt> {
    const r = (s: string) => redactor.redactString(s);
    const draft: Receipt = {
        v: 1,
        id: r(i.id),
        at: i.at,
        mode: i.mode,
        tier: i.tier,
        effect: i.effect,
        actionId: r(i.actionId),
        evidenceIds: i.evidenceIds.map(r),
        requestClass: r(i.requestClass),
        engine: i.engine,
        host: r(i.host),
        result: i.result,
        resultCode: i.resultCode === undefined ? null : r(i.resultCode),
        durationMs: i.durationMs,
        mandateId: i.mandateId == null ? null : r(i.mandateId),
        confirmationId: i.confirmationId == null ? null : r(i.confirmationId),
        idempotencyKeyDigest: i.idempotencyKey === undefined ? null : (await sha256Hex(i.idempotencyKey)).slice(0, 16),
        money: i.money ? { totalMinor: i.money.totalMinor, currency: r(i.money.currency), payee: r(i.money.payee) } : null,
        error: i.error === undefined ? null : redactor.errorToJson(i.error),
    };
    return Object.freeze(draft);
}
