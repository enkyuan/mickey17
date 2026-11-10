# Semantic conversion: IR, Base UI patterns, emitters

`mochi semantic` turns **one captured page** into a report of every Base UI
pattern it can prove, and with `--emit` into typed TSX that uses direct Base UI
primitives only under `src/components/ui/`. It does not change `mochi export`.

```sh
mochi semantic .mochi/trials/<run>/natural/capture --out /new/dir --emit
#   report.json  report.md  project/src/{components,components/ui,data,types}/
```

`--page N` picks a manifest page, `--viewport 1440x900` the evidence file
(default: the widest), `--no-js` ignores the original bundles. The output
directory must not exist. Emitted files are a project **fragment**, not a
runnable app: merge them into an exported project (`@/` alias, React 19,
`@base-ui/react` 1.9.0).

## Pipeline (all deterministic, AST/IR based; no regex or string patching)

| Stage | Module | Evidence |
| --- | --- | --- |
| IR | `semantic::ir` | captured DOM + computed style; effective role (HTML-AAM table), accessible name (labelledby > label > native > content > title), ARIA state, inert/hidden. Node ids are capture node indices. |
| Script facts | `semantic::js` | oxc AST of the page's own bundles (`capture.resources`, same host): `useState` pairs, `jsx(..)` call sites with static props, class tokens (state-dependent ones separated), handlers resolved through symbol tables (toggle, set key, toggle key), `.map` loops with static item lists, `Escape` listeners, `matchMedia` listeners with their guard, scroll-lock assignments, `useEffect` dependency effects. React Compiler memo-cache reads are not treated as definitions. |
| Linking | `semantic::patterns::Links` | DOM element ↔ JSX site by shared class tokens, literal props, literal text; component call sites kept apart from the intrinsic element inside; wrappers without classes inherit the parent site of linked children. Ties are *ambiguous* and link nothing. |
| Recognition | `semantic::patterns` | `RULES` table: candidate selector + required / supporting / contra (veto) checks with weights. Confidence = 0.5 + 0.5·(supporting − contra)/Σsupporting once all required checks pass. First accepting rule claims an element. |
| Repeats | `semantic::components` | Merkle hash (tag, role, non-state classes, attribute names, child hashes; text/values ignored) → instances; lock-step walk gives text/attribute/class-variant slots; Zhang–Shasha tree edit distance reports near duplicates. Names come from `data-component`, BEM classes (utility classes are rejected), link/button labels, headings, an ancestor `data-component`; otherwise the group is reported, **not** extracted. |
| Emit | `semantic::emit` | `ui/<primitive>.tsx` wrappers, compositions, data modules, repeated-component files, `types/custom-elements.d.ts`. |

## Rule table (what maps to what)

| Rule | Base UI | Main evidence |
| --- | --- | --- |
| `dialog.role`, `alert-dialog.role` | `Dialog` / `AlertDialog` | role, `aria-modal`, fixed layering, inert while closed, bundle state. Trigger from `aria-controls`/`aria-owns` or from the handler that toggles the bundle's open state. Escape, scroll lock, focus trap are *provided*; media-query/route resets stay app code. |
| `tabs.tablist` | `Tabs` | tablist + tabs, `aria-controls` panels, one selected. Link tabs render as `<a>`. |
| `toggle-group.pressed`, `toggle.pressed` | `ToggleGroup` / `Toggle` | sibling `aria-pressed` buttons. |
| `accordion.disclosure-group`, `collapsible.disclosure` | `Accordion` / `Collapsible` | `aria-expanded` + panel (`aria-controls`, labelledby, adjacent sibling); single/multiple and "open item cannot be closed" from the handler (`SetKey` vs `ToggleKey`). |
| `navigation-menu.hover-state` | `NavigationMenu` | pointer-enter handlers that set one state to distinct keys. |
| `navigation-menu.popup-links` | `NavigationMenu` | `aria-haspopup` triggers inside a navigation landmark whose popups are mostly links (recognized; **not emitted yet**). |
| `popover.haspopup`, `menu.role`, `menubar.role`, `tooltip.role` | `Popover`, `Menu`, `Menubar`, `Tooltip` | recognized; no composition emitter yet (the `ui` wrapper is). |
| `switch.role`, `checkbox.role`, `radio-group.*` | `Switch`, `Checkbox`, `RadioGroup` | roles / native inputs. |
| `combobox.editable`, `select.combobox` | `Combobox`, `Select` | editable vs non-editable combobox, native `<select>`. |
| `field.native`, `button.native` | `Field`, `Button` | native form controls and buttons. |

Every interactive element no rule accepts is listed under *Unrecognized* with
the reason and the rules that nearly matched.

## Measured (page 0, 1440×900, initial state)

| Site | Controls | Bound to a primitive | Notes |
| --- | ---: | --- | --- |
| Natural | 102 (19 buttons, 83 links) | all 19 buttons: Dialog 1, Tabs 7 (2 lists), ToggleGroup 4, Accordion 3, NavigationMenu 3, Button 1 | the compat export used one primitive (Button) |
| Infinite Garden | 31 | Accordion 3, Button 18 | this page has 3 `aria-expanded` triggers; other pages were not analyzed |
| Microsoft AI | 251 | Tabs 5, Switch 2, Combobox 2, NavigationMenu 4, Accordion 2, Collapsible 1, Button 6 | no JSX bundle: DOM/ARIA evidence only |

These are counts of recognized controls, not fidelity claims. The emitted
fragments for the three pages pass `tsc --strict` against `@base-ui/react`
1.9.0; none has been rendered or compared with the original (parity belongs to
`mochi check-parity`).

## Checking the emitted TSX

The fragments are type-checked outside the test suite (needs an installed export for
`node_modules`): copy the emitted `src/` beside a `node_modules` containing
`typescript`, `react`, `@types/react` and `@base-ui/react@1.9.0`, add a `tsconfig.json`
(`strict`, `jsx: react-jsx`, `moduleResolution: Bundler`, `paths: {"@/*": ["./src/*"]}`)
and run `tsc --noEmit`. The three example pages pass. This proves the output type-checks
against the Base UI typings, not that it behaves like the original.

## Known gaps (kept explicit in the report)

- Initial state of one viewport only; states the capture never entered are
  known only through recovered scripts.
- Only React/JSX bundles yield script facts; hand-written DOM scripts do not.
- Tabs without `tabpanel` (Natural) keep panel content outside the component.
- Hover-menu dropdown bodies are rendered by script and absent from the capture.
- Trigger icons that swap with state, pointer-intent timing: not recovered. Links inside
  a nested accordion that close the dialog get an `onNavigate` prop; close handlers with no
  linked element, or inside repeated-structure components, are listed as gaps.
- Accordion without a heading emits no `Accordion.Header`; the runtime behavior
  of that arrangement is not exercised.
- Original class names are kept (state modifiers dropped and listed) until the
  styles workstream plugs a `StyleBridge` backed by `semantic::tokens`.
- Utility-class detection for naming is a prefix/digit heuristic.
- `popup-links` navigation, Popover, Menu, Tooltip, Select, Combobox, Checkbox
  and Field are recognized but have no composition emitter yet.

## Later milestones and file boundaries

| Milestone | New files only | Reads |
| --- | --- | --- |
| Motion translation (scroll, entrance, state animations → `motion`) | `src/semantic/motion/**` | `semantic::js` call sites and `Ex`; CSS animations from the IR style map |
| Canvas/WebGL → named modules with owned shader source, `mochi lift` geometry | `src/semantic/graphics/**` | `canvas` nodes, `surfaces`, `lift` manifests |
| Reconstructed assets | `src/semantic/assets/**` | capture `site/` and `resources` |
| Styles bridge | `impl StyleBridge` in `src/semantic/tokens/bridge.rs` | `semantic::tokens::TokenSet` |
| Remaining composition emitters | `src/semantic/emit/{popover,menu,select,...}.rs` | `semantic::patterns::Match` |
