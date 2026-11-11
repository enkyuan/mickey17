import { canonicalJson, sha256Hex } from "./bytes.ts";
import type { Effect } from "./types.ts";

export type Begin =
    | { state: "new" }
    | { state: "in-flight" }
    /** A previous attempt may have reached the server and its outcome is unknown. */
    | { state: "unknown-outcome" }
    | { state: "completed"; result: StoredResult }
    /** Same key, different request: refused, never executed. */
    | { state: "conflict" };

export interface StoredResult {
    readonly status: string;
    readonly receiptId: string;
}

export interface IdempotencyStore {
    begin(key: string, fingerprint: string, now: number, ttlMs: number): Promise<Begin>;
    complete(key: string, fingerprint: string, result: StoredResult): Promise<void>;
    /** The request provably never left the process: the key may be used again. */
    release(key: string, fingerprint: string): Promise<void>;
    /** The request may have been sent and the outcome is unknown. */
    markUnknown(key: string, fingerprint: string): Promise<void>;
    /**
     * Human reconciliation of an unknown outcome: `not-executed` frees the key, a stored
     * result records what happened. Nothing else can clear an unknown entry.
     */
    reconcile(key: string, fingerprint: string, outcome: "not-executed" | StoredResult): Promise<boolean>;
}

export async function requestFingerprint(actionId: string, args: unknown): Promise<string> {
    return sha256Hex(canonicalJson({ actionId, args }));
}

interface Entry {
    fingerprint: string;
    expiresAt: number;
    state: "in-flight" | "unknown" | "completed";
    result?: StoredResult;
}

export class InMemoryIdempotencyStore implements IdempotencyStore {
    readonly #m = new Map<string, Entry>();

    begin(key: string, fingerprint: string, now: number, ttlMs: number): Promise<Begin> {
        const e = this.#m.get(key);
        // An entry that may have reached the server never silently expires into "new":
        // an expired in-flight or unknown entry stays blocked as unknown-outcome.
        if (e && e.expiresAt <= now && e.state !== "completed") e.state = "unknown";
        if (e && (e.expiresAt > now || e.state === "unknown")) {
            if (e.fingerprint !== fingerprint) return Promise.resolve({ state: "conflict" });
            if (e.state === "completed" && e.result) return Promise.resolve({ state: "completed", result: e.result });
            return Promise.resolve({ state: e.state === "unknown" ? "unknown-outcome" : "in-flight" });
        }
        this.#m.set(key, { fingerprint, expiresAt: now + ttlMs, state: "in-flight" });
        return Promise.resolve({ state: "new" });
    }

    complete(key: string, fingerprint: string, result: StoredResult): Promise<void> {
        const e = this.#m.get(key);
        if (e && e.fingerprint === fingerprint) {
            e.state = "completed";
            e.result = result;
        }
        return Promise.resolve();
    }

    release(key: string, fingerprint: string): Promise<void> {
        const e = this.#m.get(key);
        if (e && e.fingerprint === fingerprint && e.state === "in-flight") this.#m.delete(key);
        return Promise.resolve();
    }

    reconcile(key: string, fingerprint: string, outcome: "not-executed" | StoredResult): Promise<boolean> {
        const e = this.#m.get(key);
        if (!e || e.fingerprint !== fingerprint || e.state === "completed") return Promise.resolve(false);
        if (outcome === "not-executed") this.#m.delete(key);
        else {
            e.state = "completed";
            e.result = outcome;
        }
        return Promise.resolve(true);
    }

    markUnknown(key: string, fingerprint: string): Promise<void> {
        const e = this.#m.get(key);
        if (e && e.fingerprint === fingerprint && e.state === "in-flight") e.state = "unknown";
        return Promise.resolve();
    }
}

const NEVER_RETRY: readonly Effect[] = ["write-irreversible", "financial", "account-control"];

export interface RetryInput {
    readonly idempotent: boolean;
    readonly effect: Effect;
    readonly attempt: number; // 1-based number of attempts already made
    readonly maxAttempts: number;
    readonly retryable: boolean;
}

/**
 * Pure. Non-idempotent actions, and every irreversible / financial / account-control
 * action even if declared idempotent, are never retried by this library.
 */
export function shouldRetry(i: RetryInput): boolean {
    if (!i.idempotent) return false;
    if (NEVER_RETRY.includes(i.effect)) return false;
    return i.retryable && i.attempt < i.maxAttempts;
}

export type AttemptOutcome<T> =
    | { ok: true; value: T }
    | { ok: false; retryable: boolean; /** true only if the request provably never left the process */ neverSent: boolean };

export type RunResult<T> =
    | { kind: "executed"; attempts: number; outcome: AttemptOutcome<T> }
    | { kind: "replayed"; result: StoredResult }
    | { kind: "refused"; code: "idempotency-in-flight" | "idempotency-unknown-outcome" | "idempotency-key-conflict" };

export interface RunOptions<T> {
    readonly store: IdempotencyStore;
    readonly key: string;
    readonly fingerprint: string;
    readonly now: number;
    readonly ttlMs: number;
    readonly idempotent: boolean;
    readonly effect: Effect;
    readonly maxAttempts?: number;
    readonly sleep?: (ms: number) => Promise<void>;
    readonly toStored: (value: T) => StoredResult;
}

/**
 * Runs `attempt` under an idempotency key. A duplicate returns the stored result and is
 * never re-executed. Retries happen only where shouldRetry allows; otherwise `attempt`
 * is invoked at most once per key.
 */
export async function runWithIdempotency<T>(opts: RunOptions<T>, attempt: (n: number) => Promise<AttemptOutcome<T>>): Promise<RunResult<T>> {
    const begin = await opts.store.begin(opts.key, opts.fingerprint, opts.now, opts.ttlMs);
    if (begin.state === "completed") return { kind: "replayed", result: begin.result };
    if (begin.state === "in-flight") return { kind: "refused", code: "idempotency-in-flight" };
    if (begin.state === "unknown-outcome") return { kind: "refused", code: "idempotency-unknown-outcome" };
    if (begin.state === "conflict") return { kind: "refused", code: "idempotency-key-conflict" };

    const max = opts.maxAttempts ?? 1;
    let n = 0;
    for (;;) {
        n += 1;
        let outcome: AttemptOutcome<T>;
        try {
            outcome = await attempt(n);
        } catch {
            // Threw after possibly sending: ambiguous. Never retried here.
            await opts.store.markUnknown(opts.key, opts.fingerprint);
            return { kind: "executed", attempts: n, outcome: { ok: false, retryable: false, neverSent: false } };
        }
        if (outcome.ok) {
            await opts.store.complete(opts.key, opts.fingerprint, opts.toStored(outcome.value));
            return { kind: "executed", attempts: n, outcome };
        }
        if (shouldRetry({ idempotent: opts.idempotent, effect: opts.effect, attempt: n, maxAttempts: max, retryable: outcome.retryable })) {
            await opts.sleep?.(Math.min(30_000, 250 * 2 ** (n - 1)));
            continue;
        }
        if (outcome.neverSent) await opts.store.release(opts.key, opts.fingerprint);
        else await opts.store.markUnknown(opts.key, opts.fingerprint);
        return { kind: "executed", attempts: n, outcome };
    }
}
