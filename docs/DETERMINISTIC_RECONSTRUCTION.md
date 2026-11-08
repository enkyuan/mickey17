# Deterministic reconstruction and qualification

An authored export proves that reviewed source can be emitted with a closed module
and asset graph. It does not prove that its layout or behavior matches the source.
`authored-report.json` therefore keeps `verification: "not_run"`. Do not replace
this with a success label because a build passes or a screenshot looks plausible.

## Establish the reference

Use the captured DOM, computed styles, local stylesheet rules, fonts, and inspected
animation equations. Record the capture fingerprint, viewport in CSS pixels, device
scale, font readiness, route, scroll position, motion preference, and control state.
Compare the same state on both sides. Wait for the relevant state transition to
finish, rather than measuring a frame in the middle of it. Retain reference data
under `.mochi/`, separate from the editable application.

Source rectangles can include transforms. Preserve the coordinate system: document,
viewport, local element, SVG viewBox, and canvas backing pixels are different units.
For animated cards, inspect the final layout or controller equation before treating
a measured rectangle as a static position. Preserve responsive topology as well as
coordinates; desktop and mobile diagrams can have different nodes and edges.

## Convert ownership, not snapshots

Routes compose typed pages; feature components own their content and state.
`components/ui` contains direct Base UI exports. Navigation, dialogs, forms, tabs,
and accordions compose those exports with site styles. Repeated items need unique,
stable keys even when labels repeat. A viewport transition must remove obsolete
nodes rather than leave detached desktop cards in the mobile layout.

Move font declarations and shared design tokens into their owned stylesheets.
Translate selector relationships when splitting CSS Modules: a class from one
module cannot silently depend on an unimported class from another. Preserve media
queries and compact variants. Keep local state variables next to the feature.

For graphics, retain typed geometry, shader source, and explicit rendering resource
lifecycles. Normalize units once. SVG dash lengths must use the same units as the
rendered path; mixing normalized path lengths, scaling, and non-scaling strokes can
leave visible gaps even when the path endpoint is correct. Frame loops must respond
to reduced motion, visibility, resizing, and unmounting.

## Assert the contracts

Use numerical assertions before screenshots:

- Compare panel, container, row and column rectangles at fixed viewport widths.
  Compare typography and CSS values exactly; allow a documented subpixel tolerance
  for browser rounding, not an unexplained visual allowance.
- Evaluate original and reconstructed geometry at fixed progress values. Check
  endpoints, node counts, bounds, transforms, opacity, and reveal completion.
- Exercise state transitions: keyboard activation, selection, sorting, expansion,
  dialog dismissal, focus return, scroll locking, and reduced-motion fallback.
- Inspect routes at desktop and mobile widths for missing landmarks, broken media,
  horizontal overflow, duplicate keys, hydration errors, and missing content.
- Re-export to a new directory, install the frozen Bun lock, build, typecheck, and
  run the assertions against that output. Keep compatibility/provenance out of its
  import graph. Test an ordinary content/token edit without re-running capture.

The November 2026 trial checks include Natural's full-width menu at 1100, 1440,
and 1960 CSS-pixel widths, and Infinite Garden's footer geometry. The menu checks
compare panel, container, groups, rows, and typography with the live reference.
The flower checks evaluate all 22 rib endpoints at four sizes and compare browser
layout with the captured viewport-pinning equations. These are specific contracts;
they do not establish whole-site pixel equivalence or exhaustive behavior coverage.

Screenshots are retained as review evidence after these checks. Do not adjust
margins, font sizes, or animation constants merely to make one screenshot line up.
A mismatch should be traced to a source rule, coordinate transform, data item,
state transition, or capture gap, and the corresponding contract should be rerun.
