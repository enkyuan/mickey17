import { describe, expect, test } from "bun:test";
import { b64urlEncode, utf8 } from "../src/bytes.ts";
import { authorize } from "../src/policy.ts";
import { buildReceipt, type ReceiptInput } from "../src/receipt.ts";
import { createLogger, Redactor, REDACTED } from "../src/redact.ts";
import { confirmToken, mandateToken, moneyReq, NOW, world } from "./helpers.ts";

const CANARIES = {
    session: "CANARY-session-4f9a1c7e22b05d83",
    bearer: "CANARY-bearer-ZXhhbXBsZXRva2VuMTIz",
    apiKey: "CANARY-apikey-9d8c7b6a5f4e3d2c1b0a",
    password: "CANARY-pa$$w0rd-!#&=?",
    card: "4111111111111111",
};
const all = Object.values(CANARIES);

function seeded(): Redactor {
    const r = new Redactor();
    for (const c of [CANARIES.session, CANARIES.bearer, CANARIES.apiKey, CANARIES.password]) r.register(c);
    return r;
}

function assertClean(label: string, out: unknown, canaries: readonly string[] = all) {
    const text = typeof out === "string" ? out : JSON.stringify(out);
    for (const c of canaries) {
        expect(text.includes(c), `${label} leaked ${c}`).toBe(false);
    }
}

/** Every encoding a canary might plausibly take on its way into a string. */
function encodings(c: string): string[] {
    const b = utf8(c);
    const std = btoa(String.fromCharCode(...b));
    return [
        c,
        encodeURIComponent(c),
        JSON.stringify(c).slice(1, -1),
        std,
        std.replaceAll("=", ""),
        b64urlEncode(b),
        Array.from(b, (x) => x.toString(16).padStart(2, "0")).join(""),
    ];
}

describe("Redactor", () => {
    test("removes each registered secret in every common encoding and embedding", () => {
        const r = seeded();
        for (const c of [CANARIES.session, CANARIES.bearer, CANARIES.apiKey, CANARIES.password]) {
            for (const e of encodings(c)) {
                for (const wrap of [(x: string) => x, (x: string) => `prefix ${x} suffix`, (x: string) => `?a=1&k=${x}&b=2`, (x: string) => `{"v":"${x}"}`, (x: string) => `x${x}y`]) {
                    const out = r.redactString(wrap(e));
                    expect(out.includes(e), `${e} survived`).toBe(false);
                }
            }
        }
    });

    test("base64 of a secret embedded at every alignment inside larger base64 text", () => {
        const r = new Redactor();
        r.register("CANARY-embedded-secret-value");
        for (const pad of ["", "a", "ab", "abc", "abcd"]) {
            const blob = btoa(`${pad}CANARY-embedded-secret-value tail`);
            expect(r.redactString(blob)).not.toContain(btoa(`${pad}CANARY-embedded-secret-value tail`).slice(8, 30));
        }
    });

    test("well-known secret shapes are removed without being registered", () => {
        const r = new Redactor();
        const samples = [
            "Authorization: Bearer abcdef1234567890abcdef",
            "Cookie: sid=abc123def456; theme=dark",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
            "sk_live_51H8abcdefghijklmnop",
            "AKIAIOSFODNN7EXAMPLE",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "-----BEGIN PRIVATE KEY-----\nMIIBVQIBADANBgkq\n-----END PRIVATE KEY-----",
            '{"password":"hunter2hunter2","note":"x"}',
            "https://x.example/cb?token=abc123xyz&state=1",
            "card 4111 1111 1111 1111 exp 12/30",
            "card 4111-1111-1111-1111",
        ];
        for (const s of samples) {
            const out = r.redactString(s);
            expect(out).toContain(REDACTED);
            for (const frag of ["abcdef1234567890", "abc123def456", "SflKxwRJ", "51H8abcdef", "IOSFODNN7", "abcdefghijklmnopqrstuvwxyz0123", "MIIBVQ", "hunter2", "abc123xyz", "4111 1111", "4111-1111", "4111111111111111"]) {
                expect(out.includes(frag), `${frag} survived in: ${out}`).toBe(false);
            }
        }
    });

    test("order numbers that are not card numbers are left alone", () => {
        const r = new Redactor();
        expect(r.redactString("order 1234567890123 total")).toBe("order 1234567890123 total"); // fails Luhn
    });

    test("deep redaction: sensitive keys, nested arrays, errors, cycles", () => {
        const r = seeded();
        const cyc: Record<string, unknown> = { name: "n" };
        cyc.self = cyc;
        const out = r.redactDeep({
            headers: { Authorization: "Bearer zzzzzzzzzzzz", "Set-Cookie": "a=b", "X-Trace": CANARIES.session },
            nested: [{ deep: { password: CANARIES.password, ok: "fine" } }],
            err: new Error(`failed with ${CANARIES.bearer}`),
            cyc,
            [CANARIES.apiKey]: "key-name-is-secret",
        }) as Record<string, unknown>;
        assertClean("deep", out);
        const o = out as { headers: Record<string, string>; nested: [{ deep: { password: string; ok: string } }] };
        expect(o.headers["Set-Cookie"]).toBe(REDACTED); // blanked by key name: the value matches no secret shape
        expect(o.nested[0].deep.password).toBe(REDACTED);
        expect(o.nested[0].deep.ok).toBe("fine");
        expect(JSON.stringify(out)).toContain("fine");
        expect(JSON.stringify(out)).toContain("[Circular]");
    });

    test("short secrets are rejected at registration (a 1-char secret would shred every string)", () => {
        expect(() => new Redactor().register("ab")).toThrow();
    });
});

describe("canary scan: receipts, errors, logs, refusals", () => {
    const input = (over: Partial<ReceiptInput> = {}): ReceiptInput => ({
        id: "rcpt-1", at: NOW, mode: "delegated", tier: "T2", effect: "financial", actionId: "act_pay", evidenceIds: ["ev-1"],
        requestClass: "POST /checkout", engine: "http", host: "shop.example.com", result: "ok", durationMs: 12,
        mandateId: "mandate-1", confirmationId: "c-1", idempotencyKey: "idem-key-0001", money: { totalMinor: 4999, currency: "USD", payee: "shop.example.com" },
        ...over,
    });

    test("canaries smuggled into every string field never reach the receipt", async () => {
        const r = seeded();
        const rc = await buildReceipt(
            input({
                id: `rcpt-${CANARIES.session}`, actionId: `act-${CANARIES.apiKey}`, evidenceIds: [CANARIES.bearer], requestClass: `GET /x?token=${CANARIES.session}`,
                host: `h.example.com?k=${CANARIES.apiKey}`, resultCode: CANARIES.password, mandateId: CANARIES.bearer, confirmationId: CANARIES.session,
                idempotencyKey: CANARIES.apiKey, money: { totalMinor: 1, currency: "USD", payee: `p-${CANARIES.password}` },
                error: new Error(`upstream said Authorization: Bearer ${CANARIES.bearer} and cookie ${CANARIES.session}`),
            }),
            r,
        );
        assertClean("receipt", rc);
        expect(rc.error?.message).toContain(REDACTED);
        expect(rc.idempotencyKeyDigest).toMatch(/^[0-9a-f]{32}$/);
    });

    test("attributes not on the allowlist (headers, bodies, cookies) cannot be attached", async () => {
        const r = seeded();
        const rc = await buildReceipt({ ...input(), headers: { cookie: CANARIES.session }, body: CANARIES.card, cookies: [CANARIES.session] } as ReceiptInput, r);
        assertClean("receipt-extra", rc);
        expect(Object.keys(rc).sort()).toEqual(
            ["actionId", "at", "confirmationId", "durationMs", "effect", "engine", "error", "evidenceIds", "host", "id", "idempotencyKeyDigest", "mandateId", "mode", "money", "requestClass", "result", "resultCode", "tier", "v"].sort(),
        );
    });

    test("the logger redacts messages, fields and thrown errors, and survives a failing sink", () => {
        const lines: string[] = [];
        const log = createLogger(seeded(), (l) => lines.push(l));
        log.log("error", `request to https://x?api_key=${CANARIES.apiKey} failed`, {
            err: new Error(CANARIES.bearer), headers: { cookie: CANARIES.session, "x-note": `n ${CANARIES.password}` }, card: CANARIES.card, list: [CANARIES.session],
        });
        expect(lines).toHaveLength(1);
        assertClean("log", lines[0]!);
        expect(() => createLogger(seeded(), () => { throw new Error(CANARIES.session); }).log("info", "x")).not.toThrow();
    });

    test("refusals and errors from the policy never echo caller strings", async () => {
        const w = await world();
        const canary = "CANARYITEM-0f1e2d3c4b5a";
        const reqs = [
            moneyReq({ actionId: canary, idempotencyKey: `K-${canary}`, money: { merchant: `${canary}.example.com`, item: canary, totalMinor: 1, currency: "USD", payee: canary } }),
            moneyReq({ idempotencyKey: `K-${canary}`, money: { merchant: "shop.example.com", item: canary, totalMinor: 4_999_999, currency: "USD", payee: canary } }),
            moneyReq({ idempotencyKey: `K-${canary}`, money: { merchant: `${canary}.example.com`, item: canary, totalMinor: 1, currency: "EUR", payee: canary } }),
        ];
        for (const req of reqs) {
            const outs = [
                await authorize(req, {}, w.deps),
                await authorize(req, { mandate: canary, confirmation: canary }, w.deps),
                await authorize(req, { mandate: await mandateToken(w, { actions: [req.actionId] }), confirmation: await confirmToken(w, req, { item: "other" }) }, w.deps),
            ];
            for (const o of outs) assertClean("refusal", o, [canary]);
        }
    });

    test("an Authorization result exposes ids, never tokens", async () => {
        const w = await world();
        const req = moneyReq();
        const mt = await mandateToken(w);
        const ct = await confirmToken(w, req);
        const a = await authorize(req, { mandate: mt, confirmation: ct }, w.deps);
        assertClean("authorization", { ...a }, [mt, ct, mt.split(".")[1]!, ct.split(".")[1]!]);
    });
});
