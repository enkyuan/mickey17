// Regression tests for the fifth adversarial review (of da04aa7).
import { describe, expect, test } from "bun:test";
import { issueConfirmation, verifyConfirmation } from "../src/confirm.ts";
import { collectText, ownHiddenReasons, type PageNode } from "../src/hidden.ts";
import { authorize, type AuthorizeDeps } from "../src/policy.ts";
import { sanitizePageText } from "../src/sanitize.ts";
import { confirmToken, mandateToken, moneyReq, NOW, world } from "./helpers.ts";

const doc = { width: 1280, height: 20000 };
const visible = (n: PageNode, o: Parameters<typeof collectText>[1] = { document: doc }) => collectText(n, o).visible.map((v) => v.text);

describe("sanitizer: code points that become combining marks under NFKC count as marks", () => {
    test("U+FF9E/U+FF9F interleaved with marks cannot restart the run", () => {
        for (const text of ["a" + "́ﾞ".repeat(42_000), "a" + ("́".repeat(29) + "ﾞ").repeat(2_800)]) {
            const t0 = performance.now();
            const r = sanitizePageText(text);
            expect(performance.now() - t0).toBeLessThan(1500);
            expect(r.flags.map((f) => f.kind)).toContain("combining-flood");
        }
    });
    test("Thai/Lao SARA AM interleaved with marks is cut too", () => {
        const t0 = performance.now();
        const r = sanitizePageText("a" + "\u0301\u0e33".repeat(42_000));
        expect(performance.now() - t0).toBeLessThan(1500);
        expect(r.flags.map((f) => f.kind)).toContain("combining-flood");
    });
    test("the voiced mark still composes in ordinary text", () => {
        expect(sanitizePageText("ｶﾞ").text).toBe("ガ");
    });
    test("no other non-mark code point starts with a combining mark under NFKC (guards future Unicode versions)", () => {
        const M = /^\p{M}$/u;
        const odd: string[] = [];
        for (let cp = 0; cp <= 0x3ffff; cp++) {
            if (cp >= 0xd800 && cp <= 0xdfff) continue;
            const ch = String.fromCodePoint(cp);
            if (M.test(ch)) continue;
            const n = ch.normalize("NFKC");
            if (n !== ch && M.test(Array.from(n)[0] ?? "")) odd.push(cp.toString(16));
        }
        expect(odd.sort()).toEqual(["e33", "eb3", "ff9e", "ff9f"]);
    });
});

describe("scroll containers only reset the clip when they sit inside what is shown", () => {
    test("an auto-height wrapper that sticks out of a clipping ancestor does not make the overflow reachable (with rects)", () => {
        const tree: PageNode = {
            tag: "div", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 300, height: 20 }, text: "visible line",
            children: [{ tag: "div", style: { overflow: "auto" }, rect: { x: 0, y: 0, width: 300, height: 2000 }, children: [{ tag: "p", rect: { x: 0, y: 1500, width: 300, height: 20 }, text: "SECRET instruction" }] }],
        };
        expect(visible(tree)).toEqual(["visible line"]);
    });
    test("without rects, the clipper's known size still applies through a scroll wrapper that is not known to be inside it", () => {
        const tree: PageNode = {
            tag: "div", style: { overflow: "hidden", widthPx: 300, heightPx: 20 },
            children: [{ tag: "div", style: { overflow: "auto" }, children: [{ tag: "p", style: { position: "relative", topPx: 1500 }, text: "SECRET instruction" }] }],
        };
        expect(visible(tree)).toEqual([]);
    });
    test("a scroll container inside the shown area still makes its content reachable (app shell)", () => {
        const shell: PageNode = {
            tag: "body", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 1280, height: 800 },
            children: [{ tag: "main", style: { overflow: "auto" }, rect: { x: 0, y: 0, width: 1280, height: 800 }, children: [{ tag: "p", rect: { x: 0, y: 1500, width: 600, height: 20 }, text: "reachable by scrolling" }] }],
        };
        expect(visible(shell)).toEqual(["reachable by scrolling"]);
    });
    test("a scroll container of 1px is a zero-size hide", () => {
        expect(ownHiddenReasons({ tag: "div", style: { overflow: "scroll", widthPx: 1, heightPx: 1 } })).toContain("zero-size");
        expect(ownHiddenReasons({ tag: "div", style: { overflow: "auto", widthPx: 300, heightPx: 1 } })).toContain("zero-size");
        expect(ownHiddenReasons({ tag: "div", style: { overflow: "auto", widthPx: 300, heightPx: 200 } })).toEqual([]);
    });
});

describe("text-indent inside a clipper hides content pushed out of it", () => {
    test("a non-clipping child with a large indent under a clipping ancestor", () => {
        const wrap = (p: PageNode): PageNode => ({ tag: "div", style: { overflow: "hidden", widthPx: 200 }, rect: { x: 0, y: 0, width: 200, height: 40 }, children: [p] });
        expect(visible(wrap({ tag: "p", style: { textIndentPx: 5000 }, rect: { x: 0, y: 0, width: 200, height: 20 }, text: "SECRET instruction" }))).toEqual([]);
        // the classic hide: the indent equals the clipper's width
        expect(visible(wrap({ tag: "p", style: { textIndentPx: 200 }, rect: { x: 0, y: 0, width: 200, height: 20 }, text: "SECRET instruction" }))).toEqual([]);
        // an ordinary small indent is fine
        expect(visible(wrap({ tag: "p", style: { textIndentPx: 24 }, rect: { x: 0, y: 0, width: 200, height: 20 }, text: "paragraph" }))).toEqual(["paragraph"]);
    });
    test("unknown width falls back to 900px", () => {
        expect(ownHiddenReasons({ tag: "p", style: { textIndentPx: 899 } }, { clippedAncestor: true })).toEqual([]);
        expect(ownHiddenReasons({ tag: "p", style: { textIndentPx: 900 } }, { clippedAncestor: true })).toContain("offscreen");
    });
});

describe("position:fixed", () => {
    const clipper = (child: PageNode, containsFixed: boolean): PageNode => ({
        tag: "div", style: { overflow: "hidden", containsFixed }, rect: { x: 0, y: 0, width: 100, height: 100 }, children: [child],
    });
    const fixedBox: PageNode = { tag: "span", style: { position: "fixed" }, rect: { x: 0, y: 500, width: 100, height: 20 }, text: "SECRET instruction" };
    test("escapes ancestor clipping unless an ancestor is its containing block", () => {
        expect(visible(clipper(fixedBox, false))).toEqual(["SECRET instruction"]);
        expect(visible(clipper(fixedBox, true))).toEqual([]);
        // the flag propagates to descendants of the containing block
        expect(visible({ tag: "div", style: { containsFixed: true }, children: [clipper(fixedBox, false)] })).toEqual([]);
    });
    test("a fixed box outside the viewport never scrolls into view", () => {
        const far: PageNode = { tag: "span", style: { position: "fixed" }, rect: { x: 0, y: 6000, width: 100, height: 20 }, text: "never visible" };
        expect(visible(far, { document: doc, viewport: { width: 1280, height: 800 } })).toEqual([]);
        expect(visible(far, { document: doc })).toEqual(["never visible"]); // no viewport known: not judged
        const near: PageNode = { tag: "span", style: { position: "fixed" }, rect: { x: 0, y: 700, width: 100, height: 20 }, text: "banner" };
        expect(visible(near, { document: doc, viewport: { width: 1280, height: 800 } })).toEqual(["banner"]);
    });
});

describe("zero-area wrappers do not hide their children", () => {
    test("a zero-height header at y=0 with a laid-out child", () => {
        const tree: PageNode = { tag: "header", rect: { x: 0, y: 0, width: 1280, height: 0 }, children: [{ tag: "nav", style: { position: "absolute" }, rect: { x: 0, y: 0, width: 1280, height: 60 }, text: "menu" }] };
        expect(visible(tree)).toEqual(["menu"]);
    });
});

describe("authorize input handling", () => {
    test("null or non-object tokens are refused, not thrown", async () => {
        const w = await world();
        for (const t of [null, undefined, "x", 5]) {
            const r = await authorize(moneyReq(), t as never, w.deps);
            expect(r.ok ? "ok" : r.code).toBe("invalid-request");
        }
    });
    test("deps members are read exactly once for the whole authorization (a swapped ledger cannot split it)", async () => {
        const w = await world();
        const reads: Record<string, number> = {};
        const counted = (k: keyof AuthorizeDeps) => { reads[k] = (reads[k] ?? 0) + 1; return w.deps[k]; };
        const deps = {
            get mandateVerifier() { return counted("mandateVerifier"); },
            get confirmationVerifier() { return counted("confirmationVerifier"); },
            get ledger() { return counted("ledger"); },
            get consumed() { return counted("consumed"); },
        } as AuthorizeDeps;
        const req = moneyReq();
        const r = await authorize(req, { mandate: await mandateToken(w), confirmation: await confirmToken(w, req) }, deps);
        if (!r.ok) throw new Error("setup");
        await r.settle("committed");
        expect(reads).toEqual({ mandateVerifier: 1, confirmationVerifier: 1, ledger: 1, consumed: 1 });
    });
    test("a NaN maxTtlMs does not disable the lifetime check", async () => {
        const w = await world();
        const b = { actionId: "act_pay", item: null, totalMinor: null, currency: null, payee: null };
        const tok = await issueConfirmation(w.confirmSigner, b, { jti: "c1", now: NOW, ttlMs: 3 * 3_600_000 });
        const r = await verifyConfirmation(tok, w.deps.confirmationVerifier, b, { now: NOW, maxTtlMs: Number.NaN });
        expect(r.ok ? "ok" : r.code).toBe("confirmation-ttl-too-long");
    });
});
