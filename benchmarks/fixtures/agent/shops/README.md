# Fixture shops

Three small shops with identical observable behavior, built in three stacks so the agent
surface can be shown framework-neutral (`docs/AGENT_SURFACE.md` §1, D1, D5, D9, D11, M0).

| Stack | Client | Default port | CSRF token source → sink |
| --- | --- | --- | --- |
| `react/` | React 19 SPA (`app.jsx`, built to `public/app.js`) | 3000 | `<meta name="csrf-token">` → `X-CSRF-Token` header |
| `server-rendered/` | server-rendered HTML fragments + `public/enhance.js` (htmx-style, ~70 lines, no framework) | 8080 | hidden `_csrf` form field → `X-CSRF-Token` header |
| `svelte/` | Svelte 5 SPA (`App.svelte`, built to `public/app.js`) | 5173 | `csrf` cookie (`document.cookie`) → `X-CSRF-Token` header |

`PORT=n bun <stack>/server.ts` serves one shop. Run one at a time and stop it afterwards.
`SHOP_SEED` (default 1) seeds every server-issued id; `SHOP_CLOCK=real` uses wall-clock time
(default is a fixed clock, which a real browser's `Date.now()` will not match: signatures then
fail with `signature_expired`; the recorder pins `Date.now` instead).

## Flow and request-value classes

`bootstrap` → `catalog` → `create-cart` → `add-item` → `set-address` → `shipping-rates` →
`select-shipping` → `tokenize-payment` (hosted-field-style test stub) → `place-order` →
`order-status`. Every value the extractor must handle is present on purpose:

- **derived:** session cookie, CSRF token, cart id (response → URL), shipping-rate id and payment
  token (response → body), order id (response → URL)
- **input:** `sku`, `quantity`, the address fields, the test payment method id, and the
  idempotency key (`Idempotency-Key`, client-generated UUID; labelled `role: idempotency-key`)
- **constant:** content types, empty/`{}` bodies, `X-Requested-With: enhance` (server-rendered GETs)
- **client-computed (exactly one):** `X-Signature: v1.<ms>.<hash>` = `sign(Date.now(), rawBody)`
  from `shared/sign.js` (pure, cyrb53; not a security mechanism)

Payments are a local stub: `/payments-stub/v1/tokens` turns a *test method id* (`test_ok`,
`test_declined`) into a fake `tok_…` token. No card numbers, no provider, no egress.

## Scenarios (two runs each, different inputs and seeds)

`success`, `validation-error` (bad quantity then bad address, each 422, then success),
`out-of-stock` (409), `declined` (402). Inputs: `shared/scenarios.ts`.

## Files

- `shared/core.ts` domain (the only place behavior is defined); `shared/http.ts` JSON API for the
  SPAs; `shared/clients.ts` HTTP-level clients for the three wires; `shared/scenarios.ts`;
  `shared/truth.ts` truth validator; `shared/sign.js` the signature function
- `*/flow.truth.json` hand-labelled ground truth (steps, per-request value classes, dependency
  edges, state dependencies, mutation controls), authored from the app source
- `record.ts` recorder → `recordings/<stack>/<scenario>.run<N>.{har,trace.json}`
- `build.ts` rebuilds the committed React/Svelte bundles (`package.json` pins react, react-dom,
  svelte; install only to rebuild)
- Tests: `benchmarks/tests/agent-shops.test.ts`

## DOM contract shared by all three clients

`#start-cart`; `li[data-sku] form` with `input[name=quantity]` and `[data-action=add]`;
`#cart-summary[data-cart-id]`; `#address-form` (`name line1 city region postalCode country`) with
`#save-address`; `#rates input[name=rate][data-rate-label]` with `#save-rate`; `#payment` with
`#pay-method` and `#pay`; `[role=alert][data-error]`; `#confirmation[data-order-id]` and
`#order-status[data-status]`.

## Commands

```sh
bun test tests/agent-shops.test.ts          # from benchmarks/
bun fixtures/agent/shops/record.ts          # re-record all runs (needs happy-dom)
bun fixtures/agent/shops/build.ts           # rebuild React/Svelte bundles
```
