/** Per-host concurrency, Retry-After and spacing. Pure state transitions; the caller owns the clock. */

export interface RatePolicy {
    readonly maxConcurrent: number;
    /** Minimum gap between request starts to one host. */
    readonly minIntervalMs: number;
    /** A Retry-After beyond this is treated as "stop for now", not slept through. */
    readonly maxRetryAfterMs: number;
}

export const DEFAULT_RATE_POLICY: RatePolicy = { maxConcurrent: 2, minIntervalMs: 250, maxRetryAfterMs: 3_600_000 };

export interface HostState {
    readonly inFlight: number;
    readonly lastStart: number;
    /** No request before this instant (Retry-After, backoff, Crawl-delay). */
    readonly notBefore: number;
    readonly consecutiveThrottles: number;
}

export const INITIAL_HOST_STATE: HostState = { inFlight: 0, lastStart: Number.NEGATIVE_INFINITY, notBefore: 0, consecutiveThrottles: 0 };

export type StartDecision = { allow: true } | { allow: false; reason: "concurrency" | "backoff" | "interval"; waitMs: number | null };

export function canStart(s: HostState, p: RatePolicy, now: number): StartDecision {
    if (now < s.notBefore) return { allow: false, reason: "backoff", waitMs: s.notBefore - now };
    if (s.inFlight >= p.maxConcurrent) return { allow: false, reason: "concurrency", waitMs: null };
    const gap = s.lastStart + p.minIntervalMs - now;
    if (gap > 0) return { allow: false, reason: "interval", waitMs: gap };
    return { allow: true };
}

export function onStart(s: HostState, now: number): HostState {
    return { ...s, inFlight: s.inFlight + 1, lastStart: now };
}

export function onFinish(s: HostState): HostState {
    return { ...s, inFlight: Math.max(0, s.inFlight - 1) };
}

const IMF = /^(Mon|Tue|Wed|Thu|Fri|Sat|Sun), \d{2} (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{4} \d{2}:\d{2}:\d{2} GMT$/;

/** Delay in ms from a Retry-After header (delta-seconds or IMF-fixdate), or null if invalid. */
export function parseRetryAfter(value: string | null | undefined, now: number): number | null {
    if (typeof value !== "string") return null;
    const v = value.trim();
    if (/^\d{1,9}$/.test(v)) return Number(v) * 1000;
    if (IMF.test(v)) {
        const t = Date.parse(v);
        if (Number.isFinite(t)) return Math.max(0, t - now);
    }
    return null;
}

/**
 * Applies a response. 429 and 503 set a floor on the next request: the server's
 * Retry-After when valid, else exponential backoff. If the server asks for longer than
 * `maxRetryAfterMs` the host is parked for that long and the caller should surface
 * `blocked`, not wait.
 */
export function onResponse(s: HostState, p: RatePolicy, now: number, status: number, retryAfter: string | null | undefined): { state: HostState; tooLong: boolean } {
    if (status !== 429 && status !== 503) return { state: { ...s, consecutiveThrottles: 0 }, tooLong: false };
    const asked = parseRetryAfter(retryAfter, now);
    const backoff = Math.min(p.maxRetryAfterMs, 1000 * 2 ** Math.min(s.consecutiveThrottles, 12));
    const delay = asked !== null ? Math.max(asked, 0) : backoff;
    const tooLong = delay > p.maxRetryAfterMs;
    return {
        state: { ...s, notBefore: Math.max(s.notBefore, now + delay), consecutiveThrottles: s.consecutiveThrottles + 1 },
        tooLong,
    };
}

/** Crawl-delay from robots.txt raises the spacing floor. */
export function withCrawlDelay(p: RatePolicy, crawlDelaySeconds: number | null): RatePolicy {
    if (crawlDelaySeconds === null || !Number.isFinite(crawlDelaySeconds) || crawlDelaySeconds < 0) return p;
    return { ...p, minIntervalMs: Math.max(p.minIntervalMs, Math.min(crawlDelaySeconds * 1000, p.maxRetryAfterMs)) };
}
