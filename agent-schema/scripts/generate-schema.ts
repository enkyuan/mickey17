// Writes schema/agent-surface.schema.json, or with --check fails if the committed file differs.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { renderSchema } from "../src/json-schema";

const target = join(dirname(fileURLToPath(import.meta.url)), "..", "schema", "agent-surface.schema.json");
const generated = renderSchema();

if (process.argv.includes("--check")) {
    const committed = existsSync(target) ? readFileSync(target, "utf8") : "";
    if (committed !== generated) {
        console.error("schema/agent-surface.schema.json is out of date with the Zod schemas. Run `bun run generate` and commit the result.");
        process.exit(1);
    }
    console.log("schema/agent-surface.schema.json is up to date");
} else {
    writeFileSync(target, generated);
    console.log(`wrote ${target} (${generated.length} bytes)`);
}
