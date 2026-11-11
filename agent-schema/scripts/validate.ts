// Validate emitted documents against the schema and the code-level checks.
//
//   bun agent-schema/scripts/validate.ts [--schema <name>] <file.json>...
//
// <name> is one of: surface, result, mandate, confirmation-request, confirmation-token,
// receipt. When omitted it is inferred from the file name prefix (`surface.shop.json`) or,
// for a document with kind "agent-surface", from the content.
// Exit 0: every file valid. Exit 1: at least one invalid; a JSON report goes to stdout.
// Exit 2: usage error. This is how the Rust test suite validates its own emit.
import { readFileSync } from "node:fs";
import { basename } from "node:path";
import {
    AgentSurface,
    ConfirmationRequest,
    ConfirmationToken,
    Mandate,
    OperationResult,
    Receipt,
    checkConfirmationRequest,
    checkMandate,
    checkSurface,
    fromZodIssues,
} from "../src/index";
import type { CheckIssue } from "../src/index";
import type { z } from "zod";

const SCHEMAS = {
    surface: { zod: AgentSurface as z.ZodType, check: (d: unknown) => checkSurface(d as never) },
    result: { zod: OperationResult as z.ZodType, check: (): CheckIssue[] => [] },
    mandate: { zod: Mandate as z.ZodType, check: (d: unknown) => checkMandate(d as never) },
    "confirmation-request": { zod: ConfirmationRequest as z.ZodType, check: (d: unknown) => checkConfirmationRequest(d as never) },
    "confirmation-token": { zod: ConfirmationToken as z.ZodType, check: (): CheckIssue[] => [] },
    receipt: { zod: Receipt as z.ZodType, check: (): CheckIssue[] => [] },
} as const;
type Name = keyof typeof SCHEMAS;

const args = process.argv.slice(2);
let forced: Name | undefined;
const files: string[] = [];
for (let i = 0; i < args.length; i++) {
    const a = args[i]!;
    if (a === "--schema") {
        const n = args[++i] as Name | undefined;
        if (!n || !(n in SCHEMAS)) {
            console.error(`--schema must be one of: ${Object.keys(SCHEMAS).join(", ")}`);
            process.exit(2);
        }
        forced = n;
    } else files.push(a);
}
if (files.length === 0) {
    console.error("usage: validate.ts [--schema <name>] <file.json>...");
    process.exit(2);
}

const report: { file: string; schema: string; ok: boolean; phase?: "schema" | "check"; issues?: CheckIssue[] }[] = [];
for (const file of files) {
    const doc: unknown = JSON.parse(readFileSync(file, "utf8"));
    const prefix = basename(file).replace(/\.json$/, "").split(".")[0] as Name;
    const name: Name | undefined =
        forced ?? (prefix in SCHEMAS ? prefix : (doc as { kind?: string })?.kind === "agent-surface" ? "surface" : undefined);
    if (!name) {
        console.error(`${file}: cannot infer the schema; pass --schema`);
        process.exit(2);
    }
    const parsed = SCHEMAS[name].zod.safeParse(doc);
    if (!parsed.success) {
        report.push({ file, schema: name, ok: false, phase: "schema", issues: fromZodIssues(parsed.error) });
        continue;
    }
    const found = SCHEMAS[name].check(parsed.data);
    report.push(found.length === 0 ? { file, schema: name, ok: true } : { file, schema: name, ok: false, phase: "check", issues: found });
}
console.log(JSON.stringify(report, null, 2));
process.exit(report.every((r) => r.ok) ? 0 : 1);
