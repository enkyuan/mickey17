import { describe, expect, test } from "bun:test";
import { InMemoryConsumedStore, issueConfirmation, verifyConfirmation, type ConfirmationBinding } from "../src/confirm.ts";
import { authorize, bindingFor, decide } from "../src/policy.ts";
import { generateEd25519 } from "../src/sign.ts";
import { confirmToken, mandateToken, moneyReq, NOW, world } from "./helpers.ts";

const binding: ConfirmationBinding = { actionId: "act_pay", item: "sku-42", totalMinor: 4_999, currency: "USD", payee: "shop.example.com" };

describe("confirmation tokens", () => {
    test("verify when every bound field matches", async () => {
        const w = await world();
        const tok = await issueConfirmation(w.confirmSigner, binding, { jti: "c1", now: NOW, ttlMs: 60_000 });
        const r = await verifyConfirmation(tok, w.deps.confirmationVerifier, binding, { now: NOW + 1 });
        expect(r.ok).toBe(true);
    });

    for (const [field, value] of [["actionId", "act_other"], ["item", "sku-43"], ["totalMinor", 5_000], ["currency", "EUR"], ["payee", "evil.example.com"]] as const) {
        test(`cannot be reused for a changed ${field}`, async () => {
            const w = await world();
            const tok = await issueConfirmation(w.confirmSigner, binding, { jti: "c1", now: NOW, ttlMs: 60_000 });
            const r = await verifyConfirmation(tok, w.deps.confirmationVerifier, { ...binding, [field]: value }, { now: NOW + 1 });
            expect(r.ok).toBe(false);
            if (!r.ok) {
                expect(r.code).toBe("confirmation-binding-mismatch");
                expect(r.missing).toContain(field);
            }
        });
    }

    test("a one-minor-unit change in the total is a mismatch", async () => {
        const w = await world();
        const tok = await issueConfirmation(w.confirmSigner, binding, { jti: "c1", now: NOW, ttlMs: 60_000 });
        expect((await verifyConfirmation(tok, w.deps.confirmationVerifier, { ...binding, totalMinor: 4_998 }, { now: NOW })).ok).toBe(false);
        expect((await verifyConfirmation(tok, w.deps.confirmationVerifier, { ...binding, totalMinor: 5_000 }, { now: NOW })).ok).toBe(false);
    });

    test("null bindings must match nulls: a no-money token cannot authorize money and vice versa", async () => {
        const w = await world();
        const noMoney: ConfirmationBinding = { actionId: "act_pay", item: null, totalMinor: null, currency: null, payee: null };
        const tok = await issueConfirmation(w.confirmSigner, noMoney, { jti: "c1", now: NOW, ttlMs: 60_000 });
        expect((await verifyConfirmation(tok, w.deps.confirmationVerifier, binding, { now: NOW })).ok).toBe(false);
        const money = await issueConfirmation(w.confirmSigner, binding, { jti: "c2", now: NOW, ttlMs: 60_000 });
        expect((await verifyConfirmation(money, w.deps.confirmationVerifier, noMoney, { now: NOW })).ok).toBe(false);
    });

    test("expiry boundary, not-yet-valid, over-long ttl", async () => {
        const w = await world();
        const tok = await issueConfirmation(w.confirmSigner, binding, { jti: "c1", now: NOW, ttlMs: 60_000 });
        const v = w.deps.confirmationVerifier;
        expect((await verifyConfirmation(tok, v, binding, { now: NOW + 60_000 - 1 })).ok).toBe(true);
        expect(await verifyConfirmation(tok, v, binding, { now: NOW + 60_000 })).toMatchObject({ ok: false, code: "confirmation-expired" });
        expect(await verifyConfirmation(tok, v, binding, { now: NOW - 1 })).toMatchObject({ ok: false, code: "confirmation-not-yet-valid" });
        const long = await issueConfirmation(w.confirmSigner, binding, { jti: "c3", now: NOW, ttlMs: 24 * 3_600_000 });
        expect(await verifyConfirmation(long, v, binding, { now: NOW })).toMatchObject({ ok: false, code: "confirmation-ttl-too-long" });
    });

    test("forged, foreign-key and malformed tokens are refused", async () => {
        const w = await world();
        const other = await generateEd25519();
        const foreign = await issueConfirmation(other.signer, binding, { jti: "c1", now: NOW, ttlMs: 60_000 });
        expect((await verifyConfirmation(foreign, w.deps.confirmationVerifier, binding, { now: NOW })).ok).toBe(false);
        for (const junk of [undefined, "", "x", "a.b", 7, null]) expect((await verifyConfirmation(junk, w.deps.confirmationVerifier, binding, { now: NOW })).ok).toBe(false);
    });

    test("issuer refuses malformed bindings", async () => {
        const w = await world();
        const t = (b: ConfirmationBinding) => issueConfirmation(w.confirmSigner, b, { jti: "c1", now: NOW, ttlMs: 1000 });
        await expect(t({ ...binding, totalMinor: 0 })).rejects.toThrow();
        await expect(t({ ...binding, totalMinor: 1.5 })).rejects.toThrow();
        await expect(t({ ...binding, currency: null })).rejects.toThrow(); // money fields travel together
        await expect(t({ ...binding, item: "x".repeat(300) })).rejects.toThrow();
    });
});

describe("single use", () => {
    test("the consumed store admits the first caller only", async () => {
        const s = new InMemoryConsumedStore();
        expect(await s.consume("j", NOW + 1000, NOW)).toBe(true);
        expect(await s.consume("j", NOW + 1000, NOW)).toBe(false);
        expect(await s.consume("k", NOW + 1000, NOW)).toBe(true);
    });

    test("concurrent consumption of one token succeeds exactly once", async () => {
        const s = new InMemoryConsumedStore();
        const r = await Promise.all(Array.from({ length: 20 }, () => s.consume("j", NOW + 1000, NOW)));
        expect(r.filter(Boolean).length).toBe(1);
    });

    test("authorize: a replayed confirmation is refused and does not spend", async () => {
        const w = await world();
        const mt = await mandateToken(w);
        const req = moneyReq();
        const conf = await confirmToken(w, req);
        const first = await authorize(req, { mandate: mt, confirmation: conf }, w.deps);
        expect(first.ok).toBe(true);
        const replay = await authorize({ ...req, idempotencyKey: "idem-key-0002" }, { mandate: mt, confirmation: conf }, w.deps);
        expect(replay.ok).toBe(false);
        if (!replay.ok) expect(replay.code).toBe("confirmation-reused");
        expect(await w.ledger.spent("mandate-1")).toBe(4_999);
    });

    test("authorize: a confirmation for another amount is refused and is not consumed", async () => {
        const w = await world();
        const mt = await mandateToken(w);
        const req = moneyReq();
        const conf = await confirmToken(w, req);
        const changed = moneyReq({ money: { ...req.money!, totalMinor: 5_500 } });
        const r = await authorize(changed, { mandate: mt, confirmation: conf }, w.deps);
        expect(r.ok).toBe(false);
        if (!r.ok) expect(r.code).toBe("confirmation-binding-mismatch");
        // the honest call can still use it
        expect((await authorize(req, { mandate: mt, confirmation: conf }, w.deps)).ok).toBe(true);
    });

    test("non-financial confirmation is bound to action and item", async () => {
        const w = await world();
        const req = { ...moneyReq(), effect: "write-irreversible" as const, money: undefined, item: "order-9", credential: { kind: "oauth" as const, scopes: ["write-irreversible" as const] } };
        const conf = await confirmToken(w, req);
        expect((await authorize({ ...req, item: "order-10" }, { confirmation: conf }, w.deps)).ok).toBe(false);
        expect((await authorize(req, { confirmation: conf }, w.deps)).ok).toBe(true);
        expect(bindingFor(req).item).toBe("order-9");
    });
});

describe("decide re-checks a verified confirmation against the call about to run", () => {
    test("a confirmation verified for one binding is refused for another, and when expired", async () => {
        const w = await world();
        const req = moneyReq();
        const mt = await mandateToken(w);
        const v = await (await import("../src/mandate.ts")).verifyMandate(mt, w.mandateVerifier, { now: NOW });
        const c = await verifyConfirmation(await confirmToken(w, req), w.deps.confirmationVerifier, bindingFor(req), { now: NOW });
        if (!v.ok || !c.ok) throw new Error("setup");
        expect(decide(req, { mandate: v.mandate, confirmation: c.confirmation }).ok).toBe(true);
        const other = moneyReq({ money: { ...req.money!, totalMinor: 5_000 } });
        const r = decide(other, { mandate: v.mandate, confirmation: c.confirmation });
        expect(!r.ok && r.code).toBe("confirmation-binding-mismatch");
        const late = decide({ ...req, now: NOW + 10 * 60_000 }, { mandate: v.mandate, confirmation: c.confirmation });
        expect(late.ok).toBe(false);
        const forged = { ...c.confirmation } as never;
        expect(decide(req, { mandate: v.mandate, confirmation: forged }).ok).toBe(false);
    });
});
