# Documentation

Index of everything under `docs/`, plus the naming convention that
`bun scripts/docs/check-links.ts` enforces (it also fails on a broken relative link, a
`docs/….md` mention or `#anchor` that does not resolve, with exact case against the git file list, or a retired name from
[`scripts/docs/renamed.txt`](../scripts/docs/renamed.txt)).

## Convention

- Guides and design documents: `docs/<slug>.md`, lowercase kebab-case, named after the title or the
  command they describe (`authored-export.md`, not `AUTHORED_EXPORT.md` or `authoredExport.md`). <!-- check-links: allow -->
  A topic that needs several files gets a lowercase directory (`designs/`, later `agent-surface/`);
  do not nest further.
- Dated reports and audits: `docs/reports/YYYY-MM-DD-<slug>.md`, the date being when the measurement or
  audit was made. A document that is kept current and not tied to one date is a guide, not a report:
  `parity-named-equivalences.md`, and `real-world-trials.md`, a running log with several dated sections.
- Release notes: `docs/releases/<version>.md`, and `docs/releases/NEXT.md` for the draft of the next
  release. [`releasing.md`](releasing.md) and `AGENTS.md` name these paths; do not rename them.
- Repository-root convention files keep their names: `README.md`, `CONTRIBUTING.md`, `AGENTS.md`,
  `VALIDATION.md`.
- Renaming a doc: `git mv`, update every reference (links, `docs/….md` mentions in code, comments,
  tests and scripts), add `old -> new` to `scripts/docs/renamed.txt`, and add it to this index.
  The checker flags whatever is missed.
- Link to a section only with its GitHub slug (`#lowercase-words-with-hyphens`); the checker resolves
  it against the target's headings.

## Pending rename

These keep their current names for now because open pull requests name them by path. They are renamed in
a follow-up after those pull requests merge: `AGENT_SURFACE.md` to `agent-surface.md` (or
`agent-surface/overview.md` if the directory `agent-surface/` lands first) and `DISTRIBUTION.md` to
`distribution.md`. The checker exempts exactly these names.

## User guides

| Doc | What it covers | Status |
| --- | --- | --- |
| [export.md](export.md) | `mochi export`: TanStack Start source export, supported boundaries | implemented |
| [authored-export.md](authored-export.md) | `mochi export --authored-spec`: typed source from a reviewed semantic specification | implemented (bounded) |
| [semantic-conversion.md](semantic-conversion.md) | `mochi semantic`: Base UI pattern recognition, IR and emitters for one captured page | implemented |
| [lift.md](lift.md) | `mochi lift`: typed TypeScript from closed functions of a recovered controller | implemented |
| [tokens.md](tokens.md) | `mochi tokens`: central design-token styles for one capture | implemented, not run by default |
| [parity.md](parity.md) | `mochi check-parity`, the whole-page element pipeline and the parity contracts | implemented |
| [source-projects.md](source-projects.md) | Editable source projects: what the pipeline produces and what is still missing | partly implemented |
| [workspace.md](workspace.md) | The git-ignored `.mochi/` workspace: layout, evidence rules, `mochi clean`, ports | current |

## Internals and design

| Doc | What it covers | Status |
| --- | --- | --- |
| [project-model.md](project-model.md) | `mochi model-project`: the deterministic project model that feeds the emitters | implemented |
| [deterministic-reconstruction.md](deterministic-reconstruction.md) | What an authored export does and does not prove; the qualification rules | current |
| [semantic-interpretation.md](semantic-interpretation.md) | The optional model-assisted interpretation adapter and its contract | implemented locally; no live provider qualified |
| [parity-named-equivalences.md](parity-named-equivalences.md) | The differences the parity pipeline counts as minor, by name | current |
| [designs/authored-source.md](designs/authored-source.md) | Architecture for maintainable authored source | design; partly implemented |
| [designs/source-reconstruction.md](designs/source-reconstruction.md) | Decision map and plan for source reconstruction | design; plan of 2026-11-08 |
| [AGENT_SURFACE.md](AGENT_SURFACE.md) | A machine-readable, browser-free interface to a site | **design only, nothing implemented**; rename pending |
| [DISTRIBUTION.md](DISTRIBUTION.md) | Planned distribution channels and names | **decisions only, nothing published**; rename pending |

## Process

| Doc | What it covers |
| --- | --- |
| [releasing.md](releasing.md) | Releasing `mochi-web`: crate publication, GitHub release, validation |
| [releases/NEXT.md](releases/NEXT.md) | Draft notes of the next release (publication held) |
| [releases/0.3.0.md](releases/0.3.0.md) | Notes of the unpublished 0.3.0 candidate |

## Reports and trial logs

| Doc | What it covers |
| --- | --- |
| [real-world-trials.md](real-world-trials.md) | Log of the 2026-11-08 real-site reconstruction trials (Natural, Infinite Garden, Microsoft AI) |
| [reports/2026-11-10-parity-first-scoped.md](reports/2026-11-10-parity-first-scoped.md) | First scoped parity report of the whole-page element pipeline |
| [reports/2026-11-10-parity-default-recovery-and-semantic-assemble.md](reports/2026-11-10-parity-default-recovery-and-semantic-assemble.md) | Parity of default-recovery compat exports and a semantic assemble output |
| [reports/2026-11-10-parity-semantic-final-outputs.md](reports/2026-11-10-parity-semantic-final-outputs.md) | Parity of the semantic assemble outputs of the three example sites |
| [reports/2026-11-11-workspace-audit.md](reports/2026-11-11-workspace-audit.md) | Audit of the `.mochi/` workspace and the layout/prune plan |
| [reports/2026-11-11-orchestration-handoff.md](reports/2026-11-11-orchestration-handoff.md) | Handoff: state of the PRs, sessions, workstreams and pending decisions at the end of 2026-11-11 |

## Overlaps worth knowing about

Not merged, only noted: [source-projects.md](source-projects.md), [export.md](export.md),
[authored-export.md](authored-export.md), [designs/authored-source.md](designs/authored-source.md) and
[designs/source-reconstruction.md](designs/source-reconstruction.md) all describe the source-export track and
link to each other; [real-world-trials.md](real-world-trials.md) overlaps `VALIDATION.md` at the repository root
(trial narrative versus validation record).
