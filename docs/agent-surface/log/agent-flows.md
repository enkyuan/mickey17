# agent-flows log

Owner: flow-inference session. Scope: `agent-flows/**` (Bun/TypeScript package, strict `tsc`,
exact-pinned dev dependencies, no runtime dependencies), `agent-flows/tests/**`, this log.
Design: `docs/AGENT_SURFACE.md` §1 (framework neutrality), §3, D5, D11, milestone M2.

## Result

`agent-flows` turns two or more recorded runs of a scenario (HAR 1.2 + interaction trace +
starting cookies) into a typed **flow graph**: ordered steps, a class for every request value
(`constant | input | derived | client-computed`, or `unclassified`, which blocks the flow) and the
dependency edges between steps. Deterministic, no React/Base UI assumption anywhere.

```ts
import { infer, parseRecording, toJson } from "mochi-agent-flows";
const graph = infer([parseRecording(har1, trace1, cookies1), parseRecording(har2, trace2)], { ignorePathPrefixes: ["/static/"] });
process.stdout.write(toJson(graph));          // canonical JSON
```
CLI: `bun agent-flows/src/cli.ts --run <har>,<trace>[,<cookies>] --run … [--ignore-host H] [--ignore-path-prefix P] [--only-traced] [--out f.json]` (exit 2 when blocked).

Checks (from `agent-flows/`): `bun run check` = `tsc --noEmit` + `bun test` (58 tests). The
ground-truth tests need the fixture shops from the agent-fixtures branch
(`benchmarks/fixtures/agent/shops`): until that merges they skip with a warning unless
`MOCHI_AGENT_SHOPS=<dir>` points at them; `MOCHI_REQUIRE_AGENT_FIXTURES=1` turns "not found" into a failure
(set it in CI once merged).

## History

The first implementation was Rust (`src/agent/flows`, branch `archive/agent-flows-rust`, commit
b9c89a4, local only). On the parent's instruction (flow inference moves to TypeScript) it was
ported to `agent-flows/` and is **not** part of any PR. The port kept the algorithm and the test plan.

## Decisions

Gaps in or deviations from `docs/AGENT_SURFACE.md`, with reasons:

1. **Precedence is not "earliest source" alone.** For a value that varies across runs:
   server-issued source (cookie jar, response; earliest) > run parameter > earlier request value >
   `client-computed`. D11 says "derived: taken from an earlier step"; a user input reused in a
   retried request is still an input, and a server value is wired from the server, not from the
   client request that happened to carry it first.
2. **A relation must hold identically in every run** (same transform, prefix, suffix, source
   location). A match in some runs only is `unclassified` ("matches … in 1 of 2 runs"), never `derived`.
3. **Evidence rule has two gates.** The weak gate decides what is looked up (length 8..2048, not a
   URL/path, MIME type, ISO/HTTP date, epoch, number under 12 digits, or word under 16 letters).
   The strong gate (≥ 6 distinct characters, ≥ 2.5 bits/char, plus the exclusions) must pass in
   **at least one run**. A single strong gate per value made 8-hex ids randomly derived, blocked or
   client-computed (review M6). Residual: an 8-hex id fails the strong gate in both runs about 5% of
   the time and is then `client-computed` (resolved, never blocked).
4. **Low-entropy values are never sources**, so a really-derived small id (`1`) can only be an
   `input` (if a parameter names it) or `client-computed`. Remedy: name it as a run parameter.
5. **Inputs.** Trace-event inputs first, then run-wide `params`. A low-entropy match against a
   run-wide parameter is kept but raises `weak-input-match` (it may coincide with another
   parameter, e.g. `page=1/2` vs `quantity=1/2`); attribute inputs to events to rule it out. A token-grade
   value that follows a parameter in some runs only is `unclassified`.
6. **Declared input that did not vary** is embedded as a `constant` with an `input-did-not-vary`
   warning naming the input (the recording cannot show it is an input).
7. **Array selection** (`json:$.rates[*].id`): the picked element must be explained by a sibling
   *label* field (not a bare number or boolean) that equals a run parameter in every run and is unique in
   the array; otherwise `unclassified`. One array level only. Residual risk: a numeric-looking
   label field is rejected on purpose, so such selections block.
8. **Free-text sources** carry a locator (`before`/`after`, `origin: "page"`): the text around the first
   whole-token occurrence identical in every run, cut before any word of 8+ token characters, and
   refused if it contains a secret-looking piece. Surrounding `prefix`/`suffix` of an edge are refused the same way.
9. **Redaction (D7).** Constants are embedded except where the location is sensitive by name
   (word-bounded: `pin` does not redact `shipping`; `body`, `authorization`, `cookie`, `token`,
   `password`, `pwd`, `otp`, `jwt`, `api-key`, `signature`, `session`, `csrf`, `card`, …), the value is
   a Luhn-valid card number, or it is a URL carrying such a parameter or secret-looking value. Redacted
   constants have no value and no hash (an unsalted hash of a low-entropy secret is reversible) and
   appear as `{location}` in `path_template`. Inputs and client-computed values are never emitted.
   Blocker text never contains request paths.
10. **Alignment is positional but checked.** Same method, host and path depth are not enough: a path
    segment that differs between runs must be an id (≥ 6 chars with a digit) or follow a run
    parameter; otherwise the flow blocks ("the requests may have been reordered").
11. **Hostile input.** `time` is clamped to 24 h (a huge value would make any response eligible as a
    source), non-HTTP statuses become 0, dates are range-checked, JSON nesting is capped at 128, substring search skips targets over 4096 chars.
12. **Determinism ties.** Entries sort by start time, then id, then a hash of the whole entry; events
    without an id are named by their content; request/event/cookie order in the input is irrelevant.
13. **JSON types** are kept (`json_type` for non-strings, empty `{}`/`[]` are values); a type that
    differs between runs blocks.

## Input formats

### Trace JSON (one per run)

```jsonc
{
  "version": 1,
  "run": "run-a",              // run id (string), or a number combined with "scenario": "success.run1"
  "params": { "quantity": "2", "address": { "zip": "94110" } },   // run-wide inputs ("inputs" is a synonym); nested objects flatten to "address.zip"
  "events": [
    { "id": "e1", "t": "2026-11-09T10:00:00.000Z",   // t optional; ordering falls back to the id
      "type": "click", "target": "#add",
      "inputs": { "quantity": "2" },                  // optional: what the user supplied at this event
      "requests": ["r4", "r5"] }                      // HAR entry ids
  ]
}
```
- **HAR entry id:** `_requestId` when present, else `c-<sha256(method, url, body)[..16]>`. Give every entry a distinct `startedDateTime`.
- **Cookies** (optional third argument): JSON array of `{name, value}` or `{cookies: [...]}`
  (Playwright `storageState`): the jar *at the start of the run*.

### What is extracted (locations identical across runs)

| location | where |
| --- | --- |
| `url.path[i]`, `url.query:k[#n]` | decoded path segment (empty skipped), query parameter |
| `header:name[#n]`, `cookie:name[#n]` | header (`cookie` split into pairs; `content-length`, `host`, `date`, … ignored) |
| `setcookie:name` | `Set-Cookie` |
| `json:$.a.b[0]`, `json:$["odd key"]` | every JSON leaf incl. `null`, empty containers; numbers keep their text |
| `form:name[#n]`, `multipart:name[@filename]` | urlencoded / multipart bodies |
| `html:input[name=x]`, `html:meta[name=x]` | hidden inputs and meta tags (a tolerant scanner; comments, scripts and styles skipped) |
| `body` | opaque request body (always redacted if constant); free-text response bodies are token-searched |
| `<loc>\|path[i]`, `<loc>\|query:k` | components of any absolute URL held in a value |

Transforms in a relation (`target == prefix + transform(source) + suffix`): exact, URL-, form- and
JSON-encoded, URL-decoded, JSON-unescaped, base64 (4 alphabets/paddings) and strict base64-decoded.
Not covered: base64 blobs embedding the source with extra text, hashes, `Number()` coercions.

## Flow graph JSON (`mochi.agent.flow-graph/1`)

`schema, status ("resolved"|"blocked"), runs, inputs[{name, uses}], steps[…], edges[…], blockers[{step, location, reason}], warnings[{step, location, kind, detail}]`.
Step: `index, id, event?, method, origin, path_template, statuses, values[{location, class, value?|redacted?, json_type?, input?, source?, via?, shape?, reason?}], provides, needs`.
Edge: `from{where: initial-cookie|request|response, step|null, location, locator?, select?} → to{step, location}`, `via`, `prefix?`, `suffix?`.
Warning kinds: `constant-matches-source`, `input-did-not-vary`, `weak-input-match`, `ambiguous-order`, `event-mismatch`.
Canonical JSON: fixed key order, sorted collections, trailing newline; identical for shuffled entries, events, cookies and runs.

## Mapping to the agent-schema package (not adopted yet)

The first agent-schema PR was closed and will reopen fixed; the parent will say when to adopt it. Until
then the graph uses local types (`src/types.ts`). Intended mapping, per step `sN` → `FlowStep` with one `RequestSpec`:

| flow graph | agent-schema |
| --- | --- |
| `url.path[i]` / `url.query:k` / `header:h` / `cookie:c` | `RequestLocation {in: path\|query\|header\|cookie}` |
| `json:$.a.b[0]` / `form:f` | `{in: body-json, name: "/a/b/0"}` (RFC 6901) / `{in: body-form}` |
| `constant` / `input` / `derived` | same value classes (`constant` becomes a `PageValue`; origin page) |
| `client-computed` | `resolution: requires_browser` with `reason` from `shape` (`uuid→nonce`, `epoch-time→timestamp`, `hex→signature`, else `unknown`) |
| `unclassified` | `BlockedFlow.unclassified[]` (`no-source`/`ambiguous-source`/`multiple-sources`) |
| edge `from.where=response` | `DependencySource {kind: response, step, request, at}` |
| edge `from.where=initial-cookie` | `{kind: cookie, name}` |

Gaps (additive, to report to agent-schema for PR 2): a `request` source kind; free-text/`html`
response locations with a locator; array selection; `prefix`/`suffix` and encoding names on an edge
(`MatchKind` has no `substring`/encodings); a JSON value type on constants; an idempotency-key
resolution (`mint: uuid`; the recording only shows a fresh uuid, so it is `client-computed`); `unclassified`
reasons for `partial-evidence`/`not-aligned`. Strings in `reason`/`detail`/`location` are tool-authored but
embed page-derived identifiers (cookie and JSON key names); the schema adoption must treat them as page text (D8).

## Ground-truth comparison (agent-fixtures, `flow.truth.json`)

3 stacks × 4 scenarios × 2 runs. Every graph step matches a labelled request, every labelled value
exists, every class equals its label except the allowed divergences below, and **259 labelled
dependency edges match in source request and location** (HTML sources: a `meta` tag must be the
structured `html:meta` source; a hidden input the structured source or a located token).

| truth | ours | count | why |
| --- | --- | --- | --- |
| `input` `country`, `method` | `constant` (+ `input-did-not-vary` warning) | 21 | identical in both recorded runs |
| `input`, `role: idempotency-key` | `client-computed` (`uuid`) | 9 | no source in the recording |

Also tested: the three stacks give the same per-step (leaf, class, producing step) signature
(`rate` vs `rateId` normalised; the `Number()` coercion case no longer occurs in the current recordings),
and deleting the bootstrap request leaves no CSRF/session value derived from it and makes the first CSRF use a root.

## Tests (all can fail)

`tests/flows.test.ts`: generated HARs (CSRF cookie→header, cart-id chaining, bearer prefix, base64 header,
Location query, idempotency key, signature, timestamp, form post); mutation controls (delete the source
in all runs / in one run / blank the response of 60 generated flows and require the first consumer to be
`client-computed` and later consumers to chain to it); false edges (low-entropy correlation, `true`/`USD`/dates,
a coincidence in one run, a static key that also appears in a response); blocking (optional value, ambiguous
parameters, misalignment, one run); order independence (40 shuffles and 60 generated flows);
60 random flows classed exactly as constructed; a 200-step recording.
`tests/review.test.ts`: one regression test per verified finding of the first review (below).
`tests/units.test.ts`: evidence table, matcher, HAR helpers, JSON parser. `tests/ground-truth.test.ts`: above.

## Review

**Round 1 (Opus, high effort, adversarial) on the Rust version (b9c89a4); findings were verified by probes.** Fixed in the
port, each with a regression test in `tests/review.test.ts`:
H1 reordered same-depth requests were paired (alignment now checks path segments); H2 secrets leaked via edge
prefix/suffix, locators, `path_template`, parent URL values, incomplete name list, a crackable hash, blocker text
(all redacted or refused; hash removed; page origin on locators); M1 hostile `time` (clamped); M2 ties and
position-named events (hash tie-break, content-named events); M3 array selection invented explanations
(label-only, unique, scalar arrays and nested arrays refused); M4 low-entropy input coincidences (warning);
M5 partial input evidence silently client-computed (now blocks); M6 per-value entropy randomness (two gates);
M7 quadratic substring search (window index, length cap; 40 steps × 20 leaves in well under a second);
M8 duplicate cookies, JSON types, empty containers, `#` in names; M9 undeclared constant inputs (warning).
Not changed: source precedence vs "earliest" (decision 1, documented); first-occurrence locators now use whole tokens.
Tests the reviewer found too weak were strengthened (deleted-source test, bootstrap deletion, cross-stack
signature now includes the producing step, HTML truth sources).

**Round 2:** re-review of the fixes in the TypeScript package — see the entry below once it has run.

## Commands run

- `tsc --noEmit` (5.9.3, strict + `noUncheckedIndexedAccess` + `exactOptionalPropertyTypes`): clean.
- `bun test` in `agent-flows/` with `MOCHI_AGENT_SHOPS=…/agent-fixtures…/benchmarks/fixtures/agent/shops MOCHI_REQUIRE_AGENT_FIXTURES=1`: 58 pass, 0 fail.

## Not run / limits

- `bun install` cannot reach the registry or write its cache here (sandbox). `node_modules` was copied from the
  agent-schema package and `bun.lock` was derived by hand from its lockfile (same packages and integrity hashes);
  `bun install --frozen-lockfile` has not been run against it.
- The fixture recordings come from a sibling worktree; they are not on this branch.
- Two runs cannot tell a constant from a server value that repeats (warning only); record on fresh sessions.
- Alignment is positional; ambient traffic must be filtered with `ignoreHosts`/`ignorePathPrefixes`/`onlyTraced`.
- Optional values (present in some runs) block. Functions of an input (totals, hashes, `Number()`) are `client-computed`.
- Concurrent requests never create edges (the response must end before the request starts).
- Cost: substring search is O(target length) per varying value after indexing; untested above a few hundred requests.

## PR text (not opened: waiting for the parent and a clean re-review)

Title: `feat(agent-flows): infer typed flow graphs from recorded HAR runs`
Base: `feat/tanstack-exporter`. Body: what (package, algorithm, formats), why (M2: classes and edges from
recordings, framework-neutral), evidence (58 tests; 259 edges vs the fixture truth on three stacks; mutation
controls; review findings fixed), limits (above), review summary (this log).
