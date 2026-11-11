// Validate emitted documents against the schema and the code-level checks.
//
//   bun agent-schema/scripts/validate.ts [--schema <name>] <file.json>...
//
// <name> is one of: surface, result, mandate, confirmation-request, confirmation-token,
// receipt. When omitted it is inferred from the file name prefix (`surface.shop.json`) or,
// for a document with kind "agent-surface", from the content.
// Exit 0: every file valid. Exit 1: at least one invalid or unreadable (a JSON report always goes
// to stdout). Exit 2: usage error, including a file whose schema cannot be inferred. This is how the Rust test suite validates its own emit.
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
    checkConfirmationToken,
    checkMandate,
    checkOperationResult,
    checkReceipt,
    checkSurface,
    fromZodIssues,
} from "../src/index";
import type { CheckIssue } from "../src/index";
import type { z } from "zod";

const SCHEMAS = {
    surface: { zod: AgentSurface as z.ZodType, check: (d: unknown) => checkSurface(d as never) },
    result: { zod: OperationResult as z.ZodType, check: (d: unknown) => checkOperationResult(d as never) },
    mandate: { zod: Mandate as z.ZodType, check: (d: unknown) => checkMandate(d as never) },
    "confirmation-request": { zod: ConfirmationRequest as z.ZodType, check: (d: unknown) => checkConfirmationRequest(d as never) },
    "confirmation-token": { zod: ConfirmationToken as z.ZodType, check: (d: unknown) => checkConfirmationToken(d as never) },
    receipt: { zod: Receipt as z.ZodType, check: (d: unknown) => checkReceipt(d as never) },
} as const;
type Name = keyof typeof SCHEMAS;

const args = process.argv.slice(2);
let forced: Name | undefined;
const files: string[] = [];
for (let i = 0; i < args.length; i++) {
    const a = args[i]!;
    if (a === "--schema") {
        const n = args[++i] as Name | undefined;
        if (!n || !Object.hasOwn(SCHEMAS, n)) {
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

type Entry = { file: string; schema: string; ok: boolean; phase?: "parse" | "usage" | "schema" | "check"; issues?: CheckIssue[] };
const report: Entry[] = [];
const fail = (file: string, schema: string, phase: "parse" | "usage", message: string): Entry => ({ file, schema, ok: false, phase, issues: [{ path: [], code: phase, message }] });
for (const file of files) {
    let doc: unknown;
    try {
        doc = JSON.parse(readFileSync(file, "utf8"));
    } catch (e) {
        report.push(fail(file, "unknown", "parse", e instanceof Error ? e.message : String(e)));
        continue;
    }
    const prefix = basename(file).replace(/\.json$/, "").split(".")[0] as Name;
    const name: Name | undefined =
        forced ?? (Object.hasOwn(SCHEMAS, prefix) ? prefix : (doc as { kind?: string } | null)?.kind === "agent-surface" ? "surface" : undefined);
    if (!name) {
        report.push(fail(file, "unknown", "usage", "cannot infer the schema; pass --schema"));
        continue;
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
// 0: all valid. 1: some invalid (including unreadable JSON). 2: usage problem (schema not inferable).
process.exit(report.some((r) => r.phase === "usage") ? 2 : report.every((r) => r.ok) ? 0 : 1);
