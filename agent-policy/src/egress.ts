import { classifyAddress, parseIPv6 } from "./ip.ts";

export interface EgressPolicyInput {
    /** Exact hostnames, or `*.example.com` (subdomains only, never the apex). */
    readonly allowHosts: readonly string[];
    readonly allowHttp?: boolean;
    readonly allowedPorts?: readonly number[];
    readonly allowIpLiterals?: boolean;
    readonly maxRedirects?: number;
}

export interface EgressPolicy {
    readonly exact: ReadonlySet<string>;
    readonly suffixes: readonly string[]; // ".example.com"
    readonly allowHttp: boolean;
    readonly ports: ReadonlySet<number>;
    readonly allowIpLiterals: boolean;
    readonly maxRedirects: number;
}

/** A small, non-exhaustive guard against wildcards on public suffixes. */
const PUBLIC_SUFFIXES = new Set(["co.uk", "org.uk", "ac.uk", "gov.uk", "com.au", "net.au", "org.au", "co.jp", "co.nz", "co.in", "co.za", "com.br", "com.cn", "com.mx", "github.io", "herokuapp.com", "vercel.app", "netlify.app", "pages.dev", "workers.dev", "appspot.com", "cloudfront.net", "amazonaws.com", "s3.amazonaws.com", "blogspot.com", "web.app", "firebaseapp.com", "azurewebsites.net", "cloudapp.azure.com", "co.kr", "or.kr", "com.tr", "co.id", "com.sg", "com.hk", "com.tw", "com.ar", "co.il", "ne.jp", "or.jp", "ac.jp", "go.jp", "edu.au", "gov.au", "com.my", "com.ph", "com.vn", "co.th", "com.pl", "fly.dev", "onrender.com", "railway.app", "ngrok.io", "ngrok-free.app", "trycloudflare.com", "pythonanywhere.com", "repl.co", "glitch.me", "surge.sh"]);

const LABEL = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;

/** Throws on a malformed allowlist: a typo must not become a silent allow-all. */
export function compileEgressPolicy(p: EgressPolicyInput): EgressPolicy {
    const exact = new Set<string>();
    const suffixes: string[] = [];
    if (p.allowHosts.length === 0) throw new RangeError("empty host allowlist");
    for (const raw of p.allowHosts) {
        const h = raw.toLowerCase();
        const wild = h.startsWith("*.");
        const body = wild ? h.slice(2) : h;
        if (!wild && body.includes(":")) {
            if (!parseIPv6(body)) throw new RangeError("invalid allowlist entry");
            exact.add(body);
            continue;
        }
        const labels = body.split(".");
        if (!labels.every((l) => LABEL.test(l))) throw new RangeError("invalid allowlist entry");
        if (wild && (labels.length < 2 || PUBLIC_SUFFIXES.has(body))) throw new RangeError("wildcard must cover a registrable domain");
        if (wild) suffixes.push(`.${body}`);
        else exact.add(body);
    }
    return {
        exact,
        suffixes,
        allowHttp: p.allowHttp ?? false,
        ports: new Set(p.allowedPorts ?? [443]),
        allowIpLiterals: p.allowIpLiterals ?? false,
        maxRedirects: p.maxRedirects ?? 5,
    };
}

export type EgressRefusalCode =
    | "invalid-url"
    | "scheme-not-allowed"
    | "credentials-in-url"
    | "port-not-allowed"
    | "host-not-allowed"
    | "ip-literal-denied"
    | "internal-hostname"
    | "private-address"
    | "resolution-required"
    | "resolution-empty"
    | "redirect-limit"
    | "redirect-scheme-downgrade"
    | "redirect-cross-host-write"
    | "not-a-redirect";

const issuedEgress = new WeakSet<object>();

/** True only for an allow decision produced by evaluateEgress/evaluateRedirect in this process. */
export function isIssuedEgress(x: unknown): x is EgressAllow {
    return typeof x === "object" && x !== null && issuedEgress.has(x);
}

export type EgressAllow = { allow: true; url: string; host: string; port: number; /** connect to exactly these addresses */ pinned: readonly string[]; stripCredentials: boolean };

export type EgressDecision =
    | EgressAllow
    | { allow: false; code: EgressRefusalCode; message: string };

const deny = (code: EgressRefusalCode, message: string): EgressDecision => ({ allow: false, code, message });

const INTERNAL_SUFFIXES = [".localhost", ".local", ".internal", ".localdomain", ".home.arpa", ".lan", ".intranet", ".corp", ".private"];
const INTERNAL_EXACT = new Set(["localhost", "metadata", "metadata.google.internal", "instance-data", "kubernetes", "kubernetes.default"]);

function hostAllowed(host: string, p: EgressPolicy): boolean {
    if (p.exact.has(host)) return true;
    return p.suffixes.some((s) => host.endsWith(s) && host.length > s.length);
}

function ipLiteral(host: string): string | null {
    if (host.startsWith("[") && host.endsWith("]")) {
        const inner = host.slice(1, -1);
        return parseIPv6(inner) ? inner : "";
    }
    // WHATWG URL already canonicalised decimal/hex/octal forms to dotted quads.
    return /^\d+\.\d+\.\d+\.\d+$/.test(host) ? host : null;
}

/**
 * Phase one and two together. `resolved` is what DNS returned for the host *for this
 * connection*; the caller must connect to `pinned` and not resolve again (that second
 * lookup is the rebinding window). Every returned address must be public: a name that
 * resolves to both a public and a private address is refused.
 */
export function evaluateEgress(rawUrl: string, policy: EgressPolicy, resolved: readonly string[] | undefined, opts: { stripCredentials?: boolean } = {}): EgressDecision {
    let u: URL;
    try {
        u = new URL(rawUrl);
    } catch {
        return deny("invalid-url", "URL does not parse.");
    }
    if (u.protocol !== "https:" && !(policy.allowHttp && u.protocol === "http:")) return deny("scheme-not-allowed", "Only https is permitted.");
    if (u.username !== "" || u.password !== "") return deny("credentials-in-url", "URLs with embedded credentials are refused.");
    const port = u.port === "" ? (u.protocol === "https:" ? 443 : 80) : Number(u.port);
    if (!policy.ports.has(port)) return deny("port-not-allowed", "Port is not on the allowlist.");

    const host = u.hostname.toLowerCase().replace(/\.$/, "");
    if (host === "") return deny("invalid-url", "URL has no host.");
    const lit = ipLiteral(host);
    let addrs: readonly string[];
    if (lit !== null) {
        if (lit === "") return deny("invalid-url", "Malformed IP literal.");
        if (!policy.allowIpLiterals) return deny("ip-literal-denied", "IP-literal hosts are refused.");
        if (!hostAllowed(lit, policy)) return deny("host-not-allowed", "Host is not on the allowlist.");
        addrs = [lit];
    } else {
        if (INTERNAL_EXACT.has(host) || INTERNAL_SUFFIXES.some((s) => host.endsWith(s)) || !host.includes(".")) {
            return deny("internal-hostname", "Internal and single-label hostnames are refused.");
        }
        if (!hostAllowed(host, policy)) return deny("host-not-allowed", "Host is not on the allowlist.");
        if (resolved === undefined) return deny("resolution-required", "Resolve the host and pass the addresses for validation.");
        if (resolved.length === 0) return deny("resolution-empty", "Host resolved to no addresses.");
        addrs = resolved;
    }
    for (const a of addrs) {
        if (!classifyAddress(a).public) return deny("private-address", "Host resolves to a non-public address.");
    }
    const allowed: EgressAllow = Object.freeze({ allow: true as const, url: u.href, host, port, pinned: Object.freeze([...addrs]) as readonly string[], stripCredentials: opts.stripCredentials ?? false });
    issuedEgress.add(allowed);
    return allowed;
}

export interface RedirectInput {
    readonly from: string;
    readonly location: string;
    readonly status: number;
    readonly method: string;
    /** Redirects already followed. */
    readonly hop: number;
}

/**
 * Re-checks a redirect target from scratch (allowlist, SSRF guard, scheme). A redirect
 * may never leave https, change host on a request that carries a body or has side
 * effects, or carry credentials to another origin.
 */
export function evaluateRedirect(r: RedirectInput, policy: EgressPolicy, resolved: readonly string[] | undefined): EgressDecision {
    if (![301, 302, 303, 307, 308].includes(r.status)) return deny("not-a-redirect", "Response status is not a redirect.");
    if (r.hop + 1 > policy.maxRedirects) return deny("redirect-limit", "Too many redirects.");
    let from: URL;
    let to: URL;
    try {
        from = new URL(r.from);
        to = new URL(r.location, from);
    } catch {
        return deny("invalid-url", "Redirect target does not parse.");
    }
    if (from.protocol === "https:" && to.protocol !== "https:") return deny("redirect-scheme-downgrade", "Redirect downgrades the scheme.");
    const sameOrigin = from.origin === to.origin;
    const safeMethod = r.method.toUpperCase() === "GET" || r.method.toUpperCase() === "HEAD";
    if (!sameOrigin && !safeMethod) return deny("redirect-cross-host-write", "Cross-origin redirects are refused for requests that are not GET or HEAD.");
    return evaluateEgress(to.href, policy, resolved, { stripCredentials: !sameOrigin });
}
