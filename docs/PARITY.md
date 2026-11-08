# Reconstruction parity contracts

A successful export/build is not proof that a reconstruction matches its source.
`mochi check-parity` compares saved browser measurements against a reviewed state
contract. It returns a failure for changed runtime inputs, omitted routes, missing
states or anchors, browser errors, unsettled measurements, or differing facts.
It never upgrades an export report automatically to claim whole-site parity.

Keep contracts, captures, reports and reference material in `.mochi/parity/`.
Keep actual components, styles, fonts, SVG geometry and shaders in `src/`.

## Capture procedure

1. Inventory every captured route, section, graphic, control and state transition.
   Record unknown behavior as a coverage gap. Do not remove failing probes to pass.
2. Set the viewport and reduced-motion preference **before navigation**. Wait for
   fonts and assets. Use identical browser versions, zoom and device pixel ratio.
3. Record initial, intermediate scroll, bottom, reduced-motion, and every supported
   interaction state. Include scroll entry/exit, reverse scrolling, resize and
   keyboard behavior. Test desktop and mobile for every route. The minimum matrix
   enforced by the CLI does not substitute for this behavior inventory.
4. Measure source and candidate using reviewed semantic selectors. The browser
   expression in `scripts/parity/measure.js` captures geometry, typography, colors,
   content, ARIA state, links, image loading and SVG paths. A selector must match
   exactly one element. Collect console/network failures separately and append them
   to `errors`; a DOM expression cannot recover console errors from before it ran.
5. Use source equations and DOM measurements to fix discrepancies. Do not tune CSS
   against screenshots. Test graphics at an explicit identical time/progress; a
   matching canvas rectangle says nothing about shader output or drawn geometry.
6. Rebuild and recapture after every runtime-input change. Re-run the full matrix
   after repairs. Preserve failing reports and the original immutable reference.

## Offline gate

```sh
mochi fingerprint-project .mochi/projects/example
mochi check-parity .mochi/projects/example \
  --contract .mochi/parity/contract.json \
  --reference .mochi/parity/reference.json \
  --candidate .mochi/parity/candidate.json \
  --out .mochi/parity/report.json
```

The contract is schema version 1 with `reference_sha256` (SHA-256 of exact reference
JSON bytes), `project_sha256` (from fingerprint-project), `routes`, `states`, and
`gaps`. State names are stable descriptions, such as `home.desktop.menu.open`.
Each state declares `route`, `[width,height]` in `viewport`, `reduced_motion`,
`phase` (`initial`, `scroll`, `interaction`, or `bottom`), an `inventory` of semantic
anchor names, and numeric `tolerances` for `.bounds.*` geometry only. Tolerances
cannot exceed one CSS pixel;
unlisted facts, strings and booleans compare exactly. Do not use pixel tolerance
for unitless animation progress, matrices, colors or path equations.

Reference and candidate captures have `schema_version`, `project_sha256` (null for
reference), and `states`. Each state contains its browser context, `inventory`,
flattened scalar `facts`, `errors`, and `settled`. Every anchor needs measurements.
Both captures must contain exactly the contract's states and facts. The current
literal TanStack route declarations are parsed from TypeScript/JavaScript ASTs
and must match the route inventory. A TanStack index-route ID’s trailing slash
is normalized for this identity check only; recorded browser URLs remain exact.
Comments cannot stand in for route declarations. Source edits
and newly added files invalidate the candidate fingerprint.

The output path must be new for every invocation. Invalid inputs return nonzero
and produce no report; never reuse an earlier successful artifact.

The result is `matched_declared_contract` or `failed`. It is evidence for exactly
the recorded inputs and states, not proof about unobserved behavior, browser/GPU
combinations, capture authenticity, or other websites. Coverage must be independently
reviewed. Every new site remains unqualified until its own contract passes; known
gaps and missing evidence cannot be replaced by a successful build or a screenshot.
