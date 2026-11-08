# Next mochi-web release — publication held

The prepared [0.3.0 notes](0.3.0.md) are unpublished. Whole-page parity regressions
in the Natural, Infinite Garden and Microsoft AI qualification projects remain open. Passing
isolated geometry, route health, and build checks did not establish complete
semantic conversion. Do not publish until the missing source structure, scroll
choreography, and responsive variants are restored and qualified.

Every crates.io publication must have a matching GitHub release at the same package
commit; follow [RELEASING.md](../RELEASING.md).

Additional unpublished changes:

- Reconstruction benchmark suite with separate measurement categories, missing-case
  denominators, exact pixel differences, repeatability checks, and source/build hashes.
- Calibrated regression fixtures, including one-pixel, typography, asset, SVG,
  behavior, route, overlap and uncaught-exception failures.
- React Doctor scanning against explicit generated source files; incomplete scans
  cannot pass. Existing-project measurements are distinguished from fresh CLI output.
- Offline parity contracts reject missing states, changed inputs, unresolved coverage
  and reused report paths.
- Explicit Motion import policy and a stable shared preference context. Transitive
  upstream dependencies retain their package names; generated application imports
  use `motion`, `motion/react`, or `motion/react-m`.

These checks expose unresolved regressions. They are not a whole-site accuracy
claim and do not remove the publication hold.
