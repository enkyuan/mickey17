// D11: a transactional flow is an ordered sequence of steps with server-side state,
// plus the data dependencies between steps. Every request value carries a class
// (constant / input / derived / client-computed); an unclassifiable value blocks the flow.
import { z } from "zod";
import { registry } from "./registry";
import { SurfaceText } from "./origin";
import { ActionId, FlowId, Sha256, StepId, RequestId } from "./primitives";
import { Effect } from "./policy";
import { HumanReason } from "./result";
import { AuthRequirement, Evidence, Idempotency, Input, Postcondition, Precondition, StrongEvidence } from "./action";
import { PaymentMethod, RequestLocation, RequestSpec } from "./values";

export const FlowStep = z
    .strictObject({
        id: StepId,
        /** The action this step performs, when it corresponds to one. */
        action: ActionId.optional(),
        effect: Effect,
        requests: z.array(RequestSpec).min(1).max(64),
        preconditions: z.array(Precondition).max(64),
        postconditions: z.array(Postcondition).max(64),
        /** D5: a non-idempotent step is never retried. A financial step names where its idempotency key goes. */
        idempotency: Idempotency,
        /** Present on the step that moves money, and only there. */
        payment: PaymentMethod.optional(),
        /** Challenges the site may raise at this step. Reported as `requires_human`, never emulated. */
        human_gates: z.array(HumanReason).max(8),
    })
    .register(registry, { id: "FlowStep" });
export type FlowStep = z.infer<typeof FlowStep>;

/** Where a flow was recorded. Recording happens only on owned or staging sites, in test mode (D11). */
export const Recording = z
    .strictObject({
        environment: z.enum(["owner-local", "staging", "payment-test-mode"]),
        hash: Sha256,
    })
    .register(registry, { id: "Recording" });

/** A request value the recorder could not class. It blocks the flow; it is never guessed. */
export const UnclassifiedValue = z
    .strictObject({
        step: StepId,
        request: RequestId,
        at: RequestLocation,
        reason: z.enum(["no-source", "ambiguous-source", "multiple-sources"]),
    })
    .register(registry, { id: "UnclassifiedValue" });
export type UnclassifiedValue = z.infer<typeof UnclassifiedValue>;

const flowBase = {
    id: FlowId,
    description: SurfaceText,
    inputs: z.array(Input).max(256),
    auth: AuthRequirement,
    /** Equal to the most consequential step effect (checked in code). */
    effect: Effect,
    steps: z.array(FlowStep).min(1).max(256),
    recorded: Recording,
    /** First element declared or observed (D5/D11); emitters order evidence strongest first. */
    evidence: z.tuple([StrongEvidence], Evidence),
};

/** A flow every request value of which is classed. Executable. */
export const ReadyFlow = z
    .strictObject({ ...flowBase, status: z.literal("ready") })
    .register(registry, { id: "ReadyFlow" });

/** A flow with at least one unclassified value. Listed, not executable. */
export const BlockedFlow = z
    .strictObject({
        ...flowBase,
        status: z.literal("blocked"),
        unclassified: z.array(UnclassifiedValue).min(1).max(512),
    })
    .register(registry, { id: "BlockedFlow" });

export const Flow = z.discriminatedUnion("status", [ReadyFlow, BlockedFlow]).register(registry, { id: "Flow" });
export type Flow = z.infer<typeof Flow>;
