// D11: a transactional flow is an ordered sequence of steps with server-side state,
// plus the data dependencies between steps. Every request value carries a class
// (constant / input / derived / client-computed); an unclassifiable value blocks the flow.
import { z } from "zod";
import { SurfaceText } from "./origin";
import { ActionId, FlowId, Sha256, StepId, RequestId } from "./primitives";
import { Effect } from "./policy";
import { HumanReason } from "./result";
import { AuthRequirement, Evidence, Input, Postcondition, StrongEvidence } from "./action";
import { RequestLocation, RequestSpec } from "./values";

/** How the payment step obtains authority to pay (D11). Card data never passes through the surface. */
export const PaymentMethod = z
    .enum([
        /** The caller supplies an already tokenized or delegated credential. */
        "caller-token",
        /** A hosted checkout session the human completes or approves. */
        "hosted-session",
        /** A merchant-supported agent protocol (ACP, AP2 mandates, network agent tokens). */
        "agent-protocol",
    ])
    .meta({ id: "PaymentMethod" });
export type PaymentMethod = z.infer<typeof PaymentMethod>;

export const FlowStep = z
    .strictObject({
        id: StepId,
        /** The action this step performs, when it corresponds to one. */
        action: ActionId.optional(),
        effect: Effect,
        requests: z.array(RequestSpec).min(1).max(64),
        postconditions: z.array(Postcondition).max(64),
        /** Present on the step that moves money. */
        payment: PaymentMethod.optional(),
        /** Challenges the site may raise at this step. Reported as `requires_human`, never emulated. */
        human_gates: z.array(HumanReason).max(8),
    })
    .meta({ id: "FlowStep" });
export type FlowStep = z.infer<typeof FlowStep>;

/** Where a flow was recorded. Recording happens only on owned or staging sites, in test mode (D11). */
export const Recording = z
    .strictObject({
        environment: z.enum(["owner-local", "staging", "payment-test-mode"]),
        hash: Sha256,
    })
    .meta({ id: "Recording" });

/** A request value the recorder could not class. It blocks the flow; it is never guessed. */
export const UnclassifiedValue = z
    .strictObject({
        step: StepId,
        request: RequestId,
        at: RequestLocation,
        reason: z.enum(["no-source", "ambiguous-source", "multiple-sources"]),
    })
    .meta({ id: "UnclassifiedValue" });
export type UnclassifiedValue = z.infer<typeof UnclassifiedValue>;

const flowBase = {
    id: FlowId,
    description: SurfaceText,
    inputs: z.array(Input).max(256),
    auth: AuthRequirement,
    /** At least the most consequential step effect (checked in code). */
    effect: Effect,
    steps: z.array(FlowStep).min(1).max(256),
    recorded: Recording,
    /** First element declared or observed (D5/D11); emitters order evidence strongest first. */
    evidence: z.tuple([StrongEvidence], Evidence),
};

/** A flow every request value of which is classed. Executable. */
export const ReadyFlow = z
    .strictObject({ ...flowBase, status: z.literal("ready") })
    .meta({ id: "ReadyFlow" });

/** A flow with at least one unclassified value. Listed, not executable. */
export const BlockedFlow = z
    .strictObject({
        ...flowBase,
        status: z.literal("blocked"),
        unclassified: z.array(UnclassifiedValue).min(1).max(512),
    })
    .meta({ id: "BlockedFlow" });

export const Flow = z.discriminatedUnion("status", [ReadyFlow, BlockedFlow]).meta({ id: "Flow" });
export type Flow = z.infer<typeof Flow>;
