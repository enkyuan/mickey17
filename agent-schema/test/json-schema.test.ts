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
    test("generation ignores Zod's global registry (fresh process, polluted with a colliding id)", async () => {
        const script = `
            import { z } from "zod";
            z.object({}).meta({ id: "Action" });
            z.object({}).meta({ id: "ConsumerThing" });
            const { renderSchema } = await import(${JSON.stringify(join(root, "src", "json-schema.ts"))});
            process.stdout.write(renderSchema());
        `;
        const proc = Bun.spawnSync([process.execPath, "-e", script], { cwd: root });
        expect(proc.stderr.toString()).toBe("");
        expect(proc.stdout.toString()).toBe(committed);
    });
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
    test("free-form strings are limited to marked text and reviewed bounded fields", () => {
        // Any schema node (property, array item, record value, property name) that is a bare
        // `type: string` with no pattern, enum or const is free text. D8: page text is only carried
        // by the marked wrappers (PageText, PageValue, PageUrl, PagePath, MarkedText, MarkedValue,
        // SurfaceText); everything else must be a grammar (pattern/enum/const).
        const free = new Set<string>();
        const walk = (n: any, path: string, def: string) => {
            if (Array.isArray(n)) return n.forEach((x, i) => walk(x, `${path}[${i}]`, def));
            if (!n || typeof n !== "object") return;
            if (n.type === "string" && n.pattern === undefined && n.enum === undefined && n.const === undefined) free.add(path);
            for (const [k, v] of Object.entries<any>(n)) {
                if (k === "properties") for (const [pk, pv] of Object.entries<any>(v)) walk(pv, `${path}.${pk}`, def);
                else walk(v, k === "items" || k === "prefixItems" ? `${path}[]` : k === "propertyNames" ? `${path}{key}` : k === "additionalProperties" ? `${path}{value}` : path, def);
            }
        };
        for (const [name, def] of Object.entries(schema.$defs)) walk(def, name, name);
        expect([...free].sort()).toEqual(
            [
                "MarkedText[0].value",
                "MarkedText[1].value",
                "MarkedValue[0].value[0]",
                "MarkedValue[1].value[0]",
                "PageText.value",
                "PageTextNonEmpty.value",
                "PageUrl.value",
                "PageValue.value[0]",
                "SurfaceText.value",
                "SurfaceTextNonEmpty.value",
            ].sort(),
        );
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
            if (want.phase === "schema") {
                // ...and fail where Zod says: at the stated path, or at its parent (ajv reports a
                // missing key or an unrecognised key on the containing object).
                const expectedPaths = (readJson(join(fixtureDir("invalid"), file.replace(/\.json$/, ".expect.json"))) as { issues: { path: (string | number)[] }[] }).issues.map((i) => i.path);
                const ptr = (p: (string | number)[]) => p.map((x) => `/${x}`).join("");
                const errors = v.errors ?? [];
                for (const p of expectedPaths) {
                    // Exact path, except ajv reports a missing key, an unrecognised key, or a missing tuple element on the container.
                    const hit = errors.some((e) => e.instancePath === ptr(p) || (["required", "additionalProperties", "minItems"].includes(e.keyword) && e.instancePath === ptr(p.slice(0, -1))));
                    expect(hit).toBe(true);
                }
            }
        });
    }
});
