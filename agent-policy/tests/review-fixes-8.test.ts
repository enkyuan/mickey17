// Regression tests for the eighth (narrow) adversarial review of 4e73446.
import { describe, expect, test } from "bun:test";
import { collectText, type PageNode } from "../src/hidden.ts";

const doc = { width: 1280, height: 20000 };
const vp = { width: 1280, height: 800 };
const body = (...children: PageNode[]): PageNode => ({ tag: "body", rect: { x: 0, y: 0, width: 1280, height: 20000 }, children });
const visible = (n: PageNode, o: Parameters<typeof collectText>[1] = { document: doc, viewport: vp }) => collectText(n, o).visible.map((v) => v.text);
const fixedScroller = (box: { x: number; y: number; width: number; height: number }, text: { y: number }): PageNode =>
    body({ tag: "div", style: { position: "fixed", overflow: "auto" }, rect: box, children: [{ tag: "p", rect: { x: 0, y: text.y, width: 300, height: 20 }, text: "SECRET-fixed" }] });

describe("a fixed scroller much larger than the viewport does not expose its off-screen part", () => {
    test("tall boxes", () => {
        expect(visible(fixedScroller({ x: 0, y: 0, width: 1280, height: 100_000 }, { y: 5000 }))).toEqual([]);
        expect(visible(fixedScroller({ x: 0, y: 0, width: 1280, height: 2000 }, { y: 1500 }))).toEqual([]);
        expect(visible(fixedScroller({ x: 0, y: 792, width: 1280, height: 5000 }, { y: 2000 }))).toEqual([]);
    });
    test("drawers within the tolerance (100vh with a URL bar, 100vw with a scrollbar) still scroll", () => {
        const o = { document: doc, viewport: { width: 390, height: 750 } };
        expect(visible(fixedScroller({ x: 0, y: 0, width: 390, height: 844 }, { y: 1000 }), o)).toEqual(["SECRET-fixed"]);
        const wide = { document: doc, viewport: { width: 1265, height: 800 } };
        expect(visible(fixedScroller({ x: 0, y: 0, width: 1280, height: 800 }, { y: 1200 }), wide)).toEqual(["SECRET-fixed"]);
    });
    test("the tolerance is bounded: 150px or 15%, whichever is larger", () => {
        const at = (h: number) => visible(fixedScroller({ x: 0, y: 0, width: 1280, height: h }, { y: 1000 }));
        expect(at(800 + 150)).toEqual(["SECRET-fixed"]);
        expect(at(800 + 151)).toEqual([]);
        const big = { document: { width: 3000, height: 20000 }, viewport: { width: 3000, height: 2000 } }; // 15% of 2000 = 300
        expect(visible(fixedScroller({ x: 0, y: 0, width: 3000, height: 2290 }, { y: 2100 }), big)).toEqual(["SECRET-fixed"]);
        expect(visible(fixedScroller({ x: 0, y: 0, width: 3000, height: 2310 }, { y: 2100 }), big)).toEqual([]);
    });
});

describe("scroll offsets are distances from the start position", () => {
    const pane = (style: NonNullable<PageNode["style"]>, rect: { x: number; y: number; width: number; height: number }, child: { x: number; y: number }): PageNode => ({
        tag: "body", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 1280, height: 4000 },
        children: [{ tag: "div", style: { overflow: "auto", ...style }, rect, children: [{ tag: "p", rect: { x: child.x, y: child.y, width: 100, height: 20 }, text: "on screen" }] }],
    });
    const o = { document: { width: 1280, height: 4000 } };
    test("negative offsets (RTL and column-reverse report them) are treated as unknown, not as a region start", () => {
        expect(visible(pane({ scrollTopPx: -300 }, { x: 0, y: 100, width: 600, height: 600 }, { x: 10, y: 200 }), o)).toEqual(["on screen"]);
        expect(visible(pane({ scrollLeftPx: -200 }, { x: 100, y: 100, width: 600, height: 600 }, { x: 150, y: 200 }), o)).toEqual(["on screen"]);
    });
    test("NaN and infinite offsets are unknown too", () => {
        for (const v of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
            expect(visible(pane({ scrollTopPx: v, scrollLeftPx: v }, { x: 0, y: 100, width: 600, height: 600 }, { x: 10, y: 200 }), o)).toEqual(["on screen"]);
        }
    });
    test("with only one offset supplied the other axis is unbounded", () => {
        // column-reverse pane: only scrollLeft is sent; earlier content above the box stays reachable
        expect(visible(pane({ scrollLeftPx: 0 }, { x: 0, y: 1000, width: 600, height: 300 }, { x: 10, y: 900 }), o)).toEqual(["on screen"]);
        // and a known scrollTop still bounds the vertical axis while the horizontal one is unbounded
        expect(visible(pane({ scrollTopPx: 0 }, { x: 500, y: 1000, width: 600, height: 300 }, { x: 100, y: 1100 }), o)).toEqual(["on screen"]);
        expect(visible(pane({ scrollTopPx: 0 }, { x: 500, y: 1000, width: 600, height: 300 }, { x: 100, y: 900 }), o)).toEqual([]);
    });
    test("valid offsets still bound the region", () => {
        expect(visible(pane({ scrollTopPx: 0, scrollLeftPx: 0 }, { x: 500, y: 1000, width: 600, height: 300 }, { x: 100, y: 1100 }), o)).toEqual([]);
        expect(visible(pane({ scrollTopPx: 200, scrollLeftPx: 450 }, { x: 500, y: 1000, width: 600, height: 300 }, { x: 100, y: 900 }), o)).toEqual(["on screen"]);
    });
});

describe("the minimum visible window of a scroller is pinned", () => {
    const sliver = (h: number): PageNode => ({
        tag: "div", style: { overflow: "hidden" }, rect: { x: 0, y: 0, width: 300, height: 100 }, text: "benign",
        children: [{ tag: "div", style: { overflow: "auto" }, rect: { x: 0, y: 100 - h, width: 300, height: 100 }, children: [{ tag: "p", rect: { x: 0, y: 150, width: 100, height: 20 }, text: "SECRET" }] }],
    });
    test("a window of 7px does not reset, 8px does", () => {
        expect(visible(sliver(7), { document: doc })).toEqual(["benign"]);
        expect(visible(sliver(8), { document: doc })).toEqual(["benign", "SECRET"]);
    });
});
