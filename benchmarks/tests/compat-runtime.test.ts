import { describe, expect, test } from "bun:test";
import {
    blockedAttribute,
    booleanAttributes,
    decodePathname,
    firstVariantIndex,
    safeAnimationAttributes,
    unsafeAnimationTarget,
    variantIndex,
    dataUriAllowed,
    mobileRequest,
    pageFor,
    parseSrcset,
    serializeSrcset,
    srcsetDescriptorValid,
} from "../../src/export_template/src/lib/policy";

const svgBase64 = "data:image/svg+xml;base64,PHN2Zy8+";
const svgUtf8 = "data:image/svg+xml;charset=utf-8,%3Csvg%20xmlns='http://www.w3.org/2000/svg'/%3E";

describe("data: URI policy", () => {
    test("raster images are accepted in every URL context", () => {
        expect(dataUriAllowed("data:image/png;base64,AAAA")).toBe(true);
        expect(dataUriAllowed("DATA:IMAGE/WEBP;base64,AAAA")).toBe(true);
        expect(dataUriAllowed("data:image/gif,GIF89a")).toBe(true);
    });
    test("SVG is accepted only in image contexts", () => {
        expect(dataUriAllowed(svgBase64, true)).toBe(true);
        expect(dataUriAllowed(svgUtf8, true)).toBe(true);
        expect(dataUriAllowed("DATA:image/SVG+XML;base64,AAAA", true)).toBe(true);
        // href, iframe/embed/object, <use> and inline-style url() never pass svgImage.
        expect(dataUriAllowed(svgBase64)).toBe(false);
        expect(dataUriAllowed(svgUtf8, false)).toBe(false);
    });
    test("documents, scripts and malformed image types are refused even for images", () => {
        for (const value of [
            "data:text/html;base64,PHNjcmlwdD4=",
            "data:text/html,<script>alert(1)</script>",
            "data:application/javascript,alert(1)",
            "data:image/svg+xml-evil;base64,AAAA",
            "data:image/svgx+xml;base64,AAAA",
            "data:image/svg;base64,AAAA",
            " data:image/svg+xml;base64,AAAA",
            "javascript:data:image/svg+xml;base64,AAAA",
            "data:image/png",
            "data:text/xml;image/svg+xml,AAAA",
        ])
            expect(dataUriAllowed(value, true)).toBe(false);
    });
});

describe("srcset", () => {
    test("candidates, descriptors and commas inside data URIs", () => {
        expect(parseSrcset("/a.png 1x, /b.png 2x")).toEqual([
            { url: "/a.png", descriptor: "1x" },
            { url: "/b.png", descriptor: "2x" },
        ]);
        expect(parseSrcset(`${svgBase64} 1x,${svgUtf8}`)).toEqual([
            { url: svgBase64, descriptor: "1x" },
            { url: svgUtf8, descriptor: "" },
        ]);
        expect(parseSrcset("data:image/png;base64,AA,BB 2x")).toEqual([
            { url: "data:image/png;base64,AA,BB", descriptor: "2x" },
        ]);
        expect(parseSrcset("a.png, b.png,, c.png 100w")).toEqual([
            { url: "a.png", descriptor: "" },
            { url: "b.png", descriptor: "" },
            { url: "c.png", descriptor: "100w" },
        ]);
        expect(parseSrcset("  ,, ")).toEqual([]);
        expect(parseSrcset("a.png (min-width: 1px, x) 1x, b.png 2x")).toEqual([
            { url: "a.png", descriptor: "(min-width: 1px, x) 1x" },
            { url: "b.png", descriptor: "2x" },
        ]);
    });
    test("parentheses do not nest, as in browsers, so no candidate can hide in a descriptor", () => {
        expect(parseSrcset("/a.png 1x((,), http://evil.test/t.png 2x")).toEqual([
            { url: "/a.png", descriptor: "1x((,)" },
            { url: "http://evil.test/t.png", descriptor: "2x" },
        ]);
    });
    test("only valid descriptors are re-emitted", () => {
        for (const d of ["", "1x", "1.5x", ".5x", "2e1x", "100w", "100w 50h"]) expect(srcsetDescriptorValid(d)).toBe(true);
        for (const d of ["1x 2x", "100w 1x", "50h", "0w", "x", "1.x", "(min-width: 1px, x) 1x", "http://evil.test/x.png", "1x((,)"])
            expect(srcsetDescriptorValid(d)).toBe(false);
    });
    test("serialization keeps only candidates a browser parses back identically", () => {
        expect(serializeSrcset([{ url: "/a.png", descriptor: "1x" }, { url: svgUtf8, descriptor: "" }])).toBe(`/a.png 1x, ${svgUtf8}`);
        // A resolved URL ending in a comma would turn its descriptor into a new URL.
        expect(serializeSrcset([{ url: "http://a.test/b,", descriptor: "" }, { url: "/c.png", descriptor: "2x" }])).toBe("/c.png 2x");
        expect(serializeSrcset([{ url: "/a.png", descriptor: "http://evil.test/x.png" }])).toBeUndefined();
        expect(serializeSrcset([{ url: "/a.png", descriptor: "1x((,)" }])).toBeUndefined();
    });
});

describe("page selection", () => {
    const pages = { "/": "home", "/pay": "pay", "/request/": "request", "/?tab=a": "tab-a" };
    test("exact captured state wins", () => {
        expect(pageFor(pages, "/", "?tab=a")).toBe("tab-a");
        expect(pageFor(pages, "/pay", "")).toBe("pay");
        expect(pageFor(pages, "/request/", "")).toBe("request");
    });
    test("unknown query strings fall back to the captured path", () => {
        expect(pageFor(pages, "/", "?x=1")).toBe("home");
        expect(pageFor(pages, "/pay", "?x=1&y=2")).toBe("pay");
    });
    test("trailing slash is tolerated in both directions", () => {
        expect(pageFor(pages, "/pay/", "")).toBe("pay");
        expect(pageFor(pages, "/pay/", "?x=1")).toBe("pay");
        expect(pageFor(pages, "/request", "")).toBe("request");
    });
    test("uncaptured paths and prototype keys stay not found", () => {
        expect(pageFor(pages, "/nope", "")).toBeUndefined();
        expect(pageFor(pages, "/pay/extra", "")).toBeUndefined();
        expect(pageFor(pages, "//pay", "")).toBeUndefined();
        expect(pageFor(pages, "/__proto__", "")).toBeUndefined();
        expect(pageFor({}, "/constructor", "")).toBeUndefined();
    });
});

describe("request viewport hint", () => {
    test("client hint wins over the user agent", () => {
        expect(mobileRequest("?1", "Mozilla/5.0 (X11; Linux)")).toBe(true);
        expect(mobileRequest("?0", "Mozilla/5.0 (iPhone; Mobile)")).toBe(false);
    });
    test("falls back to the user agent, defaulting to desktop", () => {
        expect(mobileRequest(null, "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0) Mobile/15E148")).toBe(true);
        expect(mobileRequest(null, "Mozilla/5.0 (Linux; Android 14) Mobile Safari")).toBe(true);
        expect(mobileRequest(null, "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)")).toBe(false);
        expect(mobileRequest(null, null)).toBe(false);
    });
});

describe("boolean attributes", () => {
    test("inert is reflected as a present attribute", () => {
        expect(booleanAttributes.has("inert")).toBe(true);
        expect(booleanAttributes.has("hidden")).toBe(true);
    });
});

describe("viewport variant selection", () => {
    const widths = [375, 1440], heights = [812, 900];
    test("depends on width only, at every height", () => {
        for (const height of [300, 812, 900, 1400, 2000, 5000]) {
            expect(variantIndex(widths, heights, 907, height)).toBe(0);
            expect(variantIndex(widths, heights, 908, height)).toBe(1);
            expect(variantIndex(widths, heights, 375, height)).toBe(0);
            expect(variantIndex(widths, heights, 1440, height)).toBe(1);
        }
    });
    test("height only breaks exact width ties", () => {
        expect(variantIndex([800, 800], [600, 900], 800, 880)).toBe(1);
        expect(variantIndex([800, 800], [600, 900], 800, 650)).toBe(0);
    });
    test("a window midway between two widths does not depend on its height", () => {
        // 390/1440 put the switch on an integer width (915).
        for (const height of [300, 844, 900, 2000]) {
            expect(variantIndex([390, 1440], [844, 900], 915, height)).toBe(0);
            expect(variantIndex([390, 1440], [844, 900], 916, height)).toBe(1);
        }
    });
    test("single capture and three captures", () => {
        expect(variantIndex([1440], [900], 320, 480)).toBe(0);
        expect(variantIndex([375, 768, 1440], [812, 1024, 900], 600, 700)).toBe(1);
    });
    test("server starts from the request hint", () => {
        expect(firstVariantIndex([1440, 375, 768], "mobile")).toBe(1);
        expect(firstVariantIndex([375, 1440], "desktop")).toBe(1);
        expect(firstVariantIndex([375, 1440], undefined)).toBe(1);
    });
});

describe("SVG animation targets", () => {
    test("URL, handler and style targets are refused in every spelling", () => {
        for (const name of ["href", "HREF", " xlink:href ", "xlink:HREF", "src", "onclick", "onbegin", "style", "action", "formaction", "srcdoc", "data", "ping", "xml:href"])
            expect(unsafeAnimationTarget(name)).toBe(true);
    });
    test("presentation and geometry animation is kept", () => {
        for (const name of ["opacity", "transform", "d", "cx", "stroke-dashoffset", "fill", "viewBox", "points", "r", "x", "y"])
            expect(unsafeAnimationTarget(name)).toBe(false);
    });
    test("unsafe animation loses target and values, safe animation is untouched", () => {
        const bad = { attributeName: "href", to: "javascript:alert(1)", values: "javascript:alert(2)", from: "x", by: "y", dur: "1s", begin: "0s" };
        expect(safeAnimationAttributes("set", bad)).toEqual({ dur: "1s", begin: "0s" });
        expect(safeAnimationAttributes("animate", { attributename: "xlink:href", to: "javascript:alert(1)" })).toEqual({});
        const good = { attributeName: "opacity", from: "0", to: "1", dur: "1s" };
        expect(safeAnimationAttributes("animate", good)).toBe(good);
        // SVG attribute names are case-sensitive: a safe `attributename` must not
        // hide the `attributeName` the browser actually reads, in either order.
        for (const raw of [
            { attributename: "opacity", attributeName: "href", to: "javascript:alert(1)", dur: "1s" },
            { attributeName: "href", attributename: "opacity", to: "javascript:alert(1)", dur: "1s" },
        ])
            expect(safeAnimationAttributes("set", raw)).toEqual({ dur: "1s" });
        // only animation elements are inspected; other tags keep their own attributes
        const other = { attributeName: "href", to: "x" };
        expect(safeAnimationAttributes("path", other)).toBe(other);
    });
});

describe("blocked attributes", () => {
    test("handlers and side-effecting attributes never reach the page", () => {
        for (const name of ["onclick", "OnLoad", "srcdoc", "action", "formaction", "ping", "PING", "nonce", "autoplay"])
            expect(blockedAttribute(name)).toBe(true);
        for (const name of ["href", "class", "id", "aria-label", "data-ping", "poster"])
            expect(blockedAttribute(name)).toBe(false);
    });
});

describe("pathname decoding", () => {
    test("decodes each segment once", () => {
        expect(decodePathname("/%70ay")).toBe("/pay");
        expect(decodePathname("/pay/")).toBe("/pay/");
        expect(decodePathname("/")).toBe("/");
        expect(decodePathname("/a%20b/c")).toBe("/a b/c");
        // a double-encoded sequence is decoded exactly once, never twice
        expect(decodePathname("/%2570ay")).toBe("/%70ay");
    });
    test("ambiguous paths are refused", () => {
        for (const path of ["/%zz", "/%", "/pay%2F", "/pay%2f..", "/a%5Cb", "/pay%3Fx=1", "/pay%23x", "/%00", "/pay%0A", "/%2e%2e/etc", "/%2E", "/a/./b", "/a/../b", "/%c0%af", "/%ff"])
            expect(decodePathname(path)).toBeUndefined();
    });
    test("decoded paths feed page selection", () => {
        const pages = { "/pay": "pay" };
        expect(pageFor(pages, decodePathname("/%70ay") ?? "", "")).toBe("pay");
        expect(pageFor(pages, decodePathname("/%2570ay") ?? "", "")).toBeUndefined();
    });
});
