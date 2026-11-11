// Regression tests for the sixth adversarial review (of 98110b5).
import { describe, expect, test } from "bun:test";
import { collectText, ownHiddenReasons, type PageNode } from "../src/hidden.ts";
import { verifyMandate } from "../src/mandate.ts";
import { sanitizePageText } from "../src/sanitize.ts";
import { mandateToken, NOW, world, HOUR } from "./helpers.ts";

const doc = { width: 1280, height: 20000 };
const vp = { width: 1280, height: 800 };
const body = (...children: PageNode[]): PageNode => ({ tag: "body", rect: { x: 0, y: 0, width: 1280, height: 20000 }, children });
const visible = (n: PageNode, o: Parameters<typeof collectText>[1] = { document: doc }) => collectText(n, o).visible.map((v) => v.text);

describe("zero-area boxes are judged as 1px boxes", () => {
    test("off-screen text in a zero-width or zero-height box stays hidden", () => {
        expect(visible(body({ tag: "div", style: { widthPx: 0 }, rect: { x: -5000, y: 100, width: 0, height: 20 }, text: "A1" }))).toEqual([]);
        expect(visible(body({ tag: "div", style: { heightPx: 0 }, rect: { x: 0, y: -5000, width: 1280, height: 0 }, text: "A2" }))).toEqual([]);
    });
    test("a zero-height header at the top edge keeps its laid-out children, even under an overflow-x:hidden body", () => {
        const header: PageNode = { tag: "header", rect: { x: 0, y: 0, width: 1280, height: 0 }, children: [{ tag: "nav", style: { position: "absolute" }, rect: { x: 0, y: 0, width: 1280, height: 60 }, text: "menu" }] };
        expect(visible(body(header))).toEqual(["menu"]);
        const clipped: PageNode = { tag: "body", style: { overflow: "hidden auto" }, rect: { x: 0, y: 0, width: 1280, height: 20000 }, children: [header] };
        expect(visible(clipped)).toEqual(["menu"]);
        const narrow: PageNode = { tag: "body", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 1280, height: 800 }, children: [{ tag: "aside", rect: { x: 0, y: 0, width: 0, height: 700 }, children: [{ tag: "p", rect: { x: 0, y: 0, width: 200, height: 20 }, text: "side" }] }] };
        expect(visible(narrow)).toEqual(["side"]);
    });
    test("a zero-area anchor exactly on the far edge keeps its positioned children (cookie banner, toast, back-to-top)", () => {
        const o = { document: { width: 1280, height: 20000 }, viewport: vp };
        const banner = body({ tag: "div", style: { position: "fixed", heightPx: 0 }, rect: { x: 0, y: 800, width: 1280, height: 0 }, children: [{ tag: "div", style: { position: "absolute" }, rect: { x: 0, y: 680, width: 1280, height: 120 }, text: "We use cookies" }] });
        expect(visible(banner, o)).toEqual(["We use cookies"]);
        const toast = body({ tag: "div", style: { position: "absolute" }, rect: { x: 1280, y: 100, width: 0, height: 0 }, children: [{ tag: "p", rect: { x: 1000, y: 100, width: 280, height: 40 }, text: "toast" }] });
        expect(visible(toast, o)).toEqual(["toast"]);
        const top = body({ tag: "div", style: { position: "absolute", heightPx: 0 }, rect: { x: 0, y: 20000, width: 100, height: 0 }, children: [{ tag: "button", rect: { x: 0, y: 19950, width: 100, height: 50 }, text: "back to top" }] });
        expect(visible(top, o)).toEqual(["back to top"]);
        // but far beyond the edge it is still outside
        expect(visible(body({ tag: "div", rect: { x: 5000, y: 100, width: 0, height: 20 }, text: "gone" }), o)).toEqual([]);
    });
    test("a 1x1 clipping or scrolling box with a rect but no style sizes is zero-size", () => {
        expect(ownHiddenReasons({ tag: "div", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 1, height: 1 } })).toContain("zero-size");
        expect(ownHiddenReasons({ tag: "div", style: { overflow: "auto" }, rect: { x: 0, y: 0, width: 300, height: 1 } })).toContain("zero-size");
        expect(ownHiddenReasons({ tag: "div", style: { overflow: "auto" }, rect: { x: 0, y: 0, width: 300, height: 100 } })).toEqual([]);
    });
});

describe("fixed boxes and everything inside them are judged by the viewport", () => {
    const opts = { document: doc, viewport: vp };
    test("a descendant of a fixed box that lies below the viewport", () => {
        const tree = body({ tag: "div", style: { position: "fixed" }, rect: { x: 0, y: 0, width: 100, height: 20 }, text: "banner", children: [{ tag: "span", style: { position: "absolute", topPx: 5000 }, rect: { x: 0, y: 5000, width: 100, height: 20 }, text: "SECRET B1" }] });
        expect(visible(tree, opts)).toEqual(["banner"]);
    });
    test("a zero-height fixed box below the viewport, and a child laid out there", () => {
        expect(visible(body({ tag: "div", style: { position: "fixed", heightPx: 0 }, rect: { x: 0, y: 6000, width: 100, height: 0 }, text: "B2" }), opts)).toEqual([]);
        expect(visible(body({ tag: "div", style: { position: "fixed" }, rect: { x: 0, y: 0, width: 100, height: 0 }, children: [{ tag: "p", rect: { x: 0, y: 6000, width: 100, height: 20 }, text: "B3" }] }), opts)).toEqual([]);
    });
    test("a fixed box inside the viewport escapes an ancestor clip, a containing-block ancestor does not", () => {
        const fixed: PageNode = { tag: "nav", style: { position: "fixed" }, rect: { x: 0, y: 700, width: 100, height: 20 }, text: "nav" };
        const clipper = (containsFixed: boolean): PageNode => body({ tag: "div", style: { overflow: "hidden", containsFixed }, rect: { x: 0, y: 0, width: 100, height: 100 }, children: [fixed] });
        expect(visible(clipper(false), opts)).toEqual(["nav"]);
        expect(visible(clipper(true), opts)).toEqual([]);
    });
});

describe("scroll containers", () => {
    test("an app shell whose main pane starts below a header (rect sticks out of the body by the header height) is still reachable", () => {
        const shell: PageNode = {
            tag: "body", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 1280, height: 800 },
            children: [
                { tag: "header", rect: { x: 0, y: 0, width: 1280, height: 60 }, text: "title" },
                { tag: "main", style: { overflow: "auto" }, rect: { x: 0, y: 60, width: 1280, height: 800 }, children: [{ tag: "p", rect: { x: 0, y: 1500, width: 600, height: 20 }, text: "reachable" }] },
            ],
        };
        expect(visible(shell)).toEqual(["title", "reachable"]);
    });
    test("a sub-pixel rect does not defeat the reset", () => {
        const shell: PageNode = {
            tag: "body", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 1280, height: 800 },
            children: [{ tag: "main", style: { overflow: "auto" }, rect: { x: 0, y: 0, width: 1280, height: 800.5 }, children: [{ tag: "p", rect: { x: 0, y: 1500, width: 600, height: 20 }, text: "reachable" }] }],
        };
        expect(visible(shell)).toEqual(["reachable"]);
    });
    test("with capture-supplied scroll offsets, content above or left of the unscrolled origin is unreachable", () => {
        const mk = (scrollLeftPx: number, scrollTopPx: number): PageNode => ({
            tag: "body", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 1280, height: 4000 },
            children: [{ tag: "div", style: { overflow: "auto", scrollLeftPx, scrollTopPx }, rect: { x: 500, y: 3000, width: 300, height: 200 }, children: [
                { tag: "p", style: { position: "relative", topPx: -500 }, rect: { x: 500, y: 2500, width: 300, height: 20 }, text: "D1 above" },
                { tag: "p", rect: { x: 100, y: 3050, width: 300, height: 20 }, text: "D2 left" },
                { tag: "p", rect: { x: 510, y: 3100, width: 100, height: 20 }, text: "inside" },
            ] }],
        });
        expect(visible(mk(0, 0), { document: { width: 1280, height: 4000 } })).toEqual(["inside"]);
        // a pane captured while scrolled: its earlier content (above/left of the box) is reachable
        expect(visible(mk(400, 500), { document: { width: 1280, height: 4000 } })).toEqual(["D1 above", "D2 left", "inside"]);
    });
    test("without offsets the scroller's content region is unbounded (carousels, RTL and reversed panes)", () => {
        const carousel: PageNode = {
            tag: "body", rect: { x: 0, y: 0, width: 1280, height: 4000 }, style: { overflow: "hidden" },
            children: [{ tag: "ul", style: { overflow: "auto" }, rect: { x: 200, y: 100, width: 800, height: 300 }, children: [{ tag: "li", rect: { x: -600 + 700, y: 120, width: 800, height: 250 }, text: "slide 2" }, { tag: "li", rect: { x: 210, y: 60, width: 300, height: 30 }, text: "older message" }] }],
        };
        expect(visible(carousel, { document: { width: 1280, height: 4000 } })).toEqual(["slide 2", "older message"]);
    });
    test("a scroller that only slivers into the clip does not expose its content", () => {
        const sliver: PageNode = {
            tag: "div", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 300, height: 20 }, text: "benign",
            children: [{ tag: "div", style: { overflow: "auto" }, rect: { x: 0, y: 19.99, width: 300, height: 20 }, children: [{ tag: "p", rect: { x: 0, y: 30, width: 300, height: 20 }, text: "SECRET-sliver" }] }],
        };
        expect(visible(sliver)).toEqual(["benign"]);
        const shell: PageNode = {
            tag: "body", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 1280, height: 800 },
            children: [{ tag: "main", style: { overflow: "auto" }, rect: { x: 0, y: 799.5, width: 1280, height: 800 }, children: [{ tag: "p", rect: { x: 0, y: 1000, width: 100, height: 20 }, text: "SECRET-below-fold" }] }],
        };
        expect(visible(shell)).toEqual([]);
    });
    test("a fixed drawer taller than the viewport (100vh with a URL bar) or wider (scrollbar) still scrolls", () => {
        const drawer = (w: number, h: number): PageNode => body({ tag: "nav", style: { position: "fixed", overflow: "auto" }, rect: { x: 0, y: 0, width: w, height: h }, children: [{ tag: "a", rect: { x: 0, y: 1000, width: 100, height: 20 }, text: "Settings" }] });
        expect(visible(drawer(390, 844), { document: { width: 1280, height: 20000 }, viewport: { width: 390, height: 750 } })).toEqual(["Settings"]);
        expect(visible(drawer(1280, 900), { document: { width: 1280, height: 20000 }, viewport: { width: 1265, height: 800 } })).toEqual(["Settings"]);
    });
    test("an auto-height wrapper taller than its clipper is still not a scroller", () => {
        const tree: PageNode = {
            tag: "div", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 300, height: 20 },
            children: [{ tag: "div", style: { overflow: "auto" }, rect: { x: 0, y: 0, width: 300, height: 2000 }, children: [{ tag: "p", rect: { x: 0, y: 1500, width: 300, height: 20 }, text: "SECRET" }] }],
        };
        expect(visible(tree)).toEqual([]);
    });
});

describe("config hardening", () => {
    test("an Infinity maxLifetimeMs is ignored too", async () => {
        const w = await world();
        const long = await mandateToken(w, { notBefore: NOW - HOUR, expiresAt: NOW + 10_000 * 24 * HOUR });
        const r = await verifyMandate(long, w.mandateVerifier, { now: NOW, maxLifetimeMs: Number.POSITIVE_INFINITY });
        expect(r.ok ? "ok" : r.code).toBe("mandate-lifetime-too-long");
    });
    test("a NaN maxLifetimeMs does not disable the mandate lifetime check", async () => {
        const w = await world();
        const long = await mandateToken(w, { notBefore: NOW - HOUR, expiresAt: NOW + 10_000 * 24 * HOUR });
        const r = await verifyMandate(long, w.mandateVerifier, { now: NOW, maxLifetimeMs: Number.NaN });
        expect(r.ok ? "ok" : r.code).toBe("mandate-lifetime-too-long");
    });
});

describe("sanitizer: every mark-like code point is covered by the run cut", () => {
    test("each of U+FF9E, U+FF9F, U+0E33, U+0EB3 interleaved with a mark is cut", () => {
        for (const ch of ["ﾞ", "ﾟ", "ำ", "ຳ"]) {
            const t0 = performance.now();
            const r = sanitizePageText("a" + ("́" + ch).repeat(42_000));
            expect(performance.now() - t0, ch.codePointAt(0)!.toString(16)).toBeLessThan(1500);
            expect(r.flags.map((f) => f.kind)).toContain("combining-flood");
        }
    });
    test("the enumeration covers every code point and checks the actual combining class", () => {
        // A non-\p{M} character whose NFKC output starts with a non-starter would be a new hole.
        const odd: string[] = [];
        for (let cp = 0; cp <= 0x10ffff; cp++) {
            if (cp >= 0xd800 && cp <= 0xdfff) continue;
            const ch = String.fromCodePoint(cp);
            if (/^\p{M}$/u.test(ch)) continue;
            const n = ch.normalize("NFKC");
            if (n === ch) continue;
            // canonical reordering moves a ccc>0 mark before a ccc=230 one: U+0301 is 230, U+0334 is 1
            const first = Array.from(n)[0]!;
            if (/^\p{M}$/u.test(first) && (first + "̴").normalize("NFC") !== first + "̴") odd.push(cp.toString(16));
            else if (/^\p{M}$/u.test(first) && ("́" + first).normalize("NFD") !== "́" + first) odd.push(cp.toString(16));
        }
        // each such code point must be one the cut regex counts
        for (const hex of odd) expect(["ff9e", "ff9f", "e33", "eb3"], hex).toContain(hex);
    });
});
