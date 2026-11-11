# Named equivalences of the parity pipeline

An equivalence is a difference the pipeline counts as **minor** (reported by name and count in every report, never silent)
and the Rust gate sees as equal. Everything not listed is compared exactly (geometry within 1 CSS px). Implemented in
`benchmarks/src/parity/diff.ts` (`STYLE_RULES`, `comparePair`, `baseUiAddedAria`) and `facts.ts`; asserted by
`benchmarks/tests/parity-diff.test.ts`, including the check that the gate's facts differ exactly when the report has a major item.

| Rule | What counts as equal |
|---|---|
| `gradient-direction` | `linear-gradient(to top, A, B)` ≡ `linear-gradient(B, A)` |
| `zero-position-unit` | `0%` ≡ `0px` in `background-position`, `mask-position`, `object-position`, `transform-origin` |
| `href-trailing-slash` | hrefs equal after removing the site origin and a trailing slash |
| `redundant-tabindex-0` | `tabindex="0"` added to a natively focusable element (not an `<a>` without `href`) |
| `container-retag` | the tag of a `div`/`section`/`article`/`span` becomes another of them, no `role` attribute on either side |
| `added-component-attributes` | the candidate only **adds** component-state `data-*` (`data-base-ui-*`, `data-slot`, `data-state`, `data-open`, `data-closed`, `data-orientation`, `data-index`, `data-pressed`, …) |
| `base-ui-added-aria` | the candidate only **adds**, with the same role on both sides: `aria-expanded="false"`, `aria-disabled="false"` or `aria-haspopup` (any value but `false`) to an element whose role is `button` (a `<button>`/`<summary>` without an explicit role, or `role=button`) or an `<a>` with role `link`; or the implicit values of a `status` (polite + atomic true), `alert` (assertive + atomic true) or `log` (polite + atomic false) region, only when the reference had neither `aria-live` nor `aria-atomic` |

`base-ui-added-aria` was agreed with the tokens/styles session as the narrow form of "Base UI adds these on purpose" and narrowed further after an
adversarial review (values are checked, a button with role `tab`/`combobox`/`menuitem`/`switch` never qualifies, and the live-region part requires a
real live-region role with its implicit values). In the gate, only the equivalent additions are removed from the candidate's `aria` fact, so any other
ARIA difference on the same element still fails.

## Deliberately NOT equivalences (reported as real differences)

* Any ARIA **removal** or **value change** (`aria-modal`, `aria-hidden`, `aria-label`, `aria-selected`, `aria-checked`, `aria-current`,
  an `aria-expanded` that flips), and any other ARIA attribute added (`aria-hidden`, `aria-label`, `aria-selected`, …).
* `aria-expanded` / `aria-haspopup` / `aria-disabled` added to anything whose role is not `button` or a link (including `<button role="tab">`), or with a
  value that is not harmless (`aria-disabled="true"`, `aria-expanded="true"`, `aria-haspopup="false"`); `aria-live` without a `status`/`alert`/`log` role or with
  a non-implicit value.
* Role or tag swaps (`a`→`div`, `tab`↔`link`, `heading`→`link`, `switch`→`combobox`, `div`→`aside`), including when an equivalent
  ARIA attribute is added in the same swap.
* `tabindex="-1"` added (roving focus): it removes the element from the tab order, so it stays reported unless the Tab sequence is shown to match.
* Removed or renamed `data-*` that the original's CSS selects on. Note: the original sheets select on `[data-state]`, `[data-pressed]`,
  `[data-expanded]`, `[data-side]`, `[aria-expanded]`, `[aria-current]`, `[aria-checked]`, `[aria-selected]` and many component
  attributes (counted in the captured stylesheets: Natural `[data-state]` 13, `[data-pressed]` 14, `[aria-expanded]` 14; Garden
  `[data-state]` 28, `[aria-current]` 13). The styles translation resolves such selectors at capture time, so they do not break
  translated styles, but **script-toggled state styling** (open / pressed / current) is a gap that this pipeline only sees where a
  state is actually scripted.

## Effect measured on retained runs (rediff with the rule; same captures)

Items reclassified major → minor, over the 30-state set and over all executed states of each run:

| Run | 30 states | all executed states |
|---|---:|---:|
| Natural (`natural-sem-final-core-1`) | 180 (`aria-expanded` 80, `aria-disabled` 74, `aria-haspopup` 26) | 489 |
| Garden (`garden-sem-final-core-1`) | 0 | 0 |
| Microsoft AI (`msai-sem-final-core-1`) | 22 (`aria-haspopup` 17, `aria-expanded` 5) | 74 |

The live-region part matched 0 times in all three runs (Microsoft AI's live regions appear on elements that were paired with a role swap, e.g.
`div[region]` ↔ `div[status]`, which is reported as a role difference). 50 of Natural's 124 `aria-disabled` additions are on `<button role="tab">`
and stay major. Microsoft AI's other candidate-only additions are on role-swapped pairs (`input[switch]` ↔ `input[combobox]`), not on other element
kinds. No state's status changed.

Selector counts above were taken from the captured stylesheets and inline `<style>` blocks under each capture's `site/` directory
(`.mochi/evidence/trials/20261108-normalized/{natural,infinitegarden}/capture`, `.mochi/evidence/trials/20261108-microsoft-ai/capture-modules`), not from the projects.
