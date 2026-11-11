import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type { z } from "zod";
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

export const root = join(dirname(fileURLToPath(import.meta.url)), "..");
export const fixtureDir = (kind: "valid" | "invalid") => join(root, "fixtures", kind);
export const readJson = (path: string): unknown => JSON.parse(readFileSync(path, "utf8"));
export const listJson = (dir: string): string[] => readdirSync(dir).filter((f) => f.endsWith(".json")).sort();

/** Fixture schema key -> Zod schema, JSON Schema `$defs` name, and the code-level checks. */
export const SCHEMAS = {
    surface: { zod: AgentSurface as z.ZodType, def: "AgentSurface", check: (d: unknown): CheckIssue[] => checkSurface(d as never) },
    result: { zod: OperationResult as z.ZodType, def: "OperationResult", check: (): CheckIssue[] => [] },
    mandate: { zod: Mandate as z.ZodType, def: "Mandate", check: (d: unknown): CheckIssue[] => checkMandate(d as never) },
    "confirmation-request": { zod: ConfirmationRequest as z.ZodType, def: "ConfirmationRequest", check: (d: unknown): CheckIssue[] => checkConfirmationRequest(d as never) },
    "confirmation-token": { zod: ConfirmationToken as z.ZodType, def: "ConfirmationToken", check: (): CheckIssue[] => [] },
    receipt: { zod: Receipt as z.ZodType, def: "Receipt", check: (): CheckIssue[] => [] },
} as const;
export type SchemaKey = keyof typeof SCHEMAS;

/** `surface.shop.json` -> `surface`; `mandate.json` -> `mandate`. */
export const schemaKeyOf = (file: string): SchemaKey => {
    const key = file.replace(/\.json$/, "").split(".")[0] as SchemaKey;
    if (!(key in SCHEMAS)) throw new Error(`fixture ${file} does not name a known schema`);
    return key;
};

export const parseIssues = (key: SchemaKey, doc: unknown): CheckIssue[] => {
    const r = SCHEMAS[key].zod.safeParse(doc);
    return r.success ? [] : fromZodIssues(r.error);
};
