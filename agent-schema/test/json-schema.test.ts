import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import Ajv2020 from "ajv/dist/2020";
import { renderSchema } from "../src/index";
import { SCHEMAS, fixtureDir, listJson, readJson, root, schemaKeyOf } from "./helpers";

const committedPath = join(root, "schema", "agent-surface.schema.json");
const committed = readFileSync(committedPath, "utf8");
const schema = JSON.parse(committed) as { $defs: Record<string, any> };

describe("generated JSON Schema", () => {
    test("committed file equals the generated schema", () => {
        expect(committed).toBe(renderSchema());
    });
    test("generation is deterministic", () => {
        expect(renderSchema()).toBe(renderSchema());
    });
    test("committed file ends with a newline and is stable JSON", () => {
        expect(committed.endsWith("\n")).toBe(true);
        expect(`${JSON.stringify(JSON.parse(committed), null, 2)}\n`).toBe(committed);
    });
    test("uses only wire-safe keywords (no defaults, no format-only checks)", () => {
        const banned = new Set(["default", "format", "contentEncoding", "if", "then", "else", "not", "dependentSchemas"]);
        const hits: string[] = [];
        const walk = (n: unknown, path: string) => {
            if (Array.isArray(n)) n.forEach((x, i) => walk(x, `${path}/${i}`));
            else if (n && typeof n === "object") {
                for (const [k, v] of Object.entries(n)) {
                    if (banned.has(k) && !path.endsWith("/properties")) hits.push(`${path}/${k}`);
                    walk(v, `${path}/${k}`);
                }
            }
        };
        walk(schema, "");
        expect(hits).toEqual([]);
    });
    test("every object type is closed (additionalProperties false) except string-keyed records", () => {
        const open: string[] = [];
        for (const [name, def] of Object.entries(schema.$defs)) {
            const walk = (n: any, path: string) => {
                if (Array.isArray(n)) return n.forEach((x, i) => walk(x, `${path}/${i}`));
                if (!n || typeof n !== "object") return;
                if (n.type === "object" && n.properties && n.additionalProperties !== false) open.push(path);
                for (const [k, v] of Object.entries(n)) if (k !== "properties") walk(v, `${path}/${k}`);
                if (n.properties) for (const [k, v] of Object.entries(n.properties)) walk(v, `${path}/properties/${k}`);
            };
            walk(def, `#/$defs/${name}`);
        }
        expect(open).toEqual([]);
    });
    test("no property is named like a secret or an instrument", () => {
        const names = new Set<string>();
        const walk = (n: unknown) => {
            if (Array.isArray(n)) return n.forEach(walk);
            if (!n || typeof n !== "object") return;
            const obj = n as Record<string, unknown>;
            if (obj.properties && typeof obj.properties === "object") for (const k of Object.keys(obj.properties)) names.add(k);
            Object.values(obj).forEach(walk);
        };
        walk(schema);
        const forbidden = /(password|passwd|secret|cvv|cvc|pan|card_number|private_key|api_key|apikey|access_token|refresh_token|bearer|cookie_value)/i;
        expect([...names].filter((n) => forbidden.test(n))).toEqual([]);
    });
    test("free-form strings are limited to marked text, identifiers and pointers", () => {
        // A property whose schema is a bare `type: string` (no pattern, enum, const or $ref) is
        // free text. D8 says page text is only ever carried by PageText/PageValue; the only other
        // free strings allowed are bounded names and pointers authored by the emitter.
        const free: string[] = [];
        for (const [name, def] of Object.entries(schema.$defs)) {
            const walk = (n: any, path: string) => {
                if (Array.isArray(n)) return n.forEach((x, i) => walk(x, `${path}/${i}`));
                if (!n || typeof n !== "object") return;
                if (n.properties) {
                    for (const [k, v] of Object.entries<any>(n.properties)) {
                        if (v.type === "string" && !v.pattern && !v.enum && v.const === undefined) free.push(`${name}.${k}`);
                    }
                }
                for (const v of Object.values(n)) walk(v, path);
            };
            walk(def, name);
        }
        // PageText/SurfaceText/PageValue carry the marked strings; the rest are bounded
        // names, JSON pointers and capture refs written by the emitter, never by the page.
        expect([...new Set(free)].sort()).toEqual([
            "AuthRequirement.header",
            "Evidence.ref",
            "PageText.value",
            "RequestLocation.name",
            "PageUrl.value",
            "ResponseLocation.name",
            "StrongEvidence.ref",
            "SurfaceText.value",
            "Mandate.grantor",
            "DependencySource.attribute",
            "DependencySource.key",
            "DependencySource.name",
            "Postcondition.at",
        ].sort());
    });
});

describe("JSON Schema agrees with Zod on every fixture", () => {
    const ajv = new Ajv2020({ strict: false, allErrors: true });
    ajv.addSchema(schema, "agent-surface");
    const validate = (def: string) => {
        const v = ajv.getSchema(`agent-surface#/$defs/${def}`);
        if (!v) throw new Error(`no def ${def}`);
        return v;
    };
    for (const file of listJson(fixtureDir("valid"))) {
        test(`valid: ${file}`, () => {
            const v = validate(SCHEMAS[schemaKeyOf(file)].def);
            const ok = v(readJson(join(fixtureDir("valid"), file)));
            expect(v.errors ?? []).toEqual([]);
            expect(ok).toBe(true);
        });
    }
    for (const file of listJson(fixtureDir("invalid")).filter((f) => !f.endsWith(".expect.json"))) {
        const want = readJson(join(fixtureDir("invalid"), file.replace(/\.json$/, ".expect.json"))) as { schema: keyof typeof SCHEMAS; phase: string };
        test(`${want.phase === "schema" ? "rejected" : "accepted (code-level only)"}: ${file}`, () => {
            const v = validate(SCHEMAS[want.schema].def);
            const ok = v(readJson(join(fixtureDir("invalid"), file)));
            // Schema-phase fixtures must also fail the JSON Schema; check-phase ones are wire-valid.
            expect(ok).toBe(want.phase === "check");
        });
    }
});
