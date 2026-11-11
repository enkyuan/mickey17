import { describe, expect, test } from 'bun:test';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { canonStyle, diffCollections, type Collection, type El } from '../src/parity/diff';
import { gateDifferences, gateStates } from '../src/parity/facts';
import { buildInventory } from '../src/parity/inventory';
import { MUTATIONS } from '../src/parity/mutations';
import { diffRuns, statusOf } from '../src/parity/pair';
import { selectStates } from '../src/parity/run';
import type { SideRun } from '../src/parity/state';
import { Window } from 'happy-dom';

const style = { display: 'block', fontSize: '16px', color: 'rgb(0, 0, 0)', opacity: '1', backgroundImage: 'none', backgroundPosition: '0% 0%', textAlign: 'start' };
const make = (els: Partial<El>[], facts: Record<string, unknown> = {}): Collection => ({
  schema: 2, context: {}, facts: { 'page.text': 'hello', headings: ['h1:Hello'], ...facts }, styleTable: [style], pseudos: [], limits: [],
  els: els.map((e, i) => ({ tag: 'div', text: '', x: 0, y: i * 50, w: 100, h: 40, sty: 0, effOpacity: 1, effFx: '', effHidden: false, inert: false, role: '', name: '', ...e }) as El),
});
const page = () => make([
  { tag: 'h1', text: 'Hello world', role: 'heading', name: 'Hello world' },
  { tag: 'p', text: 'Some paragraph copy that is long enough', x: 0, y: 60 },
  { tag: 'a', text: 'Docs', href: 'https://site.test/docs', hrefPath: 'https://site.test/docs', role: 'link', name: 'Docs', y: 120 },
  { tag: 'img', src: '/a.png', nat: '10x10', complete: true, srcSha: 'aa', alt: 'x', y: 180 },
  { tag: 'svg', vb: '0 0 10 10', paths: ['h1:3'], svgHash: 's1', svgPaint: 'p1', svgCount: 1, y: 240 },
  { tag: 'canvas', cw: 10, ch: 10, pixelHash: 'c1', pixelBlank: false, y: 300 },
  { tag: 'button', text: 'Menu', role: 'button', name: 'Menu', aria: { 'aria-expanded': 'false' }, y: 360 },
]);
const clone = (c: Collection): Collection => structuredClone(c);
const opts = { siteOrigins: ['https://site.test'] };
const names = (c: Collection) => Object.keys(diffCollections(page(), c, opts).summary.byProperty);

describe('diff engine', () => {
  test('identical collections are identical', () => {
    const r = diffCollections(page(), page(), opts);
    expect(r.status).toBe('identical');
    expect(r.summary.matched).toBe(7);
  });
  test('every mutation class the calibration injects is a reported difference', () => {
    const cases: [string, (c: Collection) => void, string][] = [
      ['opacity', c => { c.styleTable.push({ ...style, opacity: '0' }); c.els[0].sty = 1; }, 'style.opacity'],
      ['effective opacity of a wrapper', c => { c.els[1].effOpacity = 0; }, 'effOpacity'],
      ['font variation', c => { c.styleTable.push({ ...style, fontVariationSettings: '"wght" 700' }); c.els[0].sty = 1; }, 'style.fontVariationSettings'],
      ['image src bytes', c => { c.els[3].srcSha = 'bb'; }, 'srcSha'],
      ['background', c => { c.styleTable.push({ ...style, backgroundImage: 'linear-gradient(red, blue)' }); c.els[1].sty = 1; }, 'style.backgroundImage'],
      ['text-align', c => { c.styleTable.push({ ...style, textAlign: 'center' }); c.els[1].sty = 1; }, 'style.textAlign'],
      ['text edit', c => { c.els[1].text = 'Some paragraph copy that is long enough!'; }, 'text'],
      ['hidden menu', c => { c.els[6].effHidden = true; }, 'effHidden'],
      ['aria-expanded', c => { c.els[6].aria = { 'aria-expanded': 'true' }; }, 'aria-expanded'],
      ['link target', c => { c.els[2].href = '/elsewhere'; }, 'href'],
      ['svg path', c => { c.els[4].paths = ['h2:9']; }, 'svg.paths'],
      ['viewBox', c => { c.els[4].vb = '0 0 5 5'; }, 'vb'],
      ['canvas pixels', c => { c.els[5].pixelHash = 'c2'; }, 'pixelHash'],
      ['heading level', c => { c.els[0].tag = 'h2'; }, 'tag'],
      ['two pixel move', c => { c.els[1].y += 2; }, 'bounds.y'],
    ];
    for (const [label, mutate, prop] of cases) {
      const c = page(); mutate(c);
      const r = diffCollections(page(), c, opts);
      expect(r.status, label).toBe('different');
      expect(names(c), label).toContain(prop);
    }
  });
  test('sub-pixel movement within tolerance is not a failure but one pixel more is', () => {
    const c = page(); c.els[1].y += 0.9;
    expect(diffCollections(page(), c, opts).status).toBe('identical');
    const d = page(); d.els[1].y += 1.2;
    expect(diffCollections(page(), d, opts).status).toBe('different');
  });
  test('removed and injected elements are reported, not dropped (reverse pass)', () => {
    const removed = page(); removed.els.splice(1, 1);
    const r = diffCollections(page(), removed, opts);
    expect(r.summary.refMissing).toBe(1);
    expect(r.status).toBe('different');
    const added = page(); added.els.push({ tag: 'div', text: 'extra injected', x: 0, y: 500, w: 90, h: 20 } as El);
    const a = diffCollections(page(), added, opts);
    expect(a.summary.candExtra).toBe(1);
    expect(a.status).toBe('different');
  });
  test('an element that moved far is paired and reported as moved, not missing plus extra', () => {
    const c = page(); c.els[3].y += 25;
    const r = diffCollections(page(), c, opts);
    expect(r.summary.refMissing).toBe(0);
    expect(r.summary.candExtra).toBe(0);
    expect(Object.keys(r.summary.byProperty)).toContain('bounds.y');
  });
  test('named minor equivalences are counted as minor, never hidden and never major', () => {
    const c = page();
    c.els[2].href = 'https://site.test/docs/'; // trailing slash
    c.styleTable.push({ ...style, backgroundImage: 'linear-gradient(rgb(1, 1, 1), rgb(2, 2, 2))', backgroundPosition: '0px 0px' });
    const ref = page(); ref.styleTable.push({ ...style, backgroundImage: 'linear-gradient(to top, rgb(2, 2, 2), rgb(1, 1, 1))', backgroundPosition: '0% 0%' });
    ref.els[1].sty = 1; c.els[1].sty = 1;
    c.els[6].tabindex = '0';
    const r = diffCollections(ref, c, opts);
    expect(r.status).toBe('minor_only');
    expect(r.summary.minorRules).toEqual({ 'href-trailing-slash': 1, 'gradient-direction': 1, 'zero-position-unit': 1, 'redundant-tabindex-0': 1 });
    expect(canonStyle('backgroundPosition', '0% 0%')).toBe(canonStyle('backgroundPosition', '0px 0px'));
    expect(canonStyle('margin', '0px')).toBe('0px'); // the zero rule applies to positions only
  });
  test('page-level facts and origin-only href differences', () => {
    const c = page(); c.facts['page.text'] = 'bye';
    expect(diffCollections(page(), c, opts).pageFacts.length).toBe(1);
    const d = page(); d.els[2].href = '/docs';
    expect(diffCollections(page(), d, opts).status).toBe('identical'); // origin removed on one side only
  });
  test('ignored properties are listed in the result', () => {
    const c = page(); c.els[1].text = 'Some paragraph copy that is long enough?';
    const r = diffCollections(page(), c, { ...opts, ignore: ['text'] });
    expect(r.summary.ignored).toEqual(['text']);
    expect(r.status).toBe('identical');
  });
});

const side = (c: Collection, extra: Partial<SideRun> = {}): SideRun => ({ collection: c, focusOrder: [], errors: [], blocked: [], stepErrors: [], settled: true, reachedScrollY: 0, timingMs: 1, ...extra });
describe('state-level behavior', () => {
  test('focus order, failed steps, unsettled pages and candidate-only errors are differences', () => {
    const f = (name: string, outline = 'none') => ({ tag: 'a', role: 'link', name, focusVisible: true, outline, tabindex: null });
    const a = side(page(), { focusOrder: [f('Docs')] });
    expect(statusOf(diffRuns(a, side(page(), { focusOrder: [f('Docs')] }), opts))).toBe('identical');
    expect(statusOf(diffRuns(a, side(page(), { focusOrder: [f('Pricing')] }), opts))).toBe('different');
    expect(statusOf(diffRuns(a, side(page(), { focusOrder: [f('Docs', '4px solid red')] }), opts))).toBe('different');
    expect(statusOf(diffRuns(side(page()), side(page(), { stepErrors: ['click: cand: target not found'] }), opts))).toBe('different');
    expect(statusOf(diffRuns(side(page()), side(page(), { errors: ['pageerror: boom'] }), opts))).toBe('different');
    expect(statusOf(diffRuns(side(page(), { errors: ['console: shared'] }), side(page(), { errors: ['console: shared'] }), opts))).toBe('identical');
    expect(statusOf(diffRuns(side(page()), side(page(), { settled: false }), opts))).toBe('unsettled');
    expect(statusOf(diffRuns(side(page()), side(page(), { reachedScrollY: 40 }), opts))).toBe('different');
  });
});

describe('gate fact emission', () => {
  test('anchors are the reference indices; extra candidate elements get their own anchors', () => {
    const ref = page(), cand = page();
    cand.els.push({ tag: 'div', text: 'extra', x: 0, y: 700, w: 10, h: 10 } as El);
    const d = diffCollections(ref, cand, opts);
    const spec = { name: 's', route: '/', viewport: [1440, 900] as [number, number], reducedMotion: false, phase: 'initial' as const, steps: [], tags: [] };
    const [r, c] = gateStates(spec, ref, cand, d, opts, [], [], [true, true]);
    expect(r.inventory).toContain('e00000');
    expect(r.inventory).not.toContain('x00007');
    expect(c.inventory).toContain('x00007');
    expect(r.facts['e00002.link']).toBe(c.facts['e00002.link']);
    for (const v of [...Object.values(r.facts), ...Object.values(c.facts)]) expect(['string', 'number', 'boolean']).toContain(typeof v);
  });
  test('named equivalences are canonical on both sides', () => {
    const ref = page(), cand = page();
    cand.els[2].href = 'https://site.test/docs/'; cand.els[6].tabindex = '0';
    const spec = { name: 's', route: '/', viewport: [1440, 900] as [number, number], reducedMotion: false, phase: 'initial' as const, steps: [], tags: [] };
    const [r, c] = gateStates(spec, ref, cand, diffCollections(ref, cand, opts), opts, [], [], [true, true]);
    expect(r.facts).toEqual(c.facts);
  });
});

describe('inventory generator', () => {
  const capture = () => {
    const dir = mkdtempSync(join(tmpdir(), 'parity-inv-'));
    mkdirSync(join(dir, 'site/css'), { recursive: true }); mkdirSync(join(dir, 'reconstruction/pages'), { recursive: true });
    writeFileSync(join(dir, 'site/css/a.css'), '@media (min-width:768px){a{color:red}}@media (max-width:767px){a{color:blue}}');
    const nodes = [
      { parent: -1, kind: 'element', tag: 'html', attributes: [] },
      { parent: 0, kind: 'element', tag: 'body', attributes: [] },
      { parent: 1, kind: 'element', tag: 'button', attributes: [{ name: 'aria-expanded', value: 'false' }], bounds: { x: 0, y: 0, width: 50, height: 20 } },
      { parent: 2, kind: 'text', value: 'Menu' },
      { parent: 1, kind: 'element', tag: 'canvas', attributes: [] },
      { parent: 1, kind: 'element', tag: 'a', attributes: [{ name: 'href', value: '/x' }], bounds: { x: 0, y: 30, width: 50, height: 20 } },
      { parent: 5, kind: 'text', value: 'Docs' },
      { parent: 1, kind: 'element', tag: 'input', attributes: [] },
    ];
    for (const s of ['1440x900', '375x812']) writeFileSync(join(dir, `reconstruction/pages/page-00000-${s}.json`), JSON.stringify({ viewport: { width: +s.split('x')[0], height: +s.split('x')[1] }, nodes }));
    writeFileSync(join(dir, 'manifest.json'), JSON.stringify({ external_hosts: ['t.example'], pages: [{ final_url: 'https://site.test/', title: 'T', source_evidence: ['reconstruction/pages/page-00000-1440x900.json', 'reconstruction/pages/page-00000-375x812.json'] }] }));
    return dir;
  };
  test('states, controls, breakpoints and gaps are derived from the capture', () => {
    const inv = buildInventory(capture());
    expect(inv.routes.map(r => r.route)).toEqual(['/']);
    expect(inv.siteOrigins).toEqual(['https://site.test']);
    expect(inv.viewports.map(v => v.size)).toEqual([[1440, 900], [375, 812]]);
    expect(inv.breakpoints.map(b => b.px)).toEqual([767, 768].slice(0, 1)); // 767 and 768 are one boundary
    const n = inv.states.map(s => s.name);
    for (const want of ['home.desktop1440.initial', 'home.mobile375.reduced.initial', 'home.desktop1440.scroll.down.100', 'home.desktop1440.scroll.up.25', 'home.desktop1440.scroll.up.0', 'home.desktop1440.menu.menu.open', 'home.desktop1440.menu.menu.escape', 'home.desktop1440.hover.docs', 'home.desktop1440.keyboard.tab12', 'home.desktop1440.entrance.t400', 'home.bp766.initial', 'home.bp767.initial']) expect(n, want).toContain(want);
    expect(inv.states.find(s => s.name.endsWith('scroll.up.25'))!.steps.map(s => (s as { frac: number }).frac)).toEqual([0.25, 0.5, 0.75, 1, 0.75, 0.5, 0.25]);
    expect(new Set(n).size).toBe(n.length);
    const gaps = inv.gaps.map(g => g.id);
    expect(gaps).toContain('canvas:/:desktop.canvas');
    expect(gaps).toContain('form:/:desktop.input');
    expect(gaps).toContain('external-hosts');
    expect(gaps).toContain('pointer-driven');
  });
  test('profiles never drop states silently', () => {
    const inv = buildInventory(capture());
    for (const p of ['quick', 'core', 'standard', 'full'] as const) {
      const { run, skipped } = selectStates(inv, p);
      expect(run.length + skipped.length).toBe(inv.states.length);
      expect(skipped.every(s => s.reason.length > 0)).toBe(true);
    }
    expect(selectStates(inv, 'quick').run.every(s => s.phase === 'initial')).toBe(true);
  });
});

describe('mutation catalog', () => {
  test('covers every control the task requires', () => {
    const have = new Set(MUTATIONS.map(m => m.name));
    for (const want of ['opacity', 'font-variation', 'image-src', 'background-image', 'text-align', 'injected-text', 'removed-section', 'hidden-menu']) expect(have.has(want), want).toBe(true);
    // a documented blind spot either has a state kind that detects it, or explains why it is within tolerance
    expect(MUTATIONS.filter(m => m.expect === 'miss').every(m => m.note && (MUTATIONS.some(o => o.name === m.name && o.expect === 'detect') || /tolerance/.test(m.note)))).toBe(true);
  });
});

describe('step failures, focus rings, zero-size extras', () => {
  test('a failed step is never identical, even when both sides fail the same way', () => {
    const same = diffRuns(side(page(), { stepErrors: ['hover: Timeout 5000ms exceeded.'] }), side(page(), { stepErrors: ['hover: Timeout 5000ms exceeded.'] }), opts);
    expect(statusOf(same)).toBe('not_exercised');
    expect(statusOf(diffRuns(side(page(), { stepErrors: ['click: ref: target not found'] }), side(page()), opts))).toBe('not_exercised');
    expect(statusOf(diffRuns(side(page()), side(page(), { stepErrors: ['click: cand: target not found'] }), opts))).toBe('different');
  });
  test('not_exercised outranks different, and tabindex=0 on a native control is the same focus stop', () => {
    const c = page(); c.els[1].text = 'changed copy for the second paragraph here';
    expect(statusOf(diffRuns(side(page(), { stepErrors: ['click: Timeout'] }), side(c, { stepErrors: ['click: Timeout'] }), opts))).toBe('not_exercised');
    const f = (tabindex: string | null, tag = 'button') => ({ tag, role: 'button', name: 'Products', focusVisible: true, outline: 'auto', boxShadow: 'none', tabindex });
    expect(statusOf(diffRuns(side(page(), { focusOrder: [f(null)] }), side(page(), { focusOrder: [f('0')] }), opts))).toBe('identical');
    expect(statusOf(diffRuns(side(page(), { focusOrder: [f(null, 'div')] }), side(page(), { focusOrder: [f('0', 'div')] }), opts))).toBe('different');
  });
  test('a focus ring drawn with box-shadow is compared', () => {
    const f = (boxShadow: string) => ({ tag: 'a', role: 'link', name: 'Docs', focusVisible: true, outline: 'none', boxShadow, tabindex: null });
    expect(statusOf(diffRuns(side(page(), { focusOrder: [f('none')] }), side(page(), { focusOrder: [f('rgb(255, 0, 0) 0px 0px 0px 4px')] }), opts))).toBe('different');
  });
  test('a zero-size candidate-only control is an extra element, an empty shell is not', () => {
    const c = page(); c.els.push({ tag: 'button', x: 0, y: 0, w: 0, h: 0, text: '' } as El);
    expect(diffCollections(page(), c, opts).summary.candExtra).toBe(1);
    const e = page(); e.els.push({ tag: 'div', x: 0, y: 0, w: 0, h: 0, text: '' } as El);
    expect(diffCollections(page(), e, opts).summary.candExtra).toBe(0);
  });
  test('container-retag is not applied to explicit roles, landmarks, or non-containers', () => {
    const mk = (a: Partial<El>, b: Partial<El>) => { const r = page(), c = page(); Object.assign(r.els[1], a); Object.assign(c.els[1], b); return diffCollections(r, c, opts); };
    expect(mk({ tag: 'div' }, { tag: 'section', role: 'region' }).status).toBe('minor_only');
    expect(mk({ tag: 'div' }, { tag: 'div', role: 'group', roleExplicit: true }).status).toBe('different');
    expect(mk({ tag: 'div' }, { tag: 'aside', role: 'complementary' }).status).toBe('different');
    expect(mk({ tag: 'p' }, { tag: 'div' }).status).toBe('different');
    expect(mk({ tag: 'div' }, { tag: 'section', role: 'region', roleExplicit: true }).status).toBe('different');
  });
});

describe('gate facts differ exactly when the TS report has a major item', () => {
  const spec = { name: 's', route: '/', viewport: [1440, 900] as [number, number], reducedMotion: false, phase: 'initial' as const, steps: [], tags: [] };
  const gateDiffers = (ref: Collection, cand: Collection) => {
    const d = diffCollections(ref, cand, opts);
    const [r, c] = gateStates(spec, ref, cand, d, opts, [], [], [true, true]);
    return { ts: d.status === 'different', gate: JSON.stringify(r.facts) !== JSON.stringify(c.facts) || r.inventory.join() !== c.inventory.join() };
  };
  const cases: [string, (c: Collection) => void, boolean, ((c: Collection) => void)?][] = [
    ['image loading attribute', c => { c.els[3].loading = 'lazy'; }, true],
    ['image srcset presence', c => { c.els[3].srcsetPresent = true; }, true],
    ['video poster', c => { c.els[3].poster = '/p.png'; }, true],
    ['video paused', c => { c.els[3].paused = true; }, true],
    ['svg width attribute', c => { c.els[4].svgAttrW = '80'; }, true],
    ['svg animated', c => { c.els[4].svgAnimated = true; }, true],
    ['tabindex on a link without href', c => { c.els[2].href = null as unknown as string; c.els[2].tabindex = '0'; }, true],
    ['tabindex 0 on button', c => { c.els[6].tabindex = '0'; }, false],
    ['tabindex -1 on button', c => { c.els[6].tabindex = '-1'; }, true],
    ['explicit role on same tag', c => { c.els[1].role = 'group'; c.els[1].roleExplicit = true; }, true],
    ['div to section', c => { c.els[1].tag = 'section'; c.els[1].role = 'region'; }, false, (c: Collection) => { c.els[1].tag = 'div'; c.els[1].role = ''; }],
    ['href trailing slash', c => { c.els[2].href = 'https://site.test/docs/'; }, false],
    ['text hash only', c => { c.els[1].textHash = 'abc'; }, true],
    ['canvas pixels', c => { c.els[5].pixelHash = 'zz'; }, true],
    ['image bytes', c => { c.els[3].srcSha = 'ff'; }, true],
  ];
  for (const [label, mutate, major, prep] of cases) test(label, () => {
    const ref = page(), cand = page(); prep?.(ref); prep?.(cand); mutate(cand);
    const r = gateDiffers(ref, cand);
    expect(r.ts, 'TS major').toBe(major);
    expect(r.gate, 'gate differs').toBe(major);
  });
  test('focus order, reached scroll and step failures reach the gate', () => {
    const ref = page(), cand = page(), d = diffCollections(ref, cand, opts);
    const ex = (o: object) => ({ focusOrder: [] as unknown[], stepErrors: [] as string[], reachedScrollY: 0, ...o });
    const [r, c] = gateStates(spec, ref, cand, d, opts, [], [], [true, true], [ex({}), ex({ focusOrder: [{ tag: 'a', name: 'x' }], reachedScrollY: 40, stepErrors: ['click: boom'] })]);
    expect(r.facts['page.state.focusOrder']).not.toBe(c.facts['page.state.focusOrder']);
    expect(r.facts['page.state.reachedScrollY']).not.toBe(c.facts['page.state.reachedScrollY']);
    expect(c.errors).toEqual(['step: click: boom']);
  });
});

describe('collector locating helpers (happy-dom)', () => {
  const collector = readFileSync(join(import.meta.dir, '../../scripts/parity/collect.js'), 'utf8');
  const load = (html: string) => {
    const w = new Window({ url: 'http://x.test/' });
    w.document.body.innerHTML = html;
    // happy-dom lacks these browser APIs; the collector only reads them
    Object.assign(w.document, { fonts: Object.assign([], { ready: Promise.resolve() }), getAnimations: () => [], images: [] });
    return (w as unknown as { eval(s: string): any }).eval(collector) as { find(d: object): Element | null; describe(e: Element): any };
  };
  test('html and body are collected, a data-* only wrapper is not, descriptors for li/heading roles resolve', async () => {
    const api = load('<div id="w" data-slot="x"><p>hello world</p></div><ul><li id="l" aria-expanded="false">Shop</li></ul><h3 aria-expanded="false">FAQ</h3>') as any;
    const col = await api.collect({ hashAssets: false });
    const tags = col.els.map((e: { tag: string }) => e.tag);
    expect(tags.slice(0, 2)).toEqual(['html', 'body']);
    expect(col.els.some((e: { path: string }) => e.path.endsWith('div#w'))).toBe(false);
    const li = api.find({ tag: 'li', role: 'listitem', name: 'Shop', nth: 0 });
    expect(li?.id).toBe('l');
    expect(api.find({ tag: 'h3', role: 'heading', name: 'FAQ', nth: 0 })).toBeTruthy();
  });
  test('find(describe(e)) returns e, and an out-of-range nth returns null, not another element', () => {
    const api = load('<button id="a" aria-label="Open navigation">Menu</button><button id="b">Menu</button><a id="c" href="/x">Docs</a><a id="d" href="/y">Docs</a>');
    const doc = api.find({ tag: 'a', role: 'link', name: 'Docs', nth: 1 })!.ownerDocument;
    expect(api.find({ tag: 'a', role: 'link', name: 'Docs', nth: 1 })?.id).toBe('d');
    expect(api.find({ tag: 'a', role: 'link', name: 'Docs', nth: 2 })).toBeNull();
    expect(api.find({ tag: 'button', role: 'button', name: 'Menu', nth: 0 })?.id).toBe('b'); // not the aria-labelled one
    for (const id of ['a', 'b', 'c', 'd']) {
      const el = doc.getElementById(id)!;
      expect(api.find(api.describe(el))?.id, id).toBe(id);
    }
  });
});

describe('inventory: review regressions', () => {
  const capture = (nodes: object[], css: string) => {
    const dir = mkdtempSync(join(tmpdir(), 'parity-inv2-'));
    mkdirSync(join(dir, 'site'), { recursive: true }); mkdirSync(join(dir, 'reconstruction/pages'), { recursive: true });
    writeFileSync(join(dir, 'site/a.css'), css);
    for (const v of ['1440x900', '375x812']) writeFileSync(join(dir, `reconstruction/pages/page-00000-${v}.json`), JSON.stringify({ viewport: { width: +v.split('x')[0], height: +v.split('x')[1] }, nodes }));
    writeFileSync(join(dir, 'manifest.json'), JSON.stringify({ external_hosts: [], pages: [{ final_url: 'https://site.test/', title: 'T', source_evidence: ['reconstruction/pages/page-00000-1440x900.json', 'reconstruction/pages/page-00000-375x812.json'] }] }));
    return dir;
  };
  const base = [{ parent: -1, kind: 'element', tag: 'html', attributes: [] }, { parent: 0, kind: 'element', tag: 'body', attributes: [] }];
  test('breakpoint states straddle both min-width:N and max-width:N, and CSS features without a state become gaps', () => {
    const inv = buildInventory(capture(base, '@media (max-width:767px){a{color:red}}@media (min-width:768px){a{color:blue}}@media (min-height:700px){a{x:y}}@container (min-width:300px){a{x:y}}@media (hover:none){a{x:y}}@media (min-width:100px){a{x:y}}'));
    const widths = inv.states.filter(s => s.tags.includes('breakpoint')).map(s => s.viewport[0]);
    for (const w of [766, 767, 768]) expect(widths).toContain(w); // off-by-one on either side of the boundary is visible
    const ids = inv.gaps.map(g => g.id).join(' ');
    expect(ids).toContain('container queries');
    expect(ids).toContain('height media queries');
    expect(ids).toContain('touch/pointer');
    expect(ids).toContain('breakpoints-out-of-range');
  });
  test('summary is a disclosure, native dialog and unscripted roles are gaps, not silence', () => {
    const nodes = [...base,
      { parent: 1, kind: 'element', tag: 'summary', attributes: [], bounds: { x: 0, y: 0, width: 10, height: 10 } }, { parent: 2, kind: 'text', value: 'More' },
      { parent: 1, kind: 'element', tag: 'dialog', attributes: [] },
      { parent: 1, kind: 'element', tag: 'button', attributes: [{ name: 'role', value: 'switch' }], bounds: { x: 0, y: 20, width: 10, height: 10 } }, { parent: 5, kind: 'text', value: 'Dark' }];
    const inv = buildInventory(capture(nodes, ''));
    expect(inv.states.some(s => s.tags.includes('menu') && s.name.endsWith('.open'))).toBe(true);
    const ids = inv.gaps.map(g => g.id).join(' ');
    expect(ids).toContain('dialog:/');
    expect(ids).toContain('unscripted-roles:/');
    expect(inv.gaps.map(g => g.id)).toContain('mobile-emulation');
  });
});

describe('second review regressions', () => {
  const spec = { name: 's', route: '/', viewport: [1440, 900] as [number, number], reducedMotion: false, phase: 'initial' as const, steps: [], tags: [] };
  const both = (ref: Collection, cand: Collection) => {
    const d = diffCollections(ref, cand, opts);
    const [r, c] = gateStates(spec, ref, cand, d, opts, [], [], [true, true]);
    return { ts: d.status === 'different', gate: gateDifferences(r, c).size > 0 };
  };
  test('gate and TS agree on: alt missing vs empty, input value, zero-size extras, aria/data order, focus text', () => {
    const alt = (v: string | null | undefined) => { const c = page(); c.els[3].alt = v as string; return c; };
    expect(both(alt(null), alt(''))).toEqual({ ts: true, gate: true });
    expect(both(alt(''), alt(''))).toEqual({ ts: false, gate: false });
    const withAria = (order: [string, string][]) => { const c = page(); c.els[6].aria = Object.fromEntries(order); return c; };
    expect(both(withAria([['aria-expanded', 'false'], ['aria-label', 'x']]), withAria([['aria-label', 'x'], ['aria-expanded', 'false']]))).toEqual({ ts: false, gate: false });
    const withData = (order: [string, string][]) => { const c = page(); c.els[6].data = Object.fromEntries(order); return c; };
    expect(both(withData([['data-slot', 'a'], ['data-state', 'b']]), withData([['data-state', 'b'], ['data-slot', 'a']]))).toEqual({ ts: false, gate: false });
    const extra = (el: Partial<El>) => { const c = page(); c.els.push({ tag: 'div', x: 0, y: 0, w: 0, h: 0, text: '', ...el } as El); return c; };
    expect(both(page(), extra({ role: 'status' }))).toEqual({ ts: true, gate: true });
    expect(both(page(), extra({ tabindex: '0' }))).toEqual({ ts: true, gate: true });
    expect(both(page(), extra({}))).toEqual({ ts: false, gate: false });
    const role = (explicit: boolean) => { const c = page(); c.els[6].role = 'button'; c.els[6].roleExplicit = explicit; return c; };
    expect(both(role(false), role(true))).toEqual({ ts: false, gate: false }); // <button> vs <button role="button">
  });
  test('gate bounds tolerance is one pixel, like the TS report', () => {
    const near = page(); near.els[1].y += 0.9;
    const far = page(); far.els[1].y += 1.5;
    const g = (c: Collection) => { const d = diffCollections(page(), c, opts); const [r, k] = gateStates(spec, page(), c, d, opts, [], [], [true, true]); return gateDifferences(r, k).size; };
    expect(g(near)).toBe(0);
    expect(g(far)).toBeGreaterThan(0);
  });
  test('data attributes in a different order are not a difference', () => {
    const a = page(), b = page();
    a.els[6].data = { 'data-slot': 'x', 'data-state': 'closed' }; b.els[6].data = { 'data-state': 'closed', 'data-slot': 'x' };
    expect(diffCollections(a, b, opts).status).toBe('identical');
  });
  test('the same console error from two server origins is one error', () => {
    const e = (port: number) => `console: x at http://127.0.0.1:${port}/assets/a.js:1`;
    expect(statusOf(diffRuns(side(page(), { errors: [e(8100)] }), side(page(), { errors: [e(3101)] }), opts))).toBe('identical');
  });
  test('document roots pair even when the page height differs a lot', () => {
    const mk = (h: number) => make([{ tag: 'html', x: 0, y: 0, w: 1440, h }, { tag: 'body', x: 0, y: 0, w: 1440, h }, { tag: 'h1', text: 'Hello world', y: 20 }]);
    const r = diffCollections(mk(2000), mk(2600), opts);
    expect(r.summary.refMissing).toBe(0);
    expect(r.summary.candExtra).toBe(0);
  });
  test('rediff refuses a rediff directory', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'rediff-'));
    writeFileSync(join(dir, 'run.json'), JSON.stringify({ rediffOf: '/somewhere' }));
    const { rediff } = await import('../src/parity/run');
    await expect(rediff(dir, join(dir, 'out'))).rejects.toThrow(/itself a rediff/);
  });
});

describe('base-ui-added-aria (named equivalence agreed with the tokens session)', () => {
  const spec = { name: 's', route: '/', viewport: [1440, 900] as [number, number], reducedMotion: false, phase: 'initial' as const, steps: [], tags: [] };
  const run = (mutRef: (c: Collection) => void, mutCand: (c: Collection) => void) => {
    const ref = page(), cand = page(); mutRef(ref); mutCand(cand);
    const d = diffCollections(ref, cand, opts);
    const [r, c] = gateStates(spec, ref, cand, d, opts, [], [], [true, true]);
    return { status: d.status, rules: d.summary.minorRules, gate: gateDifferences(r, c).size > 0 };
  };
  const BTN = 6; // the page() fixture's <button>
  const none = (c: Collection) => { c.els[BTN].aria = undefined; };
  test('expanded / haspopup / disabled added to a button, link or summary are minor and equal in the gate', () => {
    const r = run(none, c => { c.els[BTN].aria = { 'aria-expanded': 'false', 'aria-haspopup': 'dialog', 'aria-disabled': 'false' }; });
    expect(r).toEqual({ status: 'minor_only', rules: { 'base-ui-added-aria': 3 }, gate: false });
    expect(run(c => { c.els[2].aria = undefined; }, c => { c.els[2].aria = { 'aria-expanded': 'false' }; }).status).toBe('minor_only'); // the <a>
    const summary = (c: Collection) => { c.els[BTN].tag = 'summary'; };
    expect(run(c => { none(c); summary(c); }, c => { summary(c); c.els[BTN].aria = { 'aria-haspopup': 'menu' }; }).status).toBe('minor_only');
    const roleButton = (c: Collection) => { c.els[1].role = 'button'; };
    expect(run(c => roleButton(c), c => { roleButton(c); c.els[1].aria = { 'aria-expanded': 'false' }; }).status).toBe('minor_only');
  });
  test('everything else stays a real difference, in the report and in the gate', () => {
    const cases: [string, (c: Collection) => void, (c: Collection) => void][] = [
      ['aria-expanded added on a plain div', c => { c.els[1].aria = undefined; }, c => { c.els[1].aria = { 'aria-expanded': 'false' }; }],
      ['aria-hidden added to a button', none, c => { c.els[BTN].aria = { 'aria-hidden': 'true' }; }],
      ['aria-modal removed', c => { c.els[1].aria = { 'aria-modal': 'true' }; }, c => { c.els[1].aria = undefined; }],
      ['aria-expanded removed', c => { c.els[BTN].aria = { 'aria-expanded': 'false' }; }, none],
      ['aria-expanded value changed', c => { c.els[BTN].aria = { 'aria-expanded': 'false' }; }, c => { c.els[BTN].aria = { 'aria-expanded': 'true' }; }],
      ['aria-label added', none, c => { c.els[BTN].aria = { 'aria-label': 'Open' }; }],
      ['aria-selected added', none, c => { c.els[BTN].aria = { 'aria-selected': 'true' }; }],
      ['role swapped while aria-expanded is added', none, c => { c.els[BTN].role = 'tab'; c.els[BTN].aria = { 'aria-expanded': 'false' }; }],
      ['tabindex -1 added to a button', none, c => { c.els[BTN].aria = undefined; c.els[BTN].tabindex = '-1'; }],
    ];
    for (const [label, mr, mc] of cases) {
      const r = run(mr, mc);
      expect(r.status, label).toBe('different');
      expect(r.gate, label + ' (gate)').toBe(true);
    }
  });
  test('a status/alert/log region may gain its implicit live attributes, nothing else qualifies', () => {
    const region = (role: string, aria?: Record<string, string>) => (c: Collection) => { c.els[1].role = role; c.els[1].aria = aria; };
    const equivalent: [string, Record<string, string>, number][] = [
      ['status', { 'aria-live': 'polite', 'aria-atomic': 'true' }, 2], ['alert', { 'aria-live': 'assertive', 'aria-atomic': 'true' }, 2], ['log', { 'aria-live': 'polite', 'aria-atomic': 'false' }, 2],
    ];
    for (const [role, aria, n] of equivalent) {
      expect(run(region(role), region(role, aria)), role).toEqual({ status: 'minor_only', rules: { 'base-ui-added-aria': n }, gate: false });
    }
    const different: [string, string, Record<string, string> | undefined, Record<string, string>][] = [
      ['aria-live alone on a main landmark', 'main', undefined, { 'aria-live': 'assertive' }],
      ['assertive on a status region', 'status', undefined, { 'aria-live': 'assertive', 'aria-atomic': 'true' }],
      ['atomic value not implicit', 'status', undefined, { 'aria-live': 'polite', 'aria-atomic': 'false' }],
      ['atomic without live', 'status', undefined, { 'aria-atomic': 'true' }],
      ['reference already polite, atomic added', 'status', { 'aria-live': 'polite' }, { 'aria-live': 'polite', 'aria-atomic': 'true' }],
      ['reference had live off', 'status', { 'aria-live': 'off' }, { 'aria-live': 'polite', 'aria-atomic': 'true' }],
      ['region on a div with no role', '', undefined, { 'aria-live': 'polite', 'aria-atomic': 'true' }],
    ];
    for (const [label, role, refAria, candAria] of different) {
      const r = run(c => { if (role) c.els[1].role = role; c.els[1].aria = refAria; }, c => { if (role) c.els[1].role = role; c.els[1].aria = candAria; });
      expect(r.status, label).toBe('different');
      expect(r.gate, label + ' (gate)').toBe(true);
    }
    // the region role itself changing is a role swap, never excused by the aria addition
    const swapped = run(c => { c.els[1].tag = 'div'; c.els[1].role = ''; c.els[1].aria = undefined; }, c => { c.els[1].tag = 'section'; c.els[1].role = 'region'; c.els[1].aria = { 'aria-live': 'polite', 'aria-atomic': 'true' }; });
    expect(swapped.status).toBe('different');
  });
  test('trigger roles: tabs, comboboxes and menu items never qualify; values must be harmless', () => {
    const asRole = (role: string, aria?: Record<string, string>) => (c: Collection) => { c.els[BTN].role = role; c.els[BTN].aria = aria; };
    for (const role of ['tab', 'combobox', 'menuitem', 'switch', 'option']) {
      expect(run(asRole(role), asRole(role, { 'aria-expanded': 'false' })).status, role).toBe('different');
    }
    expect(run(c => { c.els[2].aria = undefined; c.els[2].role = 'menuitem'; }, c => { c.els[2].role = 'menuitem'; c.els[2].aria = { 'aria-haspopup': 'true' }; }).status).toBe('different');
    for (const [name, value] of [['aria-disabled', 'true'], ['aria-expanded', 'true'], ['aria-haspopup', 'false']]) {
      const r = run(none, c => { c.els[BTN].aria = { [name]: value }; });
      expect(r.status, `${name}=${value}`).toBe('different');
      expect(r.gate, `${name}=${value} (gate)`).toBe(true);
    }
    expect(run(none, c => { c.els[BTN].aria = { 'aria-haspopup': 'true' }; }).status).toBe('minor_only');
  });
  test('an equivalent addition next to a real ARIA difference does not hide it from the gate', () => {
    const r = run(none, c => { c.els[BTN].aria = { 'aria-expanded': 'false', 'aria-hidden': 'true' }; });
    expect(r.status).toBe('different');
    expect(r.gate).toBe(true);
    expect(r.rules['base-ui-added-aria']).toBe(1);
  });
});
