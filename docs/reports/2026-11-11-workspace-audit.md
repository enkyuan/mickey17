# `.mochi/` workspace audit, 2026-11-11

Read-only audit of `/Users/enkyuan/Desktop/Projects/mochi/.mochi` (nothing was written there).
Sizes are `du -sk` (allocated; APFS clones count in full) unless marked "apparent" (`mochi clean`
convention). Disk: 389 GiB of 460 GiB used, 19 GiB free (96 %). Ports 3000 and 8000 were not listening.

Total: about 5.2 GiB outside `target`, plus `target` 6.7 GiB (8.0 GB apparent).

## Findings that shape the plan

1. **Three evidence sets pin workspace paths and break if a directory is renamed**:
   - `source-packs/*/capture.txt` hold the absolute path of the canonical capture
     (`.../.mochi/trials/20261108-normalized/{natural,infinitegarden}/capture`, `.../20261108-microsoft-ai/capture-modules`).
   - `source-packs/infinitegarden/lift/*.lift.json` use a **relative** `source`:
     `../../../previous-projects/20261108-semantic-baseline/...` (sha256-pinned). It keeps working only
     while `source-packs/` and `previous-projects/` stay **siblings at the same depth**. The proposed
     layout does keep them together under `evidence/`.
   - `parity-runs/{natural,garden,msai}-inventory.json` embed the same absolute capture paths, and
     `run.json` of the hand-edited/compat runs records `project: ".mochi/mochi-exports/natural-compat"`,
     `".mochi/projects/infinitegarden"`. Editing an inventory changes its sha (`inventorySha256` in `run.json`),
     so **inventories must not be edited**; regenerate one at the new path with
     `bun benchmarks/src/parity/cli.ts inventory <capture> --out ...` when a re-run is needed.
2. **Historical reports keep their old paths.** 14 404 mentions of `.mochi/semantic-conversion` (97 files),
   474 of `qualification`, 408 of `projects`, 125 of `target` live inside untracked evidence. The repo already
   follows "historical reports retain their original recorded paths" (VALIDATION.md). The migration does not
   rewrite evidence and leaves no compatibility symlinks (a symlink is skipped by `mochi clean` and would hide
   the layout again). Cost: those paths stop resolving; the reversal map records every move.
3. **Tracked code and tests read workspace paths** (must be updated with the move; see "Tracked references").
   `tests/semantic.rs` (3 sites) and `benchmarks/suites/production.ts` read the canonical captures; tests skip
   with a message when the capture is absent, so a stale path **silently skips real-evidence tests**.
4. **`.claude/launch.json` (untracked, main checkout) is already stale**: `natural-recover`, `infinitegarden-recover`,
   `-recover-start` point at `mochi-exports/*-recover`, which no longer exist. Others use `mochi-exports/*-compat`,
   `target/debug/mochi`, `trials/...`. It needs the same path map (the parent owns that file).
5. **`mochi clean` finds only**: five `mochi-exports/*/node_modules` (155 MB apparent each) and `target` (8.0 GB
   apparent). No `.output`/`.tanstack` build output exists anywhere. It never looks at `parity-runs`, logs or
   exports themselves; hence the retention gap this change fills.
6. `cli-release/` and `cli-final/` are **empty directories** (0 bytes).
7. `.mochi/.claude`, `trials/.claude`, `projects/.claude`, `mochi-exports/.claude` hold only empty `.cc-writes`
   (tool bookkeeping). `.DS_Store` files at `.mochi/` and `projects/` and `trials/`. Ignore; do not move.

## Top-level inventory

Verdicts: **keep** (stays, only re-homed), **move** (re-home, no data change), **archive** (move to a clearly
labelled area, small, low value), **propose delete** (never executed by anything written here; needs the user's yes).

| Item | Size | Newest mtime | What it is | Referenced by | Regenerable? | Verdict (new home) |
| --- | --- | --- | --- | --- | --- | --- |
| `target/` | 6.7 GiB (8.0 GB apparent): debug 5.1 GiB, release 1.6 GiB | 2026-11-11 00:03 | Cargo build cache (`CACHEDIR.TAG`) | `.cargo/config.toml`, `.github/workflows/release.yml`, `Cargo.toml` comment, launch.json, `rebuild-examples.py` | Yes: `cargo build` (clean build about 85 s + dependencies) | **move** to `cache/target`. Optional delete of `release/` (1.6 GiB) when not needed; see proposals |
| `trials/` | 3.04 GiB; 11 384 files | 2026-11-10 17:27 (dir) / files 2026-11-08 | 12 dated trial runs of 2026-11-08: captures, generated projects, logs, screenshots. See "Inside trials" | tests/semantic.rs, benchmarks/suites/production.ts, source-packs/capture.txt, parity inventories, VALIDATION.md, real-world-trials.md, parity-named-equivalences.md, launch.json, rebuild-examples.py | **No** for captures (live sites cannot be re-captured identically). Project variants: only by re-export | **move** whole, intact, to `evidence/trials` |
| `mochi-exports/` | 1.00 GiB: 5 exports at 195-215 MB each (node_modules 194 MB each) + `measure` 5 MB | 2026-11-10 05:21 | Compat exports from `mochi export` (natural-compat, infinitegarden-compat, rec3-natural, rec3-infinitegarden, rr-natural2) and `measure/` (Playwright-style measurement JSON) | parity reports cite `natural-compat`, `infinitegarden-compat`; `tailwind.rs` locates Tailwind below it; docs/tokens.md; launch.json; run.json of 2 runs | Yes: `mochi export <capture> --out <new>` (not byte-identical across generator versions) | **move** to `generated/exports` |
| `parity-runs/` | 839 MiB; 106 entries (about 59 run dirs + logs/scripts); 6 674 files | 2026-11-10 20:55 | Raw parity run output, gates, calibration runs, run scripts | docs/parity.md, scripts/parity/sink.ts, 5 committed reports (named per run) | Re-running yields new, not identical, results (needs live reference servers, Chromium). The three `*-gate` dirs (675 MiB) **are** derived from their run (`parity-cli gate`) | **move** to `generated/parity-runs`; retention via the new `--runs`; cited runs pinned. See "Parity runs" |
| `semantic-conversion/` | 197 MiB; 6 651 files; 276 entries | 2026-11-09 01:09 | Benchmark results, before/after tarballs, calibration certificates, driver, authored template, logs. `BENCHMARK-RESULTS.md`, `servers.json` | handoff note ("read BENCHMARK-RESULTS.md"), `mochi clean` evidence name, 97 internal files | No (reviewed results and `*-before.tar.gz` snapshots) | **move** whole to `evidence/semantic-conversion` |
| `previous-projects/` | 100 MiB; 1 270 files | 2026-11-08 21:06 | Frozen `20261108-qualified` (21 MB) and `20261108-semantic-baseline` (81 MB); recovered original controllers | **pinned by `source-packs` lift specs** (relative path, sha256) | No | **move** to `evidence/previous-projects`, sibling of `source-packs` |
| `projects/` | 23 MiB: `infinitegarden` 1.4 MB, `microsoft-ai` 12 MB, `natural` 9 MB; no `node_modules`, no `.output` | 2026-11-09 20:37 | Hand-edited canonical TanStack projects (sources) | benchmarks/suites/production.ts, parity reports, docs, `rebuild-examples.py` (which would **overwrite** them), launch.json, VALIDATION.md | **No** (hand edits) | **keep** at `projects/` |
| `cli-validation/` | 21 MiB; 399 files; 3 authored exports without `node_modules` | 2026-11-08 21:06 | Output of the release CLI's authored export for the three examples (`.mochi/authored-report.json` per project); produced 10:32 on 2026-11-08 | nothing | Yes in principle (`mochi export --authored-spec`); the spec used differs from `source-packs/*/specification.json` (sha differs), so not guaranteed identical | **archive** to `generated/authored` (renamed: it is not a validation log) |
| `source-packs/` | 21 MiB; 327 files | 2026-11-08 16:55 | Reviewed specification, `source/`, `capture.txt`, lift specs for the three sites | docs/lift.md (2 lines), `mochi export --authored-spec`, benchmarks | **No** (reviewed) | **move** to `evidence/source-packs`, keep sibling of `previous-projects` |
| `qualification/` | 1.9 MiB: `fixture-v1`, `fixture-v2`, `fixture-final` | 2026-11-08 21:06 | Owned fixture builds with reference/actual screenshots | README.md of `.mochi`, `mochi clean` evidence name | No | **move** to `evidence/qualification` |
| `verification/`, `verification-gpu-disabled/` | 0.9 MiB, 0.3 MiB | 2026-11-08 07:58 / 07:56 | Viewport screenshots + `results.json` of the three examples (GPU on/off) | VALIDATION.md ("current local evidence"), `verify-examples.py` | Yes (`verify-examples.py`), but they are the cited run | **move** to `runs/verification/` (names still match `verification*`) |
| `template-deps/` | 0.3 MiB: `default`, `graphics`, `react19.2.7` (manifests, lockfiles, `install.log`; **no node_modules**) | 2026-11-08 13:12 | Dependency-resolution experiments for the exporter template's `bun.lock` | nothing | Yes | **archive** to `runs/template/deps`. Not a cache: the brief's `cache/template-deps` would mislabel it |
| `template-upgrade/` | 0.2 MiB; 15 files | 2026-11-08 13:12 | Scripts (`migrate.py`, `refine.py`, `motion.py`, `garden.py`, `styles.ts`), logs, `report.json` of the 2026-11-08 Base UI/Tailwind/Motion template upgrade, `base/` and `graphics/` manifests | nothing (its `report.json` records ports 4184-4186) | Scripts are hand-written one-offs | **archive** to `runs/template/upgrade` (scripts are the only record of that migration; not deleted) |
| `references/` | 0.2 MiB; 12 text files | 2026-11-08 08:39 | Read-only observations of caldiy, midday, base-ui, tanstack, bun | `mochi clean` evidence name | No | **move** to `evidence/references` |
| `handoff/` | 20 KiB: `CLAUDE.md` (17 KB, 2026-11-08 handoff) | 2026-11-08 12:49 | Handoff note from Codex to Claude Code | `mochi clean` evidence name. Its "about 40 GB" figure is stale | No | **move** to `evidence/handoff` |
| `cli-release/`, `cli-final/` | 0 | 2026-11-08 | Empty directories | nothing | n/a | **propose delete** (`rmdir`, fails if not empty) |
| `trials-clone-dedupe-log.json` | 1.6 MiB | 2026-11-08 21:17 | Log of the APFS `clonefile` dedupe: 5 179 files, 465 887 949 bytes cloned (all status `cloned`) | nothing | No (it is the only record of which copies were cloned) | **move** to `evidence/trials-clone-dedupe-log.json` |
| `README.md` | 1 KiB | 2026-11-08 07:52 | Describes `projects/trials/target/qualification/references` | none | n/a | **rewrite** (tracked content in `docs/workspace.md`; migration writes a new README) |
| 23 loose logs, scripts, json | each < 18 KB, all 2026-11-08; 69 KB together | 2026-11-08 | See next table | VALIDATION.md cites 4 | see table | **move** to `runs/logs` / `runs/scripts` |

### The loose files (all from 2026-11-08)

| File | Size | What | Referenced | Verdict |
| --- | --- | --- | --- | --- |
| `final-rust-tests.log` | 11 KB | cargo test output | VALIDATION.md | move to `runs/logs/` (cited evidence) |
| `final-browser-tests.log`, `final-clippy.log` | 0.7 KB, 0.1 KB | cargo browser tests, clippy | VALIDATION.md, real-world-trials.md | move to `runs/logs/` (cited evidence) |
| `examples-results.json` | 5 KB | per-example URL/build record (`cli_sha256`...) | VALIDATION.md, real-world-trials.md, `verify-examples.py` reads it | move to `runs/logs/` (cited evidence) |
| `final-release-build.log`, `check.log`, `structure-tests.log`, `bun-browser-tests.log`, `bun144-browser-tests.log`, `bun-browser-diagnose.log` | 0.1-1.5 KB | cargo/bun run logs | none | move to `runs/logs/` |
| `natural-export.log`, `natural-rebuild.log`, `infinitegarden-rebuild.log`, `microsoft-ai-rebuild.log` | 0.2-18 KB | export/rebuild logs of the examples | none | move to `runs/logs/` |
| `natural-production.log`, `infinitegarden-production.log`, `microsoft-ai-production.log` | 0.1 KB each | `Listening on 127.0.0.1:4184-4186` start/stop only | none | move to `runs/logs/` (candidates for deletion later, trivial size) |
| `template-build.log` | 32 B | `Script not found "build"` | none | move to `runs/logs/` |
| `package-files.txt`, `package-list.log` | 1.9 KB, **0 B** | `cargo package --list` output; empty log | none | move to `runs/logs/`; `package-list.log` is a delete candidate (0 bytes) |
| `rebuild-examples.py`, `verify-examples.py` | 2 KB each | Re-export the three examples into `.mochi/projects` (**would overwrite the hand-edited projects**) and screenshot them | none | move to `runs/scripts/`. Paths inside (`.mochi/trials`, `.mochi/projects`, `.mochi/target/release`, `.mochi/examples-results.json`, `.mochi/verification`) are stale after the move; the scripts are not edited (untracked evidence); fix on reuse. Mark `rebuild-examples.py` dangerous |

## Inside `trials/` (3.04 GiB)

Twelve dated runs, all from 2026-11-08 (`20261108-*`). Contents: 25 capture directories (a `manifest.json` beside `site/`,
2.58 GiB), project variants, logs, screenshots and `REPORT.md`/`results.json`.

**Canonical captures** (read by tests, benchmark suite, source-pack pointers, parity inventories, committed docs):

| Capture | Size | Used by |
| --- | --- | --- |
| `20261108-normalized/natural/capture` | 212 MiB | `tests/semantic.rs` (3), `benchmarks/suites/production.ts`, `source-packs/natural/capture.txt`, `natural-inventory.json`, parity-named-equivalences.md |
| `20261108-normalized/infinitegarden/capture` | 62 MiB | same, plus `garden-inventory.json` |
| `20261108-microsoft-ai/capture-modules` | 185 MiB | same, plus `msai-inventory.json` |

Together 459 MiB. Each also contains `reconstruction/` (derived models, evidence name).

**Non-canonical capture variants: 22 directories, 2.11 GiB** (allocated):

| Run | Natural | Infinite Garden | Microsoft AI |
| --- | --- | --- | --- |
| `030806` | 133 MB | 14 MB | |
| `reconstruction` (+ `capture-before-animation-state` x2, `capture-mid-animation`) | 212 + 212 | 62 + 62 + 18 | |
| `renderer-auto` | | 62 | |
| `surfaces-final` | 212 | 62 | |
| `surfaces-v1` / `v2` / `v4` / `v5` | 93 / 160 / - / 93 | 18 / 9 / 50 / 17 | |
| `synchronized` | 212 | 62 | |
| `microsoft-ai/capture` (early, 55 MB), `capture-current` 184, `capture-final` 185 | | | 55 + 184 + 185 |

Most of this is shared bytes. A path-wise `diff -rq` against the canonical capture shows each variant differs
in only 21-35 entries (Natural 21-27, Garden 35, `capture-current`/`-final` 38/58); the sum of the differing
files is **about 1.45 GB upper bound** (Natural 0.73 GB, Garden 0.33 GB, Microsoft AI 0.39 GB). That matches the
earlier "about 1.5 GB" estimate; it is an upper bound because variants also share content with each other.

**Clone relationships** (`trials-clone-dedupe-log.json`, 2026-11-08 21:17, 5 179 files, 465 887 949 bytes):
a `clonefile` dedupe that points duplicates at a canonical copy. Canonical copies are in
`20261108-normalized/{natural,infinitegarden}/capture` (about 59.5 MB per Natural pair, 7 pairs; about 3.5 MB per
Garden pair, 9 pairs), `20261108-microsoft-ai/capture-modules/site` (20 MB onto `capture/site`), and
`renderer-auto/infinitegarden/capture`. So:

- Deleting one side of a clone pair frees only the metadata; the data blocks stay with the other side.
- Deleting **a canonical copy** is worse than it looks: other variants keep the blocks but you lose the canonical path.
- `du` double counts clones, so the 2.58 GiB total overstates the real footprint. True freeable space from deleting
  every non-canonical variant is between 0.4 GB and 1.45 GB; it cannot be measured with `du`.

**Project variants (generated) inside trials, about 365 MB** (matches the earlier estimate): `client-recovery/`
(`natural-current/-final/-ready/-structure-bun1311` 21 MB each, `natural` 12 MB, `project*` 2.8 MB x3, `verification*`
45 MB), `microsoft-ai/` (`microsoft-ai`, `project-complete/-final/-ready/-verified` 32 MB each), and the
non-capture parts of `normalized`, `reconstruction`, `surfaces-*`, `synchronized`. They are historical outputs of earlier
generator versions; only `real-world-trials.md` / `VALIDATION.md` describe them. Not referenced by code or tests.

Verdict for all of `trials/`: **keep and move intact** as `evidence/trials/`. The captures are evidence by the repo's
own rule (`workspace.md`, `mochi clean`), the saving from variants is small and unmeasurable until the user decides
which runs matter, and splitting the three canonical captures out would change the paths that the tests, inventories
and docs cite. Deleting variants is **proposed, not done**, below.

## Parity runs

Run directories are cited by name in five committed reports (docs/reports/2026-11-10-parity-*.md, docs/parity.md,
VALIDATION.md). Cited runs (and the pins they need): `calibration-natural-1`, `-8`, `-9`, `garden-dr-bp-1`,
`garden-dr-scroll-1`, `garden-dr-static-1`, `garden-hand-bp-1`, `garden-hand-core-1`, `garden-recover-core-1`,
`garden-sem-final-core-1`, `msai-sem-final-core-1`, `natural-compat-core-1`, `natural-compat-scrollnoise-2`,
`natural-default-recovery-core-1-ABORTED-slow`, `natural-dr-scroll-1`, `natural-dr-static-1`,
`natural-sem-final-core-1`, `natural-sem-final-rerun-1`, `natural-semantic-assemble-1`, `pane-1`, `browser-test`.

Uncited and derived: the three `*-gate` directories (301 MB, 205 MB, 169 MB = 675 MB, mostly `reference.json` /
`candidate.json` copies; `bun benchmarks/src/parity/cli.ts gate <run>` rebuilds them), `*-rediff*`,
`*-final`, superseded or aborted calibration runs (`calibration-natural-2...-7`, 36 MB), `example-*` (32 KB), and
the `run-*.sh` / `.done` helper files. The `-gate` trio is the only large cheap win.

## Tracked references to workspace paths

Updated by this change (all name paths that move): `.cargo/config.toml`, `.github/workflows/release.yml`,
`docs/workspace.md`, `src/clean.rs`, `tests/clean.rs`, `tests/semantic.rs` (3 sites),
`benchmarks/suites/production.ts` (4), `src/semantic/tokens/styles/tailwind.rs`, `scripts/parity/sink.ts` (3),
`docs/{parity,semantic-conversion,lift,authored-export,tokens,real-world-trials}.md`, `docs/reports/*.md`,
`VALIDATION.md`, `Cargo.toml` (comment). Not changed because the path does not move: all `<project>/.mochi/...`
(`export.json`, `style-audit.json`, `authored-*.json`, `provenance`, `dist`), `src/main.rs` default `.mochi`,
`src/parity.rs` and `benchmarks/src/fingerprint.ts` (skip lists), `.gitignore`, `Cargo.toml` exclude.
Left for a follow-up: `benchmarks/README.md` and `examples/heap_profile.rs`, which write new scratch files at
`.mochi/*.json`; new scratch output should go to `.mochi/runs/` (README of the workspace says so).

## Proposed layout (judged)

```
.mochi/
  README.md
  evidence/                      never auto-deleted (new evidence name `evidence`)
    trials/                      was trials/ (intact, canonical + variants)
    trials-clone-dedupe-log.json
    source-packs/  previous-projects/   (siblings: lift specs use ../../../previous-projects)
    semantic-conversion/  references/  qualification/  handoff/
  projects/                      hand-edited canonical sources; unchanged
  generated/
    exports/                     was mochi-exports/
    authored/                    was cli-validation/
    parity-runs/
  runs/
    logs/  scripts/              the 23 loose files
    verification/                verification, verification-gpu-disabled
    template/{deps,upgrade}/     was template-deps, template-upgrade
  cache/
    target/
```

Differences from the brief, with reasons:

1. `evidence/captures` becomes **`evidence/trials`**. Captures sit in 12 mixed trial directories; the three canonical ones are cited
   by relative inner path (`20261108-normalized/natural/capture`) by tests and docs. Extracting them would rename every citation and
   leave `reconstruction/` derived models beside nothing. One `mv` of the whole tree is safe on one filesystem and preserves APFS clones.
2. `generated/semantic` becomes **`generated/authored`** for `cli-validation`: that directory holds authored exports, not semantic output.
   `semantic-conversion` stays evidence.
3. `cache/template-deps` becomes **`runs/template/deps`**: it has lockfiles and manifests only, no `node_modules`; a "cache" label would
   invite deleting it. `template-upgrade` joins it.
4. `verification*` goes to **`runs/verification`**, not deletable: its name still matches `verification*`, so `mochi clean` skips it
   and `--runs` retention never touches it.
5. `handoff` and `qualification` stay evidence as in the brief.

## Proposed deletions (nothing is deleted by any script written here)

| What | Size | Why | How |
| --- | --- | --- | --- |
| `cli-release/`, `cli-final/` | 0 | empty | `rmdir` |
| `package-list.log` | 0 B | empty | user decision |
| `target/` (`mochi clean --cargo`; dry-run on the real workspace: **8.0 GB apparent**) | 6.7 GiB allocated | Regenerable; frees real space on APFS | after the move, only when no cargo build runs |
| `target/release` only | 1.6 GiB | rebuilt by `cargo build --release` | `rm -rf` after the move, user decision |
| `mochi-exports/*/node_modules` x5 | 777 MB apparent (970 MB allocated) | `mochi clean --deps` dry-run lists them as `remove`. On APFS they are clones of the bun cache, so real saving is small | `mochi clean --deps` |
| `mochi-exports/rec3-natural`, `rec3-infinitegarden`, `rr-natural2` | 3 x 215/199/215 MB (about 63 MB each without `node_modules`) | same `input_sha256` as `natural-compat` and `infinitegarden-compat`, uncited by any tracked file | user decision; they are exports (regenerable), but not byte-identical to a re-export |
| `parity-runs/*-gate` x3 | 675 MiB | derived (`gate`), uncited | `bun benchmarks/src/parity/cli.ts gate` regenerates |
| superseded/aborted calibration and `*-rediff*` runs | about 60 MB | uncited | `mochi clean --runs --keep-runs N` (new) after pinning the cited ones |
| non-canonical capture variants | 0.4-1.45 GB real | evidence by the repo's rule | **only** with the user's explicit yes, per run; never by code |
| trials project variants | about 365 MB | historical generated output | user decision |

`mochi clean --deps --cargo` dry-run on the real workspace (2026-11-11): five `node_modules` (155.0-155.7 MB each) and `target` (8.0 GB)
as `remove`, 8.8 GB apparent in total, 41 evidence directories protected; no build output rows.

## Log

- 2026-11-11 (step 1): audit started; read `docs/workspace.md`, `src/clean.rs`, `tests/clean.rs`; no tracked-file edits yet.
- 2026-11-11 (step 1): sized all 40 top-level entries; inspected `cli-release`/`cli-final` (empty), `cli-validation`, `template-*`, `handoff`, `.claude` stubs.
- 2026-11-11 (step 1): `trials` captures classified; clone log analysed; source-pack pins found (`capture.txt`, relative lift `source`); parity inventory pins found; cited parity runs listed.
- 2026-11-11 (step 1): report written; messaged the parent. Phase 1 starts next.
- 2026-11-11 (step 2): Phase 1 on branch `feat/workspace-layout`: `src/clean.rs` (`evidence` protected name, `Class::Runs`, `--runs`/`--keep-runs`, `KEEP` pin, `remove_run_entry` through held descriptors, ancestor check now uses every evidence name), `src/main.rs` flags, 12 new tests in `tests/clean.rs` (38 pass), `docs/workspace.md` rewritten for the layout, tracked path references updated (`.cargo/config.toml`, release workflow, `tests/semantic.rs`, `benchmarks/suites/production.ts`, `tailwind.rs`, docs). A failing new test found that `mochi clean .mochi/runs/verification/x` was accepted (only core names were checked in ancestors); fixed.
- 2026-11-11 (step 3): Phase 2 `scripts/workspace/migrate.sh` written and exercised on a synthetic workspace: dry run default, apply, rerun (0 to move), missing item (refused, `--allow-missing`), partial state, busy port (stubbed `lsof`; the sandbox blocks real binding), reversal map appended to `handoff/workspace-moves-2026-11-11.md`. A first draft would have pre-created `evidence/handoff` and moved `handoff` into itself; fixed by ordering and a destination guard.

## Approved prune allowlist (`scripts/workspace/prune.sh`)

Approved by the user via the parent session: the four groups below. Original captures and non-canonical capture
variants are **not** on the list and are not touched. Paths are relative to `.mochi/` in the **new** layout
(run after `migrate.sh --apply`). Sizes are `du -sk` allocated (APFS clones counted in full).

| Phase (`--only` name) | Exact paths | Size | Removed by |
| --- | --- | --- | --- |
| `empties` | `cli-release`, `cli-final` | 0 | `rmdir` (fails if not empty) |
| `gates` | only `reference.json` and `candidate.json` in `generated/parity-runs/natural-compat-core-1-gate` (141.9 + 143.1 MiB), `.../garden-hand-core-1-gate` (63.5 + 80.3 MiB) and `.../garden-recover-core-1-gate` (80.2 + 80.3 MiB): 6 files | 589 MiB | `rm -f`. **Narrowed from the whole gate directories**: `docs/reports/2026-11-10-parity-first-scoped.md:117` says `*-gate/check-parity-report.json` are retained, so each gate's `check-parity-report.json` (6.5, 54.8, 2.3 MiB), `contract.json` and log stay |
| `dup-exports` | `generated/exports/rec3-natural` 210.2 MiB; `.../rec3-infinitegarden` 194.8 MiB; `.../rr-natural2` 210.2 MiB (each includes its `node_modules`) | 615 MiB | `rm -r` (an export, not a build output) |
| `trial-projects` | the 29 directories below | 336.2 MiB | `rm -r` |
| `node-modules` | `generated/exports/{infinitegarden-compat,natural-compat}/node_modules` 190.3 MiB + 189.6 MiB (the three duplicate exports' `node_modules` are already gone with `dup-exports`; they stay on the list for `--only node-modules`) | 380 MiB (777 MB apparent for all five) | `mochi clean --deps --apply` |
| `cargo-target` | `cache/target` | 6.68 GiB (8.0 GB apparent) | `mochi clean --cargo --apply` (refuses while `.cargo-lock` is held) |

Parts that need `rm`: `gates`, `dup-exports`, `trial-projects` (none is something `mochi clean` knows); `empties` needs `rmdir`.
Parts done by `mochi clean`: `node-modules`, `cargo-target`; the script first runs the dry run and refuses if any `remove`
row is not on the allowlist (for example a `node_modules` under `projects/`).

Trial project directories (29; `evidence/trials/` + path; all are generated TanStack projects with `package.json`, no
`node_modules`, no `.mochi/provenance`, no evidence-named entry, no capture layout, no entry in the clone log, no reference
from a tracked file, from `parity-runs`, `source-packs`, `previous-projects` or `qualification`):

| Directory | KiB |
| --- | ---: |
| `20261108-client-recovery/natural-current` | 21624 |
| `20261108-client-recovery/natural-ready` | 21452 |
| `20261108-client-recovery/natural-structure-bun1311` | 21032 |
| `20261108-client-recovery/natural` | 12564 |
| `20261108-client-recovery/project-current` | 2764 |
| `20261108-client-recovery/project-final` | 2760 |
| `20261108-client-recovery/project` | 2764 |
| `20261108-microsoft-ai/project-complete` | 31928 |
| `20261108-microsoft-ai/project-final` | 32108 |
| `20261108-microsoft-ai/project-ready` | 32108 |
| `20261108-microsoft-ai/project-verified` | 31904 |
| `20261108-normalized/infinitegarden/project` | 3180 |
| `20261108-normalized/natural/project` | 13044 |
| `20261108-reconstruction/infinitegarden/project-background-legacy` | 3168 |
| `20261108-reconstruction/infinitegarden/project-before-animation-state` | 3168 |
| `20261108-reconstruction/infinitegarden/project` | 3172 |
| `20261108-reconstruction/natural/project-background-legacy` | 13112 |
| `20261108-reconstruction/natural/project-before-animation-state` | 13112 |
| `20261108-reconstruction/natural/project` | 13068 |
| `20261108-renderer-auto/infinitegarden/project` | 3168 |
| `20261108-surfaces-final/infinitegarden/project` | 3168 |
| `20261108-surfaces-final/natural/project` | 13096 |
| `20261108-surfaces-v1/infinitegarden/project` | 2520 |
| `20261108-surfaces-v2/natural/project-verified` | 8884 |
| `20261108-surfaces-v2/natural/project-visible` | 8896 |
| `20261108-surfaces-v2/natural/project` | 8880 |
| `20261108-surfaces-v4/infinitegarden/project` | 1352 |
| `20261108-synchronized/infinitegarden/project` | 3168 |
| `20261108-synchronized/natural/project` | 13100 |

**Dropped from list (4):** `20261108-microsoft-ai/microsoft-ai` (31 936 KiB), and, after the second review,
`20261108-client-recovery/natural-final` (21 464 KiB) and `.../infinitegarden-final` (4 568 KiB). `VALIDATION.md:23` and
`docs/real-world-trials.md:22` name the first as the served project of the first Microsoft AI export; `VALIDATION.md:384-385` and
`docs/real-world-trials.md:91` name the other two as the trial's "final projects" (ports 4184/4185). They stay until those
statements are rewritten. The 29 that remain total **336.2 MiB** (the earlier "about 365 MB" estimate included the three cited ones).
For information: the untracked `trials/20261108-client-recovery/results.json` and `20261108-normalized/results.json` record
`normalized/*/project` paths as `model_project`; those directories are on the list, they are historical generated output and no
tracked file cites them.
The other sibling names the tests mention (`.../project-final` in `tests/clean.rs`) are synthetic fixtures built in a temp
directory, not references to the real directory. Directory-level citations of whole trial runs
(`real-world-trials.md` "Artifacts are under .../20261108-client-recovery") name the run, not these project variants; the
run's `results.json`, logs, `verification*` and captures stay.

What the script refuses on top of the allowlist: a symlink (the path or anything inside it outside `node_modules`), another
filesystem or a mount point inside, an evidence-named entry or a `manifest.json` beside a `site/` inside, a path held open
(`lsof +D`), a running cargo or rustc, `.cargo-lock` held, listeners on 3000, 8000, 4184-4186. The `rm` phases use paths, not
descriptors (documented limit in the script header); `mochi clean` phases use descriptors.

- 2026-11-11 (step 4): `scripts/workspace/prune.sh` written and exercised on a synthetic workspace (dry run, apply, rerun,
  decoy `projects/*/node_modules` refused by the allowlist cross-check, evidence-named entry and symlink inside a gate refused,
  capture layout and `microsoft-ai/microsoft-ai` untouched). Needs the Opus review before it is run for real.
- 2026-11-11 (step 5): Opus 5.5 high review of the staged diff returned 9 findings; all confirmed and fixed or handed over:
  (1) a run holding evidence (the real `calibration-natural-*/evidence/` dirs) was a REFUSED row, its `.log` was still removed and
  `--runs --apply` could never succeed: such a run is now not a candidate and its whole group is kept; (2) `remove_run_entry` did not
  compare the opened directory with the planned identity: added; (3) groups made of files could not be pinned: `KEEP` list file
  directly in `generated/parity-runs` / `runs/logs`; (4) `-gate/-final/-rediff*` runs now follow the pin of their base run;
  (5) `migrate.sh` checked only the source device: destination components must be real directories on the workspace device;
  (6) interrupted before the new README was written: a rerun now finishes it; (7) reversal map: README line moves the new one aside;
  (8) `lsof` errors passed the port check: `lsof` must work and `nc -z` is tried too (also in `prune.sh`); (9) `.claude/launch.json`
  (untracked, parent's) needs the path map. Separately, while reading the retained-reports line the gate rows of the prune allowlist were
  narrowed to the two large derived files. Tests: 40 pass. A second review of the fixes and of `prune.sh` follows.
- 2026-11-11 (step 6): second Opus 5.5 high review (fixes and `prune.sh`): 8 findings. Fixed: (1) two cited projects were on the
  allowlist (`natural-final`, `infinitegarden-final`): removed; (2) `--mochi` could run `mochi clean --deps --apply` with no allowlist
  cross-check when the five `node_modules` were already gone: `mochi clean` now runs only for what the preflight cross-checked;
  (3) symlinked parent directories were followed: every path component is now checked, again at removal; (4) a group with an
  evidence-named or capture-layout top-level member was not kept whole: now it is; (5) `KEEP` entries written as `x/` or with a path
  pinned nothing: normalized; (6) the cross-check refuses while any other build output exists (fails safe): documented in the script
  header; (7) workspace.md contradictions: corrected, `prune.sh` mentioned; (8) pin added between plan and apply is not re-read for
  file members (race only, documented as the known limit). No finding about the retention count, `derived_base`, bash 3.2 or the
  evidence checks.
