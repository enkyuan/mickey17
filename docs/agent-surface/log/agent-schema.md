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

## 2026-11-10 Review round 1 (Opus 5.5 high, adversarial) and fixes

Reviewer got the commit and the spec sections, not my conclusions. It found 15 issues (8 major); all were reproduced by probe and all are fixed in this commit.
PR #5 was opened against `main` before the review gate and closed; it is reopened against `feat/tanstack-exporter` after the re-review.

### Decisions (gaps in or deviations from docs/AGENT_SURFACE.md)

- **Standalone-action dependency edges.** D5 puts CSRF/token sources on an action, D11 defines edges between flow steps. Decision: a standalone action may use `dom`, `cookie`, `storage` sources with no `step`; a `response` source always needs a step and so only appears in flows (`edge_needs_flow`). In flows every source names a step (`dependency_missing_step`).
- **Money.** D11 says a money call "carries an idempotency key" and the payment step names how it is paid. Decision: network actions get `payment` (present iff effect financial), `FlowStep` gets `idempotency` and `preconditions`; a financial action or step must name an idempotency key location that is a caller `input` in its request. Flow effect equals the max step effect; a step's effect equals its action's effect.
- **Secrets are rejected by checks, not only by absence of fields**: constants at `Authorization`/`Cookie`/`*-api-key` headers or in cookies, bearer/JWT/`sk_` shapes, Luhn-valid 13-19 digit constants, card-named inputs, password inputs not marked credential, credential/payment-token inputs in the URL.
- **D8 grammar.** Bare strings that could be page-derived are now a token grammar (`Name`) or an enum (`AriaRole` for node roles); result field names are marked (`MarkedText`). Section 6's property "page text never appears in a surface-origin field" is a verbatim-copy check for page strings of 8+ characters (tripwire, not proof).
- **`schema_version`** pins major 1 in the wire format; `checkSchemaVersion()` implements the reader rule (same major, minor <= ours).
- **Authority shapes.** A confirmation or receipt names an action, or a flow plus step (the paying step has no action). Financial confirmation requires `mandate` and items (unit amount x quantity) must sum to the total. A completed financial receipt requires amount, confirmation, mandate, idempotency key; read-only mode records T0/T1 reads only. Receipt paths cannot carry `?`/`#`.
- **Unsupported items** may sit at any tier T0-T4 so per-tier failure counts are real; opaque reasons (canvas, webgl, drag, video) must be T4. Routes are covered at T0-T3 only.
- **Coverage** has exactly one valid encoding (`deriveCoverage`), so equal content hashes equal.
- **Accepted limit:** the evidence tuple (strong first, then any) has no `maxItems` in JSON Schema; the 64 bound is a code check (`evidence_too_long`).
- **Registry isolation.** Schemas are registered in a private registry, so a consumer's `.meta({id})` cannot change or break generation; tested in a fresh process.
- **Zod 4.6.7 age** was verified against the npm registry: published 2026-10-12 (29 days before pinning). `@types/node` is overridden to 26.6.5 (26.6.6 was 13 days old).

### Evidence

`bun run check` (strict tsc, schema drift check, 284 tests): 16 valid fixtures; 112 invalid fixtures, each asserting its exact issue list; every `CHECK_CODES` entry has a failing fixture; Ajv over the generated schema agrees with Zod on every fixture and reports schema-phase failures at the stated path (or its parent); fresh-process generation with a polluted global registry equals the committed file. Probes from the review (ghost-step edge, financial action without payment, `Authorization` constant, oauth sentence scope, result key sentence, page text in description, `schema_version` 2.0.0, duplicate coverage reason) are all rejected.

### Not run

`cargo check` (no Rust files touched; the repo check is untrusted in this session). No other-language emitter exists yet to test the cross-language hash.

### Review verdicts

Round 1: 15 findings, 15 confirmed and fixed. Round 2 (re-review of the fixes): recorded below.
