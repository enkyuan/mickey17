import { describe, expect, test } from "bun:test";
import { checkReceipt, deriveCoverage, validateSurface, withHash } from "../src/index";
import { shop } from "../scripts/fixtures";

const clone = (): any => JSON.parse(JSON.stringify(shop));
const verdict = (d: any) => {
    const r = validateSurface(withHash({ ...d, coverage: deriveCoverage(d) }));
    return r.ok ? [] : r.issues.map((i) => i.code);
};

describe("checks do not reject legitimate documents", () => {
    test("baseline shop passes", () => expect(verdict(clone())).toEqual([]));
    test("a constant content-type header and a harmless cookie are fine", () => {
        const d = clone();
        const v = d.flows[0].steps[0].requests[0].values;
        v.push({ at: { in: "header", name: "content-type" }, value: { class: "constant", value: { origin: "page", value: "application/json" } } });
        v.push({ at: { in: "cookie", name: "consent" }, value: { class: "constant", value: { origin: "page", value: 1 } } });
        expect(verdict(d)).toEqual([]);
    });
    test("a 13-digit GTIN that passes Luhn is not a card number", () => {
        const d = clone();
        d.flows[0].steps[0].requests[0].values.push({ at: { in: "body-json", name: "/gtin" }, value: { class: "constant", value: { origin: "page", value: "4006381333931" } } });
        expect(verdict(d)).toEqual([]);
    });
    test("an input named frying_pan is not a card field", () => {
        const d = clone();
        d.flows[0].inputs.push({ name: "frying_pan", type: "string", required: false, sensitivity: "none" });
        expect(verdict(d)).toEqual([]);
    });
    test("a postcondition pointer that looks like a date is not a timestamp", () => {
        const d = clone();
        d.actions[1].postconditions[1].at = "2024-total";
        expect(verdict(d)).toEqual([]);
    });
    test("a short common heading repeated in a description is not page-text copying", () => {
        const d = clone();
        d.flows[0].description.value = "Go to Checkout and pay.";
        expect(verdict(d)).toEqual([]);
    });
    test("odd but legal ids do not throw", () => {
        const d = clone();
        d.routes[0].id = "constructor";
        d.actions[0].route = "constructor";
        d.actions[0].postconditions = [];
        expect(() => verdict(d)).not.toThrow();
    });
});

describe("page-text tripwire", () => {
    test("catches a copy despite case and whitespace changes", () => {
        const d = clone();
        d.actions[1].description.value = "Note: DISHWASHER   safe.\n350 ml. applies";
        expect(verdict(d)).toEqual(["page_text_in_surface"]);
    });
    test("cost is linear: 40k page strings x 100 surface strings validate in a few seconds at most", () => {
        const d = clone();
        const nodes = Array.from({ length: 40000 }, (_, i) => ({
            id: `n_${i.toString(16).padStart(16, "0")}`,
            role: "generic",
            visibility: "visible",
            text: { origin: "page", value: `unique page sentence number ${i} about nothing` },
        }));
        d.routes[3].nodes.push(...nodes);
        for (let i = 0; i < 100; i++) d.actions.push({ ...d.actions[0], id: `nav-${i}`, description: { origin: "surface", value: `Open the page number ${i} to browse more of the shop.` } });
        const t0 = performance.now();
        const issues = verdict(d);
        const ms = performance.now() - t0;
        expect(issues).toEqual([]);
        expect(ms).toBeLessThan(4000);
    });
});

describe("receipt rules", () => {
    const base = (): any => ({
        id: "r1", at: "2026-11-10T12:00:00Z", mode: "delegated", tier: "T2", effect: "financial",
        subject: { flow: "checkout", step: "pay" }, evidence: [],
        request: { method: "POST", host: "shop.example.test", path: { origin: "page", value: "/pay" } },
        result: "blocked", duration_ms: 1, redactions: 0,
    });
    test("a blocked financial receipt needs no payment fields: the call never left", () => expect(checkReceipt(base())).toEqual([]));
    test("a requires_confirmation receipt needs none either", () => expect(checkReceipt({ ...base(), result: "requires_confirmation" })).toEqual([]));
});
