// Deterministic JSON Schema for the wire format. The committed file
// schema/agent-surface.schema.json is exactly `renderSchema()`.
//
// Shape: the root `$ref`s AgentSurface; every named type (anything with `.meta({ id })`)
// is a `$defs` entry, sorted by name, so non-TS consumers can validate any document
// (OperationResult, Mandate, Receipt...) by pointing at `#/$defs/<Name>`.
import { z } from "zod";
import "./index-types";
import { registry } from "./registry";
import { SCHEMA_VERSION } from "./primitives";

type JsonObject = Record<string, unknown>;

export function buildSchema(): JsonObject {
    const all = z.toJSONSchema(registry, {
        metadata: registry,
        target: "draft-2020-12",
        unrepresentable: "throw",
        io: "input",
        uri: (id) => `#/$defs/${id}`,
    }) as { schemas: Record<string, JsonObject> };

    const defs: JsonObject = {};
    for (const name of Object.keys(all.schemas).sort()) {
        const { $schema: _s, $id: _i, ...body } = all.schemas[name] as JsonObject;
        defs[name] = body;
    }
    return {
        $schema: "https://json-schema.org/draft/2020-12/schema",
        title: "Agent surface",
        $comment: `Generated from the Zod schemas in agent-schema/src (schema_version ${SCHEMA_VERSION}). Do not edit; run \`bun run generate\`.`,
        $ref: "#/$defs/AgentSurface",
        $defs: defs,
    };
}

export function renderSchema(): string {
    return `${JSON.stringify(buildSchema(), null, 2)}\n`;
}
