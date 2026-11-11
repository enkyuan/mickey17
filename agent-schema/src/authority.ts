// D11 authority over money: a mandate scopes what may be spent, a confirmation shows
// a human what will happen, a receipt records what did. No secret value appears in any
// of these shapes: credentials and tokenized instruments are supplied at run time (D7).
import { z } from "zod";
import { registry } from "./registry";
import { PageText, SurfaceText } from "./origin";
import {
    ActionId,
    ConfirmationId,
    Currency,
    FlowId,
    Host,
    Name,
    MandateId,
    Money,
    ReceiptId,
    Sha256,
    StepId,
    Timestamp,
} from "./primitives";
import { Effect, Mode, Tier } from "./policy";
import { OperationStatus } from "./result";
import { HttpMethod, PagePath, ResponseClass } from "./values";

/** Opaque caller-generated key. Not a credential. */
export const IdempotencyKey = z
    .string()
    .regex(/^[A-Za-z0-9_-]{16,128}$/)
    .register(registry, { id: "IdempotencyKey" });

/**
 * A mandate: scoped, expiring authority to spend (D11). A session alone is not enough.
 * It names what may be called and where, never a credential or instrument.
 */
export const Mandate = z
    .strictObject({
        id: MandateId,
        /** Opaque identifier of whoever granted it. Not a name, email or account number. */
        grantor: Name,
        mode: z.enum(["owner", "delegated"]),
        merchants: z.array(Host).min(1).max(64),
        currency: Currency,
        spend_cap_minor: z.int().min(0).max(Number.MAX_SAFE_INTEGER),
        per_call_cap_minor: z.int().min(0).max(Number.MAX_SAFE_INTEGER).optional(),
        scope: z
            .array(
                z.discriminatedUnion("kind", [
                    z.strictObject({ kind: z.literal("action"), id: ActionId }),
                    z.strictObject({ kind: z.literal("flow"), id: FlowId }),
                ]),
            )
            .min(1)
            .max(256),
        issued_at: Timestamp,
        expires_at: Timestamp,
        /** sha256 of the canonical mandate with `hash` omitted (see hash.ts). */
        hash: Sha256,
    })
    .register(registry, { id: "Mandate" });
export type Mandate = z.infer<typeof Mandate>;

/** What the call is for: an action, or one step of a flow. Exactly one of the two shapes. */
export const Subject = z
    .union([
        z.strictObject({ action: ActionId }),
        z.strictObject({ flow: FlowId, step: StepId }),
    ])
    .register(registry, { id: "Subject" });
export type Subject = z.infer<typeof Subject>;

const confirmationBase = {
    id: ConfirmationId,
    subject: Subject,
    /** Binds the confirmation to one exact request: a tampered total or cart id changes it. */
    request_digest: Sha256,
    idempotency_key: IdempotencyKey,
    issued_at: Timestamp,
    expires_at: Timestamp,
};

/**
 * What a human is shown before a call that is not a plain read (D5, D11). For money:
 * item, total, currency and payee, always. The surface never auto-confirms.
 */
export const ConfirmationRequest = z
    .discriminatedUnion("effect", [
        z.strictObject({
            ...confirmationBase,
            effect: z.literal("financial"),
            items: z
                .array(
                    z.strictObject({
                        name: PageText,
                        quantity: z.int().min(1).max(1_000_000),
                        /** Unit price. The human sees every price, so it is never optional. */
                        amount: Money,
                    }),
                )
                .min(1)
                .max(256),
            total: Money,
            payee: PageText,
            /** A session alone is not enough to move money (D11). */
            mandate: MandateId,
        }),
        z.strictObject({
            ...confirmationBase,
            effect: z.enum(["write-irreversible", "account-control"]),
            /** Authored by the surface; the page's own wording, if shown, goes in `page_context`. */
            summary: SurfaceText,
            page_context: PageText.optional(),
        }),
    ])
    .register(registry, { id: "ConfirmationRequest" });
export type ConfirmationRequest = z.infer<typeof ConfirmationRequest>;

/**
 * The caller's per-call grant (D5). It carries no secret: the executor accepts it only if
 * it matches an outstanding request by id and digest, once, before `expires_at`.
 */
export const ConfirmationToken = z
    .strictObject({
        confirmation: ConfirmationId,
        request_digest: Sha256,
        idempotency_key: IdempotencyKey,
        granted_at: Timestamp,
    })
    .register(registry, { id: "ConfirmationToken" });
export type ConfirmationToken = z.infer<typeof ConfirmationToken>;

/** Operability (section 6): what every executed action records, with secrets redacted. */
export const Receipt = z
    .strictObject({
        id: ReceiptId,
        at: Timestamp,
        mode: Mode,
        /** T4 is unsupported and never executes. */
        tier: z.enum(["T0", "T1", "T2", "T3"]),
        effect: Effect,
        subject: Subject,
        /** Hashes of the evidence the action rested on. */
        evidence: z.array(Sha256).max(64),
        /** The request class: method, host and path template. No query values, no bodies. */
        request: z.strictObject({ method: HttpMethod, host: Host, path: PagePath }),
        result: OperationStatus,
        response_class: ResponseClass.optional(),
        duration_ms: z.int().min(0).max(Number.MAX_SAFE_INTEGER),
        mandate: MandateId.optional(),
        confirmation: ConfirmationId.optional(),
        idempotency_key: IdempotencyKey.optional(),
        /** Amount moved, when the call moved money. */
        amount: Money.optional(),
        /** How many values were redacted from this record. */
        redactions: z.int().min(0).max(1_000_000),
    })
    .register(registry, { id: "Receipt" });
export type Receipt = z.infer<typeof Receipt>;
