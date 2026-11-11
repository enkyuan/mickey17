/** Address classification for the SSRF guard. Pure; unparsable input is "not public". */

export type IpClass = { public: true } | { public: false; reason: string };

export function parseIPv4(s: string): number[] | null {
    const parts = s.split(".");
    if (parts.length !== 4) return null;
    const out: number[] = [];
    for (const p of parts) {
        // decimal only, no leading zeros (ambiguous octal), no signs
        if (!/^(0|[1-9]\d{0,2})$/.test(p)) return null;
        const n = Number(p);
        if (n > 255) return null;
        out.push(n);
    }
    return out;
}

/** Returns 8 groups of 16 bits, or null. Rejects zone ids and brackets. */
export function parseIPv6(input: string): number[] | null {
    let s = input;
    if (s.includes("%") || s.includes("[") || s.includes("]")) return null;
    let tail: number[] = [];
    const lastColon = s.lastIndexOf(":");
    if (lastColon === -1) return null;
    const maybeV4 = s.slice(lastColon + 1);
    if (maybeV4.includes(".")) {
        const v4 = parseIPv4(maybeV4);
        if (!v4) return null;
        tail = [(v4[0]! << 8) | v4[1]!, (v4[2]! << 8) | v4[3]!];
        s = s.slice(0, lastColon + 1) + "0:0"; // placeholder groups, replaced below
    }
    const dbl = s.split("::");
    if (dbl.length > 2) return null;
    const grp = (x: string): number[] | null => {
        if (x === "") return [];
        const out: number[] = [];
        for (const g of x.split(":")) {
            if (!/^[0-9a-fA-F]{1,4}$/.test(g)) return null;
            out.push(parseInt(g, 16));
        }
        return out;
    };
    const head = grp(dbl[0]!);
    if (!head) return null;
    let groups: number[];
    if (dbl.length === 2) {
        const rest = grp(dbl[1]!);
        if (!rest) return null;
        const fill = 8 - head.length - rest.length;
        if (fill < 1) return null;
        groups = [...head, ...new Array<number>(fill).fill(0), ...rest];
    } else {
        groups = head;
    }
    if (groups.length !== 8) return null;
    if (tail.length) {
        groups[6] = tail[0]!;
        groups[7] = tail[1]!;
    }
    return groups;
}

function v4num(a: number[]): number {
    return ((a[0]! << 24) | (a[1]! << 16) | (a[2]! << 8) | a[3]!) >>> 0;
}

const V4_DENY: ReadonlyArray<readonly [string, number, string]> = [
    ["0.0.0.0", 8, "this-network"],
    ["10.0.0.0", 8, "private"],
    ["100.64.0.0", 10, "carrier-grade-nat"],
    ["127.0.0.0", 8, "loopback"],
    ["169.254.0.0", 16, "link-local/metadata"],
    ["172.16.0.0", 12, "private"],
    ["192.0.0.0", 24, "ietf-protocol"],
    ["192.0.2.0", 24, "documentation"],
    ["192.88.99.0", 24, "6to4-relay"],
    ["192.168.0.0", 16, "private"],
    ["198.18.0.0", 15, "benchmarking"],
    ["198.51.100.0", 24, "documentation"],
    ["203.0.113.0", 24, "documentation"],
    ["224.0.0.0", 4, "multicast"],
    ["240.0.0.0", 4, "reserved"],
    ["168.63.129.16", 32, "azure-wireserver"],
    ["255.255.255.255", 32, "broadcast"],
];

export function classifyIPv4(a: number[]): IpClass {
    const n = v4num(a);
    for (const [base, bits, reason] of V4_DENY) {
        const b = v4num(parseIPv4(base)!);
        const mask = bits === 0 ? 0 : (~0 << (32 - bits)) >>> 0;
        if (((n & mask) >>> 0) === ((b & mask) >>> 0)) return { public: false, reason };
    }
    return { public: true };
}

function inV6(g: number[], prefix: number[], bits: number): boolean {
    let remaining = bits;
    for (let i = 0; i < 8 && remaining > 0; i++) {
        const take = Math.min(16, remaining);
        const mask = (0xffff << (16 - take)) & 0xffff;
        if ((g[i]! & mask) !== (prefix[i]! & mask)) return false;
        remaining -= take;
    }
    return true;
}

const v6 = (s: string): number[] => parseIPv6(s)!;

/**
 * IPv6 is allow-listed, not deny-listed: only 2000::/3 global unicast can be public, minus
 * the special-purpose blocks inside it. IPv4-in-IPv6 forms are judged by their embedded
 * IPv4 address; 6to4 and Teredo are refused outright.
 */
export function classifyIPv6(g: number[]): IpClass {
    // ::ffff:a.b.c.d (mapped) and ::a.b.c.d (compatible, includes :: and ::1)
    const zeros5 = g.slice(0, 5).every((x) => x === 0);
    if (zeros5 && g[5] === 0xffff) return classifyEmbedded(g, "ipv4-mapped");
    if (g.slice(0, 6).every((x) => x === 0)) return { public: false, reason: g[6] === 0 && g[7]! <= 1 ? "loopback/unspecified" : "ipv4-compatible" };
    if (inV6(g, v6("64:ff9b::"), 96)) return classifyEmbedded(g, "nat64");
    if (inV6(g, v6("64:ff9b:1::"), 48)) return { public: false, reason: "nat64-local" };
    if (!inV6(g, v6("2000::"), 3)) return { public: false, reason: "not-global-unicast" };
    if (inV6(g, v6("2002::"), 16)) return { public: false, reason: "6to4" };
    if (inV6(g, v6("2001::"), 23)) return { public: false, reason: "ietf-protocol/teredo" };
    if (inV6(g, v6("2001:db8::"), 32)) return { public: false, reason: "documentation" };
    if (inV6(g, v6("3fff::"), 20)) return { public: false, reason: "documentation" };
    return { public: true };
}

function classifyEmbedded(g: number[], via: string): IpClass {
    const c = classifyIPv4([g[6]! >> 8, g[6]! & 255, g[7]! >> 8, g[7]! & 255]);
    return c.public ? { public: true } : { public: false, reason: `${via}:${c.reason}` };
}

/** `addr` is textual IPv4 or IPv6 (no brackets). Anything unparsable is not public. */
export function classifyAddress(addr: string): IpClass {
    const a4 = parseIPv4(addr);
    if (a4) return classifyIPv4(a4);
    const a6 = parseIPv6(addr);
    if (a6) return classifyIPv6(a6);
    return { public: false, reason: "unparsable" };
}
