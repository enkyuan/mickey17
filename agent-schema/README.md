# agent-schema

The single definition of the agent-surface wire format
([docs/AGENT_SURFACE.md](../docs/AGENT_SURFACE.md), D1/D2/D4/D5/D8/D10/D11, section 6).
Zod v4 schemas in `src/`; a JSON Schema generated from them is committed at
`schema/agent-surface.schema.json` as the language-neutral contract.

```bash
cd agent-schema
bun install --frozen-lockfile
bun run check      # tsc --noEmit, generated-schema check, bun test
bun run generate   # rewrite schema/agent-surface.schema.json
```

## What is here

| Area | File | Contents |
| --- | --- | --- |
| Vocabulary | `src/policy.ts` | `Mode`, `Tier`, `Effect` (ordered), confirmation rule, evidence classes |
| Origin | `src/origin.ts` | `PageText` / `SurfaceText` / `MarkedValue`: every string carries `origin: "page" \| "surface"` (D8) |
| Results | `src/result.ts` | `OperationResult`, one variant per D10 status, including `requires_human` |
| Actions | `src/action.ts` | `Action` (kind, inputs, effect, preconditions, postconditions, idempotency, evidence), `UnsupportedItem` |
| Flows | `src/flow.ts`, `src/values.ts` | `Flow`, steps, requests, `ValueClass` (constant / input / derived / client-computed) with `DependencyEdge` |
| Authority | `src/authority.ts` | `Mandate`, `ConfirmationRequest`, `ConfirmationToken`, `Receipt` |
| Document | `src/surface.ts` | `AgentSurface`: version, content hash, capture pin, mode, gates, coverage, routes/nodes, actions, flows |
| Code checks | `src/check.ts` | invariants the wire format cannot express; `validateSurface()` = parse + check |
| Hash | `src/hash.ts` | RFC 8785 canonical form and `sha256:` content address |

## Wire-format rules

- Only constructs that convert cleanly to JSON Schema: no transforms, refinements, defaults, coercion, dates.
  Anything else (cross-references, uniqueness, the hash, coverage totals, mode rules) is in `src/check.ts`
  and runs after parsing. Consumers must run both: use `validateSurface()`.
- Objects are strict. An unknown field is an error, so field drift between emitter and schema fails loudly.
- Optional fields are absent, never `null`.
- No floating point on the wire. Money is integer minor units; confidence is an integer percentage.
  This keeps the canonical hash identical in JS and Rust.
- Invariants that *can* be structural are: a non-navigate action's first evidence must be declared or
  observed; each action kind fixes its tier; a derived value must carry its dependency edge; a blocked
  flow lists its unclassified values and a ready flow has none; money confirmations carry item, total and payee.
- No secret appears anywhere. Auth names a requirement; sensitive inputs are typed, never valued; there
  is no card-number input type. A test fails if any schema property is named like a secret, and the checks
  reject embedded credentials (`Authorization`, cookies, bearer/JWT shapes), Luhn-valid card numbers, card-named
  inputs, and credentials or payment tokens placed in a URL.
- Free text is only ever inside the marked wrappers (`PageText`, `PageValue`, `PageUrl`, `PagePath`,
  `MarkedText`, `MarkedValue`, `SurfaceText`). Everything else is a grammar: ids are slugs, names and pointers
  (`Name`) are token characters with no whitespace, roles are the ARIA enum. A test walks the generated JSON Schema
  (properties, array items, record keys) and fails if a new bare string appears without review.
- `check.ts` also scans for surface-authored text that shares a run of 8 words (16 characters of unspaced script
  such as CJK or Thai) with page text, or contains a whole short page sentence (3-7 words, 20+ characters), after
  case-folding, with punctuation, whitespace and format characters ignored. Page strings shorter than a sentence never
  count, so a heading like "Checkout" does not trip it, and a template sharing a short phrase with a longer page text
  is fine. Every window is hashed once, so cost is linear in the text. It catches copying (including a sentence lifted
  from a longer paragraph, quoting or punctuation changes), not paraphrase. It is a tripwire, not a proof.
- Secret and card detection are heuristics, listed so they can be reviewed and tested (`isSecretName`,
  `isSecretValue`, `isCardName`, `isCardConstant` in `check.ts`, table-driven tests in `test/checks.test.ts`):
  names are matched on token boundaries after camelCase/punctuation splitting (so `x-author` and `token_type` are
  fine, `api_key` and `sessionId` are not); a 15-19 digit Luhn-valid run counts as a card only when grouped like one
  or in a field or URL that contains a card word (`card`, `credit`, `cc`, `pan`; whole words, so `company_id` is fine). They will have false negatives (an unknown credential shape, a card number with an
  unusual separator) and rare false positives; the run-time layer (redaction, canary-secret scans) is the real defence,
  these checks stop the obvious cases from being committed to a surface.
- Money rules are keyed on what a request carries, not only on its label: a request with a `payment-token`
  input, or a step that can raise 3-D Secure/SCA, must be `financial`, and a financial action or step names its
  payment method and an idempotency key that is a caller input of the request.
- Confirmations and receipts name a `subject`: `{action}` or `{flow, step}`, exclusively. Evidence lists are
  bounded at 64 in the wire format.
- Lengths are Unicode code points. A lone surrogate is rejected by `check.ts` and by `canonicalize`, because it
  cannot round-trip through UTF-8 JSON.

## Content hash

`hash` = `sha256:` + hex of SHA-256 over the RFC 8785 canonical JSON of the document with the `hash` field
omitted (`computeHash`). With no floats on the wire, canonical form is: UTF-8, object keys sorted by UTF-16
code unit, no whitespace, strings as `JSON.stringify` writes them, integers in shortest decimal form.
The Rust emitter must produce the same bytes; `fixtures/valid/*.json` are the shared test vectors.

## Compatibility policy

`schema_version` is `MAJOR.MINOR.PATCH` (currently `1.0.0`).

- **Within a major, changes are additive:** new optional fields, new enum values, new union variants,
  new documents. Nothing is removed, renamed, retyped or made required. A new required field is a major.
- **Minor** bumps for additive change; **patch** for documentation or a fix that accepts no new documents
  and rejects no previously valid ones.
- A reader accepts a document of the same major and a minor **at most its own**: it knows every field
  and enum value that document can use. The wire format pins the major (`1.x.y`); `checkSchemaVersion()` (also run
  by `checkSurface`) refuses a newer minor. Strict objects alone do not catch a newer minor that adds only an enum
  value or a union variant, which is why the version check exists.
- An emitter writes the lowest minor that covers what it emitted, so older readers keep working.
- Every change that alters `schema/agent-surface.schema.json` ships with the regenerated file and fixtures;
  the check script and a test fail otherwise.
- A major bump needs a migration note in `docs/agent-surface/log/agent-schema.md` and keeps the previous
  major's schema file alongside.

## How emitters validate

No emitter is a second definition (design section 6); the Zod schemas here are the source of truth.
A TypeScript emitter imports `validateSurface()` directly. Any emitter (including a Rust one) can run
two checks in CI:

1. **Emit validates against this package.** The test that runs the emitter writes each document it
   produces to a temp file and runs

   ```bash
   bun agent-schema/scripts/validate.ts <file.json>...
   ```

   A JSON report of `{file, schema, ok, phase, issues: [{path, code, message}]}` always goes to stdout.
   Exit `0`: every file passed schema and checks. Exit `1`: at least one failed (phase `schema`, `check`, or `parse`
   for unreadable JSON). Exit `2`: usage error, including a file whose schema cannot be inferred. The schema is
   inferred from `kind: "agent-surface"` or the file prefix (`result.x.json`, `receipt.x.json`), or pass
   `--schema <surface|result|mandate|confirmation-request|confirmation-token|receipt>`.
   A non-TS consumer can instead validate with `schema/agent-surface.schema.json` (draft 2020-12; point at
   `#/$defs/<Name>`), which covers the wire format but not `check.ts` invariants.
2. **Round trip in a non-TS emitter.** Deserialise every file in `fixtures/valid/` into the Rust types and
   serialise it back; the result must be byte-identical after canonicalisation. Serde structs should use
   `#[serde(deny_unknown_fields)]` and `skip_serializing_if = "Option::is_none"` to mirror strict, absent-not-null.

The invalid fixtures in `fixtures/invalid/` (each with a `.expect.json` naming phase, issue path and code)
are the other half: the Rust deserialiser should reject the schema-phase ones too.

## Fixtures

`fixtures/valid/<schema>[.<case>].json` are bare documents. `fixtures/invalid/<case>.json` sit beside
`<case>.expect.json` (`schema`, `phase`, exact `issues` as `{path, code}`, and `why`). Both are written
by `scripts/fixtures.ts` (`bun scripts/build-fixtures.ts`); a test fails if the committed files differ.
Tests assert the full issue list, so a fixture fails only for the stated reason, and also check that the
generated JSON Schema accepts every valid fixture and rejects every schema-phase invalid one.

## Dependencies

`zod` 4.6.7 (runtime), `typescript` 5.9.3, `@types/bun` 1.4.4, `ajv` 8.20.0 (dev). All exact-pinned; the
lockfile is committed; `@types/node` is overridden to a version at least two weeks old.
