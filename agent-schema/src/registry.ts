// Private registry for the named schemas, so JSON Schema generation sees exactly this
// package's types and nothing a consumer registers in Zod's global registry.
import { z } from "zod";

export const registry = z.registry<{ id?: string; description?: string }>();
