# Next mochi-web release — working draft

**Unpublished.** The version and release commit are not selected yet. Full semantic
conversion of the real-site examples and final release validation remain in
progress. This file is preparation for the matching crates.io and GitHub release;
it is not a claim that the work below has shipped.

Changes are being collected from `mochi-web-v0.2.0` onward.

## Capture and reconstruction

- Rendered DOM analysis identifies repeated structure and page-template candidates
  using deterministic subtree fingerprints.
- Source evidence records responsive observations, computed styles and custom
  properties, readable stylesheet rules, fonts, controls, resources, and screenshots.
- Offline evidence validation and project modeling produce traceable route,
  component, asset, and styling candidates for reconstruction.
- Bounded concurrent asset downloads and a validated response cache reduce repeated
  fetches. Cached bodies require server revalidation; there is no stale fallback.

## Editable source export

- TanStack Start export uses file routes and a Bun-based toolchain.
- The authored export path accepts a reviewed, evidence-bound specification for
  typed components and content, separate feature/layout/composition folders, and
  direct Base UI primitives. `components/ui` is reserved for those primitives.
- Tailwind v4, scoped CSS Modules, semantic theme aliases, and named local fonts
  provide an owned styling foundation.
- Motion support includes reduced-motion preferences and owned behavior hooks.
  Reviewed source modules can implement custom graphics and algorithms, with
  explicit imports and optional Three.js support.
- Authored specifications and reports live under the generated project's `.mochi/`
  directory. Validation checks evidence references and the declared module graph.

These capabilities do **not** mean a capture is automatically converted into a
complete semantic application. Authored export requires interpretation and a
reviewed specification. Compatibility export and authored export remain distinct.

## Interpretation

- Planning, replay, response validation, and a bounded TypeSafe adapter support
  reviewed classification of observed component and text roles.
- Jev does not author components or replace controllers. No live Jev qualification
  or provider billing reconciliation has been performed for this release draft.
- Live requests require explicit request, token, cost, price, and deadline limits;
  ordinary capture and export do not invoke a paid model.

## Migration and installation — finalize before release

- The crate is `mochi-web`; the CLI and Rust library stay `mochi`.
- Existing captures and exports are not rewritten in place. Generate a new project
  and review changes before migrating edits.
- The frontend toolchain uses Bun. Record the finalized pinned versions and any
  changed defaults here when preparing the versioned notes.
- Add the exact `cargo install mochi-web --version <version> --locked` command,
  source commit, comparison URL, and links to the relevant usage documentation.

## Pending work and validation — not release claims

- Finish semantic conversion and behavior/visual verification across all nine
  captured Natural, Infinite Garden, and Microsoft AI routes.
- Finish removing opaque source names and active recovered-controller dependencies;
  keep provenance and retired compatibility artifacts under dot directories.
- Verify the CLI produces the intended source structure, rather than relying on
  hand-edited ignored trial outputs.
- Run the complete Rust and browser checks against the final release commit,
  frozen Bun installs, generated-project builds/typechecks, and package dry run.
- Replace this section with exact results and honest remaining limitations. Do not
  publish these pending items as completed features.
- After publication, verify a fresh registry install and the matching GitHub
  release and release-build workflow.
