import { describe, expect, test } from "bun:test";
import { evaluateRobots, parseRobots, patternMatches, robotsFromFetch } from "../src/robots.ts";

const UA = "MochiAgent";
const allowed = (txt: string, path: string, ua = UA) => evaluateRobots(parseRobots(txt), ua, path).allowed;

describe("robots.txt (RFC 9309)", () => {
    test("wildcard group applies when no specific group exists", () => {
        const t = "User-agent: *\nDisallow: /private/\n";
        expect(allowed(t, "/private/x")).toBe(false);
        expect(allowed(t, "/public")).toBe(true);
        expect(allowed(t, "/private")).toBe(true); // prefix needs the slash
    });
    test("a specific group replaces the wildcard group (no merging)", () => {
        const t = "User-agent: *\nDisallow: /\n\nUser-agent: mochiagent\nDisallow: /admin\n";
        expect(allowed(t, "/shop")).toBe(true);
        expect(allowed(t, "/admin/x")).toBe(false);
        expect(allowed(t, "/shop", "OtherBot")).toBe(false);
    });
    test("longest match wins; Allow wins a tie", () => {
        const t = "User-agent: *\nDisallow: /a\nAllow: /a/b\nDisallow: /a/b/c\n";
        expect(allowed(t, "/a/x")).toBe(false);
        expect(allowed(t, "/a/b/x")).toBe(true);
        expect(allowed(t, "/a/b/c/d")).toBe(false);
        expect(allowed("User-agent: *\nDisallow: /p\nAllow: /p\n", "/p")).toBe(true);
    });
    test("wildcards and end anchors", () => {
        const t = "User-agent: *\nDisallow: /*.json$\nDisallow: /tmp*/cache\nAllow: /tmp-ok/cache\n";
        expect(allowed(t, "/a/b.json")).toBe(false);
        expect(allowed(t, "/a/b.json?x=1")).toBe(true);
        expect(allowed(t, "/a/b.jsonx")).toBe(true);
        expect(allowed(t, "/tmp123/cache")).toBe(false);
        expect(allowed(t, "/tmp-ok/cache")).toBe(true);
    });
    test("empty Disallow allows all; Disallow: / blocks all", () => {
        expect(allowed("User-agent: *\nDisallow:\n", "/x")).toBe(true);
        expect(allowed("User-agent: *\nDisallow: /\n", "/")).toBe(false);
        expect(allowed("User-agent: *\nDisallow: /\n", "/anything?q=1")).toBe(false);
    });
    test("consecutive user-agent lines share rules; comments, BOM, CRLF and case are tolerated", () => {
        const t = "﻿# hi\r\nUSER-AGENT: a\r\nuser-agent: MochiAgent # me\r\nDISALLOW: /x # no\r\n";
        expect(allowed(t, "/x/1")).toBe(false);
        expect(allowed(t, "/y")).toBe(true);
    });
    test("percent-encoding is normalised before comparison", () => {
        expect(allowed("User-agent: *\nDisallow: /a%2fb\n", "/a%2Fb")).toBe(false);
        expect(allowed("User-agent: *\nDisallow: /%7euser\n", "/~user")).toBe(false);
        expect(allowed("User-agent: *\nDisallow: /~user\n", "/%7Euser")).toBe(false);
    });
    test("no rules and unrelated groups allow", () => {
        expect(allowed("", "/x")).toBe(true);
        expect(allowed("User-agent: other\nDisallow: /\n", "/x")).toBe(true);
    });
    test("crawl-delay is exposed", () => {
        expect(evaluateRobots(parseRobots("User-agent: *\nCrawl-delay: 7\nDisallow: /z\n"), UA, "/a").crawlDelay).toBe(7);
        expect(evaluateRobots(parseRobots("User-agent: *\nCrawl-delay: abc\n"), UA, "/a").crawlDelay).toBeNull();
    });
    test("fetch outcomes: 404 allows, 5xx/unreachable/401/403 disallow, 2xx parses", () => {
        const verdict = (status: number | null, body = "") => evaluateRobots(robotsFromFetch(status, body), UA, "/x").allowed;
        expect(verdict(404)).toBe(true);
        expect(verdict(410)).toBe(true);
        for (const s of [500, 502, 503, 401, 403, 302, null]) expect(verdict(s), String(s)).toBe(false);
        expect(verdict(200, "User-agent: *\nDisallow: /x\n")).toBe(false);
        expect(verdict(200, "User-agent: *\nAllow: /\n")).toBe(true);
    });
    test("pathological patterns match in bounded time", () => {
        const pattern = "/" + "*a".repeat(500);
        const start = performance.now();
        expect(patternMatches(pattern, "/" + "a".repeat(5000) + "b")).toBe(true);
        expect(patternMatches(pattern + "$", "/" + "a".repeat(400) + "b")).toBe(false);
        expect(performance.now() - start).toBeLessThan(1500);
    });
    test("a huge file is truncated, and an over-long Disallow is kept as a prefix", () => {
        const t = "User-agent: *\nDisallow: /" + "x".repeat(3000) + "\n" + "#".repeat(600_000) + "\nDisallow: /late\n";
        expect(allowed(t, "/late")).toBe(true); // beyond 500 KiB: ignored per RFC
        expect(allowed(t, "/" + "x".repeat(3000))).toBe(false);
    });
});
