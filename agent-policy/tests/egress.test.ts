import { describe, expect, test } from "bun:test";
import { compileEgressPolicy, evaluateEgress, evaluateRedirect } from "../src/egress.ts";
import { classifyAddress, parseIPv4, parseIPv6 } from "../src/ip.ts";

const policy = compileEgressPolicy({ allowHosts: ["shop.example.com", "*.cdn.example.com", "api.example.org"] });
const PUBLIC = ["93.184.216.34"];
const ok = (url: string, res: readonly string[] | null = PUBLIC) => evaluateEgress(url, policy, res ?? undefined);
const code = (d: ReturnType<typeof ok>) => (d.allow ? "allow" : d.code);

describe("allowlist", () => {
    test("exact and wildcard hosts; wildcard excludes the apex and look-alikes", () => {
        expect(code(ok("https://shop.example.com/cart"))).toBe("allow");
        expect(code(ok("https://img.cdn.example.com/a.png"))).toBe("allow");
        expect(code(ok("https://a.b.cdn.example.com/a.png"))).toBe("allow");
        expect(code(ok("https://cdn.example.com/"))).toBe("host-not-allowed");
        expect(code(ok("https://evilcdn.example.com/"))).toBe("host-not-allowed");
        expect(code(ok("https://cdn.example.com.evil.net/"))).toBe("host-not-allowed");
        expect(code(ok("https://shop.example.com.evil.net/"))).toBe("host-not-allowed");
        expect(code(ok("https://example.com/"))).toBe("host-not-allowed");
        expect(code(ok("https://SHOP.EXAMPLE.COM./x"))).toBe("allow");
    });

    test("userinfo tricks resolve to the real host", () => {
        expect(code(ok("https://shop.example.com@evil.net/"))).toBe("credentials-in-url");
        expect(code(ok("https://:secret@shop.example.com/"))).toBe("credentials-in-url"); // password only
        expect(code(ok("https://evil.net#@shop.example.com/"))).toBe("host-not-allowed");
        expect(code(ok("https://evil.net\\@shop.example.com/"))).not.toBe("allow");
    });

    test("scheme and port", () => {
        expect(code(ok("http://shop.example.com/"))).toBe("scheme-not-allowed");
        expect(code(ok("ftp://shop.example.com/"))).toBe("scheme-not-allowed");
        expect(code(ok("file:///etc/passwd"))).toBe("scheme-not-allowed");
        expect(code(ok("javascript:alert(1)"))).toBe("scheme-not-allowed");
        expect(code(ok("https://shop.example.com:8443/"))).toBe("port-not-allowed");
        expect(code(ok("https://shop.example.com:443/"))).toBe("allow");
        const loose = compileEgressPolicy({ allowHosts: ["shop.example.com"], allowHttp: true, allowedPorts: [80, 443] });
        expect(evaluateEgress("http://shop.example.com/", loose, PUBLIC).allow).toBe(true);
    });

    test("malformed allowlists throw instead of allowing everything", () => {
        for (const bad of [[], ["*"], ["*.com"], ["https://x.example.com"], ["x.example.com/path"], ["x .example.com"], [""], ["-a.example.com"]]) {
            expect(() => compileEgressPolicy({ allowHosts: bad })).toThrow();
        }
    });
});

describe("SSRF: literal addresses", () => {
    const lit = compileEgressPolicy({ allowHosts: ["127.0.0.1", "10.0.0.5", "169.254.169.254", "93.184.216.34"], allowIpLiterals: true });
    test("IP literals are refused by default", () => {
        for (const u of ["https://93.184.216.34/", "https://[2606:4700::1111]/", "https://127.0.0.1/"]) expect(code(ok(u, []))).toBe("ip-literal-denied");
    });
    test("even when literals are allowed, non-public ones are refused", () => {
        for (const u of ["https://127.0.0.1/", "https://10.0.0.5/", "https://169.254.169.254/latest/meta-data/"]) {
            const d = evaluateEgress(u, lit, undefined);
            expect(d.allow ? "allow" : d.code).toBe("private-address");
        }
        expect(evaluateEgress("https://93.184.216.34/", lit, undefined).allow).toBe(true);
    });
    test("alternate IPv4 spellings collapse to the real address before the check", () => {
        for (const u of ["https://2130706433/", "https://0x7f000001/", "https://017700000001/", "https://127.1/", "https://0177.0.0.1/", "https://0x7f.0.0.1/", "https://127。0。0。1/", "https://①②⑦.0.0.1/"]) {
            const d = ok(u, []);
            expect(d.allow).toBe(false);
        }
    });
    test("IPv6 forms of loopback, mapped, link-local, ULA and metadata are refused", () => {
        const l6 = compileEgressPolicy({ allowHosts: ["::1", "::ffff:7f00:1", "fe80::1", "fd00:ec2::254"], allowIpLiterals: true });
        for (const h of ["[::1]", "[::ffff:127.0.0.1]", "[::ffff:7f00:1]", "[0:0:0:0:0:ffff:7f00:1]", "[fe80::1]", "[fd00:ec2::254]", "[::]", "[::127.0.0.1]"]) {
            const d = evaluateEgress(`https://${h}/`, l6, undefined);
            expect(d.allow, h).toBe(false);
        }
    });
});

describe("SSRF: DNS-rebinding-style inputs", () => {
    test("a public name resolving to a private address is refused", () => {
        for (const a of ["127.0.0.1", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254", "100.100.100.200", "0.0.0.0", "::1", "fe80::1", "fc00::1", "fd00:ec2::254", "::ffff:10.0.0.1", "::ffff:a00:1", "64:ff9b::7f00:1", "2002:7f00:1::1", "2001:0:4136:e378:8000:63bf:3fff:fdd2", "224.0.0.1", "255.255.255.255", "168.63.129.16", "192.0.0.192"]) {
            expect(code(ok("https://shop.example.com/", [a])), a).toBe("private-address");
        }
    });
    test("every address must be public: a mixed answer is refused", () => {
        expect(code(ok("https://shop.example.com/", ["93.184.216.34", "127.0.0.1"]))).toBe("private-address");
        expect(code(ok("https://shop.example.com/", ["127.0.0.1", "93.184.216.34"]))).toBe("private-address");
        expect(code(ok("https://shop.example.com/", ["93.184.216.34", "2606:2800:220:1:248:1893:25c8:1946"]))).toBe("allow");
    });
    test("unresolved, empty and unparsable resolver output are refused", () => {
        expect(code(ok("https://shop.example.com/", null))).toBe("resolution-required");
        expect(code(ok("https://shop.example.com/", []))).toBe("resolution-empty");
        for (const junk of ["", "not-an-ip", "93.184.216", "93.184.216.34.5", "093.184.216.34", "1.2.3.256", "[::1]", "::1%eth0", "fe80::1%lo0", "93.184.216.34 "]) {
            expect(code(ok("https://shop.example.com/", [junk])), junk).toBe("private-address");
        }
    });
    test("the allowed result pins the validated addresses", () => {
        const d = ok("https://shop.example.com/x");
        expect(d.allow && d.pinned).toEqual(PUBLIC);
    });
    test("allowlisted hosts that look internal are still refused", () => {
        const p = compileEgressPolicy({ allowHosts: ["localhost", "db.internal", "metadata.google.internal", "x.local", "intranet"] });
        for (const h of ["localhost", "db.internal", "metadata.google.internal", "x.local", "intranet"]) {
            expect(evaluateEgress(`https://${h}/`, p, PUBLIC).allow, h).toBe(false);
        }
    });
});

describe("classification", () => {
    test("public addresses", () => {
        for (const a of ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700:4700::1111", "2a00:1450:4001:81b::200e", "::ffff:8.8.8.8"]) {
            expect(classifyAddress(a).public, a).toBe(true);
        }
    });
    test("v4 range edges", () => {
        const pub = (a: string) => classifyAddress(a).public;
        expect([pub("172.15.255.255"), pub("172.16.0.0"), pub("172.31.255.255"), pub("172.32.0.0")]).toEqual([true, false, false, true]);
        expect([pub("100.63.255.255"), pub("100.64.0.0"), pub("100.127.255.255"), pub("100.128.0.0")]).toEqual([true, false, false, true]);
        expect([pub("169.253.255.255"), pub("169.254.0.0"), pub("169.254.255.255"), pub("169.255.0.0")]).toEqual([true, false, false, true]);
        expect([pub("198.17.255.255"), pub("198.18.0.0"), pub("198.19.255.255"), pub("198.20.0.0")]).toEqual([true, false, false, true]);
        expect([pub("11.0.0.0"), pub("9.255.255.255"), pub("126.255.255.255"), pub("128.0.0.0")]).toEqual([true, true, true, true]);
        expect([pub("223.255.255.255"), pub("224.0.0.0"), pub("240.0.0.1")]).toEqual([true, false, false]);
    });
    test("parsers reject ambiguous spellings", () => {
        expect(parseIPv4("01.2.3.4")).toBeNull();
        expect(parseIPv4("1.2.3")).toBeNull();
        expect(parseIPv4("1.2.3.4")).toEqual([1, 2, 3, 4]);
        expect(parseIPv6("::1")).toEqual([0, 0, 0, 0, 0, 0, 0, 1]);
        expect(parseIPv6("::ffff:1.2.3.4")).toEqual([0, 0, 0, 0, 0, 0xffff, 0x0102, 0x0304]);
        expect(parseIPv6("1::2::3")).toBeNull();
        expect(parseIPv6("12345::1")).toBeNull();
        expect(parseIPv6("1:2:3:4:5:6:7:8:9")).toBeNull();
        expect(parseIPv6("fe80::1%eth0")).toBeNull();
    });
});

describe("redirects are re-checked", () => {
    const r = (location: string, over = {}) => evaluateRedirect({ from: "https://shop.example.com/a", location, status: 302, method: "GET", hop: 0, ...over }, policy, PUBLIC);
    test("same host and allowlisted host pass; unlisted host is refused", () => {
        expect(r("/b").allow).toBe(true);
        expect(r("https://api.example.org/x").allow).toBe(true);
        expect(code(r("https://evil.net/x"))).toBe("host-not-allowed");
    });
    test("redirect to a private address or metadata is refused even via an allowlisted name", () => {
        const d = evaluateRedirect({ from: "https://shop.example.com/a", location: "https://api.example.org/x", status: 302, method: "GET", hop: 0 }, policy, ["169.254.169.254"]);
        expect(d.allow ? "allow" : d.code).toBe("private-address");
        expect(code(r("https://169.254.169.254/latest"))).toBe("ip-literal-denied");
        expect(code(r("http://127.0.0.1:8080/"))).toBe("redirect-scheme-downgrade");
    });
    test("scheme downgrade and hop limit", () => {
        expect(code(r("http://shop.example.com/b"))).toBe("redirect-scheme-downgrade");
        expect(code(r("/b", { hop: 5 }))).toBe("redirect-limit");
        expect(r("/b", { hop: 4 }).allow).toBe(true);
    });
    test("cross-origin redirects of non-GET requests are refused; credentials are stripped for GET", () => {
        expect(code(r("https://api.example.org/x", { method: "POST", status: 307 }))).toBe("redirect-cross-host-write");
        expect(r("/b", { method: "POST", status: 307 }).allow).toBe(true);
        const d = r("https://api.example.org/x");
        expect(d.allow && d.stripCredentials).toBe(true);
        const same = r("/b");
        expect(same.allow && same.stripCredentials).toBe(false);
    });
    test("userinfo or junk in Location", () => {
        expect(code(r("https://user:pw@shop.example.com/b"))).toBe("credentials-in-url");
        expect(r("https://").allow).toBe(false);
    });
});
