import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import { AgentSurface, OperationResult } from "../src/index";
import { fixtureDir, readJson } from "./helpers";

// Standard Schema v1 interface (https://standardschema.dev), copied so the test depends on
// the interface and not on our library. A consumer typed against this needs no zod import.
interface StandardSchemaV1<Input = unknown, Output = Input> {
    readonly "~standard": {
        readonly version: 1;
        readonly vendor: string;
        readonly validate: (value: unknown) => Result<Output> | Promise<Result<Output>>;
        readonly types?: { readonly input: Input; readonly output: Output } | undefined;
    };
}
type Result<Output> = { readonly value: Output; readonly issues?: undefined } | { readonly issues: ReadonlyArray<{ readonly message: string; readonly path?: ReadonlyArray<PropertyKey | { readonly key: PropertyKey }> | undefined }> };

/** What a consumer that only knows Standard Schema would write. */
async function validateWith<I, O>(schema: StandardSchemaV1<I, O>, value: unknown) {
    const r = await schema["~standard"].validate(value);
    return r.issues ? { ok: false as const, issues: r.issues } : { ok: true as const, value: r.value };
}

describe("Standard Schema interop", () => {
    test("exposes the ~standard interface with a vendor and version 1", () => {
        for (const s of [AgentSurface, OperationResult]) {
            expect(s["~standard"].version).toBe(1);
            expect(s["~standard"].vendor).toBe("zod");
            expect(typeof s["~standard"].validate).toBe("function");
        }
    });
    test("a valid surface validates through ~standard", async () => {
        const doc = readJson(join(fixtureDir("valid"), "surface.shop.json"));
        const r = await validateWith(AgentSurface, doc);
        expect(r.ok).toBe(true);
        if (r.ok) expect(r.value.kind).toBe("agent-surface");
    });
    test("an invalid surface reports issues with paths through ~standard", async () => {
        const doc = readJson(join(fixtureDir("invalid"), "action-unknown-effect.json"));
        const r = await validateWith(AgentSurface, doc);
        expect(r.ok).toBe(false);
        if (!r.ok) {
            expect(r.issues.length).toBe(1);
            expect(r.issues[0]!.path).toEqual(["actions", 1, "effect"]);
        }
    });
    test("operation results validate through ~standard", async () => {
        const ok = await validateWith(OperationResult, { status: "requires_human", reason: "3ds" });
        expect(ok.ok).toBe(true);
        const bad = await validateWith(OperationResult, { status: "requires_human" });
        expect(bad.ok).toBe(false);
    });
});
