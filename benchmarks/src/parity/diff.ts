// Bidirectional comparison of two scripts/parity/collect.js captures.
// Pass 1 pairs text-bearing elements, pass 2 pairs everything else by kind and
// geometry, and the reverse pass re-examines what is left on the candidate side
// so an element that moved far is reported as moved, not as missing plus extra.
// Every unpaired element on either side is a failure. Nothing is dropped.

export type Dimension = 'layout' | 'typography' | 'styling' | 'motion' | 'content' | 'assets' | 'graphics' | 'behavior' | 'routing' | 'semantics' | 'interaction';
export type Severity = 'major' | 'minor';

export interface El {
  tag: string; role?: string; roleExplicit?: boolean; textHash?: string; nameHash?: string; name?: string; path?: string; cls?: string; text?: string;
  x: number; y: number; w: number; h: number; sty?: number;
  effOpacity?: number; effFx?: string; effHidden?: boolean; inert?: boolean;
  aria?: Record<string, string>; ariaRefs?: string; data?: Record<string, string>; tabindex?: string;
  href?: string | null; hrefPath?: string; target?: string; rel?: string; download?: boolean;
  type?: string; disabled?: boolean; placeholder?: string; required?: boolean; checked?: boolean; value?: string; open?: boolean; modal?: boolean;
  src?: string; alt?: string | null; nat?: string; complete?: boolean; loading?: string; srcsetPresent?: boolean; srcSha?: string; bgSha?: string[]; maskSha?: string[]; poster?: string;
  paused?: boolean; muted?: boolean; loop?: boolean; currentTime?: number;
  vb?: string; par?: string; svgAttrW?: string; svgAttrH?: string; svgCount?: number; svgHash?: string; paths?: string[]; svgPaint?: string; svgAnimated?: boolean;
  cw?: number; ch?: number; pixelHash?: string | null; pixelBlank?: boolean | null; pixelError?: string;
  pcontent?: string; host?: string; pst?: Record<string, string>;
}
export interface Collection {
  schema: number; context: Record<string, unknown>; active?: Record<string, unknown>;
  facts: Record<string, unknown>; styleTable: Record<string, string>[]; els: El[]; pseudos: El[]; limits: string[];
  assets?: { hashed: number; unhashed: number };
}
export interface DiffItem { dim: Dimension; prop: string; ref: unknown; cand: unknown; severity: Severity; rule?: string }
export interface Pair { ref: Brief; cand: Brief; items: DiffItem[]; delta: number }
export interface Brief { i: number; tag: string; text: string; path: string; x: number; y: number; w: number; h: number }
export interface Options {
  /** Origins of the live site; stripped from hrefs so a mirror host is not a difference. */
  siteOrigins?: string[];
  /** CSS pixel tolerance for x/y/w/h. The Rust gate caps this at 1. */
  tolerance?: number;
  /** Properties to ignore. Ignored properties are listed in the result, never hidden. */
  ignore?: string[];
}
export interface Result {
  status: 'identical' | 'minor_only' | 'different';
  summary: {
    refElements: number; candElements: number; matched: number; refMissing: number; candExtra: number;
    pairsWithDifferences: number; majorItems: number; minorItems: number; maxGeometryDelta: number;
    byDimension: Record<string, number>; byProperty: Record<string, number>; minorRules: Record<string, number>;
    pageFactDifferences: number; ignored: string[];
  };
  pairs: Pair[]; missing: Brief[]; extra: Brief[]; pageFacts: DiffItem[]; limits: string[];
  /** Index pairing (reference, candidate) so other tools use the same alignment. */
  matches: { els: [number, number][]; pseudos: [number, number][] };
}

const GEO = ['x', 'y', 'w', 'h'] as const;
// `aside` is excluded: it is a landmark, not a plain container.
const CONTAINERS = new Set(['div', 'section', 'article', 'span']);
const IMPLICIT_CONTAINER_ROLES = new Set(['', 'region', 'article']);
const brief = (e: El, i: number): Brief => ({ i, tag: e.tag, text: (e.text ?? '').slice(0, 60), path: e.path ?? e.host ?? '', x: e.x, y: e.y, w: e.w, h: e.h });
const key = (t = '') => t.toLowerCase().replace(/\s+/g, '').slice(0, 50);
const kindOf = (e: El) => (e.tag.startsWith('::') ? 'pseudo' : ['img', 'svg', 'canvas', 'video', 'audio', 'iframe'].includes(e.tag) ? 'media' : 'box');
const dist = (a: El, b: El) => Math.max(...GEO.map(k => Math.abs(a[k] - b[k])));
const flat = (a: El, b: El) => Math.abs(a.x - b.x) + Math.abs(a.y - b.y);

const LAYOUT = new Set(['display', 'position', 'float', 'boxSizing', 'overflowX', 'overflowY', 'zIndex', 'flexDirection', 'flexWrap', 'flexGrow', 'flexShrink', 'flexBasis', 'alignItems', 'alignSelf', 'alignContent', 'justifyContent', 'justifySelf', 'justifyItems', 'rowGap', 'columnGap', 'gridTemplateColumns', 'gridTemplateRows', 'gridAutoFlow', 'gridColumnStart', 'gridColumnEnd', 'gridRowStart', 'gridRowEnd', 'aspectRatio', 'paddingTop', 'paddingRight', 'paddingBottom', 'paddingLeft', 'marginTop', 'marginRight', 'marginBottom', 'marginLeft', 'objectFit', 'objectPosition', 'order']);
const TYPO = new Set(['fontFamily', 'fontSize', 'fontWeight', 'fontStyle', 'fontVariationSettings', 'fontFeatureSettings', 'fontStretch', 'fontKerning', 'letterSpacing', 'wordSpacing', 'lineHeight', 'textAlign', 'textTransform', 'textDecorationLine', 'textDecorationStyle', 'textDecorationColor', 'textIndent', 'textWrap', 'whiteSpace', 'wordBreak', 'overflowWrap', 'textOverflow', 'textShadow', 'verticalAlign', 'hyphens', 'direction', 'writingMode', 'webkitTextStrokeWidth', 'webkitTextStrokeColor', 'webkitTextFillColor']);
const MOTION = new Set(['transitionProperty', 'transitionDuration', 'transitionTimingFunction', 'transitionDelay', 'animationName', 'animationDuration', 'animationTimingFunction', 'animationDelay', 'animationIterationCount', 'animationDirection', 'animationFillMode', 'animationPlayState', 'willChange', 'scrollBehavior', 'scrollSnapType', 'scrollSnapAlign', 'overscrollBehavior', 'touchAction']);
const INTERACTION = new Set(['cursor', 'pointerEvents', 'userSelect']);
/** tabindex="0" added to a natively focusable element: the tab order is unchanged. */
export const redundantTabindex = (a: El, b: El) => ['button', 'a', 'input', 'select', 'textarea', 'summary'].includes(a.tag) && a.tag === b.tag && a.tabindex === undefined && b.tabindex === '0' && (a.tag !== 'a' || (a.href != null && b.href != null));
/** An empty zero-size element is ignored when only the candidate has it; a zero-size control, media or role-bearing element is not. */
export const isInvisibleShell = (e: El) => e.w === 0 && e.h === 0 && !e.text && !e.role && e.tabindex === undefined && !['button', 'a', 'input', 'select', 'textarea', 'summary', 'img', 'svg', 'canvas', 'video', 'iframe', 'dialog'].includes(e.tag);
/** The focus-order entry as compared: tabindex="0" on a natively focusable element is the same stop (the redundant-tabindex-0 equivalence). */
export const FOCUS_KEYS = ['tag', 'role', 'name', 'focusVisible', 'outline', 'boxShadow', 'tabindex'] as const;
export const focusSig = (f: Record<string, unknown>): unknown[] => FOCUS_KEYS.map(k => (k === 'tabindex' && f.tabindex === '0' && ['button', 'a', 'input', 'select', 'textarea', 'summary'].includes(String(f.tag)) ? null : f[k] ?? null));
export const dimOfStyle = (p: string): Dimension => (LAYOUT.has(p) ? 'layout' : TYPO.has(p) ? 'typography' : MOTION.has(p) ? 'motion' : INTERACTION.has(p) ? 'interaction' : 'styling');

/** Named, auditable equivalences. Anything not listed here is compared exactly. */
const POSITION_PROPS = new Set(['backgroundPosition', 'maskPosition', 'objectPosition', 'transformOrigin']);
const canonGradient = (v: string) => v.replace(/linear-gradient\(to top, (rgba?\([^)]*\)), (rgba?\([^)]*\))\)/g, 'linear-gradient($2, $1)');
export const STYLE_RULES: { name: string; applies: (prop: string) => boolean; canon: (v: string) => string }[] = [
  // `to top, A, B` renders identically to the default direction `B, A`.
  { name: 'gradient-direction', applies: p => p === 'backgroundImage' || p === 'maskImage', canon: canonGradient },
  // `0% 0%` and the minifier's `0 0` / `0px 0px` are the same position.
  { name: 'zero-position-unit', applies: p => POSITION_PROPS.has(p), canon: v => v.replace(/(^|\s)0(%|px)(?=\s|$)/g, '$10') },
];
/** Canonical form of a computed style value (used by the Rust-gate fact emitter). */
export const canonStyle = (prop: string, v: string) => STYLE_RULES.reduce((x, r) => (r.applies(prop) ? r.canon(x) : x), v);
const minorRuleFor = (prop: string, a: string, b: string) => STYLE_RULES.find(r => r.applies(prop) && r.canon(a) === r.canon(b))?.name;
export const stripOrigin = (h: string | null | undefined, origins: string[]) => {
  if (h == null) return null;
  let x = h;
  for (const o of origins) if (x.startsWith(o)) { x = x.slice(o.length) || '/'; break; }
  return x;
};
export const trimSlash = (h: string | null) => (h && h.length > 1 && !h.includes('?') && !h.includes('#') ? h.replace(/\/+$/, '') : h);

function push(items: DiffItem[], dim: Dimension, prop: string, ref: unknown, cand: unknown, rule?: string, ignore?: Set<string>) {
  if (ignore?.has(prop)) return;
  items.push({ dim, prop, ref, cand, severity: rule ? 'minor' : 'major', rule });
}

function compareStyles(a: Record<string, string> | undefined, b: Record<string, string> | undefined, items: DiffItem[], ignore: Set<string>) {
  if (!a || !b) return;
  for (const p of new Set([...Object.keys(a), ...Object.keys(b)])) {
    if (a[p] === b[p]) continue;
    const rule = a[p] !== undefined && b[p] !== undefined ? minorRuleFor(p, String(a[p]), String(b[p])) : undefined;
    push(items, dimOfStyle(p), `style.${p}`, a[p], b[p], rule, ignore);
  }
}

export function comparePair(r: Collection, c: Collection, a: El, b: El, o: Options): DiffItem[] {
  const items: DiffItem[] = [];
  const ign = new Set(o.ignore ?? []);
  const tol = o.tolerance ?? 1;
  for (const k of GEO) if (Math.abs(a[k] - b[k]) > tol) push(items, 'layout', `bounds.${k}`, a[k], b[k], undefined, ign);
  if (a.tag.startsWith('::')) {
    if (a.pcontent !== b.pcontent) push(items, 'content', 'pseudo.content', a.pcontent, b.pcontent, undefined, ign);
    compareStyles(a.pst, b.pst, items, ign);
    for (const p of ['bgSha', 'maskSha'] as const) if (JSON.stringify(a[p] ?? null) !== JSON.stringify(b[p] ?? null)) push(items, 'assets', p, a[p], b[p], undefined, ign);
    return items;
  }
  compareStyles(r.styleTable[a.sty ?? -1], c.styleTable[b.sty ?? -1], items, ign);
  if ((a.text ?? '').replace(/\s+/g, ' ') !== (b.text ?? '').replace(/\s+/g, ' ')) push(items, 'content', 'text', a.text, b.text, undefined, ign);
  // Lifting a div into a section/article is intended; it is counted as a named minor rule. A heading,
  // control, list or paragraph changing tag or role is never covered by it.
  // Only when the tag itself changed and neither side sets an explicit role attribute.
  const retag = a.tag !== b.tag && CONTAINERS.has(a.tag) && CONTAINERS.has(b.tag) && !a.roleExplicit && !b.roleExplicit;
  if (a.tag !== b.tag) push(items, 'semantics', 'tag', a.tag, b.tag, retag ? 'container-retag' : undefined, ign);
  if ((a.role ?? '') !== (b.role ?? '')) push(items, 'semantics', 'role', a.role, b.role, retag && IMPLICIT_CONTAINER_ROLES.has(a.role ?? '') && IMPLICIT_CONTAINER_ROLES.has(b.role ?? '') ? 'container-retag' : undefined, ign);
  if ((a.name ?? '') !== (b.name ?? '') || a.nameHash !== b.nameHash) push(items, 'semantics', 'accessibleName', a.name, b.name, undefined, ign);
  if (a.textHash !== b.textHash) push(items, 'content', 'textHash', a.textHash, b.textHash, undefined, ign);
  // Base UI parts and data-* state names are reported; an original without them differs by construction.
  const sortedJson = (o: Record<string, string> | undefined) => JSON.stringify(Object.entries(o ?? {}).sort(([x], [y]) => (x < y ? -1 : 1)));
  if (sortedJson(a.data) !== sortedJson(b.data)) {
    const added = Object.keys(b.data ?? {}).filter(k => !(k in (a.data ?? {})));
    const onlyAdded = Object.keys(a.data ?? {}).every(k => (b.data ?? {})[k] === (a.data ?? {})[k]) && added.length > 0 && added.every(k => /^data-(base-ui|slot|state|open|closed|popup|orientation|side|align|starting|ending|highlighted|checked|unchecked|disabled|pressed|selected|active|inactive|expanded|panel|trigger|index|value|type|variant|size)/.test(k));
    push(items, 'semantics', 'data-attributes', a.data ?? {}, b.data ?? {}, onlyAdded ? 'added-component-attributes' : undefined, ign);
  }
  for (const k of ['effOpacity', 'effFx', 'effHidden'] as const) if (a[k] !== b[k]) push(items, 'styling', k, a[k], b[k], undefined, ign);
  if (a.inert !== b.inert) push(items, 'behavior', 'inert', a.inert, b.inert, undefined, ign);
  const names = new Set([...Object.keys(a.aria ?? {}), ...Object.keys(b.aria ?? {})]);
  for (const n of names) if ((a.aria ?? {})[n] !== (b.aria ?? {})[n]) push(items, 'behavior', n, (a.aria ?? {})[n], (b.aria ?? {})[n], undefined, ign);
  if ((a.ariaRefs ?? '') !== (b.ariaRefs ?? '')) push(items, 'behavior', 'aria-relationships', a.ariaRefs, b.ariaRefs, undefined, ign);
  if ((a.tabindex ?? '') !== (b.tabindex ?? '')) {
    // tabindex="0" on a natively focusable element does not change the tab order
    const native = redundantTabindex(a, b);
    push(items, 'behavior', 'tabindex', a.tabindex, b.tabindex, native ? 'redundant-tabindex-0' : undefined, ign);
  }
  for (const k of ['type', 'disabled', 'placeholder', 'required', 'checked', 'value', 'open', 'modal'] as const) if (a[k] !== b[k]) push(items, 'behavior', k, a[k], b[k], undefined, ign);
  // routing: hrefs compared after removing the site origin; a trailing slash is its own named minor rule
  if ('href' in a || 'href' in b) {
    const ah = stripOrigin(a.href, o.siteOrigins ?? []), bh = stripOrigin(b.href, o.siteOrigins ?? []);
    if (ah !== bh) push(items, 'routing', 'href', a.href, b.href, trimSlash(ah) === trimSlash(bh) ? 'href-trailing-slash' : undefined, ign);
    for (const k of ['target', 'rel', 'download'] as const) if (a[k] !== b[k]) push(items, 'routing', k, a[k], b[k], undefined, ign);
  }
  // assets and graphics
  for (const k of ['src', 'nat', 'complete', 'alt', 'srcSha', 'poster', 'loading', 'srcsetPresent'] as const) {
    if (k === 'src' && a.srcSha && b.srcSha && a.srcSha === b.srcSha && a.srcSha !== 'unhashed') continue; // same bytes under another name
    if (a[k] !== b[k]) push(items, k === 'alt' || k === 'loading' || k === 'srcsetPresent' ? 'semantics' : 'assets', k, a[k], b[k], undefined, ign);
  }
  for (const k of ['bgSha', 'maskSha'] as const) if (JSON.stringify(a[k] ?? null) !== JSON.stringify(b[k] ?? null)) push(items, 'assets', k, a[k], b[k], undefined, ign);
  for (const k of ['paused', 'muted', 'loop'] as const) if (a[k] !== b[k]) push(items, 'behavior', k, a[k], b[k], undefined, ign);
  for (const k of ['vb', 'par', 'svgAttrW', 'svgAttrH', 'svgCount', 'svgHash', 'svgPaint', 'svgAnimated', 'cw', 'ch', 'pixelHash', 'pixelBlank'] as const) {
    if (k === 'svgAttrW' || k === 'svgAttrH') { if ((a[k] ?? '') !== (b[k] ?? '')) push(items, 'graphics', k, a[k], b[k], undefined, ign); continue; }
    if (a[k] !== b[k]) push(items, 'graphics', k, a[k], b[k], undefined, ign);
  }
  if (JSON.stringify(a.paths ?? null) !== JSON.stringify(b.paths ?? null)) push(items, 'graphics', 'svg.paths', (a.paths ?? []).length + ' paths', (b.paths ?? []).length + ' paths (d hashes differ)', undefined, ign);
  return items;
}

export function diffCollections(ref: Collection, cand: Collection, options: Options = {}): Result {
  const R = ref.els, C = cand.els;
  const usedC = new Set<number>(), usedR = new Set<number>();
  const pairs: [number, number][] = [];
  // pass 1: text-bearing elements. Candidates with equal text; closest geometry wins, document order breaks ties.
  R.forEach((e, i) => {
    if (!e.text) return;
    const k = key(e.text);
    let best = -1, bd = Infinity;
    C.forEach((f, j) => {
      if (usedC.has(j) || !f.text) return;
      if (f.w === 0 && e.w > 0) return;
      const fk = key(f.text);
      if (fk !== k && !(k.length > 8 && fk.startsWith(k.slice(0, 20)))) return;
      const d = flat(e, f) + (e.tag === f.tag ? 0 : 0.5);
      if (d < bd) { bd = d; best = j; }
    });
    if (best >= 0) { usedC.add(best); usedR.add(i); pairs.push([i, best]); }
  });
  // pass 2: non-text elements by kind and geometry (nearest within 1.5 px)
  R.forEach((e, i) => {
    if (e.text) return;
    let best = -1, bd = Infinity;
    C.forEach((f, j) => {
      if (usedC.has(j) || f.text || kindOf(f) !== kindOf(e)) return;
      if (kindOf(e) !== 'box' && f.tag !== e.tag) return;
      if (kindOf(e) === 'pseudo' && f.pcontent !== e.pcontent) return;
      const d = dist(e, f) + (e.tag === f.tag ? 0 : 0.01);
      if (d < bd) { bd = d; best = j; }
    });
    if (best >= 0 && bd <= 1.5) { usedC.add(best); usedR.add(i); pairs.push([i, best]); }
  });
  // reverse pass: leftovers on the candidate side look for a leftover reference partner within 40 px
  C.forEach((f, j) => {
    if (usedC.has(j)) return;
    let best = -1, bd = Infinity;
    R.forEach((e, i) => {
      if (usedR.has(i) || kindOf(e) !== kindOf(f) || e.tag !== f.tag || Boolean(e.text) !== Boolean(f.text)) return;
      if (e.text && key(e.text) !== key(f.text ?? '')) return;
      if (kindOf(e) === 'pseudo' && f.pcontent !== e.pcontent) return;
      const d = dist(e, f);
      if (d < bd) { bd = d; best = i; }
    });
    if (best >= 0 && (bd <= 40 || f.tag === 'html' || f.tag === 'body')) { usedC.add(j); usedR.add(best); pairs.push([best, j]); } // the document roots pair whatever their height
  });
  const withP = (L: El[], P: El[]) => ({ L, P });
  void withP;
  // pseudo-elements live in their own lists and are paired the same way
  const pseudoPairs: [number, number][] = [];
  const usedPC = new Set<number>(), usedPR = new Set<number>();
  ref.pseudos.forEach((e, i) => {
    let best = -1, bd = Infinity;
    cand.pseudos.forEach((f, j) => {
      if (usedPC.has(j) || f.tag !== e.tag || f.pcontent !== e.pcontent) return;
      const d = dist(e, f);
      if (d < bd) { bd = d; best = j; }
    });
    if (best >= 0 && bd <= 40) { usedPC.add(best); usedPR.add(i); pseudoPairs.push([i, best]); }
  });

  const out: Pair[] = [];
  let maxDelta = 0;
  const run = (a: El[], b: El[], list: [number, number][]) => {
    for (const [i, j] of list) {
      const e = a[i], f = b[j];
      const items = comparePair(ref, cand, e, f, options);
      const delta = Math.round(Math.max(...GEO.map(k => Math.abs(e[k] - f[k]))) * 100) / 100;
      maxDelta = Math.max(maxDelta, delta);
      if (items.length) out.push({ ref: brief(e, i), cand: brief(f, j), items, delta });
    }
  };
  run(R, C, pairs);
  run(ref.pseudos, cand.pseudos, pseudoPairs);

  // unpaired elements are failures, except the invisible shells the old harness also ignored
  // an empty zero-size shell is ignored, but a zero-size control or role-bearing element is not
  const invisible = isInvisibleShell;
  const missing = [...R.map((e, i) => [e, i] as const).filter(([, i]) => !usedR.has(i)).map(([e, i]) => brief(e, i)), ...ref.pseudos.map((e, i) => [e, i] as const).filter(([, i]) => !usedPR.has(i)).map(([e, i]) => brief(e, i))];
  const extra = [...C.map((e, j) => [e, j] as const).filter(([e, j]) => !usedC.has(j) && !invisible(e)).map(([e, j]) => brief(e, j)), ...cand.pseudos.map((e, j) => [e, j] as const).filter(([, j]) => !usedPC.has(j)).map(([e, j]) => brief(e, j))];

  // page-level facts
  const pageFacts: DiffItem[] = [];
  const ign = new Set(options.ignore ?? []);
  const fkeys = new Set([...Object.keys(ref.facts), ...Object.keys(cand.facts)]);
  for (const k of fkeys) {
    if (k.startsWith('info.')) continue; // informational facts (timing-dependent, e.g. which font faces happened to load) are recorded, not compared
    const a = ref.facts[k], b = cand.facts[k];
    if (JSON.stringify(a) === JSON.stringify(b)) continue;
    const dim: Dimension = k.startsWith('scroll.lock') || k.startsWith('count.inert') || k.startsWith('count.dialog') || k.startsWith('count.ariaHidden') ? 'behavior' : k === 'page.text' || k === 'headings' || k === 'doc.title' ? 'content' : k.startsWith('count.') || k === 'landmarks' ? 'semantics' : k.startsWith('main.') || k.startsWith('scroll.') ? 'layout' : k.startsWith('fonts') ? 'typography' : k.startsWith('animations') ? 'motion' : 'content';
    push(pageFacts, dim, 'fact.' + k, a, b, undefined, ign);
  }

  const items = [...out.flatMap(p => p.items), ...pageFacts];
  const byDimension: Record<string, number> = {}, byProperty: Record<string, number> = {}, minorRules: Record<string, number> = {};
  for (const it of items) {
    byDimension[it.dim] = (byDimension[it.dim] ?? 0) + 1;
    byProperty[it.prop] = (byProperty[it.prop] ?? 0) + 1;
    if (it.rule) minorRules[it.rule] = (minorRules[it.rule] ?? 0) + 1;
  }
  const majorItems = items.filter(i => i.severity === 'major').length;
  const different = majorItems > 0 || missing.length > 0 || extra.length > 0;
  return {
    status: different ? 'different' : items.length ? 'minor_only' : 'identical',
    summary: {
      refElements: R.length + ref.pseudos.length, candElements: C.length + cand.pseudos.length, matched: pairs.length + pseudoPairs.length,
      refMissing: missing.length, candExtra: extra.length, pairsWithDifferences: out.length, majorItems, minorItems: items.length - majorItems,
      maxGeometryDelta: maxDelta, byDimension, byProperty, minorRules, pageFactDifferences: pageFacts.length, ignored: [...ign],
    },
    pairs: out, missing, extra, pageFacts, limits: [...new Set([...(ref.limits ?? []), ...(cand.limits ?? [])])],
    matches: { els: pairs, pseudos: pseudoPairs },
  };
}
