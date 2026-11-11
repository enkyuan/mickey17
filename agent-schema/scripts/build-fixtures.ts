// Writes fixtures/valid/*.json and fixtures/invalid/*.json (+ .expect.json) from scripts/fixtures.ts.
import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { invalidFixtures, validFixtures } from "./fixtures";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "fixtures");
const json = (v: unknown) => `${JSON.stringify(v, null, 2)}\n`;

for (const dir of ["valid", "invalid"]) {
    mkdirSync(join(root, dir), { recursive: true });
    for (const f of readdirSync(join(root, dir))) if (f.endsWith(".json")) rmSync(join(root, dir, f));
}
for (const [name, doc] of Object.entries(validFixtures)) writeFileSync(join(root, "valid", `${name}.json`), json(doc));
for (const [name, { doc, expect }] of Object.entries(invalidFixtures)) {
    writeFileSync(join(root, "invalid", `${name}.json`), json(doc));
    writeFileSync(join(root, "invalid", `${name}.expect.json`), json(expect));
}
console.log(`wrote ${Object.keys(validFixtures).length} valid and ${Object.keys(invalidFixtures).length} invalid fixtures`);
