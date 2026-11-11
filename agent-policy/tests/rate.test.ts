import { describe, expect, test } from "bun:test";
import { canStart, DEFAULT_RATE_POLICY, INITIAL_HOST_STATE, onFinish, onResponse, onStart, parseRetryAfter, withCrawlDelay, type RatePolicy } from "../src/rate.ts";

const P: RatePolicy = { maxConcurrent: 2, minIntervalMs: 100, maxRetryAfterMs: 60_000 };
const T = Date.UTC(2026, 0, 1, 12, 0, 0);

describe("per-host concurrency and spacing", () => {
    test("caps concurrent requests and frees a slot on finish", () => {
        let s = INITIAL_HOST_STATE;
        expect(canStart(s, P, T).allow).toBe(true);
        s = onStart(s, T);
        expect(canStart(s, P, T + 100).allow).toBe(true);
        s = onStart(s, T + 100);
        expect(canStart(s, P, T + 500)).toEqual({ allow: false, reason: "concurrency", waitMs: null });
        s = onFinish(s);
        expect(canStart(s, P, T + 500).allow).toBe(true);
    });
    test("minimum interval", () => {
        const s = onStart(INITIAL_HOST_STATE, T);
        expect(canStart(s, P, T + 99)).toEqual({ allow: false, reason: "interval", waitMs: 1 });
        expect(canStart(s, P, T + 100).allow).toBe(true);
    });
    test("finish never goes negative", () => {
        expect(onFinish(INITIAL_HOST_STATE).inFlight).toBe(0);
    });
    test("crawl-delay raises the floor and never lowers it", () => {
        expect(withCrawlDelay(P, 5).minIntervalMs).toBe(5000);
        expect(withCrawlDelay(P, 0.01).minIntervalMs).toBe(100);
        expect(withCrawlDelay(P, null)).toBe(P);
        expect(withCrawlDelay(P, Number.NaN)).toBe(P);
        expect(withCrawlDelay(P, 1e9).minIntervalMs).toBe(60_000);
    });
});

describe("Retry-After", () => {
    test("delta-seconds and HTTP-date", () => {
        expect(parseRetryAfter("120", T)).toBe(120_000);
        expect(parseRetryAfter(" 0 ", T)).toBe(0);
        expect(parseRetryAfter("Thu, 01 Jan 2026 12:01:00 GMT", T)).toBe(60_000);
        expect(parseRetryAfter("Thu, 01 Jan 2026 11:00:00 GMT", T)).toBe(0); // in the past
    });
    test("garbage is not a delay", () => {
        for (const v of ["-5", "1.5", "abc", "", "1e3", "0x10", "9999999999", "Thursday", "2026-01-01T12:01:00Z", null, undefined]) expect(parseRetryAfter(v as never, T)).toBeNull();
    });
    test("429 and 503 set a floor; other statuses reset the throttle count", () => {
        let r = onResponse(INITIAL_HOST_STATE, P, T, 429, "30");
        expect(r.state.notBefore).toBe(T + 30_000);
        expect(canStart(r.state, P, T + 29_999).allow).toBe(false);
        expect(canStart(r.state, P, T + 30_000).allow).toBe(true);
        r = onResponse(r.state, P, T + 30_000, 503, undefined);
        expect(r.state.notBefore).toBe(T + 30_000 + 2000); // second throttle: 2s backoff
        r = onResponse(r.state, P, T + 40_000, 200, undefined);
        expect(r.state.consecutiveThrottles).toBe(0);
    });
    test("a later Retry-After never shortens an earlier floor", () => {
        const a = onResponse(INITIAL_HOST_STATE, P, T, 429, "50").state;
        expect(onResponse(a, P, T + 1000, 429, "1").state.notBefore).toBe(T + 50_000);
    });
    test("an absurd Retry-After is flagged, not slept through", () => {
        const r = onResponse(INITIAL_HOST_STATE, P, T, 429, "86400");
        expect(r.tooLong).toBe(true);
        expect(r.state.notBefore).toBe(T + 86_400_000);
        expect(onResponse(INITIAL_HOST_STATE, P, T, 429, "59").tooLong).toBe(false);
    });
    test("backoff is exponential and capped without a header", () => {
        let s = INITIAL_HOST_STATE;
        const waits: number[] = [];
        for (let i = 0; i < 8; i++) {
            const r = onResponse(s, P, T, 503, "nonsense");
            waits.push(r.state.notBefore - T);
            s = { ...r.state, notBefore: 0 };
        }
        expect(waits.slice(0, 4)).toEqual([1000, 2000, 4000, 8000]);
        expect(Math.max(...waits)).toBeLessThanOrEqual(DEFAULT_RATE_POLICY.maxRetryAfterMs);
    });
});
