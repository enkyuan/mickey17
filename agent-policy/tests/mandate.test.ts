import { describe, expect, test } from "bun:test";
import { b64urlDecode, b64urlEncode, constantTimeEqual } from "../src/bytes.ts";
import { checkMandateScope, InMemorySpendLedger, isVerifiedMandate, verifyMandate, type VerifiedMandate } from "../src/mandate.ts";
import { authorize, decide } from "../src/policy.ts";
import { ed25519Verifier, generateEd25519, hmacSigner } from "../src/sign.ts";
import { issueConfirmation } from "../src/confirm.ts";
import { baseMandate, confirmToken, HOUR, mandateToken, moneyReq, NOW, world } from "./helpers.ts";

const spend = { actionId: "act_pay", merchant: "shop.example.com", payee: "shop.example.com", currency: "USD", amountMinor: 4_999 };

async function verified(w: Awaited<ReturnType<typeof world>>, over = {}): Promise<VerifiedMandate> {
    const r = await verifyMandate(await mandateToken(w, over), w.mandateVerifier, { now: NOW });
    if (!r.ok) throw new Error(`unexpected refusal ${r.code}`);
    return r.mandate;
}

describe("mandate verification", () => {
    test("a well-formed mandate verifies and is branded", async () => {
        const w = await world();
        const m = await verified(w);
        expect(isVerifiedMandate(m)).toBe(true);
        expect(isVerifiedMandate({ ...m })).toBe(false); // a copy is not verified
        expect(Object.isFrozen(m)).toBe(true);
    });

    test("expired: refused exactly at the expiry instant", async () => {
        const w = await world();
        const tok = await mandateToken(w);
        const at = await verifyMandate(tok, w.mandateVerifier, { now: baseMandate.expiresAt });
        expect(!at.ok && at.code).toBe("mandate-expired");
        const before = await verifyMandate(tok, w.mandateVerifier, { now: baseMandate.expiresAt - 1 });
        expect(before.ok).toBe(true);
    });

    test("not yet valid: refused before notBefore, accepted at it", async () => {
        const w = await world();
        const tok = await mandateToken(w);
        const early = await verifyMandate(tok, w.mandateVerifier, { now: baseMandate.notBefore - 1 });
        expect(!early.ok && early.code).toBe("mandate-not-yet-valid");
        expect((await verifyMandate(tok, w.mandateVerifier, { now: baseMandate.notBefore })).ok).toBe(true);
    });

    test("bad signature: other key, tampered payload, truncated, extra segment", async () => {
        const w = await world();
        const other = await generateEd25519();
        const forged = await mandateToken({ ...w, mandateSigner: other.signer });
        expect((await verifyMandate(forged, w.mandateVerifier, { now: NOW }))).toMatchObject({ ok: false, code: "mandate-bad-signature" });

        const tok = await mandateToken(w);
        const [seg, sig] = tok.split(".") as [string, string];
        const payload = JSON.parse(new TextDecoder().decode(b64urlDecode(seg)!));
        payload.capMinor = 9_999_999;
        const tampered = `${b64urlEncode(new TextEncoder().encode(JSON.stringify(payload)))}.${sig}`;
        expect(await verifyMandate(tampered, w.mandateVerifier, { now: NOW })).toMatchObject({ ok: false, code: "mandate-bad-signature" });

        expect(await verifyMandate(tok.slice(0, -4), w.mandateVerifier, { now: NOW })).toMatchObject({ ok: false });
        expect(await verifyMandate(`${tok}.x`, w.mandateVerifier, { now: NOW })).toMatchObject({ ok: false, code: "mandate-malformed" });
        for (const junk of [undefined, null, 5, "", "a.b", {}]) expect((await verifyMandate(junk, w.mandateVerifier, { now: NOW })).ok).toBe(false);
    });

    test("a confirmation token cannot be presented as a mandate (domain separation, same key)", async () => {
        const ed = await generateEd25519();
        const conf = await issueConfirmation(ed.signer, { actionId: "act_pay", item: null, totalMinor: null, currency: null, payee: null }, { jti: "c1", now: NOW, ttlMs: 1000 });
        expect(await verifyMandate(conf, ed.verifier, { now: NOW })).toMatchObject({ ok: false, code: "mandate-bad-signature" });
    });

    test("unknown, missing and out-of-range payload fields are refused", async () => {
        const w = await world();
        const bad: Array<Record<string, unknown>> = [
            { extra: 1 }, { capMinor: 0 }, { capMinor: 1.5 }, { capMinor: -5 }, { capMinor: Number.MAX_SAFE_INTEGER + 2 },
            { currency: "usd" }, { currency: "US" }, { merchants: [] }, { merchants: ["*.example.com"] }, { merchants: ["Shop.Example.com"] },
            { actions: [] }, { actions: ["*"] }, { perTxCapMinor: 20_000 }, { expiresAt: baseMandate.notBefore }, { v: 2 },
        ];
        for (const over of bad) {
            const r = await verifyMandate(await mandateToken(w, over as never), w.mandateVerifier, { now: NOW });
            expect(r.ok).toBe(false);
        }
    });

    test("lifetime cap and revocation", async () => {
        const w = await world();
        const long = await mandateToken(w, { notBefore: NOW - HOUR, expiresAt: NOW + 400 * 24 * HOUR });
        expect(await verifyMandate(long, w.mandateVerifier, { now: NOW })).toMatchObject({ ok: false, code: "mandate-lifetime-too-long" });
        const tok = await mandateToken(w);
        expect(await verifyMandate(tok, w.mandateVerifier, { now: NOW, revoked: new Set(["mandate-1"]) })).toMatchObject({ ok: false, code: "mandate-revoked" });
    });

    test("public-key verifier derived from raw key bytes cannot sign", async () => {
        const ed = await generateEd25519();
        const v = await ed25519Verifier(ed.publicKeyRaw);
        expect("sign" in v).toBe(false);
        const tok = await mandateToken({ mandateSigner: ed.signer } as never);
        expect((await verifyMandate(tok, v, { now: NOW })).ok).toBe(true);
    });

    test("HMAC-signed mandates also verify (symmetric option), and reject a wrong key", async () => {
        const a = await hmacSigner(new Uint8Array(32).fill(1));
        const b = await hmacSigner(new Uint8Array(32).fill(2));
        const tok = await mandateToken({ mandateSigner: a } as never);
        expect((await verifyMandate(tok, a, { now: NOW })).ok).toBe(true);
        expect((await verifyMandate(tok, b, { now: NOW })).ok).toBe(false);
        await expect(hmacSigner(new Uint8Array(16))).rejects.toThrow();
    });
});

describe("mandate scope checks", () => {
    test("accepts a call inside every bound", async () => {
        expect(checkMandateScope(await verified(await world()), spend, NOW)).toBeNull();
    });

    test("wrong merchant, case-insensitive match, no suffix tricks", async () => {
        const m = await verified(await world());
        expect(checkMandateScope(m, { ...spend, merchant: "evil.example.com" }, NOW)?.code).toBe("mandate-merchant-not-allowed");
        expect(checkMandateScope(m, { ...spend, merchant: "shop.example.com.evil.net" }, NOW)?.code).toBe("mandate-merchant-not-allowed");
        expect(checkMandateScope(m, { ...spend, merchant: "xshop.example.com" }, NOW)?.code).toBe("mandate-merchant-not-allowed");
        expect(checkMandateScope(m, { ...spend, merchant: "SHOP.Example.com." }, NOW)).toBeNull();
    });

    test("wrong currency, wrong action, over cap, over per-call cap, bad amounts", async () => {
        const m = await verified(await world());
        expect(checkMandateScope(m, { ...spend, currency: "EUR" }, NOW)?.code).toBe("mandate-currency-mismatch");
        expect(checkMandateScope(m, { ...spend, actionId: "act_other" }, NOW)?.code).toBe("mandate-action-not-scoped");
        expect(checkMandateScope(m, { ...spend, amountMinor: 6_001 }, NOW)?.code).toBe("mandate-per-tx-cap-exceeded");
        expect(checkMandateScope(m, { ...spend, amountMinor: 6_000 }, NOW)).toBeNull();
        const noPerTx = await verified(await world(), { perTxCapMinor: null });
        expect(checkMandateScope(noPerTx, { ...spend, amountMinor: 10_001 }, NOW)?.code).toBe("mandate-cap-exceeded");
        expect(checkMandateScope(noPerTx, { ...spend, amountMinor: 10_000 }, NOW)).toBeNull();
        for (const amountMinor of [0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY, Number.MAX_SAFE_INTEGER + 1]) {
            expect(checkMandateScope(m, { ...spend, amountMinor }, NOW)?.code).toBe("mandate-amount-invalid");
        }
    });

    test("scope check re-applies expiry at call time", async () => {
        const m = await verified(await world());
        expect(checkMandateScope(m, spend, baseMandate.expiresAt)?.code).toBe("mandate-expired");
        expect(checkMandateScope(m, spend, baseMandate.notBefore - 1)?.code).toBe("mandate-not-yet-valid");
    });

    test("an object that was not produced by verification is refused", () => {
        const forged = { ...baseMandate, __verified: true } as unknown as VerifiedMandate;
        expect(checkMandateScope(forged, spend, NOW)?.code).toBe("mandate-unverified");
        const req = moneyReq();
        expect(decide(req, { mandate: forged }).ok).toBe(false);
    });
});

describe("cumulative spend (authorize + ledger)", () => {
    test("cap is enforced across calls, not just per call", async () => {
        const w = await world();
        const mt = await mandateToken(w);
        const call = async (key: string, total: number) => {
            const req = moneyReq({ idempotencyKey: key, money: { ...moneyReq().money!, totalMinor: total } });
            return authorize(req, { mandate: mt, confirmation: await confirmToken(w, req) }, w.deps);
        };
        expect((await call("idem-key-0001", 4_999)).ok).toBe(true); // 4999
        expect((await call("idem-key-0002", 4_999)).ok).toBe(true); // 9998
        const third = await call("idem-key-0003", 3);
        expect(third.ok).toBe(false);
        if (!third.ok) expect(third.code).toBe("mandate-cap-exceeded");
        expect(await w.ledger.spent("mandate-1")).toBe(9_998);
        expect((await call("idem-key-0004", 2)).ok).toBe(true); // exactly 10000
        expect(await w.ledger.spent("mandate-1")).toBe(10_000);
    });

    test("releasing a failed spend frees budget; an unknown outcome is not released", async () => {
        const w = await world();
        const mt = await mandateToken(w);
        const run = async (key: string, total: number) => {
            const req = moneyReq({ idempotencyKey: key, money: { ...moneyReq().money!, totalMinor: total } });
            return authorize(req, { mandate: mt, confirmation: await confirmToken(w, req) }, w.deps);
        };
        const a = await run("idem-key-0001", 5_000);
        const b = await run("idem-key-0002", 5_000);
        expect(a.ok && b.ok).toBe(true);
        expect((await run("idem-key-0003", 1)).ok).toBe(false);
        if (a.ok) await a.settle("released");
        expect((await run("idem-key-0004", 5_000)).ok).toBe(true);
        if (b.ok) await b.settle("committed");
        expect(await w.ledger.spent("mandate-1")).toBe(10_000);
    });

    test("concurrent calls cannot jointly exceed the cap", async () => {
        const w = await world();
        const mt = await mandateToken(w, { perTxCapMinor: null });
        const results = await Promise.all(
            Array.from({ length: 10 }, async (_, i) => {
                const req = moneyReq({ idempotencyKey: `idem-key-00${i}0`, money: { ...moneyReq().money!, totalMinor: 3_000 } });
                return authorize(req, { mandate: mt, confirmation: await confirmToken(w, req) }, w.deps);
            }),
        );
        expect(results.filter((r) => r.ok).length).toBe(3);
        expect(await w.ledger.spent("mandate-1")).toBe(9_000);
    });

    test("the same reservation reference cannot be spent twice", async () => {
        const l = new InMemorySpendLedger();
        expect((await l.reserve("m", "r1", 10, 100)).ok).toBe(true);
        expect((await l.reserve("m", "r1", 10, 100)).ok).toBe(false);
        expect(await l.spent("m")).toBe(10);
    });

    test("authorize rejects each mandate failure before spending anything", async () => {
        const w = await world();
        const req = moneyReq();
        const conf = await confirmToken(w, req);
        const cases: Array<[string, Record<string, unknown>]> = [
            ["mandate-expired", { notBefore: NOW - 2 * HOUR, expiresAt: NOW - HOUR }],
            ["mandate-merchant-not-allowed", { merchants: ["other.example.com"] }],
            ["mandate-currency-mismatch", { currency: "EUR" }],
            ["mandate-per-tx-cap-exceeded", { perTxCapMinor: 100 }],
            ["mandate-cap-exceeded", { capMinor: 100, perTxCapMinor: null }],
            ["mandate-action-not-scoped", { actions: ["act_other"] }],
        ];
        for (const [code, over] of cases) {
            const r = await authorize(req, { mandate: await mandateToken(w, over as never), confirmation: conf }, w.deps);
            expect(r.ok ? "ok" : r.code).toBe(code);
        }
        expect(await w.ledger.spent("mandate-1")).toBe(0);
    });
});

describe("constantTimeEqual", () => {
    test("equal and unequal, different lengths, empty", () => {
        expect(constantTimeEqual("abc", "abc")).toBe(true);
        expect(constantTimeEqual("abc", "abd")).toBe(false);
        expect(constantTimeEqual("xbc", "abc")).toBe(false); // difference in the first byte only
        expect(constantTimeEqual("axc", "abc")).toBe(false); // difference in the middle only
        expect(constantTimeEqual("abc", "abcd")).toBe(false);
        expect(constantTimeEqual("abcd", "abc")).toBe(false);
        expect(constantTimeEqual("", "")).toBe(true);
        expect(constantTimeEqual("", "a")).toBe(false);
        expect(constantTimeEqual(new Uint8Array([1, 2]), new Uint8Array([1, 2]))).toBe(true);
        expect(constantTimeEqual(new Uint8Array([0]), new Uint8Array([]))).toBe(false); // zero padding must not equal absence
    });
});
