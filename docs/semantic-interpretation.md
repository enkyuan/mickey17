# Semantic interpretation

Implemented locally on 2026-11-08. The TypeSafe adapter and pipeline have been tested with
controlled responses and an owned loopback HTTP server. No live Jev request, account access,
invoice reconciliation, accuracy evaluation or calibration has been performed.

Algorithms still reconstruct the site. Jev receives a few examples of an already selected
component and chooses among fixed semantic roles: product/content card, navigation,
header/footer, collection, form field, button, content section or `unknown`. Separate text
questions offer title, description, price, label, body text or `unknown`. A companion Noul
question asks whether any supported component role applies. Questions are independent;
Rust combines and validates the answers afterwards. The model cannot supply source code,
filenames, dependency names, routes, arbitrary identifiers or replacement content.

All model decisions require review. `unknown` or applicability at most 0.5 produces an
abstention; 0.5 merely distinguishes a signal favoring yes from one that does not. It is
**not** a calibrated acceptance threshold. Distributions, confidence, applicability, source
references and response provenance remain available. Certain attribute roles (for example,
an anchor's destination or an image's source) are derived deterministically and carry no
model confidence. No interpretation automatically changes the project model or emitted code.

## Local workflow

After capture and `model-project`, prepare the exact requests without credentials or network:

```sh
mochi plan-interpretation ./capture --max-candidates 8 --examples 3 --batch-candidates 4
```

Review `reconstruction/interpretation/plan.json`. It includes request bodies, candidate/prop
targets, skipped items, configuration, evidence digest, question version and pinned model
`jev-1.14.0`. Changing these inputs invalidates the corresponding cache identity.

```sh
mochi interpret ./capture --replay
mochi export ./capture --compatibility -o ./app --interpretation
```

`interpret` defaults to replay. It requires a complete, valid local cache, reads no API key
and makes no network requests. An empty cache fails with an explanation; it cannot manufacture
decisions. Ordinary capture/model/export commands also make no model calls. Export without
`--interpretation` ignores any interpretation report. Explicit attachment validates the saved
report and adds review-required comments to component files plus provenance in
`mochi.export.json`; names, props, routes, styling and primitive mappings stay mechanical.

## Future live operation

Live qualification remains a separate, explicitly budgeted action. The operator configures
`TYPESAFE_API_KEY` in their own shell; keys are not CLI arguments or saved artifacts. The
adapter posts directly to `https://api.typesafe.ai/v1/systemone` with bearer authentication,
the pinned model and typed Choice/Noul questions. Redirects and images/long-context opt-ins
are disabled. Provider error bodies are not printed or saved.

An example for a **separately authorized** future run, after reviewing the plan and checking
the current account price:

```sh
mochi interpret ./capture --live --provider typesafe \
  --max-requests 10 --max-input-tokens 655360 --max-cost-usd 0.03 \
  --input-price-per-million-usd 0.042 --deadline-seconds 60 --max-retries 0
```

All five request/token/cost/rate/deadline options are required for live calls; retries default
to zero and may be set to at most two. Decimal money is parsed exactly to micro-USD. The
rate above reflects the documented $0.042 per million input tokens and free output checked
on 2026-11-08; this is a caller-supplied accounting assumption, not verified account billing.

Before any dispatch, the run must afford one worst-case attempt for every uncached request.
Every attempt durably reserves **65,536 input tokens**, the documented default request
context, and the rounded-up cost at the configured rate ($0.002753 in this example).
Validated reported usage releases unused reservation. Failed, timed-out, malformed or
otherwise ambiguous attempts retain the full reservation; each retry needs a new one.
Reported usage beyond the contract stops the run and remains in its journal.

Limits apply per invocation, including retries. Existing journals remain evidence of earlier
possible charges; a new invocation does not roll them into its new limits. There is no
documented tokenizer integration or vendor-side account spending cap. Account pricing,
unreported billing and provider contract violations can only be qualified with the provider;
the local ledger is not a guarantee about an invoice. The deadline bounds the request/replay
loop after local preflight; final local source/report validation may finish afterwards.

Only HTTP 429/500/502/503/504/529 are retry candidates, with finite exponential backoff.
`Retry-After` must be integer seconds at most 30; unsupported/long delays stop for a later
run. Authentication failures and transport failures stop immediately. Each HTTP attempt
has a timeout at most 30 seconds and at most the remaining run deadline.

## Evidence, bounds and saved artifacts

Defaults select at most 64 candidates, eight varying text props per candidate, three examples
and four candidates per batch. Configurable maxima are 256 candidates, eight text props,
three examples and eight candidates per batch. Examples prefer distinct routes, then distinct
logical occurrences. Each selected subtree has at most 128 nodes; full selected text is kept.
Payload caps are 8 KiB of shared state, 16 KiB of request JSON, 12 KiB for state plus each
question, and 32 questions per request. Oversized candidates/questions are explicitly skipped,
not silently clipped. These conservative byte caps do not claim an exact provider token count.

The projection supplies relative node positions, selected semantic attributes, display,
four ancestor tags, route paths and sampled text/resource props. Control values, event
handlers, classes/IDs, other computed styles, hostnames and URL query values are omitted.
Page text, paths and selected attributes can still contain sensitive information: inspect
the saved requests before enabling a live run. Captured content is marked as untrusted data.

```text
reconstruction/interpretation/
  plan.json              # deterministic request plan
  cache/<sha256>.json    # validated response, usage, origin and integrity digest
  report.json            # last complete plan, records and review-required decisions
  runs/run-*.json        # per-invocation reservations, outcomes and usage
  .lock                  # OS lock; a crashed process releases ownership
```

Response validation requires the pinned model, exact question IDs/types, allowed choices,
finite [0,1] signals, complete probability distributions summing to one, a maximum-probability
selected choice and bounded usage. Duplicate answer/probability keys are rejected. HTTP and
cache responses are capped at 1 MiB; plans/reports/journals at 16 MiB. Replays regenerate the
questions and decisions and verify hashes and current source/model inputs. These hashes
detect inconsistency, not authenticity against someone able to rewrite all local artifacts.

Plan/run operations take a nonblocking project lock, reject symlink paths, and write
atomically. Reports are published only after all entries validate and source/plan inputs
are rechecked. A failed run preserves the last complete report; valid partial cache entries
and its journal support a later resume. Replaying the same validated responses produces
identical report bytes; fresh model responses need not be identical. Journals deliberately
remain separate from the deterministic report. Controlled fixtures are labeled
`controlled_fixture`, never presented as live Jev evidence.

The next qualification step is a frozen, labeled evaluation set: ambiguous/negative cases,
role and prop accuracy, abstention, mapping errors, latency and actual usage/cost. Keep tuning
cases separate from held-out cases. Auto-application and semantic source naming remain
unqualified until those results justify specific policies.

Provider contracts checked via the official [System One API](https://docs.typesafe.ai/api/system-one),
[models and pricing](https://docs.typesafe.ai/models), [Choice](https://docs.typesafe.ai/primitives/choice),
[Noul](https://docs.typesafe.ai/primitives/noul) and [errors](https://docs.typesafe.ai/api/errors)
documentation on 2026-11-08.
