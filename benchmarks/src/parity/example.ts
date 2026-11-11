// Runnable end-to-end example, offline and port-free: bun benchmarks/src/parity/example.ts [--out dir]
// Two fixture sites are served through request interception (no sockets):
//   reference  the "original"
//   candidate  same rendering, different markup (article instead of div): must be accepted everywhere
// and nine VARIANTS of the reference, each with exactly one planted regression. A variant must be
// rejected in the state that exposes it and must stay identical in a state that cannot see it
// (a hover rule is invisible without hover; a 700px breakpoint is invisible at 1280px).
// Needs Playwright + a Chromium binary (see docs/parity.md); otherwise it exits 3 and says why.
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { extname, join, resolve } from 'node:path';
import { launch, loadPlaywright, runState } from './driver';
import { diffRuns, statusOf } from './pair';
import { write } from './run';
import type { StateSpec } from './state';

const fixtures = resolve(import.meta.dir, '../../fixtures/parity');
const types: Record<string, string> = { '.html': 'text/html', '.css': 'text/css', '.png': 'image/png' };
const OTHER_PIXEL = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNkYPgPAAEDAQAIicLsAAAAAElFTkSuQmCC', 'base64');
type Edit = (file: 'html' | 'css', text: string) => string;
const serve = (site: 'reference' | 'candidate', edit?: Edit, otherImage = false) => (u: URL) => {
  const rel = u.pathname === '/' || !extname(u.pathname) ? 'index.html' : u.pathname.slice(1);
  const file = join(fixtures, site, rel);
  if (!existsSync(file)) return null;
  let body: Buffer | string = readFileSync(file);
  if (otherImage && rel === 'pixel.png') body = OTHER_PIXEL;
  else if (edit && (rel === 'index.html' || rel === 'style.css')) body = edit(rel === 'index.html' ? 'html' : 'css', body.toString());
  return { status: 200, contentType: types[extname(file)] ?? 'application/octet-stream', body };
};
const must = (text: string, from: string | RegExp, to: string) => { const out = text.replace(from, to); if (out === text) throw new Error('fixture edit did not apply: ' + from); return out; };
export interface Variant { name: string; edit?: Edit; otherImage?: boolean; seenIn: string; expect: string; unseenIn: string }
export const VARIANTS: Variant[] = [
  { name: 'letter-spacing', edit: (f, t) => (f === 'css' ? must(t, 'letter-spacing:-.02em', 'letter-spacing:.05em') : t), seenIn: 'initial.desktop', expect: 'typography:style.letterSpacing', unseenIn: '' },
  { name: 'missing-card', edit: (f, t) => (f === 'html' ? must(t, /<div class="card"><h3>Card 3<\/h3>.*?<\/div>/, '') : t), seenIn: 'initial.desktop', expect: 'missing', unseenIn: '' },
  { name: 'link-target', edit: (f, t) => (f === 'html' ? must(t, 'href="/docs"', 'href="/wrong"') : t), seenIn: 'initial.desktop', expect: 'routing:href', unseenIn: '' },
  { name: 'svg-path', edit: (f, t) => (f === 'html' ? must(t, 'M2 2 L18 2 L10 18 Z', 'M2 2 L18 2 L10 14 Z') : t), seenIn: 'initial.desktop', expect: 'graphics:svg.paths', unseenIn: '' },
  { name: 'image-bytes', otherImage: true, seenIn: 'initial.desktop', expect: 'assets:srcSha', unseenIn: '' },
  { name: 'body-background', edit: (f, t) => (f === 'css' ? must(t, 'background:#fafafa}', 'background:#fff}') : t), seenIn: 'initial.desktop', expect: 'styling:style.backgroundColor', unseenIn: '' },
  { name: 'breakpoint', edit: (f, t) => (f === 'css' ? must(t, '@media (max-width:700px)', '@media (max-width:1100px)') : t), seenIn: 'breakpoint.1000', expect: 'layout:', unseenIn: 'initial.desktop' },
  { name: 'hover-rule', edit: (f, t) => (f === 'css' ? must(t, 'nav a:hover,nav button:hover{background:#def}', 'nav a:hover,nav button:hover{background:#fdd}') : t), seenIn: 'hover.docs', expect: 'styling:style.backgroundColor', unseenIn: 'initial.desktop' },
  { name: 'focus-ring', edit: (f, t) => (f === 'css' ? must(t, 'outline:2px solid #06c}', 'outline:none;box-shadow:0 0 0 3px #06c}') : t), seenIn: 'keyboard.tab4', expect: 'behavior:focus.order', unseenIn: 'initial.desktop' },
  { name: 'menu-handler', edit: (f, t) => (f === 'html' ? must(t, "b.addEventListener('click'", "b.addEventListener('noop'") : t), seenIn: 'menu.open', expect: 'behavior:aria-expanded', unseenIn: 'initial.desktop' },
];
const D = { tag: 'button', role: 'button', name: 'Menu', text: 'Menu', nth: 0 };
const link = { tag: 'a', role: 'link', name: 'Docs', text: 'Docs', nth: 0 };
export const STATES: StateSpec[] = [
  { name: 'initial.desktop', route: '/', viewport: [1280, 800], reducedMotion: false, phase: 'initial', steps: [], tags: ['initial'] },
  { name: 'initial.mobile', route: '/', viewport: [390, 800], reducedMotion: false, phase: 'initial', steps: [], tags: ['initial'] },
  { name: 'breakpoint.1000', route: '/', viewport: [1000, 800], reducedMotion: false, phase: 'initial', steps: [], tags: ['breakpoint'] },
  { name: 'scroll.down', route: '/', viewport: [1280, 800], reducedMotion: false, phase: 'bottom', steps: [{ op: 'scroll', frac: 1 }], tags: ['scroll'] },
  { name: 'scroll.up', route: '/', viewport: [1280, 800], reducedMotion: false, phase: 'scroll', steps: [{ op: 'scroll', frac: 1 }, { op: 'scroll', frac: 0.4 }], tags: ['scroll'] },
  { name: 'menu.open', route: '/', viewport: [1280, 800], reducedMotion: false, phase: 'interaction', steps: [{ op: 'click', target: D }], tags: ['menu'] },
  { name: 'menu.escape', route: '/', viewport: [1280, 800], reducedMotion: false, phase: 'interaction', steps: [{ op: 'click', target: D }, { op: 'press', key: 'Escape' }], tags: ['menu'] },
  { name: 'hover.docs', route: '/', viewport: [1280, 800], reducedMotion: false, phase: 'interaction', steps: [{ op: 'hover', target: link }], tags: ['hover'] },
  { name: 'keyboard.tab4', route: '/', viewport: [1280, 800], reducedMotion: false, phase: 'interaction', steps: [{ op: 'tabs', count: 4 }], tags: ['keyboard'] },
  { name: 'entrance.t200', route: '/', viewport: [1280, 800], reducedMotion: false, phase: 'initial', steps: [], clockMs: 200, tags: ['entrance'] },
  { name: 'reduced.initial', route: '/', viewport: [1280, 800], reducedMotion: true, phase: 'initial', steps: [], tags: ['reduced'] },
];

export interface ExampleOutcome {
  candidate: Record<string, ReturnType<typeof statusOf>>;
  variants: Record<string, { seenIn: ReturnType<typeof statusOf>; items: string[]; unseenIn?: ReturnType<typeof statusOf> }>;
}
const describeItems = (r: ReturnType<typeof diffRuns>) => [...new Set([...r.pairs.flatMap(p => p.items.map(i => `${i.dim}:${i.prop}${i.severity === 'minor' ? '(minor)' : ''}`)), ...r.pageFacts.map(i => `${i.dim}:${i.prop}`), ...r.stateItems.map(i => `${i.dim}:${i.prop.replace(/\[\d+\]/, '')}`), ...(r.summary.refMissing ? ['missing'] : []), ...(r.summary.candExtra ? ['extra'] : [])])];

export async function runExample(out?: string, log: (m: string) => void = console.log): Promise<ExampleOutcome> {
  const pw = loadPlaywright();
  const browser = await launch(pw);
  const O = 'http://parity.test';
  const byName = Object.fromEntries(STATES.map(s => [s.name, s]));
  const pair = async (spec: StateSpec, cand: ReturnType<typeof serve>) => {
    const s = structuredClone(spec);
    const ref = await runState(browser, { origin: O, label: 'ref' }, s, { allowedOrigins: [O], fulfill: serve('reference') });
    const c = await runState(browser, { origin: O, label: 'cand' }, s, { allowedOrigins: [O], fulfill: cand });
    return diffRuns(ref, c, { siteOrigins: [O] });
  };
  if (out) mkdirSync(out, { recursive: true });
  const outcome: ExampleOutcome = { candidate: {}, variants: {} };
  for (const spec of STATES) {
    const r = await pair(spec, serve('candidate'));
    outcome.candidate[spec.name] = statusOf(r);
    log(`equivalent-markup  ${spec.name.padEnd(16)} ${statusOf(r).padEnd(10)} ${describeItems(r).join(' ')}`);
  }
  for (const v of VARIANTS) {
    const cand = serve('reference', v.edit, v.otherImage);
    const seen = await pair(byName[v.seenIn], cand);
    const entry: ExampleOutcome['variants'][string] = { seenIn: statusOf(seen), items: describeItems(seen) };
    log(`${v.name.padEnd(18)} ${v.seenIn.padEnd(16)} ${entry.seenIn.padEnd(10)} ${entry.items.slice(0, 4).join(' ')}`);
    if (v.unseenIn) { entry.unseenIn = statusOf(await pair(byName[v.unseenIn], cand)); log(`${v.name.padEnd(18)} ${v.unseenIn.padEnd(16)} ${entry.unseenIn} (must not see it)`); }
    outcome.variants[v.name] = entry;
  }
  await browser.close();
  if (out) write(join(out, 'example-results.json'), outcome);
  return outcome;
}
export function verdict(o: ExampleOutcome) {
  const problems: string[] = [];
  for (const [s, st] of Object.entries(o.candidate)) if (st !== 'identical' && st !== 'minor_only') problems.push(`equivalent markup rejected in ${s}: ${st}`);
  for (const v of VARIANTS) {
    const r = o.variants[v.name];
    if (r.seenIn !== 'different') problems.push(`${v.name} not rejected in ${v.seenIn}: ${r.seenIn}`);
    if (!r.items.some(i => i.startsWith(v.expect))) problems.push(`${v.name}: expected ${v.expect} among ${r.items.join(', ')}`);
    if (v.unseenIn && r.unseenIn !== 'identical' && r.unseenIn !== 'minor_only') problems.push(`${v.name} unexpectedly visible in ${v.unseenIn}: ${r.unseenIn}`);
  }
  return problems;
}

if (import.meta.main) {
  const i = process.argv.indexOf('--out');
  try {
    const problems = verdict(await runExample(i < 0 ? undefined : process.argv[i + 1]));
    console.log(problems.length ? 'example FAILED:\n' + problems.join('\n') : 'example OK: equivalent markup accepted; every planted regression rejected where visible and not elsewhere');
    process.exit(problems.length ? 1 : 0);
  } catch (e) {
    console.error('cannot run the example: ' + (e as Error).message.split('\n')[0]);
    process.exit(3);
  }
}
