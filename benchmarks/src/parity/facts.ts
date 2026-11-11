// Emit `mochi check-parity` inputs (src/parity.rs schema 1) from paired collections.
// The Rust gate stays the authority on exact, scalar, contract-bound comparison; this
// file only flattens the aligned element lists into anchors and facts. Named minor
// equivalences (diff.ts STYLE_RULES, href origin/trailing slash, redundant tabindex=0)
// are canonicalised on BOTH sides, so the gate sees exactly what the TS report calls
// "identical or minor"; their counts stay visible in the TS report. tests/parity-diff.test.ts asserts
// that the gate differs exactly when the TS report has a major item, over a catalogue of mutations.
import { baseUiAddedAria, canonStyle, comparePair, focusSig, isInvisibleShell, stripOrigin, trimSlash, type Collection, type El, type Options, type Result } from './diff';
import type { StateSpec } from './state';

type Fact = string | number | boolean;
export interface GateState { route: string; viewport: [number, number]; reduced_motion: boolean; inventory: string[]; facts: Record<string, Fact>; errors: string[]; settled: boolean }
const GEO: [keyof El, string][] = [['x', 'x'], ['y', 'y'], ['w', 'width'], ['h', 'height']];

const sortedJson = (o: Record<string, string> | undefined) => JSON.stringify(Object.entries(o ?? {}).sort(([x], [y]) => (x < y ? -1 : 1)));

function elFacts(col: Collection, e: El, o: Options): Record<string, Fact> {
  const f: Record<string, Fact> = {};
  for (const [k, n] of GEO) f[`bounds.${n}`] = e[k] as number;
  const sty = col.styleTable[e.sty ?? -1] ?? {};
  f.tag = e.tag;
  if (e.tag.startsWith('::')) {
    f.pcontent = e.pcontent ?? '';
    f['style.pseudo'] = JSON.stringify(Object.fromEntries(Object.entries(e.pst ?? {}).map(([p, v]) => [p, canonStyle(p, v)])));
    f.assets = JSON.stringify({ bg: e.bgSha ?? [], mask: e.maskSha ?? [] });
    return f;
  }
  f.role = e.role ?? ''; f.name = `${e.name ?? ''}|${e.nameHash ?? ''}`; f.text = (e.text ?? '').replace(/\s+/g, ' ') + '|' + (e.textHash ?? '');
  const groups: Record<string, Record<string, string>> = {};
  const { STYLE_GROUPS } = GROUPS;
  for (const [g, props] of Object.entries(STYLE_GROUPS)) { const o2: Record<string, string> = {}; for (const p of props) if (p in sty) o2[p] = canonStyle(p, sty[p]); groups[g] = o2; }
  for (const [g, v] of Object.entries(groups)) f[`style.${g}`] = JSON.stringify(v);
  f.effective = `${e.effOpacity}|${e.effFx}|${e.effHidden}|${e.inert}`;
  const sorted = sortedJson;
  f.aria = sorted(e.aria); f.ariaRefs = e.ariaRefs ?? ''; f.data = sorted(e.data);
  f.tabindex = e.tabindex ?? ''; // the redundant-tabindex-0 equivalence is applied per pair below
  // undefined and '' differ in the TS comparison, so they differ here too (JSON turns undefined into null)
  f.form = JSON.stringify([e.type, e.disabled, e.placeholder, e.required, e.checked, e.value, e.open, e.modal].map(x => (x === undefined ? null : x)));
  if (e.tag === 'a') f.link = JSON.stringify([trimSlash(stripOrigin(e.href, o.siteOrigins ?? [])), e.target ?? '', e.rel ?? '', e.download ?? false]);
  f.media = JSON.stringify([e.src ?? '', e.srcSha ?? '', e.nat ?? '', e.complete ?? '', (e.alt === undefined ? null : e.alt), e.bgSha ?? [], e.maskSha ?? [], e.loading ?? '', e.srcsetPresent ?? '', e.poster ?? '', e.paused ?? '', e.muted ?? '', e.loop ?? '']);
  f.graphics = JSON.stringify([e.vb ?? '', e.par ?? '', e.svgAttrW ?? '', e.svgAttrH ?? '', e.svgCount ?? '', e.svgHash ?? '', e.paths ?? [], e.svgPaint ?? '', e.svgAnimated ?? '', e.cw ?? '', e.ch ?? '', e.pixelHash ?? '', e.pixelBlank === undefined ? null : e.pixelBlank]);
  // same bytes under another file name are equal: compare the content hash, not the path
  if (e.srcSha && e.srcSha !== 'unhashed') f.media = JSON.stringify(['', e.srcSha, e.nat ?? '', e.complete ?? '', (e.alt === undefined ? null : e.alt), e.bgSha ?? [], e.maskSha ?? [], e.loading ?? '', e.srcsetPresent ?? '', e.poster ?? '', e.paused ?? '', e.muted ?? '', e.loop ?? '']);
  return f;
}
import { GROUPS } from './groups';

/** Aligned reference/candidate states for the Rust gate. Candidate-only anchors carry an `x` prefix. */
export interface StateExtras { focusOrder: unknown[]; stepErrors: string[]; reachedScrollY: number }
export function gateStates(spec: StateSpec, ref: Collection, cand: Collection, d: Result, o: Options, refErrors: string[], candErrors: string[], settled: [boolean, boolean], extras: [StateExtras, StateExtras] = [{ focusOrder: [], stepErrors: [], reachedScrollY: 0 }, { focusOrder: [], stepErrors: [], reachedScrollY: 0 }]): [GateState, GateState] {
  const rf: Record<string, Fact> = {}, cf: Record<string, Fact> = {};
  const ra = new Set<string>(), ca = new Set<string>();
  const put = (anchor: string, col: Collection, e: El, into: Record<string, Fact>, set: Set<string>) => { set.add(anchor); for (const [k, v] of Object.entries(elFacts(col, e, o))) into[`${anchor}.${k}`] = v; };
  const idx = (n: number, p: string) => p + String(n).padStart(5, '0');
  const mapR = new Map(d.matches.els.map(([i, j]) => [i, j])), mapP = new Map(d.matches.pseudos.map(([i, j]) => [i, j]));
  const usedC = new Set<number>(), usedPC = new Set<number>();
  ref.els.forEach((e, i) => {
    const a = idx(i, 'e'); put(a, ref, e, rf, ra);
    const j = mapR.get(i);
    if (j === undefined) return;
    put(a, cand, cand.els[j], cf, ca); usedC.add(j);
    // pair-aware named equivalences: where the TS report counts tag/role/data as minor, the gate sees the reference value
    for (const it of comparePair(ref, cand, e, cand.els[j], o)) {
      if (it.severity !== 'minor') continue;
      const key = it.prop === 'tag' ? 'tag' : it.prop === 'role' ? 'role' : it.prop === 'data-attributes' ? 'data' : it.prop === 'tabindex' ? 'tabindex' : '';
      if (key) cf[`${a}.${key}`] = rf[`${a}.${key}`];
    }
    // base-ui-added-aria: drop only the equivalent additions from the candidate's aria fact; any other aria difference stays
    const dropped = Object.keys(cand.els[j].aria ?? {}).filter(n => baseUiAddedAria(n, e, cand.els[j]));
    if (dropped.length) cf[`${a}.aria`] = sortedJson(Object.fromEntries(Object.entries(cand.els[j].aria ?? {}).filter(([n]) => !dropped.includes(n))));
  });
  ref.pseudos.forEach((e, i) => { const a = idx(i, 'p'); put(a, ref, e, rf, ra); const j = mapP.get(i); if (j !== undefined) { put(a, cand, cand.pseudos[j], cf, ca); usedPC.add(j); } });
  cand.els.forEach((e, j) => { if (!usedC.has(j) && !isInvisibleShell(e)) put(idx(j, 'x'), cand, e, cf, ca); }); // same shell rule as the TS report
  cand.pseudos.forEach((e, j) => { if (!usedPC.has(j)) put(idx(j, 'xp'), cand, e, cf, ca); });
  const pageFacts = (col: Collection, into: Record<string, Fact>, set: Set<string>) => { set.add('page'); for (const [k, v] of Object.entries(col.facts)) if (!k.startsWith('info.')) into[`page.${k}`] = typeof v === 'number' || typeof v === 'boolean' || typeof v === 'string' ? v : JSON.stringify(v ?? ''); };
  pageFacts(ref, rf, ra); pageFacts(cand, cf, ca);
  // state-level behavior the element lists cannot show: tab order with focus ring, reached scroll offset
  const ex = (x: StateExtras) => ({ focus: JSON.stringify(x.focusOrder.map(f => focusSig(f as Record<string, unknown>))), scroll: x.reachedScrollY }); // exactly what the TS comparison looks at
  rf['page.state.focusOrder'] = ex(extras[0]).focus; cf['page.state.focusOrder'] = ex(extras[1]).focus;
  rf['page.state.reachedScrollY'] = extras[0].reachedScrollY; cf['page.state.reachedScrollY'] = extras[1].reachedScrollY;
  const base = { route: spec.route, viewport: spec.viewport, reduced_motion: spec.reducedMotion };
  // failed interaction steps are browser errors for the gate: a state whose steps did not run cannot pass
  const errs = (list: string[], x: StateExtras) => [...list, ...x.stepErrors.map(e => 'step: ' + e)];
  return [{ ...base, inventory: [...ra].sort(), facts: rf, errors: errs(refErrors, extras[0]), settled: settled[0] }, { ...base, inventory: [...ca].sort(), facts: cf, errors: errs(candErrors, extras[1]), settled: settled[1] }];
}

/** Fact keys on which the Rust gate would report a difference between two aligned states (bounds within 1 px, everything else exact). */
export function gateDifferences(a: GateState, b: GateState): Set<string> {
  const out = new Set<string>();
  for (const k of new Set([...Object.keys(a.facts), ...Object.keys(b.facts)])) {
    const x = a.facts[k], y = b.facts[k];
    if (typeof x === 'number' && typeof y === 'number' && /\.bounds\.(x|y|width|height)$/.test(k)) { if (Math.abs(x - y) > 1) out.add(k); continue; }
    if (x !== y) out.add(k);
  }
  const ai = new Set(a.inventory);
  for (const k of b.inventory) if (!ai.has(k)) out.add('inventory:' + k);
  for (const k of a.inventory) if (!b.inventory.includes(k)) out.add('inventory:' + k);
  return out;
}
