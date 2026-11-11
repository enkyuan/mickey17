# Mochi reconstruction benchmark

This suite measures declared source/candidate browser states. It is deliberately
separate from `cargo bench`, which measures the Rust editor's throughput.

It reports eight independent accuracy dimensions: layout, typography, styling,
content, assets, graphics, behavior, and routing. Every scheduled probe stays in
the denominator, including failures, missing selectors, timeouts and unexecuted
cases. Unmeasured categories are null. There is no blended overall score.
Case averages and worst-case scores prevent large easy pages from hiding failures.
These percentages are exact declared-probe matches, not a perceptual quality score.
Equivalent renamed CSS/font identifiers require an independently justified probe
mapping; changing probes to conceal a visible or behavioral mismatch is invalid.

Screenshots have exact changed-pixel counts, channel error and dimensions. No blur,
thresholding, masks or rescaling conceal small differences. Reference repeats
expose nondeterminism. A canvas box alone is not graphics fidelity. Animation must
be compared at identical source-supported clock/progress states; otherwise record
the limitation and leave the benchmark unqualified.

Runtime results include navigation timings, resource counts/transfer sizes,
DOM counts, available heap estimates, buffered long tasks and sampled idle-frame
intervals. Cases with actions also collect a bounded frame trace during the actions
and settling, separately labelled with driver overhead and truncation status.
These are lab observations, not field Core Web Vitals, INP, or a React profiler.
Cross-origin resource timing may report zero bytes. Warm caches and live networks
must not be presented as controlled load-speed comparisons.

React Doctor runs against explicit source files. A zero-file, incomplete or skipped
scan cannot pass. Diagnostic counts, source locations and tool version are retained.
Its advisory score is not an accuracy or performance metric.
An AST audit also records routes, dependency imports, explicit `any`, raw HTML,
runtime evaluation and opaque filenames. These observations do not establish that
the components have meaningful semantics. Direct dependencies
or imports from `framer-motion` fail source checks; `motion` may legitimately carry
it transitively. Findings require code review, including false positives.

## Run

Use Bun. Create a dedicated gstack browser so other tabs cannot contaminate logs:

```sh
mkdir -p .mochi/benchmarks
bun install --cwd benchmarks --frozen-lockfile
bun run --cwd benchmarks check
bun test --cwd benchmarks
# Set these in the wrapper supplied to --browser:
# BROWSE_STATE_FILE=/absolute/private/run-directory/browser.json
# BROWSE_SESSION=""
# exec /path/to/gstack/browse "$@"
bun benchmarks/src/run.ts --suite .mochi/benchmark-suite.json \
  --calibration .mochi/calibration-certificate.json \
  --browser /absolute/path/to/dedicated-browse-wrapper \
  --out .mochi/benchmarks/new-run
```

Output directories must be new. Raw browser captures, screenshots, console/network
logs, command output, source hashes, probe failures and the final JSON report are
retained under the output directory. Do not publish captured private site content.
The runner never submits arbitrary forms. Review all configured click/keyboard
traces before using a live reference. Local calibration actions are safe fixtures.

## Compatibility export runtime checks

`bun test` runs `tests/compat-runtime.test.ts`: unit tests of the pure helpers the
compatibility-export template ships in `src/export_template/src/lib/policy.ts`
(data-URI policy, srcset, page selection, viewport variants, SVG animation,
pathname decoding). Server rendering of a real export is checked by
`tests/compat-ssr.ts`, which needs a built export (it needs the TanStack
toolchain, so it is not built inside `bun test`):

```sh
cd <export> && bun install --frozen-lockfile && bun run check && bun run build
bun benchmarks/tests/compat-ssr.ts <export> [--out <dir to keep the HTML>]
MOCHI_COMPAT_EXPORTS=<export1>:<export2> bun test --cwd benchmarks tests/compat-ssr.test.ts
```

It imports the Nitro bundle and calls its fetch handler (no port, no browser), so
it verifies server HTML only. Hydration and viewport switching need a browser.
Its `ping` and SVG-animation checks only fire when an export contains such
markup; the current natural and Infinite Garden captures contain none, so those
cases are covered by the unit tests.

`tests/compat-dev-react.test.ts` covers the dev-only React clash that recovered
programs can cause (a production-built react-three-fiber reconciler sharing the
app's development React makes `jsx()` throw `dispatcher.getOwner is not a
function`). The `installOwnerGuard` unit tests always run; the real-React tests
mount into happy-dom using an installed export's `react`/`react-dom`:

```sh
MOCHI_COMPAT_EXPORTS=<export> bun test --cwd benchmarks tests/compat-dev-react.test.ts
```

## Calibration before site ratings

```sh
mkdir -p .mochi
PORT=4197 bun benchmarks/fixtures/server.ts
PORT=4198 bun benchmarks/fixtures/server.ts  # separate terminal
bun benchmarks/suites/calibration.ts .mochi/calibration-suite.json
bun benchmarks/src/run.ts --suite .mochi/calibration-suite.json \
  --browser /path/to/dedicated-browse-wrapper --out .mochi/calibration-control
bun benchmarks/tests/browser-mutations.ts /path/to/dedicated-browse-wrapper \
  .mochi/calibration-suite.json .mochi/calibration-mutations
bun benchmarks/driver/certify.ts .mochi/calibration-control/report.json \
  .mochi/calibration-mutations/report.json /path/to/dedicated-browse-wrapper \
  .mochi/calibration-certificate.json
```

The installed gstack version must capture uncaught page errors as well as console
messages. Some versions only capture the latter. The mutation test exposes this
blind spot. For those versions, create a private instrumented driver copy:

```sh
bun benchmarks/driver/prepare.ts /path/to/gstack .mochi/benchmark-driver
```

Use the values in `.mochi/benchmark-driver/environment.json` in the wrapper above.
This adds a read-only `pageerror` listener. It does not modify the installed tool,
page scripts, CDP allowlist or permission checks. The driver copy uses the installed
gstack dependencies; retain `driver-provenance.json` with the results. Then rerun
both the control and mutation suite. Non-fixture runs require `--calibration` with
a passing identical control and all ten detections tied to the same driver wrapper; no scan can silently claim runtime-error coverage without it.

The control has two routes, desktop/mobile initial and reduced-motion states,
forward/reverse scrolling, menus, Escape/focus, dialog validation and navigation,
each repeated three times. The mutation suite must detect one-pixel movement,
small tracking/color differences, removed copy, missing images, changed SVG paths,
broken menus, wrong links, overlapping cards and runtime exceptions. Calibration
is evidence about the benchmark's sensitivity, **not Mochi reconstruction accuracy**.
CI runs scoring/coverage/PNG/React Doctor regression tests on every Rust CI job.
Browser calibration requires gstack and is a separate recorded check.

## A site suite must cover the actual tool

See `src/schema.ts` and `suites/calibration.ts` for the typed manifest. Supply every
captured route and both desktop/mobile plus reduced motion, with at least three
repeats. Add tablet, wide desktop, breakpoint boundaries, forward/reverse scroll,
hover, keyboard/focus, menu/dialog states, form validation, route navigation and
SVG/WebGL/shader states as applicable. Probe complete section/content inventories
and all source-driven transitions. Explicitly list unresolved coverage gaps.

Set `referenceKind` to `captured-build` when using the preserved production build;
provide `sourceCapture` to hash that evidence. `live` results identify their moving
reference and cannot be substituted for immutable build comparisons. Preserve
capture time, response URLs and asset bodies alongside that input.

For end-to-end tool measurements, configure `reconstruction` with the actual Mochi
CLI argument vector and `build` with Bun install/check/build steps, then serve that
fresh output. `candidateProject` binds results to source before/after measurement. Supply
`candidateBuild` (for example the app’s `.output` directory) to bind build files
separately. Both inventories include file content hashes, not just timestamps.
Do not benchmark an independently hand-edited app and call it automatic codegen.
`generationMode` and `generationIncluded` stay in the report. Reviewed authored
specifications, compatibility recovery, and autonomous interpretation must be
reported as separate tracks. A paid/live model track needs its own budget approval.

Navigation traces may declare `expectedRoute`; the runner checks the resulting
route and origin, not just link attributes. Unsupported options (including an
unimplemented graphics-clock setting) fail validation rather than being ignored.
A navigation that replaces the document marks its action-frame sample unmeasured.

A successful command alone never approves a site. All declared states, pixel
comparisons, runtime failures, source checks and coverage gaps must be reviewed.
`matched_declared_benchmark` only names that tested scope; it cannot guarantee
unseen sites, framework variants, backend behavior, browsers or GPU combinations.

## Three-site baseline manifests

```sh
bun benchmarks/suites/production.ts . .mochi/benchmark-suites
```

This creates Natural, Infinite Garden and Microsoft AI manifests covering the
nine captured routes, four viewport sizes, six scroll positions, reduced motion,
and three repetitions: 756 scheduled paired captures. They intentionally contain
coverage gaps. Generic whole-page/count probes do not replace individual menu,
form, SVG, shader, animation and reverse-scroll traces. The generated manifests
cannot qualify a site until those gaps are resolved. Their track is explicitly
`existing-project`; they do not rate automatic source lifting.

Reports include a Markdown evidence index and JSON detail. Failed or interrupted
cases retain all their required probes in the denominator. Numbers measure only
the named corpus and case distribution. Calibration fixtures are not production
sites; repeated measurements are not independent sites and do not justify a
population-level confidence interval or a promise about arbitrary websites.

The production templates map the Garden essay’s `article` to the candidate’s
`article`, and the Microsoft models page’s `h2` to the candidate’s `h1`. Its title
text uses the source’s explicit `data-split-original` node to avoid counting its
visual animation duplicate twice. Selector absence still fails; these mappings
were derived from the captured DOM, not adjusted to improve a score. Whole-page
`text` probes retain DOM/screen-reader duplication and are raw text comparisons.

## Whole-page parity pipeline (original capture vs generated app)

`src/parity/` compares **every painting, text, media and control element** of the original
capture (served by `mochi serve`) with a generated app at identical scripted states. It
reuses this package's conventions (new output directories only, retained raw evidence,
calibration with injected regressions, no blended score) and feeds `mochi check-parity`.
The method, schema, matching rules, the Browser-pane + sink driver and what it can and
cannot detect are documented in [docs/parity.md](../docs/parity.md#whole-page-element-pipeline).

```sh
bun test --cwd benchmarks tests/parity-diff.test.ts tests/parity-sink.test.ts   # pure logic, no browser
bun benchmarks/src/parity/example.ts                    # offline end-to-end demo (needs Chromium)
bun benchmarks/src/parity/cli.ts inventory <capture> --out inv.json
bun benchmarks/src/parity/cli.ts calibrate --inventory inv.json --reference http://127.0.0.1:8100 --out <new dir>
bun benchmarks/src/parity/cli.ts run --inventory inv.json --reference http://127.0.0.1:8100 \
  --candidate http://127.0.0.1:3101 --out <new dir> --profile standard --controls
bun benchmarks/src/parity/cli.ts rediff <run dir> --out <new dir>   # same raw captures, current comparison code
bun benchmarks/src/parity/cli.ts gate <run dir> --project <app> --out <new dir>   # mochi check-parity
```

Run directory contents: `inventory.json`, `run.json` (versions, collector and pipeline
hashes, scheduled / executed / skipped-with-reason), `failures.json` (states that crashed), `states/<name>/{ref,cand,ref2}.json.gz`
(raw captures), `diff.json.gz`, `summary.json` (or `error.json`), `report.json`, `REPORT.md`. The suite is
separate from `run.ts`/`schema.ts` manifests: those rate declared probes with the gstack
driver; this one inventories the capture itself and diffs complete element lists. Its
`pair` command also diffs captures posted by the Browser pane. `tests/parity-browser.test.ts`
runs the offline fixtures through real Chromium when Playwright and a browser binary are
present and is reported as skipped otherwise.

Calibration (`calibrate`) is evidence about the instrument: the original is loaded twice
(zero items required), then 30+ planted regressions and shifted breakpoints must be
detected, and the documented blind spots (hover rule, focus ring,
removed handler in an initial state) must stay undetected until the state class that
covers them runs. A miss, or a nonzero control, fails calibration and the failing report is
kept. Passing it says nothing about any candidate.
