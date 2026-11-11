import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { MUTATIONS } from "../scripts/mutations.ts";

// If a refactor moves or rewrites a mutated line, the mutation would silently stop
// testing anything. Every mutation must still apply to exactly one place.
test("every mutation's `find` text occurs exactly once in its file", () => {
    for (const m of MUTATIONS) {
        const text = readFileSync(new URL(`../src/${m.file}`, import.meta.url), "utf8");
        expect(text.split(m.find).length - 1, `${m.id} (${m.file})`).toBe(1);
        expect(m.replace).not.toBe(m.find);
    }
    expect(new Set(MUTATIONS.map((m) => m.id)).size).toBe(MUTATIONS.length);
});
