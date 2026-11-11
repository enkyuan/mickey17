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

Checks (from `agent-flows/`): `bun run check` = `tsc --noEmit` + `bun test` (98 tests). The
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
   The weak gate has no entropy test, so which values are looked up never depends on their characters.
   The strong gate (≥ 6 distinct characters, ≥ 2.5 bits/char, plus the exclusions) must pass in
   **at least one run**; a match that exists in some runs is counted in the others when the raw values
   are equal there. A weak-only match neither links nor blocks; a consistent one is reported as a
   `weak-source-ignored` warning (values of 8+ characters; shorter ones are not looked up at all). A single strong gate per value made 8-hex ids randomly derived, blocked or
   client-computed (review M6). Residual: an 8-hex id fails the strong gate in both runs about 5% of
   the time and is then `client-computed` (resolved, never blocked).
4. **Low-entropy values are never sources**, so a really-derived small id (`1`) can only be an
   `input` (if a parameter names it) or `client-computed`. Remedy: name it as a run parameter.
5. **Inputs.** Trace-event inputs first, then run-wide `params`. A low-entropy match against a
   run-wide parameter is kept but raises `weak-input-match` (it may coincide with another
   parameter, e.g. `page=1/2` vs `quantity=1/2`); attribute inputs to events to rule it out. A token-grade
   value that follows a parameter in some runs only is `unclassified`; a low-entropy one is
   `client-computed` with a `weak-input-partial` warning. A full input match wins over *partial* server
   evidence (a user input echoed by the server in one run is still an input).
6. **Declared input that did not vary** (event-scoped or run-wide) is embedded as a `constant` with an
   `input-did-not-vary` warning naming the alphabetically first matching input.
7. **Array selection** (`json:$.rates[*].id`): the picked element must be explained by a sibling
   *label* field (not a bare number or boolean) that equals a run parameter in every run and is unique in
   the array; otherwise `unclassified`. One array level only. Residual risk: a numeric-looking
   label field is rejected on purpose, so such selections block.
8. **Free-text sources** carry a locator (`before`/`after`, `origin: "page"`): the text around the first
   whole-token occurrence identical in every run, cut before any word of 8+ token characters, and
   refused if it contains a secret-looking piece. Surrounding `prefix`/`suffix` of an edge are refused the same way.
9. **Redaction (D7) is by name and by value.** A constant is withheld when its location is sensitive by name
   (word-bounded: `pin` does not redact `shipping`; `body`, `authorization`, `cookie`, `token`, `password`,
   `pwd`, `otp`, `jwt`, `api-key`, `hmac`, `signature`, `session`, `csrf`, `card`, `code`, `email`, …) **or** by what it
   looks like: a Luhn-valid card number anywhere in it, a token-grade piece (14+ characters with a digit,
   or 20+), an email or SSN, or a URL (absolute or relative, query, fragment or `;` form) with a sensitive
   parameter or secret-looking value. Components of a redacted URL are redacted with it. Over-redaction is the
   accepted failure direction: public keys (`pk-test-…`) and replayable constants such as `tokenType: "Bearer"` are
   withheld too and must come from the executor's credential store. Redacted constants carry no value and no hash and appear as `{location}`
   in `path_template`. Inputs and client-computed values are never emitted. Blocker text carries no request paths,
   and a location that embeds a token-grade key (a session id used as a JSON key) is printed `<redacted-location>`.
   An edge `prefix`/`suffix` is refused if it looks like a secret, and on a sensitive target only a short auth
   scheme word (`Bearer `) may precede the credential.
10. **Alignment is positional but checked.** Same method, host and path depth are not enough: a path
    segment that differs between runs must be an id/slug (≥ 6 chars with a digit, `-`/`_` or mixed case) or
    follow a run parameter, and the operation named by an RPC-style request (`operationName`, JSON-RPC `method`,
    `action`/`op` fields or query parameters) must agree or follow a parameter; otherwise the flow blocks
    ("the requests may have been reordered"). A heuristic for protocol-level endpoints: other same-endpoint
    reorderings are not detected (they surface as unexplained `client-computed` values).
11. **Hostile input.** `time` is clamped to 24 h (a huge value would make any response eligible as a
    source), non-HTTP statuses become 0, dates are range-checked, JSON nesting is capped at 128, substring search skips targets over 4096 chars.
12. **Determinism and ties.** Entries sort by start time, then id, then a hash of the whole entry with sorted
    keys; events without an id are named by their content; request/event/cookie order and JSON key order in the
    input are irrelevant. Requests with the same start time and no `_requestId` cannot be paired across runs, so
    the flow **blocks** (with explicit ids the id breaks the tie).
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
Warning kinds: `constant-matches-source`, `input-did-not-vary`, `weak-input-match`, `weak-input-partial`, `weak-source-ignored`, `entry-skipped`, `event-mismatch`.
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

**Round 2 (Opus, high effort) on the TypeScript port (d6e4ba4).** The fuzzer found no crashes (3000 mutations).
Verified findings and what was done:
- *Fixed, with tests:* H-a RPC-style same-endpoint reordering (operation key check); H-b/H-c constants redacted by name only
  (now by value too, child slots follow their parent URL); H-d secretish bypassed by digitless keys, spaced card numbers,
  path-like locator text (reworked; auth-scheme-only prefixes on sensitive targets); M-a a no-op encoding in one run blocked a
  legitimate flow (no-op transforms agree); M-b weak-only matches blocked before inputs were tried (weak neither links nor
  blocks; full input beats partial server evidence); M-c selecting an element by its own value (now an input); M-d silent
  `client-computed` on ignored consistent matches (warnings); M-e URL-valued fields (parent refused, children redacted); M-f
  quadratic regex scanning of HTML (single-pass scanner); M-g token-grade keys echoed in locations (`<redacted-location>`); M-h
  determinism holes (sorted keys in hashes, alphabetical input choice, ties block); M-i prefix-colliding variants (length-class lookup);
  duplicate jar and `Set-Cookie` names; prototype-named HTML entities; invalid-URL entries reported (`entry-skipped`); slug path
  segments; run-wide inputs in `input-did-not-vary`; stricter tests (digitless secrets, `code=`, child slots, GraphQL, key order, ties).
- *Not fixed, documented:* page-derived key names in `location`/`select.field`/`reason` carry no `origin: "page"` marker (schema
  adoption must wrap them, D8); storage and DOM sources (storageState `origins[]`) are not read, so such values are
  `client-computed`; a missing or string HAR `time` counts as instantaneous; weak-only values shorter than 8 characters are never looked up.

**Round 3 (Opus, high) re-review of d72cbb5.** Confirmed holding: no-op aliases (exact path), weak-only handling, input-beats-partial,
the operation check for `operationName` bodies, tie blocking (the fixtures use explicit ids and still resolve), `entry-skipped`,
sorted-key hashing, token-grade keys in locations, auth-scheme-only prefixes. New verified findings, all fixed with tests:
card number with its expiry glued on (digit windows + Luhn); over-redaction of replayable constants (`content-type`,
`user-agent`, slugs, SKUs, `currencyCode`: benign headers are exempt from value checks, slugs and readable paths are not
token-grade, a no-digit token needs mixed case); no-op aliases missing on the substring and fill-in paths; operation check missed
GET `?operationName=`, batched bodies and query-only GraphQL (any depth, plus the operation name inside the GraphQL document);
quadratic `piiish` regex (linear scan); crowded length-class lookup (at most 64 lengths per window); duplicate jar names
keyed by value order (now not sources, with a `jar-duplicate-names` warning); short secrets and PII (names `pw`,
`passcode`, `pincode`, `totp`, `key`; `name=value` and JSON-text pairs with percent-decoded names; emails and SSNs in
prefixes, locators and locations; trace selectors have `value=` attributes removed); HTML scanner (non-ASCII before a tag, `<!-->`,
markup inside attribute values: every tag is now parsed through its quoted attributes); explicit-id ties sort by natural order;
`entry-skipped` no longer embeds the entry index; event `type`/`target` text is cleaned.
Known and kept: a persisted-query `sha256Hash` or any other 14+ character digit-bearing token constant is redacted (it cannot
be told from a secret); Chrome/Playwright HARs without `_requestId` and with same-millisecond requests will block on `ambiguous` order.

**Round 4 (Opus, high) re-review of db88b4e.** Confirmed holding: card windows, kept constants, duplicate jar names, GET/batched/query-only
GraphQL blocks, a linear HTML scanner on hostile input, linear `piiish`, selector cleaning, encoded names. New findings, fixed with tests:
slug/word-shaped secrets under ordinary names were emitted by the round-3 exemptions (the slug exemption now applies only under
product/path/label location names; a digitless token needs 20+ characters again); the 64-length cap silently dropped source
lengths (the longest are kept and the rest reported as `variant-index-truncated`); `operationOf` falsely blocked nested `method` fields,
server-issued `action` values and free text starting with "query" (now only top-level/batched operation fields, `query` fields for the GraphQL document, and id-like
values are ignored); the benign-header exemption covered credentials (limited to descriptive headers; `user-agent` only when it is a product token);
non-English password names, numeric and single-quoted JSON text, `.`/`_`/`/` card separators, automation-style selectors (`fill("…")`, `text=…`);
the HTML scanner now treats `textarea`, `title`, `xmp`, `noscript`, `template` as inert, `<![CDATA[`/`<?` as bogus comments and accepts `--!>`;
`cardIn` and `natCmp` bounded (no BigInt). Known and kept: a duplicate jar cookie name in only one run blocks (the name is then not a source in that run).

**Round 5:** re-review of the round-4 fixes — see below once it has run.

## Commands run

- `tsc --noEmit` (5.9.3, strict + `noUncheckedIndexedAccess` + `exactOptionalPropertyTypes`): clean.
- `bun test` in `agent-flows/` with `MOCHI_AGENT_SHOPS=…/agent-fixtures…/benchmarks/fixtures/agent/shops MOCHI_REQUIRE_AGENT_FIXTURES=1`: 98 pass, 0 fail (259 truth edges).

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
recordings, framework-neutral), evidence (98 tests; 259 edges vs the fixture truth on three stacks; mutation
controls; review findings fixed), limits (above), review summary (this log).
