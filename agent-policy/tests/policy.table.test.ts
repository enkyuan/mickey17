import { describe, expect, test } from "bun:test";
import { authorize, decide, modeCell, type ActionRequest } from "../src/policy.ts";
import { EFFECTS, MODES, TIERS, type Effect, type Mode, type Tier } from "../src/types.ts";
import { bindingOf, confirmToken, mandateToken, moneyReq, NOW, world } from "./helpers.ts";

// Written out by hand, independent of the implementation's rules.
const ALLOWED: ReadonlySet<string> = new Set([
    "read-only/T0/read",
    "read-only/T1/read",
    "owner/T0/read", "owner/T1/read", "owner/T3/read",
    "owner/T2/read", "owner/T2/write-reversible", "owner/T2/write-irreversible", "owner/T2/financial", "owner/T2/account-control",
    "delegated/T0/read", "delegated/T1/read", "delegated/T3/read",
    "delegated/T2/read", "delegated/T2/write-reversible", "delegated/T2/write-irreversible", "delegated/T2/financial", "delegated/T2/account-control",
]);

// mode x effect at T2, the literal 15-cell table of D2 + D5 + D11.
const MODE_EFFECT: Record<Mode, Record<Effect, boolean>> = {
    "read-only": { read: false, "write-reversible": false, "write-irreversible": false, financial: false, "account-control": false },
    owner: { read: true, "write-reversible": true, "write-irreversible": true, financial: true, "account-control": true },
    delegated: { read: true, "write-reversible": true, "write-irreversible": true, financial: true, "account-control": true },
};

/** Independent oracle for *which* refusal a denied cell must produce. */
function expectedDenyCode(mode: Mode, tier: Tier): string {
    if (tier === "T4") return "tier-opaque";
    if (mode === "read-only") return tier !== "T0" && tier !== "T1" ? "mode-read-only-tier" : "mode-read-only-effect";
    return "tier-effect-incoherent";
}

function requestFor(mode: Mode, tier: Tier, effect: Effect): ActionRequest {
    const base: ActionRequest = {
        actionId: "act_pay",
        mode,
        tier,
        effect,
        evidence: "observed",
        idempotent: true,
        now: NOW,
        ...(mode === "read-only" ? { thirdParty: { robots: "allowed" as const, tos: "ok" as const } } : {}),
        ...(mode === "owner" ? { credential: { kind: "service" as const, scopes: [effect] } } : {}),
        ...(mode === "delegated" ? { credential: { kind: "oauth" as const, scopes: [effect] } } : {}),
        ...(effect === "financial" ? { idempotencyKey: "idem-key-0001", money: moneyReq().money! } : {}),
        ...(effect === "write-irreversible" || effect === "account-control" ? { item: "target-1" } : {}),
    };
    return base;
}

describe("mode x tier x effect (every cell, with every proof supplied)", () => {
    for (const mode of MODES) {
        for (const tier of TIERS) {
            for (const effect of EFFECTS) {
                const cell = `${mode}/${tier}/${effect}`;
                test(cell, async () => {
                    const w = await world();
                    const req = requestFor(mode, tier, effect);
                    const needsConfirm = effect === "financial" || effect === "write-irreversible" || effect === "account-control";
                    const tokens = {
                        ...(effect === "financial" ? { mandate: await mandateToken(w) } : {}),
                        ...(needsConfirm ? { confirmation: await confirmToken(w, req) } : {}),
                    };
                    const r = await authorize(req, tokens, w.deps);
                    if (ALLOWED.has(cell)) {
                        expect(r.ok).toBe(true);
                    } else {
                        expect(r.ok).toBe(false);
                        if (!r.ok) {
                            expect(r.missing.length).toBeGreaterThan(10);
                            expect(r.message.length).toBeGreaterThan(10);
                            expect(r.code).toBe(expectedDenyCode(mode, tier));
                        }
                    }
                });
            }
        }
    }
});

describe("mode x effect table at T2 (modeCell)", () => {
    for (const mode of MODES) {
        for (const effect of EFFECTS) {
            test(`${mode} x ${effect}`, () => {
                expect(modeCell(mode, "T2", effect) === null).toBe(MODE_EFFECT[mode][effect]);
            });
        }
    }
});

describe("what each effect demands when proofs are missing", () => {
    const cases: Array<[Effect, string, string]> = [
        ["write-reversible", "ok", ""],
        ["write-irreversible", "requires_confirmation", "confirmation-missing"],
        ["account-control", "requires_confirmation", "confirmation-missing"],
        ["financial", "requires_auth", "mandate-missing"],
    ];
    for (const [effect, status, code] of cases) {
        test(effect, () => {
            const r = decide(requestFor("owner", "T2", effect));
            if (status === "ok") expect(r.ok).toBe(true);
            else {
                expect(r.ok).toBe(false);
                if (!r.ok) {
                    expect(r.status).toBe(status as never);
                    expect(r.code).toBe(code);
                    expect(r.missing).not.toBe("");
                }
            }
        });
    }

    test("financial with mandate but no confirmation names the confirmation", async () => {
        const w = await world();
        const req = requestFor("delegated", "T2", "financial");
        const r = await authorize(req, { mandate: await mandateToken(w) }, w.deps);
        expect(r.ok).toBe(false);
        if (!r.ok) expect(r.code).toBe("confirmation-missing");
    });

    test("financial without an idempotency key is refused", () => {
        const { idempotencyKey: _k, ...rest } = requestFor("delegated", "T2", "financial");
        const r = decide(rest as ActionRequest);
        expect(!r.ok && r.code).toBe("idempotency-key-missing");
    });
});

describe("default deny and evidence", () => {
    test("unknown enum values are refused, never allowed", () => {
        for (const bad of [{ mode: "admin" }, { tier: "T9" }, { effect: "delete" }, { evidence: "vibes" }, { actionId: "" }, { actionId: "a b" }, { now: Number.NaN }]) {
            const r = decide({ ...requestFor("owner", "T2", "read"), ...bad } as unknown as ActionRequest);
            expect(r.ok).toBe(false);
        }
    });

    test("derived or inferred evidence never suffices for T2/T3", () => {
        for (const evidence of ["derived", "inferred"] as const) {
            for (const tier of ["T2", "T3"] as const) {
                const r = decide({ ...requestFor("owner", tier, "read"), evidence });
                expect(!r.ok && r.code).toBe("evidence-insufficient");
                expect(!r.ok && r.status).toBe("unsupported");
            }
        }
        for (const evidence of ["declared", "observed"] as const) expect(decide({ ...requestFor("owner", "T2", "read"), evidence }).ok).toBe(true);
    });

    test("inferred evidence may support a T0/T1 read", () => {
        expect(decide({ ...requestFor("read-only", "T1", "read"), evidence: "inferred" }).ok).toBe(true);
    });

    test("read-only needs a positive robots and ToS result", () => {
        const base = requestFor("read-only", "T1", "read");
        expect(decide({ ...base, thirdParty: { robots: "disallowed", tos: "ok" } }).ok).toBe(false);
        expect(decide({ ...base, thirdParty: { robots: "unknown", tos: "ok" } }).ok).toBe(false);
        expect(decide({ ...base, thirdParty: { robots: "allowed", tos: "unchecked" } }).ok).toBe(false);
        expect(decide({ ...base, thirdParty: { robots: "allowed", tos: "violates" } }).ok).toBe(false);
        const { thirdParty: _t, ...none } = base;
        expect(decide(none as ActionRequest).ok).toBe(false);
    });

    test("credential kind must match the mode; scope must cover the effect", () => {
        const base = requestFor("owner", "T2", "write-reversible");
        expect(decide({ ...base, credential: { kind: "oauth", scopes: ["write-reversible"] } }).ok).toBe(false);
        expect(decide({ ...base, credential: { kind: "service", scopes: ["read"] } }).ok).toBe(false);
        expect(decide({ ...base, credential: undefined } as unknown as ActionRequest).ok).toBe(false);
        const d = requestFor("delegated", "T2", "write-reversible");
        expect(decide({ ...d, credential: { kind: "service", scopes: ["write-reversible"] } }).ok).toBe(false);
        expect(decide({ ...d, credential: { kind: "user-session", scopes: ["write-reversible"] } }).ok).toBe(true);
    });

    test("an authenticated read needs a credential; a public read does not", () => {
        const base = requestFor("owner", "T2", "read");
        const { credential: _c, ...noCred } = base;
        expect(decide(noCred as ActionRequest).ok).toBe(true);
        expect(decide({ ...noCred, requiresAuth: true } as ActionRequest).ok).toBe(false);
    });

    test("neverRetry follows idempotency and effect", () => {
        const r = (effect: Effect, idempotent: boolean) => {
            const d = decide({ ...requestFor("owner", "T2", effect), idempotent }, {});
            return d.ok ? d.neverRetry : null;
        };
        expect(r("read", true)).toBe(false);
        expect(r("read", false)).toBe(true);
        expect(r("write-reversible", true)).toBe(false);
        expect(r("write-reversible", false)).toBe(true);
    });

    test("authorize reports neverRetry for every effect that must not be retried, even when declared idempotent", async () => {
        for (const effect of ["write-irreversible", "account-control", "financial"] as const) {
            const w = await world();
            const req = requestFor("owner", "T2", effect);
            const needs = { ...(effect === "financial" ? { mandate: await mandateToken(w) } : {}), confirmation: await confirmToken(w, req) };
            const a = await authorize({ ...req, idempotent: true }, needs, w.deps);
            expect(a.ok && a.neverRetry, effect).toBe(true);
        }
        const w = await world();
        const rev = await authorize({ ...requestFor("owner", "T2", "write-reversible"), idempotent: true }, {}, w.deps);
        expect(rev.ok && rev.neverRetry).toBe(false);
    });

    test("bindingOf agrees with the confirmation the policy expects", async () => {
        const w = await world();
        const req = moneyReq();
        const r = await authorize(req, { mandate: await mandateToken(w), confirmation: await confirmToken(w, req) }, w.deps);
        expect(r.ok).toBe(true);
        expect(bindingOf(req).totalMinor).toBe(4_999);
    });
});
