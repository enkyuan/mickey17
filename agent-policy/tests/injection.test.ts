import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { collectText, type HiddenReason, type PageNode } from "../src/hidden.ts";
import { sanitizePageText, type FlagKind } from "../src/sanitize.ts";
import { describeTool, findPageLeaks, surfaceFields, surfaceId } from "../src/tooldesc.ts";
import { pageText, unwrapPageText, wrapPageText } from "../src/wrap.ts";
import { assertWrapInvariants } from "./invisible.ts";

type Item =
    | { id: string; title: string; kind: "text"; input: string | { $repeat: string; count: number }; expect: { flags: FlagKind[]; absentFromOutput: string[]; quarantined?: string[]; maxOutputChars?: number } }
    | { id: string; title: string; kind: "dom"; input: PageNode; expect: { reasons: HiddenReason[]; absentFromVisible: string[]; presentInVisible: string[] } }
    | { id: string; title: string; kind: "tool"; input: { pageLabel: string; paramLabel: string }; expect: { absentFromSurface: string[] } };

const corpus = JSON.parse(readFileSync(new URL("../fixtures/injection/corpus.json", import.meta.url), "utf8")) as Item[];
const materialise = (i: string | { $repeat: string; count: number }) => (typeof i === "string" ? i : i.$repeat.repeat(i.count));

test("corpus covers every category and is non-trivial", () => {
    expect(corpus.length).toBeGreaterThanOrEqual(30);
    expect(new Set(corpus.map((i) => i.id)).size).toBe(corpus.length);
    expect(new Set(corpus.map((i) => i.kind))).toEqual(new Set(["text", "dom", "tool"]));
});

describe("adversarial corpus: text items are neutralised", () => {
    for (const item of corpus.filter((i): i is Extract<Item, { kind: "text" }> => i.kind === "text")) {
        test(item.id, () => {
            const raw = materialise(item.input);
            const s = sanitizePageText(raw);
            const block = wrapPageText(pageText(raw), "node_0123abcd");

            assertWrapInvariants(block);
            // flagged as expected
            const kinds = s.flags.map((f) => f.kind);
            for (const k of item.expect.flags) expect(kinds, `expected flag ${k}`).toContain(k);
            // the dangerous strings are gone
            for (const bad of item.expect.absentFromOutput) expect(block.includes(bad), `output still contains ${JSON.stringify(bad)}`).toBe(false);
            // the sanitised text round-trips through the wrapper unchanged
            expect(unwrapPageText(block)?.text).toBe(s.text);
            // smuggled text is quarantined for audit, never in the output
            for (const q of item.expect.quarantined ?? []) {
                expect(s.quarantined).toContain(q);
                expect(block.includes(q)).toBe(false);
            }
            if (item.expect.maxOutputChars !== undefined) expect(block.length).toBeLessThan(item.expect.maxOutputChars);
        });
    }

    test("forged markers never count as structure", () => {
        for (const id of ["fake-close-marker", "fake-open-marker", "line-separator-break"]) {
            const item = corpus.find((i) => i.id === id) as Extract<Item, { kind: "text" }>;
            const block = wrapPageText(pageText(materialise(item.input)), "node_0123abcd");
            const lines = block.split("\n");
            expect(lines.filter((l) => l.startsWith("[[")).length).toBe(2);
        }
    });

    test("homoglyph folding is opt-in and only touches flagged tokens", () => {
        const input = "pаypаl and привет";
        expect(sanitizePageText(input).text).toBe(input);
        const folded = sanitizePageText(input, { foldConfusables: true }).text;
        expect(folded.startsWith("paypal and ")).toBe(true);
        expect(folded.endsWith("привет")).toBe(true); // real Russian word untouched
    });
});

describe("adversarial corpus: hidden-text items never reach T0", () => {
    for (const item of corpus.filter((i): i is Extract<Item, { kind: "dom" }> => i.kind === "dom")) {
        test(item.id, () => {
            const c = collectText(item.input);
            const visible = c.visible.map((v) => v.text).join("\n");
            for (const bad of item.expect.absentFromVisible) expect(visible.includes(bad), `hidden text leaked into T0`).toBe(false);
            for (const good of item.expect.presentInVisible) expect(visible).toContain(good);
            const reasons = c.excluded.flatMap((e) => e.reasons);
            for (const r of item.expect.reasons) expect(reasons, `expected reason ${r}`).toContain(r);
            // the excluded record carries lengths, never the text
            expect(JSON.stringify(c.excluded)).not.toContain(item.expect.absentFromVisible[0]!);
            expect(c.hiddenOptIn).toHaveLength(0);
            // explicit opt-in returns it, tagged with the reasons
            const opt = collectText(item.input, { includeHidden: true });
            expect(opt.hiddenOptIn.some((h) => item.expect.absentFromVisible.some((b) => h.text.includes(b)) && h.reasons.length > 0)).toBe(true);
            expect(opt.visible.map((v) => v.text).join("\n").includes(item.expect.absentFromVisible[0]!)).toBe(false);
        });
    }
});

describe("adversarial corpus: tool-description injection", () => {
    for (const item of corpus.filter((i): i is Extract<Item, { kind: "tool" }> => i.kind === "tool")) {
        test(item.id, () => {
            const spec = {
                id: surfaceId("act", "0123456789abcdef"),
                kind: "submit" as const, effect: "financial" as const, tier: "T2" as const, idempotent: true,
                params: [{ id: surfaceId("param", "00ff00ff00ff00ff"), type: "number" as const, required: true }],
                pageLabel: pageText(item.input.pageLabel),
                paramLabels: { [surfaceId("param", "00ff00ff00ff00ff")]: pageText(item.input.paramLabel) },
            };
            const d = describeTool(spec);
            const surface = JSON.stringify(surfaceFields(d));
            for (const bad of item.expect.absentFromSurface) expect(surface.includes(bad)).toBe(false);
            expect(findPageLeaks(surfaceFields(d), [item.input.pageLabel, item.input.paramLabel])).toEqual([]);
            // page text only appears in the marked field
            expect(d.pageLabels.origin).toBe("page");
            expect(d.pageLabels.action?.origin).toBe("page");
            // wrapped for the reader
            assertWrapInvariants(wrapPageText(d.pageLabels.action!, "act_0123456789abcdef"), "act_0123456789abcdef");
        });
    }
});

describe("wrapper contract", () => {
    test("rejects a ref that is not a surface-generated id", () => {
        for (const bad of ["", "A", "has space", "x]]\n", "../x", "a".repeat(65)]) expect(() => wrapPageText(pageText("x"), bad)).toThrow();
    });
    test("empty text still wraps; unwrap refuses tampered blocks", () => {
        const b = wrapPageText(pageText(""), "node_0123abcd");
        expect(unwrapPageText(b)?.text).toBe("");
        expect(unwrapPageText(b.replace("sha=", "sha=0"))).toBeNull();
        expect(unwrapPageText(wrapPageText(pageText("hello"), "node_0123abcd").replace("| hello", "| hellp"))).toBeNull();
        expect(unwrapPageText(wrapPageText(pageText("hello"), "node_0123abcd").replace("| hello", "hello"))).toBeNull();
        expect(unwrapPageText("")).toBeNull();
    });
});
