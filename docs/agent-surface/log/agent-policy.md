# agent-policy log

Owner: the policy-library session. Files: `agent-policy/**` and this log. Design source:
`docs/AGENT_SURFACE.md` (D2, D5, D7, D8, D10, D11, section 6 Security).

Status: library implemented and tested. 283 tests, strict `tsc --noEmit` clean, 124 of 124
one-edit mutants killed. Not yet migrated to `agent-schema` (its first PR is unmerged; see
"Interface notes").

## 1. Threat model

The library is the last check before a request leaves the executor. It assumes the
*agent* (the model, and anything it was talked into) is adversarial or confused, and
that *page content* is adversarial. It assumes the operator's process, clock, key store
and the human who approves a confirmation are honest.

### Assets

1. Money: a spend the owner did not authorize, or more than they authorized.
2. Irreversible or account-level changes (delete, change email/password, close account).
3. Credentials and instruments: sessions, API keys, tokens, card numbers.
4. Internal network reachability of the executor host (cloud metadata, intranet).
5. Third-party sites' terms: robots.txt, rate limits, `Retry-After`.
6. The agent's own instruction channel (D8).

### Adversaries and what they try

| # | Adversary | Attempt | Control (section) |
| --- | --- | --- | --- |
| A1 | Page author | Hidden or invisible text telling the agent to act | 7: hidden-text exclusion, invisible-character stripping, wrapping |
| A2 | Page author | Forged delimiters, fake system turns, markdown/HTML exfil | 7: escape, per-line prefix, hash-bearing markers |
| A3 | Page author | Text in labels/placeholders that ends up in a tool description | 7: closed-vocabulary `describeTool`, separate marked `pageLabels` |
| A4 | Confused or injected agent | Calls a write or money action in `read-only` mode | 2.1: mode x tier x effect, default deny |
| A5 | Confused or injected agent | Calls a money action with no human in the loop | 2.2 and 2.3: mandate and confirmation both required |
| A6 | Confused or injected agent | Re-uses an old confirmation for a bigger amount, another payee, a second call | 2.3: field-bound, single-use, short-lived |
| A7 | Confused or injected agent | Spends past the cap by splitting calls or racing them | 2.2: atomic cumulative ledger |
| A8 | Confused or injected agent | Retries a failed charge hoping it goes through | 2.4: no automatic retry for non-idempotent or money/irreversible effects; unknown outcome blocks re-execution |
| A9 | Confused agent / page | Forges a "verified" mandate or confirmation object | 2.2: branded via module-private `WeakSet`; only verification creates one |
| A10 | Network attacker or page | Redirects, DNS rebinding, IP spelling tricks into metadata or intranet | 6: allowlist, SSRF classification, pinned addresses, redirect re-check |
| A11 | Compromised executor *process* (not agent) | Mint a mandate | 2.2: Ed25519, executor holds the public key only |
| A12 | Anyone reading output | Harvest secrets from receipts, errors, logs | 5: allowlisted receipt fields + redactor + canary tests |
| A13 | Hostile server | Huge/garbled `Retry-After`, robots.txt designed to stall matching | 6: strict parsers, caps, linear-time matcher |
| A14 | Hostile server | Slow-leak via pathological inputs (1 MB text, very long tokens) | 7: truncation and flags |

### Explicitly out of scope (stated so nobody assumes otherwise)

- **A malicious in-process caller.** JavaScript has no privilege boundary inside one
  process. Anything that can import this package can call `verifyMandate` with its own
  verifier. The boundary is *key custody*: the agent must not hold a signing key, and
  must not be able to construct the executor's `AuthorizeDeps`. Run the policy in the
  executor, not in the agent's process.
- **Hardware constant time.** `constantTimeEqual` removes the data-dependent branch; it
  cannot promise that the JS engine's timing is flat. The only secret comparisons are MAC
  verification (done by WebCrypto) and token digests; neither is compared with `===`.
- **Enforcing DNS pinning.** The library returns the validated addresses
  (`EgressDecision.pinned`). The HTTP client must connect to exactly those. A second
  lookup reopens the rebinding window. This is an executor obligation.
- **Clock honesty.** Every function takes `now` from the caller. A caller that lies about
  the time defeats expiry. The executor owns the clock.
- **Deciding that a confirmation was shown correctly.** The library proves a token was
  issued for exactly this action/item/total/currency/payee. Whether the human saw those
  values is the confirmation UI's job.
- **Unicode confusables completeness.** The confusable table is a small, deliberate
  subset (Cyrillic/Greek look-alikes of Latin letters). It flags, it does not prove
  absence.

## 2. Decisions

### 2.1 Mode x tier x effect (`policy.ts`)

`modeCell(mode, tier, effect)` is the whole matrix, default deny:

| | T0/T1 | T2 | T3 | T4 |
| --- | --- | --- | --- | --- |
| `read-only` | read | refused | refused | refused |
| `owner`, `delegated` | read | any effect | read (local state, no network) | refused (`unsupported`) |

- A write effect on T0/T1/T3 is refused as incoherent (only T2 carries side effects).
- Evidence: T2/T3 need `declared` or `observed`; `derived` or `inferred` alone is refused
  (D5, section 3). Reads at T0/T1 may rest on any evidence.
- `read-only` additionally needs a positive robots result *and* a recorded ToS check
  (D2, legal gates). The library takes these as evaluated inputs; it does not fetch.
- Credentials: any non-read, or an authenticated read, needs `service` (owner) or
  `user-session`/`oauth` (delegated), scoped to the effect. A mismatched kind is refused.
- Confirmation is required for `write-irreversible`, `financial`, `account-control`
  (D5). `financial` additionally needs a verified mandate and an idempotency key.
- Every refusal has a stable `code`, a D10 `status`, `missing` (what to supply) and a
  `message`. Messages never echo caller strings, only enumerated values, so a refusal
  cannot carry a secret or injected text (tested with canaries).
- Unknown enum values, malformed ids, non-finite `now` are refused (`invalid-request`).

`authorize()` is the executor entry point. Order: static checks, token verification,
proof binding, and only then side effects (reserve spend, then consume the confirmation;
if consuming fails the reservation is released). A refusal before the side effects
changes nothing.

### 2.2 Mandates (`mandate.ts`, `sign.ts`)

A mandate: id, issuer, subject, exact-host merchant allowlist (no wildcards), exact
action ids (no wildcards), one ISO currency, cumulative cap, optional per-call cap,
`notBefore`, `expiresAt`. Amounts are positive safe integers in minor units; floats,
NaN, negatives and unsafe integers are rejected. Unknown or missing payload keys are
rejected (fail closed).

**Signature choice: Ed25519 (WebCrypto), HMAC-SHA256 supported.**

- Mandates are issued by the *user's* side and verified by the *executor*. With HMAC the
  verifier holds the signing key, so a compromised executor, or anything that can read
  its key, can mint authority over money. With Ed25519 the executor holds only the
  public key (`ed25519Verifier(rawPublicKey)` returns an object with no `sign`).
- Ed25519 verification compares no secret, so there is no MAC-compare timing to get
  wrong; WebCrypto does the verification.
- Ed25519 is in Bun, Node 20+, and current browsers. A malformed or wrong-length
  signature is `false`, never a throw.
- HMAC is kept for confirmation tokens, where issuer and verifier are the same trusted
  side. It is used through `subtle.verify`, which compares the tag in constant time.
  HMAC mandates also work (tested) for single-party deployments.
- The algorithm is fixed by the *verifier's key*, never read from the token, so there is
  no algorithm-confusion path. Signatures cover `domain + "\n" + payloadSegment` with
  distinct domains for mandates and confirmations, so one cannot be replayed as the
  other even under the same key (tested).
- Token = `base64url(canonical JSON) "." base64url(signature)`. Decoding is strict
  (canonical base64url only), size-capped (8 KiB), and the payload is parsed only after
  the signature verifies.

Checks: bad signature, malformed, not yet valid, expired (the expiry instant itself is
expired), lifetime over the maximum (default 7 days), revoked, action not in scope,
merchant not on the allowlist (exact match after lowercase and trailing-dot removal; no
suffix or substring match), currency mismatch, amount invalid, per-call cap, cap.

**Cumulative spend.** `SpendLedger.reserve(mandateId, ref, amount, cap)` is atomic: the
check and the write happen with no `await` between them in the in-memory implementation,
and the interface requires atomicity of any real store. Ten concurrent 3000-unit calls
against a 10000 cap admit exactly three. A failed call releases its reservation; an
ambiguous outcome is *not* released (over-counting is the safe direction). A duplicate
reservation `ref` is refused.

**Forgery.** `VerifiedMandate`/`VerifiedConfirmation` are frozen objects registered in a
module-private `WeakSet`. A hand-built object, or a spread copy of a real one, is not
accepted by `decide`/`checkMandateScope` (tested). This stops accidents and a confused
caller; it is not a defence against code in the same process (see out of scope).

### 2.3 Confirmation tokens (`confirm.ts`)

Bound to `actionId`, `item`, `totalMinor`, `currency`, `payee`. Fields that do not apply
are explicit `null` and must be `null` on both sides, and the three money fields travel
together, so a no-money token cannot be widened to authorize money and vice versa.
Any one-minor-unit change in the total is a mismatch (`confirmation-binding-mismatch`,
naming the field). Short TTL (default maximum 15 minutes), `iat`/`exp` checked at both
verification and again in `decide` against the call about to run. Single use: `jti` is
consumed in an atomic store after every other check has passed; a replay is refused and
releases any spend reserved for it. A mismatched or expired token is *not* consumed, so
the honest call can still use it.

### 2.4 Idempotency and retries (`idempotency.ts`)

`IdempotencyStore` interface + in-memory implementation. A key is bound to a request
fingerprint (SHA-256 of canonical JSON of action id + args): same key with a different
request is a `conflict` and never runs. States: `new`, `in-flight`, `unknown-outcome`,
`completed` (returns the stored result, never re-executes).

`shouldRetry` is pure. It is false when the action is not idempotent, and *also* for
`write-irreversible`, `financial` and `account-control` even if declared idempotent
(D5 and D11: money calls are never retried automatically). Only idempotent `read` and
`write-reversible` retry, only if the failure is marked retryable, up to `maxAttempts`
(default 1: no retry unless asked). An attempt that throws, or fails after the request may
have been sent, leaves `unknown-outcome` and blocks re-execution until the key expires or
a human reconciles. Only a failure marked `neverSent` releases the key.

### 2.5 Receipts and redaction (`receipt.ts`, `redact.ts`)

Receipts are built field by field from an allowlist (mode, tier, effect, action id,
evidence ids, request class, engine, host, result, timing, mandate and confirmation ids,
money summary, redacted error). Headers, bodies, cookies and query strings have no field
to land in. The idempotency key is stored as a 16-hex digest. Every string then passes
the `Redactor`.

`Redactor` holds run-time secrets and removes each in raw, percent-encoded,
JSON-escaped, hex, base64 (all three alignments inside a larger string) and base64url
form, plus unregistered shapes: bearer/basic tokens, JWTs, `sk_live_`-style keys, AWS
and GitHub token shapes, PEM private keys, `key=value` and `"key":"value"` for sensitive
names, and Luhn-valid card numbers. Object keys that look sensitive are blanked
regardless of value. Cycle-safe. Registering a secret under 4 characters throws (a
one-character "secret" would shred every string; failing loudly beats failing open).
The logger redacts messages, fields and errors and swallows sink failures without
surfacing the input.

Canary tests seed secrets into every string field of a receipt, into an error's message,
into log fields, into every mandate/confirmation failure path, and scan the JSON output.

### 2.6 Egress and rate (`egress.ts`, `ip.ts`, `rate.ts`, `robots.ts`)

All pure; the caller does I/O and passes results in.

- `compileEgressPolicy` throws on a malformed allowlist (bare `*`, `*.com`, scheme or
  path in an entry): a typo must not become allow-all. Wildcards are subdomain-only.
- `evaluateEgress(url, policy, resolved)`: https only (opt-in http), no userinfo
  (including password-only), port allowlist, host lowercased and dot-stripped, internal
  names and single-label hosts refused even if allowlisted, IP literals refused by
  default. The WHATWG URL parser normalizes decimal/hex/octal/short/fullwidth IPv4
  spellings before the check (tested with `2130706433`, `0x7f000001`, `017700000001`,
  `127.1`, `127。0。0。1`).
- DNS rebinding: the name's resolved addresses are an *input*. Missing, empty,
  unparsable, or *any* non-public address in the set refuses the request. The decision
  returns `pinned` addresses to connect to. No second lookup.
- IPv4 deny list (RFC 1918, CGNAT, loopback, link-local/metadata, benchmarking,
  documentation, multicast, reserved, broadcast, Azure wireserver). IPv6 is an
  *allowlist*: only 2000::/3 minus 2001::/23 (Teredo, protocol assignments),
  2001:db8::/32, 2002::/16 (6to4), 3fff::/20. Mapped (`::ffff:a.b.c.d` in either
  notation), compatible and NAT64 forms inherit the embedded IPv4 verdict; ULA, link-local,
  multicast, loopback and unspecified are outside 2000::/3. Zone ids are refused.
  Property tests compare IPv4 classification to an independently written reference
  (including boundary-biased inputs) and check the mapped/NAT64 inheritance.
- `evaluateRedirect` re-runs everything on the target; refuses scheme downgrade, hop
  overflow, and any cross-origin redirect of a non-GET/HEAD request; asks the caller to
  strip credentials on cross-origin hops.
- Rate: per-host concurrency cap, minimum spacing, `Retry-After` (delta-seconds or
  IMF-fixdate only; anything else is "no value" and falls back to exponential backoff;
  a later header never shortens an earlier floor; a value over the cap is reported as
  `tooLong` so the caller returns `blocked` rather than sleeping), `Crawl-delay` raises
  the spacing floor.
- robots.txt per RFC 9309: group selection by product token (a specific group replaces
  `*`, no merging), longest-pattern match, Allow wins ties, `*` and `$`, percent-encoding
  normalized, 500 KiB cap. The matcher is iterative (no regex): a 1000-star pattern runs
  in bounded time. Fetch outcomes: 2xx parse; 404/410 allow; 5xx, redirect/other and
  unreachable disallow (RFC 9309); **401/403 also disallow, stricter than the RFC**
  (which allows 4xx), because a site refusing the robots file is not inviting automated
  access. Say so to site owners if it ever matters.

### 2.7 Prompt-injection hygiene (`sanitize.ts`, `wrap.ts`, `hidden.ts`, `tooldesc.ts`)

- `sanitizePageText` strips zero-width and format characters (including soft hyphen,
  word joiner, BOM, ZWJ), bidi controls, Unicode **tag characters** and variation
  selectors (both are used to smuggle text invisibly), C0/C1 controls, private-use and
  unassigned code points; converts all line-break variants to `\n`; applies NFKC; then
  flags (and optionally folds) mixed-script tokens and all-confusable tokens; caps length.
  It returns flags with counts, never positions or page text. Tag-smuggled text is
  decoded into `quarantined` for audit only and must never reach a surface field.
  Known cost: stripping ZWJ breaks emoji ZWJ sequences into their parts. Safety over
  fidelity; revisit if it matters.
- `wrapPageText` output: `[[PAGE-CONTENT ... ref= sha= flags=]]`, a fixed rule line,
  then every content line prefixed `| `, then a closing marker with the same hash.
  Content is escaped (`& < > [ ] backtick`) so no markup, markdown link/image or fence
  survives. The prefix is the structural guarantee: no content line can equal a marker
  line, whatever the content (property-tested over hostile strings including forged
  markers). The hash is a tamper check, not a security boundary. `unwrapPageText` is
  strict and used by the tests to prove a round trip. `ref` must be a surface-generated
  id.
- `collectText` implements the hidden-text rules over a plain node description carrying
  computed style: excluded tags, `display:none`, `visibility` (with descendant opt-in),
  `hidden`, `aria-hidden`, `inert`, `opacity` <= 0.01, font < 2px, contrast < 1.1 or
  transparent colour, off-screen positioning or `text-indent`, `clip`/`clip-path`,
  zero-size overflow-hidden, `scale(0)`, `content-visibility:hidden`, closed `<details>`.
  A hidden box hides its subtree; computed font size, colour and visibility are judged
  per node. Excluded segments record reasons and length only, never the text. Opt-in
  `includeHidden` returns it tagged with reasons.
- `describeTool` builds names and descriptions from closed vocabularies and ids of the
  form `act_<hex>`; any field outside the vocabulary throws a `TypeError` that does not
  echo the input; vocabulary membership uses `Object.hasOwn` (so `__proto__`,
  `constructor` are not vocabulary). Page labels travel in a separate `pageLabels` field
  marked `origin: "page"`. Property test: for arbitrary page labels on valid specs no
  page string reaches name, description or parameters; for arbitrary strings in any
  surface field the call throws and the error does not contain them.
- Corpus: `fixtures/injection/corpus.json`, 35 items generated by
  `scripts/gen-corpus.ts` (invisible characters are `\uXXXX` escapes in the file). 16
  hidden-DOM cases, 18 text cases (zero-width, bidi, tag and variation-selector
  smuggling, controls, three homoglyph kinds, fullwidth/ligature, forged open/close
  markers, line-separator breakout, role-prefix lines, fence/tool-call breakout, HTML,
  markdown exfil image, 1M characters, 5000-char token), and a tool-description
  injection case. Each has machine-checked expectations; the same invariants are applied
  to every item and to random hostile strings.

## 3. Test inventory

`bun test` (283 tests, ~1 s):

| File | What it pins |
| --- | --- |
| `policy.table.test.ts` | all 75 mode x tier x effect cells, end to end through `authorize` with every proof supplied, against a hand-written table; the 15-cell mode x effect table; what each effect demands when proofs are missing; default deny; evidence; credentials; `neverRetry` |
| `mandate.test.ts` | signature, expiry/notBefore boundaries, over-cap, per-call cap, wrong merchant (suffix and substring tricks), currency, action scope, lifetime, revocation, unknown keys, forged brand, cumulative cap, concurrent reserve, release/ambiguous, domain separation, public-key verifier cannot sign |
| `confirm.test.ts` | each bound field mismatch, one-unit total change, null bindings, expiry, TTL, foreign key, single use incl. concurrency and replay-does-not-spend, non-consumption on mismatch, re-check in `decide` |
| `idempotency.test.ts` | retry rules for every effect, conflict, in-flight, unknown outcome, never-sent release, expiry, fingerprint stability |
| `redact.test.ts` | canary scan over receipts, errors, logs, refusals, authorization results; every encoding and embedding |
| `egress.test.ts`, `rate.test.ts`, `robots.test.ts` | allowlist and wildcard edges, userinfo tricks, IP spellings, IPv6 forms, rebinding sets, redirects; Retry-After; RFC 9309 behaviour |
| `injection.test.ts`, `hidden.test.ts` | corpus; wrapper contract; hidden rules |
| `properties.test.ts` | fast-check (4.5.3, exact-pinned): wrapper structure, sanitizer invariants and idempotence, tool descriptions never interpolate page text, IPv4 vs reference, mapped/NAT64 inheritance, global-unicast-only IPv6, rebinding sets, confirmation binding equality, ledger never exceeds cap, redaction, codecs, robots matching. `FC_SEED=<n>` reproduces a run |
| `mutation-guard.test.ts` | every mutation still applies to exactly one place |

**Mutation tests.** `bun run mutate` applies each of 124 one-edit mutants (flip a
comparison, drop a negation, remove an escape, narrow a CIDR, widen a cap, delete a
check) to a throwaway copy and requires `bun test` to go red; it refuses to run on a red
baseline and reports the first failing test per mutant. Result: **124/124 killed**.
First run killed 120/125: the survivors exposed real gaps, now closed with tests:
`neverRetry` for irreversible/financial/account-control in `authorize` was untested;
a password-only userinfo URL was untested; sensitive-key blanking was masked by the
value patterns; propagation of hidden reasons to descendants was untested. One further
mutant (removing the explicit `%` check in `parseIPv6`) is an equivalent mutant (the group
parser rejects `%` anyway) and was dropped, noted in `scripts/mutations.ts`.

## 4. API sketch

```ts
import { authorize, InMemorySpendLedger, InMemoryConsumedStore, ed25519Verifier, hmacSigner } from "agent-policy";

const r = await authorize(request, { mandate, confirmation }, {
    mandateVerifier, confirmationVerifier, ledger, consumed,
});
if (!r.ok) return { status: r.status, code: r.code, missing: r.missing }; // D10
try { /* send; never retry when r.neverRetry */ await r.settle("committed"); }
catch { /* outcome unknown: do not release */ }
```

Executor obligations (library cannot do them): connect only to `EgressDecision.pinned`;
own the clock; evaluate robots per URL and pass the verdict; use `runWithIdempotency` for
every non-read call; build receipts with a `Redactor` that has every run-time secret
registered; never log before redacting.

## 5. Interface notes for the parent session (agent-schema)

`agent-schema` branch `claude/agent-schema-82d14a` (unmerged) was read, not changed.

Aligned already (identical vocabularies): `Mode`, `Tier`, `Effect`, evidence source order,
`OperationStatus`. My `types.ts` can be replaced by re-exports once merged.

Differences to agree before migration:

1. **Mandate shape.** Schema `Mandate` has `spend_cap_minor`, `per_call_cap_minor`,
   `scope` of `{kind: action|flow, id}`, `issued_at`/`expires_at`, and a content `hash`,
   but **no signature**. The policy library signs the *canonical mandate* in an envelope
   (`token = payload.sig`). Proposal: keep `Mandate` as the payload that is signed and add
   a `SignedMandate = { mandate, signature }` (or keep the compact token) to the schema; I
   will adapt field names (`capMinor` -> `spend_cap_minor`, etc.) and accept `flow` scope.
   Wildcard scopes should stay out of the schema's `merchants`/`scope`.
2. **Confirmation binding.** Schema binds a confirmation to `request_digest` +
   `idempotency_key` (one exact request). Mine binds explicit fields (action, item, total,
   currency, payee), which gives a precise mismatch message but is weaker if two requests
   share those fields. Proposal: bind **both** (fields for the refusal message, digest for
   exactness) and add `request_digest` to my token.
3. **PageText.** Schema: `{origin:"page", value}`; mine: `{origin:"page", text, flags}`.
   I will switch to `value` and carry `flags` beside it.
4. **Blocked reasons.** Schema's `BlockedReason` enum (`robots`, `rate_limit`,
   `host_not_allowed`, `private_address`, `redirect_off_host`, `mode_forbids_tier`,
   `mode_forbids_effect`, `mandate_*`, `confirmation_mismatch`, ...) is the right surface
   for my refusal `code`s. I will map each code to a `BlockedReason` plus keep my finer
   code as `detail`.
5. **IdempotencyKey** pattern in the schema is `[A-Za-z0-9_-]{16,128}`; mine accepts
   printable ASCII 8-128. I will tighten to the schema's.

## 6. Open items

- Migrate to `agent-schema` types after its first PR merges (section 5).
- Real `SpendLedger`, `IdempotencyStore`, `ConsumedStore` for a multi-process executor
  need an atomic backing store; the interfaces state the required atomicity.
- Decide whether `read-only` T1 navigation to *other* hosts needs its own allowlist
  entry (it does today: egress is allowlist-only).
- ToS check result is a caller-supplied boolean-like input; its provenance is recorded
  in the manifest, not verified here.
- Emoji ZWJ sequences are split by sanitization (documented cost).

---

## 7. Entry 2026-11-10: adversarial review round 1 and fixes (663b5e9)

Done: Opus review (high effort) of 2de943e returned 17 findings; the ones below were
reproduced or reasoned through and fixed in 663b5e9, each with a regression test in
`tests/review-fixes.test.ts` (or the existing table/hidden tests) and, where a one-edit
mutant is meaningful, a mutant in `scripts/mutations.ts`.

Verdicts and decisions (each fills a gap in or tightens `docs/AGENT_SURFACE.md`):

| # | Verdict | Fix and decision |
| --- | --- | --- |
| 1 HIGH | confirmed | The mandate allowlist did not constrain `payee`. Decision: for a financial action `payee` must equal `merchant` (`payee-merchant-mismatch`), and the mandate check requires *both* on the allowlist. New helper `hostMatchesMerchant(egressHost, merchant)`: the executor must assert the egress host equals the authorized merchant (D11 "tied to a merchant allowlist"). |
| 2 MED-HIGH | confirmed | Ledger `settle("released")` deleted committed spend. Now only a `reserved` entry can be released or committed; `Authorization.settle` is one-shot. |
| 3 MED | confirmed | D2: read-only has "no replay of private endpoints". Now `requiresAuth` or any credential on a read-only request is refused (`read-only-private-endpoint`). |
| 4 MED | confirmed | An in-flight or unknown idempotency entry never expires into "new"; after its TTL it becomes `unknown-outcome` (needs human reconciliation). Completed entries still expire. |
| 5 MED | confirmed | `redactDeep` now blanks byte arrays/ArrayBuffers wholesale and judges numbers/bigints by their decimal text; added form-encoded (`+`), lowercase `%xx` and `encodeURI` variants. |
| 6 MED | confirmed | `Cookie`, `Set-Cookie`, `Authorization`, `Proxy-Authorization` header values are redacted to end of line (unquoted values only, so JSON structure survives). |
| 7 MED | confirmed | Contrast is judged against the *effective* backdrop (ancestor backgrounds composited, white page by default); text alpha < 0.1 is hidden; `relative`/`sticky` and positive offsets count for off-screen; `clip` is normalized of spaces and commas; `clip-path: inset()` is parsed. |
| 8 | confirmed | `visibleAttributeText` removed; `collectText` returns `attributes` (alt, aria-label) using the same inherited-hidden rules. |
| 9 | confirmed | `PageText` is issued only by `pageText()` (module-private `WeakSet`, deep-frozen); `wrapPageText` and `describeTool` refuse anything else. |
| 10 | confirmed | `paramLabels` keys must be declared parameter ids. |
| 11 | confirmed | `evaluateRobots` resolves the target through the URL parser (dot-segments, `%2e%2e`); patterns are compared UTF-8 percent-encoded. |
| 12 | confirmed | The consumed-token store prunes against the highest clock value seen plus a 24 h retention, so a backwards clock step cannot make a spent token spendable. A clock regression larger than 24 h is out of scope (clock honesty is an executor obligation). |
| 13 | confirmed | All `Default_Ignorable_Code_Point` characters and U+2800 are stripped (FE0E/FE0F only kept directly after an emoji); mixed-script detection covers 15 script groups (CJK counts as one); truncation now happens before the per-token scan (bounded cost on 1M-char input). |
| 14 | confirmed | `write-irreversible` and `account-control` require `item` (`item-missing`). The proposal to also bind a `request_digest` (section 5) stands. |
| 15 | partly | Versioned UA lines, normalized pattern specificity, public-suffix wildcard guard (small list, not exhaustive) and "status must be 301/302/303/307/308" fixed. Not done: a side-effecting GET redirected cross-host is still decided by method only. |
| 16 | partly | `decide` is documented as side-effect-free and not an entry point; digest widened to 128 bits. Not done: mandates are bearer tokens (`subject`/`issuer` are not bound to a credential); noted as a limitation. |
| 17 | confirmed | Table test now asserts the exact refusal code of every denied cell and the evidence refusal code. |

Equivalent mutants (documented, not tested): removing the `%` check in `parseIPv6`;
removing the single-side `>= 100%` test in `insetHidesAll` (the opposite-sides sum covers it).

Commands and results:

```
cd agent-policy && bun test            # 317 pass, 0 fail
bunx tsc --noEmit                      # clean
bun scripts/mutate.ts --jobs 6         # 150/155 first run; 5 survivors triaged:
                                       #   3 real gaps -> tests added (uppercase %xx form,
                                       #   bare Bearer, one-shot settle, alpha 0.05)
                                       #   1 equivalent (inset) -> dropped; all killed after
```

Round 2: a second Opus review of the fix diff (663b5e9) was started; its verdicts are
recorded in the next entry.

Not run / limits: nothing here exercises a real network, DNS, a real browser or a real
payment provider; every input is synthetic. `cargo check` (the repo's declared check) was
not run because no Rust changed.

## 8. Entry 2026-11-10: review rounds 2 and 3 (bc2434e and the round-3 fix commit)

Round 2 (Opus, high; diff 663b5e9) found two HIGH regressions introduced by the round-1
fixes plus several partial fixes. Round 3 (diff bc2434e) found one HIGH (NFKC expansion),
several MEDIUM and LOW. Verdicts and decisions:

| Finding | Verdict | Fix and decision |
| --- | --- | --- |
| R2-1 HIGH: forward clock jump flushed the consumed-token table | confirmed (regression of round 1) | Retention runs on the store's own monotonic clock, never the caller's `now`. R3-6: retention also follows the token's remaining validity (24 h after expiry, capped at 7 d). Limits: per-process state; a multi-process executor needs a shared atomic store (interface says so); restart forgets consumed tokens. |
| R2-2 HIGH: sanitizer quadratic on emoji+FE0F | confirmed | O(1) previous-code-point tracking; raw input capped relative to `maxChars` (`min(1M, 4*maxChars+4096)`). |
| R3-1 HIGH: NFKC expansion bypassed the cap (3 MB input, 3.4 GB RSS) | confirmed | Re-cap after NFKC; timing test relative to plain text of equal length. |
| R2-3 / R3-2: hidden-text rules over-corrected, then failed open | confirmed | Large positive offsets are not "hidden" without a bounding box; with a box, wholly-outside-document and outside-ancestor-clip boxes are hidden; positive offsets/text-indent hide when an ancestor (or the node) clips overflow; opacity compounds and is folded into the text colour for contrast; an unsampled background image makes the backdrop *unknown* and the segment is returned in `Collected.unjudged` (visible but flagged), a `backgroundSample` colour from the capture is judged. Decision: the capture should supply bounding boxes, the document size and sampled backdrop colours; without them T0 reports what it could not judge instead of guessing. Known evasions left: `rect(1px...)`, `circle(0 at ...)`, polygon clip paths. |
| R2-4 / R3-4: merchant not bound to where the request goes; TOCTOU | confirmed | `authorize` snapshots the request (each field read once, frozen) and returns the frozen request to send; the egress decision is now a branded object issued only by `evaluateEgress`/`evaluateRedirect` and its host must equal the merchant (`egress-decision-missing`/`egress-host-mismatch`). **Interface change:** `MoneyDetails.egress` (an issued `EgressAllow`) replaces the earlier string. `decide`/`checkProofs` are not exported from the package index and `package.json` `exports` exposes only the index. |
| R2-5 | confirmed | Robots: lone surrogates, `//` and `\` targets, slash runs recreated by dot-segments, versioned product token; a 500 KiB cut drops the final partial line instead of widening a rule. |
| R2-6 | confirmed | `requiresAuth` must be boolean everywhere; read-only requires an explicit `false`. |
| R2-7 / R3-11 | partly | Strict (`!'()*`), plus-for-space, lowercase and double-encoded forms; numbers redacted only on exact registered match or Luhn card shape (IIN 2-6). Known: ~10% of 13-digit integers starting 2-6 are Luhn-valid (epoch-ms after 2033-05); numeric secrets with leading zeros and HTML-entity encodings are not covered. |
| R2-8 / R3-7 | confirmed | Latin inside CJK/Thai running text is not flagged; CJK/Thai/Hangul look-alikes of Latin letters (`〇 口 エ ㄒ ล ...`) are; more scripts listed; keycap emoji keep their selector. |
| R2-9 / R3-3 | confirmed | `describeTool` reads every caller value exactly once into locals (getters/proxies tested). |
| R2-10 / R3-5 | confirmed | `reconcile(key, fp, outcome, now)` applies only to `unknown` entries or `in-flight` ones past their TTL (a live request can never be reconciled, so no double execution). It is an operator action and must never be reachable from the agent/tool surface. |
| R3-9 | partly right | `r2-surrogate-cut` is observable only in flags; `r2-robots-cut-surrogate` was not equivalent. Both mutants replaced; robots now cuts at a line boundary and a test pins it. Two mutants (`r2-robots-double-slash`, `r2-robots-backslash`) are equivalent after the post-resolution slash merge. |
| R2-11 / R3-12 | confirmed | `mutate.ts` requires a *named failing test* for a kill (a crash is INVALID) and has a 120 s per-run timeout; tests no longer wrapped in `if (out)`. |

Not done (stated): a side-effecting GET redirected cross-host is decided by method only;
mandates are bearer tokens (`subject`/`issuer` not bound to a credential); completed
idempotency entries expire (money is protected by the permanent ledger entry);
`reconcile` has no authentication of its own.

Commands and results:

```
cd agent-policy
./node_modules/.bin/tsc --noEmit          # clean
bun test                                  # 363 pass, 0 fail (3 consecutive runs)
bun scripts/mutate.ts --jobs 6            # 191/191 killed (2 equivalent mutants documented)
```

Round 4: a final Opus review of the round-3 fix diff is recorded in the next entry.

## 9. Entry 2026-11-10: review round 4

Round 4 (Opus, high; diff bc2434e..5caa46c) found 1 HIGH, 2 MEDIUM and several LOW.
All fixed in the round-4 commit:

| Finding | Verdict | Fix and decision |
| --- | --- | --- |
| HIGH: sanitizer cost still unbounded: NFKC canonical reordering is quadratic in a run of combining marks (84 KB page = 17 s) | confirmed | Stream-Safe Text Format (UAX #15): a run of more than 30 `\p{M}` is cut before normalization, flagged `combining-flood`. Test with two mixed-class floods (< 1.5 s). |
| MEDIUM: clipping fixes evadable with `overflow: clip`, `hidden auto`, two-value forms | confirmed | One parser for `overflow` (either axis, `hidden` or `clip`) used for the node itself, children and `zero-size`. |
| MEDIUM: new false positives hid visible content (masonry grids, app-shell scroll containers, fixed nav) | confirmed | With a bounding box only the box decides (document and accumulated clip); `overflow:auto|scroll` makes its content a fresh reachable region; `position:fixed` escapes ancestor clips; without a box, an offset is judged only against a clipper of *known size*; the text-indent image-replacement trick still fires on a box that clips itself. Decision: the capture must supply rects, document size and clipper sizes for T0 to be both safe and complete; otherwise text is kept and reported rather than guessed hidden. |
| LOW: `authorize` threw on `null` members | confirmed | Snapshot passes non-objects through; `checkStatic` refuses; `null` request refused. |
| LOW: `deps` read several times | confirmed | `deps` destructured once. |
| LOW: confirmation TTL above the 7-day retention window could be replayed | confirmed | Maximum accepted confirmation lifetime is clamped to retention minus 24 h (6 days). |
| LOW: robots rules containing `//` unmatchable after the merge | confirmed | Rule patterns are merged the same way as paths. |
| LOW: CJK look-alike set had dead Hangul entries and flagged ordinary Japanese | confirmed | Set reduced to look-alikes of Latin letters (`〇 口 丨 ㄒ ㄥ ㄚ ล`) plus the NFKC-normalized Hangul jamo; common katakana/kanji (`ー 一 中 人 カ タ エ ロ ニ ハ ト`) removed (documented residual: katakana look-alikes such as エ/ロ are no longer flagged). |
| LOW: `unjudged` unused; tests soft | confirmed | Still reported only (the T0 emitter, not this library, decides what to do with `Collected.unjudged`). Getter tests now assert unconditionally and also cover `credential.scopes`, `thirdParty`, null members. |

Equivalent mutant added: `r4-scroll-clipped-flag`.

Results: `bun test` 372 pass; `tsc` clean; mutation run 206/206 non-equivalent killed
(16 new `r4-*` mutants).

## 10. Entry 2026-11-10: review round 5, and mismatches with agent-schema (PR #7)

Round 5 (Opus, high; diff 5caa46c..da04aa7) found 1 HIGH and 3 MEDIUM (all in my round-4
patches), now fixed:

| Finding | Verdict | Fix |
| --- | --- | --- |
| HIGH: U+FF9E/U+FF9F (category Lm) become combining marks under NFKC, so interleaving them with marks defeats the run cut (84 KB = 8.4 s) | confirmed; the reviewer's claim that these were the *only* two was wrong | The run cut counts them, and also U+0E33 and U+0EB3 (Thai/Lao SARA AM, found by the enumeration test that now scans code points 0..0x3FFFF for any non-mark whose NFKC starts with a mark). |
| MEDIUM: any `overflow:auto|scroll` wrapper reset the clip, even an auto-height wrapper sticking out of a clipping ancestor | confirmed | A scroll container resets the clip only when it lies inside the inherited clip (or nothing clips above it); otherwise it is clipped like a hidden box. A 1 px scroll container is `zero-size`. |
| MEDIUM: `text-indent` hiding inside a clipping ancestor was no longer detected | confirmed | A child of a clipper is hidden when its indent reaches the clipper's width (900 px when unknown). |
| MEDIUM: `position:fixed` assumed to escape every clipping ancestor | confirmed | New `NodeStyle.containsFixed` (capture sets it for transform/filter/contain/... ancestors); fixed boxes are judged against `CollectOptions.viewport`. |
| LOW | fixed | zero-area wrappers no longer count as "outside"; null/non-object `tokens` refused; NaN `maxTtlMs` ignored; `deps` read-once is now tested. |

Capture requirements this makes explicit (so T0 is both safe and complete): bounding rects,
document and viewport size, clipper sizes, sampled backdrop colour for image backgrounds,
and `containsFixed`. Without them text is kept and reported (`unjudged`), never guessed.

Results: `bun test` 388 pass; `tsc` clean; mutation run 216/216 non-equivalent killed.

### Mismatches between this library and `agent-schema` (origin/feat/agent-schema b2ec4dc, PR #7)

I read the schema from the local remote ref only (fetch is blocked in this sandbox: github.com:22).
None is adopted yet; adoption waits for #7 to merge. For the parent:

1. **Mandate has no signature.** Schema `Mandate` carries `hash` (content hash), `mode`,
   `grantor`, `scope: [{kind: action|flow}]`, `spend_cap_minor`, optional
   `per_call_cap_minor` (min 0), `issued_at`/`expires_at`. Nothing authenticates it. This
   library needs a signed envelope (`token = payload.sig`, Ed25519 verifier held by the
   executor). Proposal: schema adds `SignedMandate = { mandate, signature }` (or keeps the
   compact token) and the executor verifies it; mandate `flow` scope must be supported here;
   I require caps >= 1 (a zero cap authorizes nothing).
2. **Confirmation binding.** Schema binds `request_digest` + `idempotency_key` + `subject`;
   mine binds the field tuple (action, item, total, currency, payee). Proposal: bind both.
   Mine also lacks `subject = {flow, step}`; I will add it.
3. **Payee type.** Schema `payee` is `PageTextNonEmpty` (page-origin text). My policy requires
   the payee to be a *host* that equals the merchant and is on the mandate allowlist; page
   text cannot be compared to an allowlist. Proposal: `payee` becomes a `Host` (surface
   value) and any display name stays a separate `PageText`.
4. **Items.** Schema financial confirmation has `items[]` with `quantity` and required
   `amount`, total checked against the sum; mine binds a single `item` string and a total.
   I will bind a digest of the item list.
5. **Idempotency key pattern.** Schema `[A-Za-z0-9_-]{16,128}`; mine accepts printable
   ASCII 8-128. I will tighten to the schema's.
6. **Ids.** Schema slugs `^[a-z][a-z0-9_-]{0,63}$` and `n_<16 hex>`; mine accept
   `[A-Za-z0-9._:-]{1,128}` for action/mandate ids and `act_<hex>` for tool ids. I will
   tighten to the schema.
7. **PageText** is `{origin, value}` in the schema; mine `{origin, text, flags}`, issued
   only by `pageText()`. I will carry `value` and keep flags beside it.
8. **Receipt.** Schema `request: {method, host, path}`, `amount: Money`, tier excludes T4
   (matches mine), read-only receipts T0/T1 reads (matches the matrix here). Mine has
   `requestClass` and a money summary; I will adopt the schema's shape.
9. **Refusal vocabulary.** The schema's `BlockedReason` enum is coarser than my refusal
   codes; each of mine maps to one `BlockedReason` and keeps its code as detail.
10. **Enforcement matrix** is not in the schema (it says so); agreed: it stays here, and the
    schema's tier/effect/mode enums are identical to mine.
