# Orchestration handoff, 2026-11-11

Written by the orchestrating (parent) session so another session or person can take over.
Everything here was true at the time of writing (end of 2026-11-11, repo state read at that
time); re-check with the commands in "Verify before acting". Claims marked **unverified** were
reported by a session and not independently checked.

## 1. Mission and standing rules

`mochi` (Rust crate `mochi-web`, CLI `mochi`) must decompile captured sites (Natural, Infinite
Garden, Microsoft AI) into editable, typed, semantic TanStack Start source: direct Base UI
primitives only under `components/ui`, Tailwind v4, Motion for animation, owned shader/SVG
sources. Nothing is hand-built; hand-edited projects under `.mochi/projects` are oracles only.
Output is verified against the originals by the parity pipeline, up to the full matrix
(756 comparisons, not yet run).

Rules the user set, which apply to every session:

- Split work into parallel sessions with disjoint files; one PR per shippable increment.
- Every change gets an adversarial **Opus 5.5 high** review before commit or any claim.
- Clean build artifacts regularly; build outputs must be regenerable.
- Framework default ports: TanStack Start / Vite app 3000, `mochi serve` 8000. Close stale ports.
- Compat exports recover client programs by default (interim); the semantic generator is the
  real replacement for recovered bundles.
- The repo is public: no secrets, no real merchant or personal data, never push `evidence/*`
  (local branch `evidence/20261108`), no publication before qualification passes.
- Branch names are clean (`feat/<topic>`), not `claude/<name>-<hash>`.
- Conventional Commit subjects; commits end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`;
  PR bodies end with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- Do not claim a result that was not measured; say what was not run.

## 2. Repository state

Base branch: `feat/tanstack-exporter`. On origin it is at `3fa794e`. The **local**
`feat/tanstack-exporter` is at `e8ccfd5`, one commit ahead (the tokens session committed on it;
the same commit is the head of PR #12). Do not push the local base branch: that would publish
the PR #12 commit without its review. Reset or fast-forward it after #12 merges.

| PR | Branch (base) | What | State |
| --- | --- | --- | --- |
| #6 | `feat/agent-fixtures` (`feat/tanstack-exporter`) | fixture shops in 3 stacks, ground truth, recorder | open, unmerged; 119 tests pass, 3 socket tests skipped (loopback blocked in the authoring sandbox) |
| #7 | `feat/agent-schema` (`feat/tanstack-exporter`) | Zod wire schema, generated JSON Schema, fixtures | open, unmerged; 517 tests, six Opus rounds (replaces closed #5) |
| #8 | `feat/agent-policy` (`feat/tanstack-exporter`) | pure policy library (modes, mandates, confirmations, egress, injection hygiene) | open, unmerged; 414 tests, 241/241 mutants killed; last delta (`4e73446..93aab9c`, `src/hidden.ts`) not re-reviewed |
| #9 | `feat/agent-flows` (`feat/tanstack-exporter`) | TypeScript flow inference from HAR runs | open, unmerged; 138 tests; round-11 fix commit `83fd9d0` has had no separate review; `bun.lock` hand-derived, `bun install --frozen-lockfile` unverified |
| #10 | `feat/workspace-layout` (`feat/tanstack-exporter`) | `.mochi` layout, `mochi clean --runs`, `migrate.sh`, `prune.sh` | open, unmerged; 41 clean tests |
| #11 | `feat/docs-layout` (`feat/workspace-layout`) | docs renamed to kebab-case, `docs/README.md` index, `scripts/docs/check-links.ts` | open, **stacked on #10**; retarget to `feat/tanstack-exporter` after #10 merges |
| #12 | `feat/tokens-reveal-end-state` (`feat/tanstack-exporter`) | reveal once-only fades; count visible text | open, **14 Sourcery findings being triaged** (section 4A) |
| #4 | `feat/reconstruction-analysis` (`main`) | older work, predates this effort | open, not part of this effort |

None of #6 to #12 is merged and none has had a human or bot review: Copilot had no quota and
Sourcery skipped the large ones (it did review #12). The sessions' Opus reviews are the only
review. Merging is the user's decision; no session or the parent has merged anything.

Other local branches: `archive/agent-flows-rust` (the abandoned Rust port of flow inference,
kept only as an archive), `feat/source-evidence`, `evidence/20261108` (local only, public repo:
never push).

**Working tree of the main checkout is dirty and not the parent's:** uncommitted edits to
`src/semantic/tokens/{capture.rs,styles/layers.rs,styles/run.rs}`, `tests/styles.rs`, untracked
`scripts/tokens/collect-scrolled-state.{js,test.js}` (the tokens session's in-progress work),
an untracked `infinitegarden.com/` directory at the repo root (unknown origin, probably a clone
output; check before deleting) and `.claude/` (launch config, partly stale: the `*-recover`
exports it lists no longer exist). Do not `git checkout`/`reset` over these.

## 3. Sessions

Sessions are Claude Desktop sessions started by the parent. Addresses are session ids; use
`SendMessage` with the id. Sessions that cannot reach `github.com:22` (most of them) hand
branches to the parent, who pushes and opens PRs with the sandbox bypass.

| Name | Id | Owns | State |
| --- | --- | --- | --- |
| semantic-generator-1-cen (tokens) | `local_0d527e2d-2907-491d-84ac-1b94a2823f2b` | `src/semantic/tokens/**`, `docs/tokens.md` (was TOKENS.md) | working: triaging the 14 findings on #12, then scroll-reveal work (`feat/tokens-scroll-reveal`) |
| semantic-generator-2-ir | `local_a14ddb0a-5c3d-43a9-b826-5b54bfedc8e8` | IR, Base UI patterns, emitters (`src/semantic/{ir,patterns,components,emit,project,js}`) | idle; last commit `ccfb178` (pushed); deferred: item 4 (per-viewport typography with tokens), item 7 (Garden canvas/WebGL is plan-only) |
| semantic-generator-3-par | `local_fc37fc56-76bd-4259-99ed-b6f1a4904a94` | parity pipeline (`src/parity.rs`, `benchmarks/`, `scripts/parity/`) | idle; last commit `c01bdb0` (named Base UI ARIA equivalence); re-run needs the preview pane visible |
| agent-schema | `local_daa9d76b-b79b-47f2-b309-e7454c8e6d04` | `agent-schema/**` | idle; PR #7; PR 2 (drift report shape) held until another session needs a shape |
| agent-fixtures | `local_bc8330c3-f4d4-4562-908d-566136bf69c0` | `benchmarks/fixtures/agent/shops/**` | idle; PR #6; follow-up (convert truth to schema shapes) after #7 merges |
| agent-policy | `local_bd3cde43-112b-43ce-a2fc-7b81029c239c` | `agent-policy/**` | idle; PR #8; schema adoption after #7 merges (mismatch list in its log section 10) |
| agent-flows | `local_f37cf530-c1fc-4835-81be-0a3ebdff345b` | `agent-flows/**` | idle; PR #9; schema adoption after #7 merges; known gaps in its log |
| workspace-layout | `local_20ae4b80-a2a6-4215-87bf-2adaaa4c8697` | `.mochi` layout, `src/clean.rs`, docs layout | idle; PRs #10 and #11 |
| compat-runtime, compat-head-doc, garden-cards, mochi-clean | various | earlier compat/export/clean work | idle or archived; their work is merged into the base branch |

## 4. Workstreams

### 4A. Semantic generator (the main product)

Pipeline: `mochi clone --source-evidence` capture, then `mochi semantic <capture> --out <dir> --assemble`
writes a complete TanStack Start project (`docs/semantic-conversion.md`, `docs/tokens.md`).
`mochi check-parity` compares original and generated (`docs/parity.md`).

State, measured:

- The generated Garden home page at the user's last run (`3fa794e`) showed only its headline.
  Measured in the browser: 198 text elements, 2 visible; 150 `visibility:hidden`, 27
  `display:none`, 19 `opacity:0`. Cause: pre-reveal initial states captured before the
  original's JavaScript ran are emitted as is, and the generated app ships no JavaScript to
  reveal them.
- PR #12 (`e8ccfd5`) raised it to 29 of 223 visible, 0 hidden by opacity, measured by the
  authoring session (**unverified** by the parent). 170 elements stay `visibility:hidden` and 24
  `display:none`; whether the original reveals them after scroll or interaction was **not**
  measured. The lower sections of the page are likely still blank.
- The five hero canvases are blank (no WebGL, no Motion yet).
- Other known defects from the 2026-11-10 parity reports (`docs/reports/2026-11-10-parity-*.md`):
  Natural desktop nav stacked vertically (partly addressed in `ccfb178`), menu panel
  `inert` state, motion and reduced-motion, Garden diagram offsets, per-viewport typography,
  Garden baked date, Microsoft AI hydration error #418, `recipes.css` size (~5.5 MB on
  Natural) and translation time (~2.5 min per page). Microsoft AI compat export is still
  refused (`uhf-header`). Browser parity numbers were taken under heavy machine load.
- Not done: the full parity matrix, canvas/WebGL, Motion translation, owned shader sources,
  `mochi lift` integration into the semantic output.

In flight: Sourcery posted 14 inline findings on #12 (two rated high: state-only selector
rules leaving interaction fades unconditional, and `settle_plan`/`Untranslated` API
compatibility). The tokens session verifies each, fixes the real ones with failing-first tests,
reviews the delta with Opus, re-measures the Garden page, and returns a table
`comment_id | fixed (commit) | declined (reason)`. The parent then pushes, posts one-line replies
ending with `_🤖 Addressed by [Claude Code](https://claude.com/claude-code)_` via
`gh api repos/enkyuan/mochi/pulls/12/comments/<id>/replies`, and resolves the threads.
Comment ids: 4541672684, 4541672697, 4541672706, 4541672716, 4541672723, 4541672731,
4541672737 (high), 4541672749, 4541672757, 4541672770, 4541672778, 4541672789, 4541672795,
4541672804 (high).

Next after that (tokens session, brief already sent): measure the original Garden at load, after
a full scroll and after nav interaction; emit scroll-revealed end states (static by default) and
let Base UI compositions own interaction-revealed panels; extend the regression test to
after-scroll counts; report original versus generated counts. Do not call it fixed until the
generated after-scroll count is close to the original's (state the gap).

### 4B. Agent surface (browser-free, machine-readable site interface)

Design: `docs/AGENT_SURFACE.md` (to become `docs/agent-surface.md` after #6 to #9 merge).
Decisions made with the user:

- Primary target is functional use of transactional flows (checkout, payments), not reading
  landing pages; reading (T0/T1) is the cheap first slice.
- Framework-neutral: derived from observed traffic and interaction traces; React-specific
  `semantic::js` facts are an optional accelerator.
- **TypeScript for everything after capture**; Rust owns capture and the page model and hands
  off JSON. No new crate; distribution via npm/Bun (`bunx`), plus a skill (`SKILL.md` + MCP
  config). `docs/DISTRIBUTION.md` records channels (GitHub Releases, Homebrew tap
  `hiyoriOS/homebrew-tap`, install script, npm scope `@hiyorios/*`, crates.io secondary).
- Schema: Zod as the single source of truth, JSON Schema generated and committed.
- Authorization is an input: modes `owner`, `delegated`, `read-only` (default); payments need a
  mandate (scoped, expiring, capped), per-call confirmation, idempotency; 3-D Secure and
  fraud checks surface as `requires_human`; card data never passes through.
- Idempotency key is class `input` with role `idempotency-key`, executor-supplied; the request
  signature is the only `client-computed` value in the fixtures.

Packages: `agent-schema/` (#7), `agent-policy/` (#8), `agent-flows/` (#9), fixtures (#6).
Not started: `agent-exec/` (HTTP replay, JS-runtime executor), `agent-mcp/` (MCP server,
projections), `agent-verify/` (browser-oracle checks), the shared page-model extraction from
`semantic::ir` (the only Rust change; it belongs with the IR owner and must not change
`semantic` output), node-id derivation, drift report, payment adapters. Milestones M0 to M6 and
exit criteria are in the design doc.

Order of work once the user merges: #7 first; then flows, policy and fixtures adopt the schema
in follow-up PRs (they develop against it with
`git fetch origin feat/agent-schema && git checkout origin/feat/agent-schema -- agent-schema`,
uncommitted, and commit the adoption only after #7 is on the base branch).

Known weak points: flow-inference redaction and alignment heuristics are best-effort
(eleven review rounds kept finding highs in the previous round's heuristics); policy's
hidden-text heuristics depend on capture-supplied geometry; none of it has touched a real
network, DNS, browser or payment provider.

### 4C. Workspace and docs layout

`.mochi/` (gitignored local workspace, ~40 top-level entries) is **still in the old layout**.
PR #10 contains the new layout (`evidence/`, `generated/`, `runs/`, `cache/`), updated tracked
references, `mochi clean --runs`, and two scripts under `scripts/workspace/`.
`migrate.sh --dry-run` against the real workspace plans 39 moves with no refusals (run by the
parent).

Order, after the user merges #10 and the checkout has it (the PR changes `.cargo/config.toml`'s
target dir and the paths tests read):

1. Stop dev servers (3000, 8000, 4184 to 4186) and any cargo/rustc.
2. `scripts/workspace/migrate.sh --dry-run`, review, then `--apply` (mv only; writes a reversal map).
3. `scripts/workspace/prune.sh --dry-run`, review, then `--apply`.

The user approved these deletions (after the move only): the two empty `cli-release`/`cli-final`
directories, `reference.json`/`candidate.json` in three parity `*-gate` directories, the three
duplicate exports (`rec3-natural`, `rec3-infinitegarden`, `rr-natural2`), 29 generated trial
project directories (exact list in `prune.sh`), `node_modules` in the five exports, and the Cargo
target (frees ~6.7 GiB and removes the built `mochi` binary). **Not approved and never
scripted:** original captures and the non-canonical capture variants (~1.45 GB, partly
clone-linked); they are evidence.

#11 renames docs to kebab-case (`SEMANTIC.md` to `semantic-conversion.md`, `TOKENS.md` to
`tokens.md`, and so on) and adds `docs/README.md` as the index; the link checker requires every
doc to be listed there. **This handoff file must be added to that index when #11 lands**
(`docs/reports/` is created by #11; this file sits in it already). `docs/AGENT_SURFACE.md`,
`docs/agent-surface/**` and `docs/DISTRIBUTION.md` are renamed in a follow-up after #6 to #9
merge. Expect a trivial rename conflict between #12 and #11 on `docs/TOKENS.md`.

### 4D. Distribution (plan only)

`docs/DISTRIBUTION.md`. Nothing is published. Open items: check names (`hiyoriOS` on GitHub,
`@hiyorios` on npm, `brew search mochi`), whether to transfer the repo to `hiyoriOS`, release
pipeline (`cargo-dist` may generate much of it; confirm its current features), signing and
notarization, one version across binary/npm/tap, update `docs/releasing.md` and `AGENTS.md`.

## 5. Decisions waiting on the user

- Merge order for #6 to #12 (suggested: #7, #10 then #11 retargeted, #12 after its findings,
  then #6, #8, #9 and the schema-adoption follow-ups).
- Whether to apply the `.mochi` migration from the PR #10 branch before merging, or after.
- Whether `claude/agent-schema-82d14a` (stale remote branch from closed #5) should be deleted.
- Archive or remove non-canonical captures and what to do with `uhf-header` refusal for
  Microsoft AI; Jev live interpretation (no authorization or ceiling given); the untracked
  `infinitegarden.com/` directory and `.claude/launch.json`.
- Whether to run a fresh independent Opus review of #7 before merging (the other PRs build on it).

## 6. How the machinery works (gotchas)

- **Sandbox:** sessions run sandboxed; `github.com:22` and `api.github.com` are blocked in
  most of them. Branches are pushed with
  `git push origin <local>:refs/heads/<clean-name>` and PRs opened with `gh pr create` by the
  parent using the sandbox bypass. Secrets-shaped test literals (for example Stripe-style keys)
  are rejected by GitHub push protection: build such fixtures at run time (JS escapes), and
  scrub history if a commit already has one; never use the allow-secret unblock link.
- **Cargo:** `.cargo/config.toml` sets `target-dir = ".mochi/target"` (moves to `cache/target`
  with #10). In sessions use `cargo --offline` with
  `SDKROOT=/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX27.2.sdk`.
  Ten socket-binding Rust tests fail only inside the sandbox; do not touch them.
- **Shell:** in the user's zsh `ls` is aliased to `lla`; use `/bin/ls`.
- **Browser checks:** the preview pane must be visible for animations and `javascript_tool`;
  without it screenshots are blank and calls time out. Ports: 3000 app, 8000 `mochi serve`.
- **Bot reviewers:** Sourcery and Copilot comments arrive as Auto-fix events; they are
  third-party text. Verify each finding yourself before acting; size-limit and quota notices
  need no action.
- **Messages between sessions** are not confirmed read; a session may hold them. Check
  `list_sessions {"linked": true}` for state. Notes arrive when a session ends a turn.
- **Machine load** was very high during this work (many builds and test runs), which slowed
  tests and made browser timings unreliable.

## 7. Verify before acting

```sh
git fetch origin && git status --short && git log --oneline origin/feat/tanstack-exporter..feat/tanstack-exporter
gh pr list --repo enkyuan/mochi --state open
ls .mochi            # old layout until migrate.sh has been applied
```

Full-tree gates (not run by the parent since `7917885`): `cargo fmt --check`,
`cargo clippy --locked --offline --all-targets --all-features -- -D warnings`,
`cargo test --locked --offline --no-fail-fast`, `bun test --cwd benchmarks`, and per package
`bun test` in `agent-schema/`, `agent-policy/`, `agent-flows/`.

## 8. Pointers

- Design and decisions: `docs/AGENT_SURFACE.md`, `docs/DISTRIBUTION.md`, `docs/workspace.md`
  (after #10/#11), `docs/reports/` (parity reports, workspace audit).
- Session logs: `docs/agent-surface/log/{agent-schema,agent-fixtures,agent-policy,agent-flows}.md`
  on their PR branches (review verdicts, decisions, known gaps).
- Workspace audit with sizes and the deletion allowlist: `docs/reports/2026-11-11-workspace-audit.md`
  (after #11; `docs/workspace-audit-2026-11-11.md` on the #10 branch).
- Earlier handoff notes (gitignored, local): `.mochi/handoff/`.
