# agent-schema log

Owner: the agent-schema session. Scope: `agent-schema/**`, `docs/agent-surface/log/agent-schema.md`.

## 2026-11-10 PR 1: core types and generator

Built `agent-schema/` (Bun, TypeScript, Zod 4.6.7 exact-pinned: published 2026-10-12, 29 days old at pin time).

Decisions that other sessions depend on:

- **Import path.** `agent-schema/src/index.ts` (package name `mochi-agent-schema`, private). Types are `z.infer` of the schemas.
- **Origin marking.** Strings are `{ origin: "page" | "surface", value }` (`PageText`, `SurfaceText`; scalars `PageValue`/`MarkedValue`).
  Surface-authored fields (descriptions, reasons, messages) are `SurfaceText` and structurally cannot hold page origin.
- **Ids.** Node id `n_<16 hex>`; every other id (route, action, flow, step, request, mandate, confirmation, receipt)
  is a lowercase slug `^[a-z][a-z0-9_-]{0,63}$`, unique within the document, MCP-tool-name safe. Emitters should derive slugs
  from structure, not page text.
- **Action.** Discriminated on `kind`; tier is fixed by kind (navigate T1, state T3, submit/fetch/download T2);
  navigate and state are `effect: "read"`. Non-navigate evidence is a tuple whose first element is declared or observed
  (D5), emitters order evidence strongest first. `confidence` exists only on navigate. Unsupported items are a separate
  `unsupported[]` array (tier T4), not actions.
- **Flow.** `status: "ready" | "blocked"`; a blocked flow lists `unclassified` values, a ready flow has none (D11: unclassified blocks).
  Value classes are `constant | input | derived | client-computed`; `derived` carries a `DependencyEdge` (source: response, dom,
  cookie, storage); `client-computed` resolves to a `lift` spec hash or `requires_browser`. Exactly the financial step declares
  `payment` (`caller-token | hosted-session | agent-protocol`).
- **Result.** `OperationResult` discriminated on `status` with the nine D10 statuses; `ok` carries `fields` of `MarkedValue`.
- **Authority.** `Mandate`, `ConfirmationRequest` (financial variant shows items, total, payee; others carry a surface summary),
  `ConfirmationToken` (binds confirmation id, request digest, idempotency key; carries no secret), `Receipt`.
- **Wire rules.** Strict objects, absent not null, no floats (money in minor units, confidence integer percent).
- **Hash.** `sha256:` over RFC 8785 canonical JSON with `hash` omitted.
- **Validation entry points.** `validateSurface()` (parse + checks) and `bun agent-schema/scripts/validate.ts` for the Rust side.

Evidence that it holds: `bun run check` (tsc strict, generated-schema check, 159 tests): every invalid fixture fails with the exact
stated issue list; Ajv over the generated JSON Schema agrees with Zod on every fixture; mutating the committed JSON Schema fails the check.

Not yet done (PR 2+): node-id derivation function and stability tests (owned with the emitter), per-mode/effect enforcement matrix
(agent-policy owns enforcement), Markdown/MCP projection shapes, drift report shape, a `schema_version` reader helper.
