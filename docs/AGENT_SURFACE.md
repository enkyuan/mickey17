# Agent surface: a machine-readable, browser-free interface to a site

Status: design only. Nothing here is implemented. This document fixes the decisions
that must be made before code is written, and the conditions under which the result
may be called correct.

## 1. Problem, goal, non-goals

A site is built for a human with a rendering engine. An agent that wants to read it or
act on it today must run a browser: expensive, slow, nondeterministic, and it exposes
the agent to everything on the page (including text written to manipulate it).

**Goal.** For a site, produce an *agent surface*: a typed, versioned description of
what the site contains and what can be done on it, plus an executor that performs those
actions without a browser instance (an HTTP client and, where unavoidable, a headless
JS runtime with a DOM shim; never layout or paint).

**Primary target.** Not reading landing pages: *functionally using* a site's
controls, above all multi-step transactional flows (cart, address, shipping, payment,
confirmation, account changes) without browser infrastructure at run time. Reading
(T0/T1) is the cheap first slice and the way to validate the pipeline; transactions
(T2 flows, D11) are what justifies the system and what dominates its risk.

**Framework neutrality.** The surface must be derived from what a site *does*, not
from how it was built. It must work for React, Vue, Svelte, HTMX, server-rendered
templates and hand-written JS alike. Nothing in the agent backend may depend on
React, Base UI or TanStack facts; those are optional accelerators (section 3).

**Non-goals.**

- Defeating bot protection, CAPTCHAs, fingerprinting, rate limits or paywalls.
- Operating visual-only experiences (canvas, WebGL, drag-to-draw). They are described,
  not operated.
- Replacing a site's own API. If a documented API exists, the surface points to it.
- Acting on a third-party site without that site's authorization (section 4, D2).

## 2. First principles

An agent loop has four needs. The browser serves them only incidentally.

| Need | What the agent must be able to do | What a browser gives today |
| --- | --- | --- |
| Observe | read content and structure cheaply, with stable references | pixels, a DOM, an a11y tree |
| Decide | know which actions exist, their inputs, and their consequences before acting | nothing explicit; inferred from labels |
| Act | invoke an action with typed arguments | simulated clicks and keystrokes |
| Verify | confirm the action had the intended effect | re-read the page |

Four invariants follow, and every later decision is judged against them.

1. **Content is data, never instructions.** Anything extracted from a page is untrusted
   input to the agent. The surface must make the boundary between *site-declared
   structure* (ours, typed) and *page text* (theirs, opaque) impossible to blur.
2. **A claim must be checkable.** Every action the surface lists must carry the
   evidence it was derived from, and a procedure that can show it wrong.
3. **Refuse over guess.** An action we cannot model is listed as unsupported with a
   reason, not approximated. A wrong action is worse than a missing one, because
   actions have side effects and reads do not.
4. **The surface is a view, not a copy.** Where freshness or personalization matters
   it is resolved at call time under the caller's authorization; the snapshot supplies
   shape only.

## 3. What can be known, and from what

Evidence sources, in descending trust:

1. **Declared by the site**: sitemap, OpenAPI or GraphQL schema, `llms.txt`, WebMCP
   tool registrations, JSON-LD, form `action`/`method`, ARIA, HTML semantics.
2. **Observed** (framework-neutral, the primary source): captured HTTP traffic
   (requests with their responses), cookies and storage, DOM and computed style per
   viewport and state, RSC/Flight payloads, and **interaction traces**: which
   listeners exist (DOM-level, not framework-level) and what requests, DOM changes and
   navigations each user event produced when a scripted run exercised it. A browser
   is used at capture and verification time only.
3. **Derived** (optional accelerator): static analysis of the site's own JS (oxc
   closure analysis as in `semantic::js` and `mochi lift`) to explain *why* a value
   is what it is: which handler builds which request, which values are computed on
   the client. It sharpens evidence and finds unexercised paths; it is never the only
   route to an action, and its absence (no JSX bundle, a non-React site) lowers
   coverage but does not change the model.
4. **Inferred**: heuristics (role, label, similarity). Allowed for *reading*, never
   sufficient alone for an *action*.

This ordering decides the fidelity tiers in D1.

## 4. Decisions

Each decision: the question, the choice, why, and what it costs.

### D1. Fidelity is tiered, and each tier has its own proof obligation

| Tier | Contents | Evidence required | Proof |
| --- | --- | --- | --- |
| T0 Content | text, headings, landmarks, media alt, structured data | DOM + ARIA + computed style (hidden/inert applied) | text and a11y-tree equality with the rendered page |
| T1 Navigation | links, routes, canonical URLs, pagination | `href`, router tables, sitemap | every reachable route fetches and renders T0 |
| T2 Network actions | forms and handlers that call an endpoint | observed request/response pair, or declared schema | replay returns the recorded response class; mutation controls fail |
| T3 Local state | tabs, accordions, menus, filters (no network) | handler analysis plus DOM after interaction | state machine reproduces captured DOM states |
| T4 Opaque | canvas, WebGL, drag, video | none | listed as `unsupported` with `reason` |

Rationale: the cost and risk of a claim rise sharply per tier. A surface that is
excellent at T0/T1 is already useful and safe; T2 needs authorization (D2) and
side-effect typing (D5); T3 exists mostly so an agent need not click menus to reach
content that T0 already holds, so most T3 modeling can be skipped by exposing the
*union* of reachable content instead of the state machine.

Cost: no single "supports site X" bit. Reports state tier coverage per route.

For the primary target, T2 is the centre of gravity and its unit is a **flow**, not a
single action (D11).

### D2. Authorization is a first-class input, not a runtime accident

The surface is produced and executed in exactly one of three **modes**, recorded in
the manifest and enforced by the executor:

| Mode | Who it is for | Allowed |
| --- | --- | --- |
| `owner` | site you operate | T0-T3 and T2 writes with service credentials |
| `delegated` | user acting for themselves on a site they use | T0-T3; T2 under the user's own session or OAuth grant, scoped, never stored in the manifest |
| `read-only` | third-party site without explicit permission | T0/T1 only, subject to robots.txt, ToS check and rate limits; no T2 writes, no replay of private endpoints |

Mode is chosen by the operator, not inferred. The executor refuses an action whose
tier or side-effect class exceeds the mode. Rationale: replaying an endpoint is the
step that turns reading into acting for someone, and its legitimacy depends on who is
asking, which a tool cannot determine by inspecting a page.

### D3. Snapshot gives shape, execution is live

Capture happens once and yields **schema**: routes, content skeletons, action
signatures, evidence. Execution happens at call time and returns **current data**
through the same schema. A snapshot-only mode exists (static sites, offline use) and
is labelled with `captured_at` so staleness is visible to the agent.

Drift detection is mandatory: a scheduled re-capture diffs the schema against the
last, classifies changes (cosmetic, content, action signature), and marks affected
actions `stale` until re-verified. An action whose signature changed is disabled, not
retried with the old shape.

### D4. One canonical model, several projections

The canonical artifact is a versioned JSON document (`agent-surface.json`),
content-addressed by hash and pinned to the capture it came from. Its schema is
defined once, in Zod (section 6, *Schema*). Everything else is generated from it:

- per-route **Markdown projection** (for reading; `llms.txt`-style, with stable node ids);
- **MCP server** exposing one tool per supported action, plus `read_route` and
  `search`;
- optional **OpenAPI** for T2 actions that are really HTTP calls.

Why not just serve the accessibility tree: it has no action typing, no side-effect
class, no provenance, and no way to separate page text from structure. It is an input
to T0, not the product.

Node ids are stable across captures when the underlying element is the same (derived
from role, accessible name, structural position and route), so an agent's references
survive a re-capture; the mapping is tested (section 6).

### D5. Actions are typed, classed and guarded

An action is more than a button. It carries:

```
id            stable identifier
kind          navigate | submit | fetch | state | download
inputs        typed fields with constraints (from HTML validation, schema or handler analysis)
effect        read | write-reversible | write-irreversible | financial | account-control
preconditions authenticated?, required prior state, CSRF/token source
postconditions expected response class, DOM or data change used to verify
idempotent    bool, and the key if it takes one
evidence      list of sources (declared / observed / derived) with hashes
confidence    only for read-only inference; actions require evidence >= observed
```

Rules the executor enforces:

- `write-irreversible`, `financial` and `account-control` actions require an explicit
  per-call confirmation token from the caller; the surface never auto-confirms.
- A non-idempotent action is never retried automatically.
- Preconditions are checked before the request; a missing CSRF token is fetched the
  way the site's own client does (a legitimate step), not forged.

### D6. Execution engine: smallest runtime that is correct

In order, and the executor records which it used:

1. **HTTP replay** for server-rendered content and declared/observed endpoints.
2. **JS runtime + DOM shim** only when the content or a token only exists after
   client execution (CSR, hydration data). No layout, no paint, network allowlisted to
   the site's own hosts, timeouts and memory limits, deterministic clock.
3. **Browser fallback** is outside this system. If the action requires it, the surface
   says `requires_browser` and stops.

`mochi lift` is the right tool for step 2 when only pure helper logic is needed: its
reviewed, hash-pinned specs and differential checker already refuse unsound cases.
It is not a general proof and must not be presented as one.

### D7. Credentials and secrets stay out of the artifact

The manifest names an auth *requirement* (`session-cookie`, `oauth:scope=...`,
`api-key header`) and never a value. Credentials are supplied to the executor at run
time from the operator's store and are redacted from logs, receipts and error
messages. Session reuse applies only to the user's own machine and account (mode
`delegated`). Bot-protection challenges are reported as `blocked: challenge` and are
not solved or evaded.

### D8. Prompt-injection and untrusted content are designed in

Page text is the main attack channel against an agent that reads it.

- Every string originating from the page is emitted in a field marked
  `origin: "page"`; every string we author (tool names, descriptions, parameter docs)
  is `origin: "surface"`. Tool descriptions never interpolate page text.
- Projections wrap page text in a clearly delimited, escaped block with a stated
  rule: nothing inside is an instruction.
- Hidden or inert text (`display:none`, `aria-hidden`, off-screen, zero-opacity) is
  excluded from T0 by default, with an explicit opt-in, because it is the usual
  carrier for hidden instructions and was never shown to a human.
- Tool results returned by actions pass through the same marking.
- Rate limits, a per-host concurrency cap, `robots.txt` and `Retry-After` handling are
  in the executor, not left to callers.

### D9. The browser is the oracle, not the runtime

Correctness is defined by agreement with a real browser on the same site, measured by
the existing parity machinery (`check-parity`: collector, bidirectional matcher,
capture-derived inventory, mutation calibration):

- T0: surface text and a11y tree vs the browser's, per route and viewport.
- T1: every listed route resolves; every link in the browser's DOM is in the surface
  or listed as excluded with reason.
- T2: replayed action produces the recorded response class; a deliberately broken
  input (mutation control) must be rejected. A check that cannot fail does not count.
- T3: scripted interaction in the browser and the surface's state transition land on
  the same DOM text.

This reuses the ruler we already trust, and keeps the claim honest: the surface is
"as good as the parity run says", reported per tier.

### D10. Failure semantics are explicit

Every operation returns one of: `ok`, `stale`, `unsupported(reason)`,
`requires_auth`, `requires_confirmation`, `requires_human`, `requires_browser`,
`blocked(reason)`, `error(class)`. No silent fallbacks. "Unsupported" is a normal, reported outcome and
is counted in coverage, not hidden.

### D11. Transactional flows (checkout, payments) are modeled as flows, and payments are the hard limit

Using a checkout is not one action; it is an ordered sequence with server-side state.

**Flow graph.** Steps, the requests each performs, and the **data dependencies**
between them: a value from a response, the DOM, a cookie or storage that reappears in
a later request's header, body or URL (CSRF token, cart id, shipping-rate id, payment
intent, idempotency key). Dependencies are recovered from a recorded, scripted run by
matching values across requests (exact, then structured: JSON path, form field, URL
segment, header). Every request value then gets a class:

| Class | Meaning | Executor behaviour |
| --- | --- | --- |
| `constant` | same in every run | embedded |
| `input` | varies with caller intent (quantity, address) | typed parameter |
| `derived` | taken from an earlier step | wired automatically |
| `client-computed` | no source in the recording: signature, timestamp, nonce, fingerprint | run the producing function in the JS runtime (`lift`) or the step is `requires_browser` |

A value that cannot be classified blocks the flow; it is never guessed.

**Recording is a controlled act.** Flows are recorded on a site you own or operate,
in staging or the payment provider's test mode, with test instruments and throwaway
accounts. A browser is used to record and verify, not to execute. Recording against a
live production checkout with real instruments is out of scope: capture would itself
place orders.

**Payments.**
- Card data never passes through the surface or its logs. Payment fields on most
  sites are cross-origin hosted fields that tokenize inside the payment provider's own
  script; reproducing that is out of scope and usually prohibited by provider terms.
- The payment step instead requires one of: (a) a credential the caller supplies that
  is already tokenized or delegated; (b) a hosted checkout session the human completes
  or approves; (c) a protocol the merchant supports for agent-initiated purchases
  (Agentic Commerce Protocol, AP2 mandates, card-network agent tokens; confirm the
  current specs before binding to any).
- In `owner` mode the clean design is often not to replay the web flow at all: expose
  the merchant's own server operations (create cart, set shipping, create and confirm a
  payment intent with its secret key) as the typed actions, with the recorded web flow
  as the evidence of what they are and how they must be called.
- Fraud scoring, 3-D Secure / strong customer authentication and device checks exist
  to require a human or a real device. They are reported as `requires_human` or
  `requires_browser` at the step that raised them and never emulated.

**Authority over money.** The unit of authority is a **mandate**: scoped, expiring,
and tied to a merchant allowlist, a spend cap and a currency; a session alone is not
enough. Each call that moves money requires explicit confirmation showing item,
total, currency and payee; carries an idempotency key; is never retried automatically;
and writes a receipt. Cancel/refund paths are modeled when the site has them.

**Verification.** Replay against staging in test mode, then check final state (the
order exists, the amount equals the quote). Mutation controls must fail: tampered
total, stale cart id, replayed idempotency key, expired mandate. Negative paths
(declined card, invalid address, out of stock) must reproduce their recorded error
shapes.

**Honest limit.** For a third-party merchant with no cooperative protocol, browser-free
checkout is not generally achievable legitimately. It is achievable for sites you own
and for merchants that support an agent protocol or hosted session, and that is the
scope this design commits to.

## 5. Where it sits in `mochi`

The agent backend must not hang off `semantic::ir`: that IR is shaped for emitting
React/Base UI/TanStack source and carries framework-specific facts. Both backends
should consume a smaller framework-neutral layer beneath it, so they agree on what the
page contains and neither constrains the other.

```
capture ──► page model (DOM, ARIA, computed style, network, cookies/storage,
 (Rust)     listeners, interaction traces)  ── emitted as JSON files ──┐
              ├─► semantic::{ir,emit}   (Rust; TanStack Start source)   │
              └─────────────────────────────────────────────────────────┘
                                         │ files: page model, HAR, traces
                                         ▼
                      agent-* packages (TypeScript, Bun): framework-neutral
```

**Language split (decided).** Rust owns capture and the page model and hands off JSON
files (page model, HAR 1.2, interaction traces). Everything that consumes them is
TypeScript: schema, flow inference, policy, executor, verifier, MCP server. Reasons:
the executor needs a JS engine anyway for client-computed values (the same reason
`mochi lift`'s differential checker runs in Bun); one Zod definition replaces a
Zod/serde pair; agent skills and MCP servers are run with `bunx`/`npx`, so no
per-platform binary or Rust toolchain is needed at run time; and the repo's parity
and lift tooling is already Bun. No new crate is published for the agent surface. The
only Rust change is the page-model extraction in `mochi-web`, released through the
existing process.

`semantic::js` (React bundle facts) stays an optional evidence provider.

| Package / module | Responsibility | Reuses |
| --- | --- | --- |
| `agent-schema/` (TS) | Zod schema, generated JSON Schema, node-id derivation, tiering | page model (roles, names, state, inert/hidden; HTML-AAM and accessible-name rules shared with `semantic::ir`, not forked) |
| `agent-flows/` (TS) | flow recording format, cross-request dependency inference, value classing, action graph | HAR and interaction traces; optional `semantic::js` facts; `lift` for client-computed values |
| `agent-policy/` (TS) | modes, effects, mandates, confirmations, redaction, egress, injection hygiene | none |
| `agent-mcp/` (TS) | `agent-surface.json`, Markdown projections, MCP server, OpenAPI | `agent-schema`, `agent-policy` |
| `agent-exec/` (TS) | HTTP replay, JS runtime, receipts | `lift` differential runner patterns, `agent-policy` |
| `agent-verify/` (TS) | tier-by-tier checks against a browser | `check-parity` |
| CLI | `mochi-agent <capture> --out … [--mode owner|delegated|read-only]` (npm package, `bunx`); `mochi` stays the capture front end | |

No change to `export` or `semantic` output is required. The shared page model needs
three additions that benefit both backends: a node-id scheme that is stable across
captures, interaction traces (event to request/DOM-change association), and cookie and
storage snapshots tied to requests. Extracting that layer from what `semantic::ir`
already does must not change `semantic` output; its tests guard that.

## 6. What "engineered properly" requires

### Correctness
- **Schema.** One source of truth: Zod schemas in a small TypeScript package
  (`agent-schema/`), used by the executor, MCP server and verifier (all Bun). Zod
  implements the Standard Schema interface, so consumers can validate with their own
  library without depending on ours. A JSON Schema is generated from the Zod types and
  committed, as the language-neutral contract for non-TS consumers; the schema is
  restricted to constructs that convert cleanly (no transforms or refinements in the
  wire format; those run in code after parsing). There is no second definition: every
  producer of surface data is TypeScript and validates against the Zod schema. The
  Rust side emits only the page model; its JSON is validated by a Zod page-model schema
  in `bun test` over every fixture capture. The MCP server refuses a surface that does
  not validate.
  Alternative considered: schema in Rust (`schemars`) and generate TS from it. Rejected
  because the executor and verifier live in TS, and runtime validation of untrusted
  input is where the schema earns its keep.
- Deterministic output: same capture and same flags produce byte-identical files
  (the semantic pipeline is already specified as deterministic; this gate must be
  tested here too, not assumed).
- Every action lists its evidence; an action with only inferred evidence cannot be
  `kind != navigate`.
- Node-id stability tested: re-capture an unchanged page, ids are equal; change one
  element, only that id changes.

### Testing (all must be able to fail)
- **Golden fixtures** for small synthetic sites covering each tier and each failure
  class, checked into `benchmarks/fixtures/agent/`.
- **Differential tests** against a browser per D9, on the three captures
  (Natural, Garden, Microsoft AI) as the standing corpus.
- **Mutation controls**: perturb a captured request, a form constraint, a hidden
  instruction; the verifier must flag each.
- **Property tests**: for any DOM, hidden/inert nodes never appear in T0; page text
  never appears in a `surface`-origin field.
- **Adversarial corpus** for D8: pages carrying hidden instructions, tool-description
  injection, homoglyph labels, zero-width characters, oversized text. The surface must
  neutralize each.
- **Drift test**: apply a scripted change to a fixture site; stale marking and action
  disabling must trigger.

### Security
- Executor network allowlist and egress logging; no redirects to other hosts without
  re-check; SSRF guard for private ranges.
- Secrets never written to disk by the tool, never in receipts; tested by scanning
  outputs for seeded canary secrets.
- Resource limits on the JS runtime (time, memory, output size), run in a separate
  process.
- Effect-class enforcement covered by tests that attempt each class in each mode.

### Operability
- Versioned schema with an explicit compatibility policy (additive within a major).
- Receipts: each executed action records mode, tier, evidence ids, request class,
  result, and timing, with secrets redacted.
- Coverage report per site: counts per tier, per failure class, and `unsupported`
  reasons, so progress is measurable and regressions are visible.
- Clear refusal messages that name the missing authorization or evidence.

### Legal and ethical gates (before any third-party use)
- Mode must be chosen explicitly; default is `read-only`.
- `robots.txt`, `llms.txt` and ToS check results recorded in the manifest.
- No distribution of a surface derived from a third-party site that embeds its
  private endpoint details; read-only surfaces carry T0/T1 only.
- Release follows the repo's existing publication gate; no publication until
  qualification passes.

## 7. Milestones and exit criteria

| M | Scope | Exit criteria |
| --- | --- | --- |
| M0 | Zod schema package, generated JSON Schema, node-id scheme, flow-recording format, **fixture shops in three stacks** (React, server-rendered/HTMX-style, Vue or Svelte) with a test-mode payment stub | schema validates fixtures; id-stability test passes; deterministic emit; same flow graph from all three stacks is the framework-neutrality test, fixed before any extraction code |
| M1 | Shared page model extraction (no `semantic` output change), T0/T1 emit, hidden/inert exclusion, D8 marking | `semantic` golden outputs unchanged; parity T0/T1 on the 3 captures within stated thresholds; adversarial corpus neutralized |
| M2 | Flow recorder and dependency inference on the fixture shops (cart through checkout) | every request value classed; unclassified values block rather than guess; matches hand-labelled ground truth on all three stacks |
| M3 | HTTP-replay executor, effect typing, idempotency, `owner` mode | replay and final-state checks pass on fixtures; mutation controls (tampered total, stale cart id, replayed key) fail as required |
| M4 | Payment steps: caller-supplied token, hosted session, one protocol adapter; mandates, confirmations, spend caps; `delegated` mode and secrets handling | canary-secret scan clean; money-moving calls refuse without confirmation or inside an expired/over-cap mandate; 3DS and fraud challenges surface as `requires_human` |
| M5 | JS-runtime executor for client-computed values and CSR sites; drift detection | a fixture whose checkout needs a client-computed signature works without a browser; scripted drift disables exactly the affected steps |
| M6 | MCP server, per-tier coverage report across the 3 captures plus fixtures, docs | coverage and `unsupported` reasons published; limits documented |

Files are disjoint from the semantic generator (`docs/AGENT_SURFACE.md`,
`docs/agent-surface/**`, `benchmarks/{src,tests,fixtures}/agent/**`, and the new
`agent-schema/`, `agent-flows/`, `agent-policy/`, `agent-mcp/`, `agent-exec/` and
`agent-verify/` TypeScript packages). M1's page-model extraction is the only Rust
change: it touches `semantic::ir`, belongs with the IR owner, and must land with no
change to `semantic` output.

## 8. Risks and open questions

- **Client-computed request values.** The most likely reason a flow cannot run without
  a browser: signatures, device fingerprints, anti-bot tokens and payment-provider
  tokenization are produced by obfuscated client code. Some are liftable
  (deterministic, pure); many are deliberately not. Expect `requires_browser` on a
  real share of production checkouts, and measure that share instead of assuming it.
- **Coverage depends on the recording.** Steps never exercised in the scripted run have
  no observed requests, and static JS analysis (React-only today) is the only way to
  find them. Recording therefore needs scripted scenarios (success, validation error,
  out of stock, declined card), authored per site or generated from the form and
  ARIA model, and it must run only against owned or staging sites.
- **State the server keeps.** Carts, sessions, inventory and pricing change between
  recording and execution; a flow valid yesterday can be wrong today. Every step needs
  postconditions checked at run time, and drift detection (D3) must cover flows, not
  just routes.
- **Side effects during capture.** Recording a checkout places orders. Capture of
  actions must be limited to owned or authorized sites in staging/test mode;
  read-only capture must not submit forms.
- **Agent-commerce protocols are young.** Their shapes will change; bind to them
  through a thin adapter and verify current specs at implementation time.
- **Stable ids under redesigns.** Any id scheme loses identity when a site restructures;
  the policy is to fail visibly (id changes, action marked stale), not to remap.
- **What counts as "site-declared".** Whether to trust `llms.txt`/WebMCP content as
  authoritative or still treat the text inside it as page-origin.
- **Cost of the JS runtime.** If many sites need it, the "no browser" claim narrows to
  "no layout engine"; report it as such.
