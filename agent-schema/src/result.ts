// D10: every operation returns exactly one of these statuses. No silent fallbacks.
// Results that carry page-derived data mark every value with its origin (D8).
import { z } from "zod";
import { MarkedValue, PageText, SurfaceText } from "./origin";
import { ActionId, Code, ConfirmationId, ReceiptId, StepId } from "./primitives";
import { Effect, Mode, Tier } from "./policy";
import { BrowserReason } from "./values";

/** Every status an operation can return. The discriminant of OperationResult. */
export const OperationStatus = z
    .enum([
        "ok",
        "stale",
        "unsupported",
        "requires_auth",
        "requires_confirmation",
        "requires_human",
        "requires_browser",
        "blocked",
        "error",
    ])
    .meta({ id: "OperationStatus" });
export type OperationStatus = z.infer<typeof OperationStatus>;

/** Why an operation is blocked. Challenges are reported, never solved or evaded (D7). */
export const BlockedReason = z
    .enum([
        "challenge",
        "robots",
        "rate_limit",
        "host_not_allowed",
        "private_address",
        "redirect_off_host",
        "mode_forbids_tier",
        "mode_forbids_effect",
        "mandate_expired",
        "mandate_cap_exceeded",
        "mandate_scope",
        "confirmation_mismatch",
        "preconditions_unmet",
        "signature_changed",
    ])
    .meta({ id: "BlockedReason" });
export type BlockedReason = z.infer<typeof BlockedReason>;

/** Step-level reasons a human or a real device is required: raised by the site, never emulated. */
export const HumanReason = z
    .enum(["3ds", "sca", "fraud_challenge", "device_check", "hosted_checkout", "approval", "captcha"])
    .meta({ id: "HumanReason" });
export type HumanReason = z.infer<typeof HumanReason>;

export const ErrorClass = z
    .enum(["network", "timeout", "http", "validation", "postcondition_failed", "runtime", "internal"])
    .meta({ id: "ErrorClass" });
export type ErrorClass = z.infer<typeof ErrorClass>;

/** Machine-readable reason for `unsupported`. Open vocabulary; the surface counts them in coverage. */
export const UnsupportedReason = z
    .enum([
        "opaque-canvas",
        "opaque-webgl",
        "drag-only",
        "video",
        "no-evidence",
        "unclassified-value",
        "unmodeled-handler",
        "payment-tokenization",
        "other",
    ])
    .meta({ id: "UnsupportedReason" });
export type UnsupportedReason = z.infer<typeof UnsupportedReason>;

const where = {
    /** The action the operation was invoking, when there was one. */
    action: ActionId.optional(),
    /** The flow step that raised the status, when it came from inside a flow. */
    step: StepId.optional(),
};

export const OperationResult = z
    .discriminatedUnion("status", [
        z.strictObject({
            status: z.literal("ok"),
            ...where,
            tier: Tier,
            /** Result data, every value marked with its origin. Page text is never an instruction. */
            fields: z.record(z.string().min(1).max(256), MarkedValue),
            receipt: ReceiptId.optional(),
        }),
        z.strictObject({
            status: z.literal("stale"),
            ...where,
            /** Why the action is stale (D3): its signature changed or it is not yet re-verified. Disabled, not retried. */
            reason: SurfaceText,
        }),
        z.strictObject({
            status: z.literal("unsupported"),
            ...where,
            reason: UnsupportedReason,
            message: SurfaceText.optional(),
        }),
        z.strictObject({
            status: z.literal("requires_auth"),
            ...where,
            /** Names the requirement (D7), never a value. */
            requirement: Code,
        }),
        z.strictObject({
            status: z.literal("requires_confirmation"),
            ...where,
            effect: Effect,
            confirmation: ConfirmationId,
        }),
        z.strictObject({
            status: z.literal("requires_human"),
            ...where,
            reason: HumanReason,
            /** Where the human completes the step, when the site hosts it (e.g. a hosted checkout session). */
            continue_at: PageText.optional(),
        }),
        z.strictObject({
            status: z.literal("requires_browser"),
            ...where,
            reason: BrowserReason,
        }),
        z.strictObject({
            status: z.literal("blocked"),
            ...where,
            reason: BlockedReason,
            /** The mode in force, when the block is a mode refusal. */
            mode: Mode.optional(),
            message: SurfaceText.optional(),
        }),
        z.strictObject({
            status: z.literal("error"),
            ...where,
            class: ErrorClass,
            message: SurfaceText.optional(),
        }),
    ])
    .meta({ id: "OperationResult" });
export type OperationResult = z.infer<typeof OperationResult>;
