/** robots.txt per RFC 9309, evaluated purely over text the caller fetched. */

export interface RobotsRule {
    readonly allow: boolean;
    readonly pattern: string;
}
export interface RobotsGroup {
    readonly agents: readonly string[];
    readonly rules: readonly RobotsRule[];
    readonly crawlDelay: number | null;
}
export type Robots =
    | { readonly kind: "rules"; readonly groups: readonly RobotsGroup[] }
    | { readonly kind: "allow-all"; readonly reason: string }
    | { readonly kind: "disallow-all"; readonly reason: string };

const MAX_BYTES = 500 * 1024;
const MAX_PATTERN = 1024;

export function parseRobots(text: string): Robots {
    let clipped = text;
    if (clipped.length > MAX_BYTES) {
        let cut = MAX_BYTES;
        clipped = clipped.slice(0, cut);
        // drop the final partial line: a truncated Allow/Disallow would become a different rule
        const nl = Math.max(clipped.lastIndexOf("\n"), clipped.lastIndexOf("\r"));
        clipped = nl >= 0 ? clipped.slice(0, nl) : "";
    }
    const groups: { agents: string[]; rules: RobotsRule[]; crawlDelay: number | null }[] = [];
    let cur: { agents: string[]; rules: RobotsRule[]; crawlDelay: number | null } | null = null;
    let lastWasAgent = false;
    for (const raw of clipped.replace(/^﻿/, "").split(/\r\n|\r|\n/)) {
        const line = raw.replace(/#.*$/, "").trim();
        if (line === "") continue;
        const i = line.indexOf(":");
        if (i === -1) continue;
        const field = line.slice(0, i).trim().toLowerCase();
        const value = line.slice(i + 1).trim();
        if (field === "user-agent") {
            if (!cur || !lastWasAgent) {
                cur = { agents: [], rules: [], crawlDelay: null };
                groups.push(cur);
            }
            cur.agents.push(value.toLowerCase().replace(/\/.*$/, "").trim()); // "MochiAgent/1.0" -> "mochiagent"
            lastWasAgent = true;
            continue;
        }
        lastWasAgent = false;
        if (!cur) continue;
        if (field === "allow" || field === "disallow") {
            if (value === "") continue; // empty Disallow allows everything; empty Allow is a no-op
            if (value.length > MAX_PATTERN) {
                // an over-long Allow is dropped (safe); an over-long Disallow is kept as a prefix
                if (field === "disallow") cur.rules.push({ allow: false, pattern: value.slice(0, MAX_PATTERN) });
                continue;
            }
            cur.rules.push({ allow: field === "allow", pattern: value });
        } else if (field === "crawl-delay") {
            const n = Number(value);
            if (/^\d+(\.\d+)?$/.test(value) && Number.isFinite(n)) cur.crawlDelay = n;
        }
    }
    return { kind: "rules", groups };
}

/**
 * Maps the fetch outcome of /robots.txt to a policy. `status === null` is a network
 * failure. 404/410 mean "no robots file". 5xx and failures disallow everything (RFC 9309
 * 2.3.1.4). We also treat 401/403 as disallow-all, stricter than the RFC: a site that
 * refuses the robots file is not inviting automated access.
 */
export function robotsFromFetch(status: number | null, body: string): Robots {
    if (status === null) return { kind: "disallow-all", reason: "robots.txt unreachable" };
    if (status >= 200 && status < 300) return parseRobots(body);
    if (status === 404 || status === 410) return { kind: "allow-all", reason: "no robots.txt" };
    if (status === 401 || status === 403) return { kind: "disallow-all", reason: "robots.txt access denied" };
    return { kind: "disallow-all", reason: "robots.txt unavailable or unrecognised status" };
}

/** Uppercase %XX; decode %XX of unreserved characters so equal paths compare equal. */
function normalizePct(input: string): string {
    // non-ASCII is compared in its UTF-8 percent-encoded form (RFC 9309 2.2.2)
    const wellFormed = input.replace(/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g, "\ufffd");
    const s = wellFormed.replace(/[^\x00-\x7f]/gu, (c) => encodeURIComponent(c));
    // slash runs are merged (as lenient servers do) on both the rule and the path
    return s.replace(/\/{2,}/g, "/").replace(/%([0-9a-fA-F]{2})/g, (_m, h: string) => {
        const c = String.fromCharCode(parseInt(h, 16));
        return /[A-Za-z0-9\-._~]/.test(c) ? c : `%${h.toUpperCase()}`;
    });
}

/** Prefix match with `*` and a trailing `$` anchor, iterative (no regex, no ReDoS). */
export function patternMatches(patternRaw: string, pathRaw: string): boolean {
    const pattern = normalizePct(patternRaw);
    const path = normalizePct(pathRaw);
    const anchored = pattern.endsWith("$");
    const p = anchored ? pattern.slice(0, -1) : pattern;
    let pi = 0;
    let si = 0;
    let star = -1;
    let mark = 0;
    while (si < path.length) {
        if (pi >= p.length && !anchored) return true; // pattern fully consumed: prefix match
        if (pi < p.length && p[pi] === "*") {
            star = pi++;
            mark = si;
        } else if (pi < p.length && p[pi] === path[si]) {
            pi++;
            si++;
        } else if (star !== -1) {
            pi = star + 1;
            si = ++mark;
        } else {
            return false;
        }
    }
    while (pi < p.length && p[pi] === "*") pi++;
    return pi === p.length;
}

export interface RobotsVerdict {
    readonly allowed: boolean;
    readonly reason: string;
    readonly crawlDelay: number | null;
}

/** `productToken` is our crawler token, e.g. "MochiAgent". `target` is path plus optional query. */
export function evaluateRobots(robots: Robots, productToken: string, target: string): RobotsVerdict {
    if (robots.kind === "allow-all") return { allowed: true, reason: robots.reason, crawlDelay: null };
    if (robots.kind === "disallow-all") return { allowed: false, reason: robots.reason, crawlDelay: null };
    const token = productToken.toLowerCase().replace(/\/.*$/, "").trim();
    const matching = robots.groups.filter((g) => g.agents.includes(token));
    const groups = matching.length > 0 ? matching : robots.groups.filter((g) => g.agents.includes("*"));
    const delay = groups.map((g) => g.crawlDelay).find((d) => d !== null) ?? null;
    if (groups.length === 0) return { allowed: true, reason: "no applicable group", crawlDelay: null };
    // Evaluate the path the server will actually see: dot-segments and %2e are resolved by the URL parser.
    let path: string;
    try {
        // Backslashes and leading slash runs are merged the way lenient servers do, so a
        // target like "//admin/x" is judged as "/admin/x" and not parsed as a host.
        const pathOnly = ("/" + target.replaceAll("\\", "/")).replace(/^\/+/, "/");
        const u = new URL(`https://robots.invalid${pathOnly}`);
        // dot-segments can recreate a slash run ("/x/..//admin"): merge after resolution too
        path = u.pathname.replace(/\/{2,}/g, "/") + u.search;
    } catch {
        return { allowed: false, reason: "unparsable target", crawlDelay: delay };
    }
    let best: RobotsRule | null = null;
    for (const g of groups) {
        for (const r of g.rules) {
            if (!patternMatches(r.pattern, path)) continue;
            const len = normalizePct(r.pattern).length;
            const bestLen = best ? normalizePct(best.pattern).length : -1;
            if (!best || len > bestLen || (len === bestLen && r.allow && !best.allow)) best = r;
        }
    }
    if (!best) return { allowed: true, reason: "no matching rule", crawlDelay: delay };
    return { allowed: best.allow, reason: best.allow ? "allowed by rule" : "disallowed by rule", crawlDelay: delay };
}
