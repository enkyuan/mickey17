import { describe, expect, test } from "bun:test";
import { CONFIRMATION_EFFECTS, EFFECT_ORDER, Effect, maxEffect, requiresConfirmation } from "../src/index";

describe("effect classes (D5)", () => {
    test("order covers every effect exactly once", () => {
        expect([...EFFECT_ORDER].sort()).toEqual([...Effect.options].sort());
    });
    test("irreversible, financial and account-control require confirmation; reads and reversible writes do not", () => {
        expect(CONFIRMATION_EFFECTS).toEqual(["write-irreversible", "financial", "account-control"]);
        expect(requiresConfirmation("read")).toBe(false);
        expect(requiresConfirmation("write-reversible")).toBe(false);
        expect(requiresConfirmation("financial")).toBe(true);
    });
    test("maxEffect picks the more consequential", () => {
        expect(maxEffect("read", "financial")).toBe("financial");
        expect(maxEffect("account-control", "write-irreversible")).toBe("account-control");
    });
});
