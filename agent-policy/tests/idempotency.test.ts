import { describe, expect, test } from "bun:test";
import { InMemoryIdempotencyStore, requestFingerprint, runWithIdempotency, shouldRetry, type AttemptOutcome } from "../src/idempotency.ts";
import { EFFECTS, type Effect } from "../src/types.ts";
import { NOW } from "./helpers.ts";

const opts = (store: InMemoryIdempotencyStore, over: Partial<Parameters<typeof runWithIdempotency<string>>[0]> = {}) => ({
    store,
    key: "idem-key-0001",
    fingerprint: "fp1",
    now: NOW,
    ttlMs: 60_000,
    idempotent: true,
    effect: "read" as Effect,
    maxAttempts: 3,
    sleep: async () => {},
    toStored: (v: string) => ({ status: "ok", receiptId: v }),
    ...over,
});
const fail = (retryable: boolean, neverSent = false): AttemptOutcome<string> => ({ ok: false, retryable, neverSent });

describe("shouldRetry", () => {
    test("non-idempotent is never retried, whatever else is true", () => {
        for (const effect of EFFECTS) expect(shouldRetry({ idempotent: false, effect, attempt: 1, maxAttempts: 9, retryable: true })).toBe(false);
    });
    test("irreversible, financial and account-control are never retried even if declared idempotent", () => {
        for (const effect of ["write-irreversible", "financial", "account-control"] as const) {
            expect(shouldRetry({ idempotent: true, effect, attempt: 1, maxAttempts: 9, retryable: true })).toBe(false);
        }
    });
    test("idempotent reads and reversible writes retry up to the cap", () => {
        for (const effect of ["read", "write-reversible"] as const) {
            expect(shouldRetry({ idempotent: true, effect, attempt: 1, maxAttempts: 3, retryable: true })).toBe(true);
            expect(shouldRetry({ idempotent: true, effect, attempt: 2, maxAttempts: 3, retryable: true })).toBe(true);
            expect(shouldRetry({ idempotent: true, effect, attempt: 3, maxAttempts: 3, retryable: true })).toBe(false);
            expect(shouldRetry({ idempotent: true, effect, attempt: 1, maxAttempts: 3, retryable: false })).toBe(false);
        }
    });
});

describe("runWithIdempotency", () => {
    test("a non-idempotent action runs once even when the failure is retryable", async () => {
        let calls = 0;
        const r = await runWithIdempotency(opts(new InMemoryIdempotencyStore(), { idempotent: false, effect: "write-reversible" }), async () => (calls++, fail(true)));
        expect(calls).toBe(1);
        expect(r.kind === "executed" && r.attempts).toBe(1);
    });

    for (const effect of ["financial", "write-irreversible", "account-control"] as const) {
        test(`${effect} runs once even if declared idempotent and retryable`, async () => {
            let calls = 0;
            await runWithIdempotency(opts(new InMemoryIdempotencyStore(), { effect }), async () => (calls++, fail(true)));
            expect(calls).toBe(1);
        });
    }

    test("an idempotent read retries up to maxAttempts, then stops", async () => {
        let calls = 0;
        const r = await runWithIdempotency(opts(new InMemoryIdempotencyStore()), async () => (calls++, fail(true)));
        expect(calls).toBe(3);
        expect(r.kind === "executed" && r.attempts).toBe(3);
    });

    test("success is stored and a duplicate returns it without executing", async () => {
        const store = new InMemoryIdempotencyStore();
        let calls = 0;
        const ok = async (): Promise<AttemptOutcome<string>> => (calls++, { ok: true, value: "rcpt-1" });
        await runWithIdempotency(opts(store, { effect: "financial" }), ok);
        const dup = await runWithIdempotency(opts(store, { effect: "financial" }), ok);
        expect(calls).toBe(1);
        expect(dup).toEqual({ kind: "replayed", result: { status: "ok", receiptId: "rcpt-1" } });
    });

    test("same key with a different request is a conflict and does not execute", async () => {
        const store = new InMemoryIdempotencyStore();
        await runWithIdempotency(opts(store), async () => ({ ok: true, value: "r" }));
        let calls = 0;
        const r = await runWithIdempotency(opts(store, { fingerprint: "fp-other" }), async () => (calls++, { ok: true, value: "r" }));
        expect(calls).toBe(0);
        expect(r).toEqual({ kind: "refused", code: "idempotency-key-conflict" });
    });

    test("an in-flight duplicate is refused", async () => {
        const store = new InMemoryIdempotencyStore();
        let release!: () => void;
        const gate = new Promise<void>((res) => (release = res));
        const first = runWithIdempotency(opts(store, { effect: "financial" }), async () => (await gate, { ok: true, value: "r" }));
        const second = await runWithIdempotency(opts(store, { effect: "financial" }), async () => ({ ok: true, value: "r" }));
        expect(second).toEqual({ kind: "refused", code: "idempotency-in-flight" });
        release();
        await first;
    });

    test("a thrown attempt leaves an unknown outcome that blocks re-execution", async () => {
        const store = new InMemoryIdempotencyStore();
        const r1 = await runWithIdempotency(opts(store, { effect: "financial" }), async () => {
            throw new Error("socket reset");
        });
        expect(r1.kind === "executed" && r1.attempts).toBe(1);
        let calls = 0;
        const r2 = await runWithIdempotency(opts(store, { effect: "financial" }), async () => (calls++, { ok: true, value: "r" }));
        expect(calls).toBe(0);
        expect(r2).toEqual({ kind: "refused", code: "idempotency-unknown-outcome" });
    });

    test("a request that provably never left the process releases the key", async () => {
        const store = new InMemoryIdempotencyStore();
        await runWithIdempotency(opts(store, { effect: "financial" }), async () => fail(false, true));
        let calls = 0;
        await runWithIdempotency(opts(store, { effect: "financial" }), async () => (calls++, { ok: true, value: "r" }));
        expect(calls).toBe(1);
    });

    test("an ambiguous failure (sent, no answer) is not released", async () => {
        const store = new InMemoryIdempotencyStore();
        await runWithIdempotency(opts(store, { effect: "financial" }), async () => fail(false, false));
        const r = await runWithIdempotency(opts(store, { effect: "financial" }), async () => ({ ok: true, value: "r" }));
        expect(r).toEqual({ kind: "refused", code: "idempotency-unknown-outcome" });
    });

    test("keys expire", async () => {
        const store = new InMemoryIdempotencyStore();
        await runWithIdempotency(opts(store), async () => ({ ok: true, value: "r" }));
        let calls = 0;
        await runWithIdempotency(opts(store, { now: NOW + 60_000 }), async () => (calls++, { ok: true, value: "r" }));
        expect(calls).toBe(1);
    });

    test("fingerprints are stable under key order and differ on any argument change", async () => {
        const a = await requestFingerprint("act", { x: 1, y: [1, 2], z: { b: 1, a: 2 } });
        expect(await requestFingerprint("act", { z: { a: 2, b: 1 }, y: [1, 2], x: 1 })).toBe(a);
        expect(await requestFingerprint("act", { x: 2, y: [1, 2], z: { b: 1, a: 2 } })).not.toBe(a);
        expect(await requestFingerprint("act2", { x: 1, y: [1, 2], z: { b: 1, a: 2 } })).not.toBe(a);
    });
});
