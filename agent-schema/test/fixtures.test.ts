import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { invalidFixtures, validFixtures } from "../scripts/fixtures";
import type { Expect } from "../scripts/fixtures";
import { SCHEMAS, fixtureDir, listJson, parseIssues, readJson, schemaKeyOf } from "./helpers";

const json = (v: unknown) => `${JSON.stringify(v, null, 2)}\n`;

describe("fixtures are reproducible", () => {
    test("committed valid fixtures equal the builder output", () => {
        const files = listJson(fixtureDir("valid"));
        expect(files).toEqual(Object.keys(validFixtures).map((n) => `${n}.json`).sort());
        for (const [name, doc] of Object.entries(validFixtures)) {
            expect(readFileSync(join(fixtureDir("valid"), `${name}.json`), "utf8")).toBe(json(doc));
        }
    });
    test("committed invalid fixtures equal the builder output", () => {
        const files = listJson(fixtureDir("invalid"));
        const names = Object.keys(invalidFixtures);
        expect(files).toEqual(names.flatMap((n) => [`${n}.expect.json`, `${n}.json`]).sort());
        for (const [name, { doc, expect: e }] of Object.entries(invalidFixtures)) {
            expect(readFileSync(join(fixtureDir("invalid"), `${name}.json`), "utf8")).toBe(json(doc));
            expect(readFileSync(join(fixtureDir("invalid"), `${name}.expect.json`), "utf8")).toBe(json(e));
        }
    });
});

describe("valid fixtures", () => {
    for (const file of listJson(fixtureDir("valid"))) {
        test(file, () => {
            const key = schemaKeyOf(file);
            const doc = readJson(join(fixtureDir("valid"), file));
            expect(parseIssues(key, doc)).toEqual([]);
            expect(SCHEMAS[key].check(doc)).toEqual([]);
        });
    }
    test("every schema has at least one valid fixture", () => {
        const covered = new Set(listJson(fixtureDir("valid")).map(schemaKeyOf));
        expect([...covered].sort() as string[]).toEqual(Object.keys(SCHEMAS).sort());
    });
    test("every OperationResult status has a valid fixture (D10)", () => {
        const statuses = listJson(fixtureDir("valid"))
            .filter((f) => f.startsWith("result."))
            .map((f) => (readJson(join(fixtureDir("valid"), f)) as { status: string }).status)
            .sort();
        expect(statuses).toEqual(["blocked", "error", "ok", "requires_auth", "requires_browser", "requires_confirmation", "requires_human", "stale", "unsupported"]);
    });
});

describe("invalid fixtures fail for the stated reason", () => {
    const files = listJson(fixtureDir("invalid")).filter((f) => !f.endsWith(".expect.json"));
    test("there are invalid fixtures, each with an expectation", () => {
        expect(files.length).toBeGreaterThan(30);
        for (const f of files) expect(existsSync(join(fixtureDir("invalid"), f.replace(/\.json$/, ".expect.json")))).toBe(true);
    });
    for (const file of files) {
        test(file, () => {
            const doc = readJson(join(fixtureDir("invalid"), file));
            const want = readJson(join(fixtureDir("invalid"), file.replace(/\.json$/, ".expect.json"))) as Expect;
            expect(want.why.length).toBeGreaterThan(10);
            const schemaIssues = parseIssues(want.schema, doc);
            if (want.phase === "schema") {
                expect(schemaIssues.map(({ path, code }) => ({ path, code }))).toEqual(want.issues);
            } else {
                // Must be structurally valid, and fail only the code-level invariant.
                expect(schemaIssues).toEqual([]);
                const found = SCHEMAS[want.schema].check(doc);
                expect(found.map(({ path, code }) => ({ path, code }))).toEqual(want.issues);
            }
        });
    }
    test("each issue code and path is exercised by exactly the fixtures that name it", () => {
        const codes = new Set(Object.values(invalidFixtures).flatMap((f) => f.expect.issues.map((i) => i.code)));
        for (const c of ["hash_mismatch", "duplicate_id", "dangling_reference", "hidden_not_opted_in", "mode_forbids_tier", "coverage_mismatch", "flow_effect_below_step", "dependency_not_earlier", "undeclared_input", "payment_effect_mismatch", "time_order", "cap_order"]) {
            expect(codes.has(c)).toBe(true);
        }
    });
});
