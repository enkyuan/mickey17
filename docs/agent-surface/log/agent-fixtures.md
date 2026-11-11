# agent-fixtures log

Owner: fixture-shops session. Scope: `benchmarks/fixtures/agent/shops/**`,
`benchmarks/tests/agent-shops.test.ts`, this log. Design: `docs/AGENT_SURFACE.md` (§1, D1, D5, D9, D11, M0).

## Result

Three shops (React, server-rendered with a ~70-line htmx-style script, Svelte), one flow, four
scenarios × two runs each, hand-labelled ground truth per shop, a recorder that writes HAR 1.2 +
interaction traces, and `benchmarks/tests/agent-shops.test.ts`.

`bun test tests/agent-shops.test.ts` (from `benchmarks/`): **103 pass, 3 skipped, 0 fail** (after review round 1; was 89/3/0 at 359bf0c). `tsc
--noEmit` over the test and its imports is clean (run from a scratch tsconfig, see below).

## What could not be run here

- **Anything that binds a socket.** Loopback listening is denied in this sandbox
  (`Bun.serve` fails with EADDRINUSE on port 0). The 3 skipped tests are the real-socket runs
  (one per stack); the `import.meta.main` launchers (`bun <stack>/server.ts`, `PORT`, default
  ports 3000/8080/5173) were never started. Everything else runs in-process: each shop is a
  `fetch(Request) → Response` handler, the cross-stack test calls it directly, and the recorder
  routes the page's `fetch` to it. This is HTTP semantics without a socket, not a network test.
- **A real browser.** The recorder runs the real client code (React bundle, Svelte bundle,
  `enhance.js`) in happy-dom. It is a stand-in for the Chromium recorder D11 names; the HAR and
  trace formats are the same so it can be swapped. It has no DOM-level listener inventory
  (D3 evidence); traces carry event → request ids and added/removed element ids only.
- **npm.** The registry is blocked. React 19.2.3, react-dom 19.2.3 and Svelte 5.34.1 were taken
  from copies already on this machine (`~/Desktop/Projects/landing`, `…/www`) and the bundles were
  built with `bun build.ts`. The built bundles are committed (`react/public/app.js` 188 KB,
  `svelte/public/app.js` 43 KB). `shops/package.json` pins those versions for a rebuild;
  `shops/node_modules` is git-ignored (`shops/.gitignore`).
- **Vue and htmx were not available offline**, so the third framework is Svelte, and the
  "HTMX-style" stack uses an in-house `enhance.js` with `data-*` attributes (not the htmx library).
  The pattern is the same: server returns HTML fragments, the client swaps them by id and fires
  `data-get` loads.

## Design decisions the extractor work should know

1. **Behavior is defined once** (`shared/core.ts`); stacks differ only in wire format and client
   code. So the cross-stack test mostly proves each wire/renderer is faithful, and the recorder
   run proves each client's JS issues the requests the HTTP clients do (final shop state equal).
2. **Wire differences are real and intended:** JSON + PUT for React/Svelte; urlencoded POST forms
   and HTML fragments for the server-rendered shop (ids in `data-*` attributes and form `action`s).
   Ids line up across stacks because all draw from one seeded PRNG in the same order.
3. **Exactly one client-computed value per stack**: `X-Signature`. The timestamp is *inside* that
   value (not a second header) so the count stays one. The same `shared/sign.js` is bundled into
   React/Svelte and served as `/static/sign.js` to `enhance.js`; if you want three independent
   implementations to stress `lift`, that is a small change.
4. **Idempotency key vs D11's four classes.** The task named the idempotency key and the
   client-computed signature as separate value kinds and asked for exactly one client-computed
   value; D11's table, read literally, would class a `crypto.randomUUID()` nonce as
   `client-computed` (the reviewer's point). I labelled it `class: "input"`, `role:
   "idempotency-key"`, `generator: "uuid-v4"`, which also matches agent-schema's separate
   `FlowStep.idempotency` field. **Decision for the parent:** confirm, or relabel it
   `client-computed` (then each stack has two, and `clientComputedCount`/`validateTruth` change).
   Either way the executor can mint a fresh UUID with no site code.
5. **CSRF sources differ per stack** (meta / hidden field / cookie) on purpose; the server-rendered
   shop also re-renders the same token in every returned form, so "source of the token" is
   ambiguous there (truth lists the bootstrap document as canonical).
6. **Browser-managed values are labelled `automatic: true`** (Cookie header: `sid`, plus `csrf` on
   Svelte). The graph-equality test ignores them and constants.
7. **Payments:** same-origin `/payments-stub/v1/tokens`, JSON, no CSRF, test method ids only.
   `tokenize-payment` carries `payment: {hostedField: true, tokenOnly: true}` in the truth. Token is
   single-use; decline is decided by the method id; declines are not idempotency-cached.
8. The server-rendered shop needs its script for payment (the signature); forms work without JS for
   the earlier steps only.

## Ground truth

`react|svelte|server-rendered/flow.truth.json`, schema tag `mochi.agent.flow-truth/0.1`: steps
(with D5 `effect`, `idempotent`), requests with per-value `class`
(constant/input/derived/client-computed) and locators, dependency `edges` (`<req>:res.<where>` →
`<req>:<where>`), `stateDependencies`, mutation `controls` (D11 tampered/stale/replayed cases with
expected status and code) and observed response classes per scenario. Written by hand from the
app source. The Svelte and server-rendered files were derived from the React file with a one-off
editing script (not an extractor) and then reviewed against their own sources.

**Mapping to the agent-schema package:** not merged when this was written, so plain JSON. Intended
mapping: `steps[].effect` → D5 `effect`; `values[].class` → D11 value class; `edges` → the flow
graph's dependency edges; `values[].role: idempotency-key` → D5 `idempotent` key; `controls` →
D11 mutation controls; `locators` → whatever the schema uses for request/response positions.
Re-express once its first PR merges.

What checks the truth (without running an extractor): `validateTruth` (structure, every derived
value has an inbound edge, one client-computed, one idempotency key), `graphSignature` equality
across the three stacks (M0's neutrality test), and `checkTruthAgainstHar` against every committed
HAR: each recorded request matches a labelled request, no unlabelled header/cookie/body field,
constants equal their label, each derived value occurs in its declared source response, the
signature equals `sign(timestamp, body)` and appears in no response. Tampered truth/HAR mutations
are asserted to fail.

## Recordings

`recordings/<stack>/<scenario>.run{1,2}.{har,trace.json}`: 24 HARs + 24 traces, ~680 KB total,
HAR 1.2 with `_requestId`/`_eventId` extensions. Deterministic (seeded ids, pinned `Date.now`
and `crypto.randomUUID`, virtual times); the happy-dom test re-records every run and requires
byte equality with the committed files. Static asset bodies are omitted. Runs differ in seed
(so cart/rate/order/session/CSRF values differ), sku, quantity, address, shipping rate.

## Tests (what each proves)

- Per scenario run (8 × 3 stacks): identical status sequence, identical response views, identical
  final shop snapshot, and the status sequence equals the scripted expectation.
- Final order equals quote (success); no order and stock untouched (declined, out-of-stock).
- Per stack: CSRF required, bad/stale/missing signature, idempotency key required/replayed/bound,
  stale or foreign cart id, closed cart, reused token, stale rate, incomplete cart, payment stub
  refuses a card-number-shaped input.
- Truth checks above; recordings are structurally HAR 1.2 and trace ids link.

Not covered: DOM/a11y parity (T0), route enumeration, drift scripts (M5), any extractor.

## Follow-ups for others

- agent-schema: re-express truth in its types; decide the idempotency-key class (above).
- Extractor (M2): `checkTruthAgainstHar`/`graphSignature` are reusable as the comparison
  baseline; the truth `controls` are M3's mutation-control list.
- Someone with loopback: run `bun test tests/agent-shops.test.ts` (3 more tests activate) and
  `PORT=… bun <stack>/server.ts` once per stack, one at a time.
- Typecheck note: `benchmarks/tsconfig.json` does not include `fixtures/`; the test's imports are
  checked transitively. `tsc` was run from a scratch tsconfig extending it.

## Decisions (deviations from / gaps in docs/AGENT_SURFACE.md)

- D11 value-class table has no entry for a caller-mintable idempotency key (see 4 above).
- D11 payments: the stub is a same-origin, in-flow tokenization step (`tokenize-payment`), which
  the design treats as out of scope for real providers; here it exists so extraction has a
  hosted-field-shaped step to classify. agent-schema's `PaymentMethod` has no value for "in-flow
  test tokenization"; closest is `caller-token`.
- Truth locators (`url.path[n]`, `res.html.<css>@attr`, `cookie.<n>`) differ from agent-schema's
  `{in, name}` + `DependencySource` (response/dom/cookie/storage). DOM sources (CSRF meta tag,
  hidden field, `data-cart-id`) need `n_<hex>` node ids that only a capture can supply, so they
  cannot be expressed in the schema from the app source alone. Not adopted: waiting for the fixed
  PR's fixtures (parent's message).
- Strict quantity parsing (whole decimal text) is now the server contract; React/Svelte send the
  text as typed (they previously coerced with `Number()`, which accepted `2.0`, `1e1`, `0x2`).
- `GET shipping-rates` is idempotent for an unchanged cart (the quote is cached until the cart's
  items or address change), so labelling it `effect: read` is true.

## 2026-11-10 entry: review round 1 (Opus, adversarial) and fixes

Reviewer got the commit and doc sections, not my conclusions. Verdicts:

| # | Finding | Verdict | Action |
| --- | --- | --- | --- |
| 1 | cross-session authorization test could not fail (new `Shop`, not a second session) | confirmed | test now uses a second session of the same shop for cart ops and order read |
| 2 | CSRF tested only on place-order | confirmed | per-endpoint test for create-cart, add-item, set-address, select-shipping, wrong and missing token; reads need none |
| 3 | quantity parsing differed (React/Svelte `Number()` vs SSR strict) | confirmed | clients send text as typed; test over 11 odd inputs on all stacks; truth `encoding` updated; bundles rebuilt, runs re-recorded |
| 4 | SSR routes looser than JSON stacks (`/items/anything`) | confirmed | `!seg[n]` checks added |
| 5 | untested core properties | confirmed | tests: idempotency scoped per session, stock re-check at order time, name length, quote invalidation, `cart_closed` in select-rate |
| 6 | graph-equality partly by construction (Svelte/SSR truth derived from React) | confirmed, partly accepted | the per-stack truth is now checked against that stack's own HARs (stricter checker, below); equality remains a consistency check, not independent proof |
| 7 | `shipping-rates` labelled read but regenerated the quote on each GET | confirmed | server fix (cached quote), test for repeated read and stale quote |
| 8 | idempotency key class vs D11 | open | see Decisions; parent to confirm |
| 9 | replay control does not hold after a decline | confirmed (documented) | control reworded to "after a 201"; note on declines; no code change (spent token, uncached failure is deliberate) |
| – | payment tokens not session-bound | accepted | the stub models a separate provider; documented limit |
| – | HAR sizes in characters, Set-Cookie missing from headers, `domChanges` blind to alerts | confirmed | bytes via `Buffer.byteLength`, `set-cookie` headers added, `[role=…]` elements tracked |
| – | test sent a card-number-shaped string to the stub | confirmed | now `not_a_test_method` |
| 6b | truth checker accepted wrong truth (derived->input, relabelled statuses/effect, idempotency->constant, …) | confirmed | `validateTruth`: read steps must be GET, idempotency key only on a financial step; `checkTruthAgainstHar`: statuses must be labelled, every header and open path segment labelled, an input must not equal a server-issued value; new `checkTruthAcrossRuns`: constants constant, derived/computed/idempotency never repeat across runs, every labelled status observed. 10 mutations x 3 stacks are asserted to be rejected |

Commands: `bun test tests/agent-shops.test.ts` (103 pass, 3 skip, 0 fail; skips are the loopback
socket tests), scratch-tsconfig `tsc --noEmit` clean, `bun build.ts`, `bun record.ts`.
Two false positives in my first stricter checker (echoed form values, enum option lists) were fixed
in the checker, not by weakening the mutations: values the caller already sent, and enumerated
domains, are exempt.

## PR text (not opened until round-2 review is clean; base `feat/tanstack-exporter`)

**Title:** `test(agent): fixture shops in three stacks with ground truth, recorder and cross-stack test`

**What:** React, server-rendered (htmx-style) and Svelte shops with one flow and four scenarios;
hand-labelled `flow.truth.json` per stack; HAR 1.2 + interaction-trace recorder (24 committed
runs); `agent-shops.test.ts`. **Why:** M0 neutrality test and M2 ground truth. **Evidence:** 103
pass / 3 skip; mutation tests for the truth checks; recordings byte-reproducible. **Limits:**
no socket runs, happy-dom not Chromium, no listener inventory, idempotency-key class pending,
truth not yet in agent-schema types. **Review:** Opus adversarial round 1 (table above), round 2
below.
