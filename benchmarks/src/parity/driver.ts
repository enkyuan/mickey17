// Headless driver: runs one StateSpec against one origin with Playwright + a Chromium
// binary and returns the collector output. It does not spawn servers; point it at an
// already running `mochi serve <capture>` (original) and generated app (candidate).
// Playwright is resolved at run time (MOCHI_PLAYWRIGHT, local node_modules, gstack)
// so this package gains no dependency. Without a Chromium binary, use the Browser
// pane path in docs/PARITY.md instead.
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import type { Mutation } from './mutations';
import type { Descriptor, SideRun, StateSpec, Step } from './state';

export const COLLECT_JS = resolve(import.meta.dir, '../../../scripts/parity/collect.js');
const SETTLE_MS = 600;

export function loadPlaywright(): any {
  const candidates = [process.env.MOCHI_PLAYWRIGHT, resolve(import.meta.dir, '../../node_modules'), join(homedir(), '.claude/skills/gstack/node_modules')].filter(Boolean) as string[];
  const errors: string[] = [];
  for (const dir of candidates) {
    try { return createRequire(join(dir, 'noop.js'))('playwright-core'); } catch (e) { errors.push(`${dir}: ${(e as Error).message.split('\n')[0]}`); }
  }
  throw new Error('playwright-core not found. Set MOCHI_PLAYWRIGHT to a node_modules directory containing it.\n' + errors.join('\n'));
}
export function findChromium(): string | undefined {
  if (process.env.MOCHI_CHROME) return process.env.MOCHI_CHROME;
  const cache = join(homedir(), 'Library/Caches/ms-playwright'), linux = join(homedir(), '.cache/ms-playwright');
  for (const root of [cache, linux]) {
    if (!existsSync(root)) continue;
    const shells = readdirSync(root).filter(n => n.startsWith('chromium_headless_shell-')).sort().reverse();
    for (const s of shells) {
      const base = join(root, s);
      for (const sub of readdirSync(base)) {
        const exe = join(base, sub, 'chrome-headless-shell');
        if (existsSync(exe)) return exe;
      }
    }
  }
  return undefined; // let Playwright use its own default resolution
}

export interface Side { origin: string; label: string }
export interface RunOptions {
  /** Origins the page may talk to. Everything else is aborted and listed in `blocked`. */
  allowedOrigins: string[];
  mutation?: Mutation;
  /** Internal: this run exists only to take the pixel proof; skip collection. */
  onlyProof?: boolean;
  /** Absolute scroll targets resolved on the reference side (by frac) and replayed on the candidate. */
  scrollY?: Record<number, number>;
  hashAssets?: boolean;
  /** The captured color scheme (inventory.colorScheme); dark when unknown. */
  colorScheme?: 'dark' | 'light';
  /** Force preserveDrawingBuffer on WebGL so canvas pixels can be read (identical on both sides). */
  forceWebglReadback?: boolean;
  /** Serve allowed-origin requests from memory (fixtures, offline captures) instead of the network. */
  fulfill?: (url: URL) => { status: number; contentType: string; body: Buffer | string } | null;
}

const desc = (d: Descriptor) => `${d.tag}${d.role ? '[' + d.role + ']' : ''} "${d.name || d.text}" #${d.nth}`;

export async function launch(pw: any) {
  const executablePath = findChromium();
  return pw.chromium.launch({ executablePath, args: ['--no-sandbox', '--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--force-color-profile=srgb', '--font-render-hinting=none', '--disable-lcd-text'] });
}

/** (Re)install the collector; a click that navigates resets page globals. */
export async function inject(page: any) {
  const has = await page.evaluate(() => Boolean((globalThis as any).__mochiParity));
  if (!has) await page.evaluate('(' + readFileSync(COLLECT_JS, 'utf8') + '\n, 0)');
}

async function settle(page: any, extraMs = 0): Promise<boolean> {
  await page.evaluate(() => document.fonts.ready);
  await inject(page);
  // Settled == the visual signature (scroll, document height, every transformed/transparent box)
  // and the pending-image count are unchanged for ~1.2 s. JS-driven animations are covered too.
  let last = '', stable = 0;
  const end = Date.now() + 10000;
  while (Date.now() < end) {
    const [sig, running] = await page.evaluate(() => [(globalThis as any).__mochiParity.signature() + '|' + [...document.images].filter(i => !i.complete).length, document.getAnimations().filter(a => a.playState === 'running' && Number.isFinite(a.effect?.getComputedTiming?.().endTime ?? Infinity)).length] as [string, number]);
    stable = sig === last ? stable + 1 : 0;
    last = sig;
    // a finite animation still pending (inside its delay) or running means the page has not come to rest, whatever the signature says
    if (stable >= 4 && running === 0) { if (extraMs) await page.waitForTimeout(extraMs); return true; }
    await page.waitForTimeout(300);
  }
  return false;
}

async function scrollTo(page: any, y: number): Promise<number> {
  // Wheel events first, so wheel-driven handlers and smooth scrollers see a user-like scroll.
  // Smooth scrollers overshoot or lag by a few pixels, and the two sides need not do so equally,
  // so the position is then forced to the exact target; both sides must end at the same offset.
  const vh = await page.evaluate(() => innerHeight);
  await page.mouse.move(40, vh / 2);
  for (let guard = 0; guard < 400; guard++) {
    const cur = await page.evaluate(() => Math.round(scrollY));
    const delta = y - cur;
    if (Math.abs(delta) <= 1) break;
    await page.mouse.wheel(0, Math.max(-600, Math.min(600, delta)));
    await page.waitForTimeout(16);
  }
  await page.waitForTimeout(400); // let a smooth scroller finish its tail before correcting
  for (let attempt = 0; attempt < 3; attempt++) {
    if (Math.abs((await page.evaluate(() => Math.round(scrollY))) - y) <= 1) break;
    await page.evaluate((top: number) => window.scrollTo({ top, behavior: 'instant' as ScrollBehavior }), y);
    await page.waitForTimeout(300);
  }
  return page.evaluate(() => Math.round(scrollY));
}

async function handleFor(page: any, d: Descriptor) {
  await inject(page);
  const h = await page.evaluateHandle((x: Descriptor) => (globalThis as any).__mochiParity.find(x), d);
  const el = h.asElement();
  if (!el) { await h.dispose(); return null; }
  return el;
}

export async function runState(browser: any, side: Side, spec: StateSpec, opt: RunOptions): Promise<SideRun> {
  const t0 = Date.now();
  const lap = (what: string) => { if (process.env.MOCHI_PARITY_DEBUG) console.error(`[${side.label}] ${((Date.now() - t0) / 1000).toFixed(1)}s ${what}`); };
  const context = await browser.newContext({ viewport: { width: spec.viewport[0], height: spec.viewport[1] }, deviceScaleFactor: 1, reducedMotion: spec.reducedMotion ? 'reduce' : 'no-preference', colorScheme: opt.colorScheme ?? 'dark', locale: 'en-US', timezoneId: 'UTC', serviceWorkers: 'block' });
  const page = await context.newPage();
  const errors: string[] = [], blocked = new Set<string>(), stepErrors: string[] = [];
  page.on('pageerror', (e: Error) => errors.push('pageerror: ' + e.message.slice(0, 200)));
  page.on('console', (m: any) => {
    if (m.type() !== 'error') return;
    let host = ''; try { host = new URL(m.location().url).host; } catch { /* no location */ }
    if (host && blocked.has(host)) return; // our own abort of a blocked third party, listed in `blocked`
    errors.push('console: ' + m.text().slice(0, 200));
  });
  page.on('response', (r: any) => { const u = new URL(r.url()); if (opt.allowedOrigins.includes(u.origin) && r.status() >= 400) errors.push(`http ${r.status()}: ${u.pathname}`); });
  await page.route('**/*', (route: any) => {
    const u = new URL(route.request().url());
    if (opt.fulfill && opt.allowedOrigins.includes(u.origin)) { const f = opt.fulfill(u); return f ? route.fulfill(f) : route.fulfill({ status: 404, body: '' }); }
    if (u.protocol === 'data:' || u.protocol === 'blob:' || opt.allowedOrigins.includes(u.origin)) return route.continue();
    blocked.add(u.host); return route.abort();
  });
  if (opt.forceWebglReadback) {
    await page.addInitScript(() => {
      const orig = HTMLCanvasElement.prototype.getContext;
      (HTMLCanvasElement.prototype as any).getContext = function (type: string, attrs: any) {
        if (/webgl/.test(type)) attrs = { ...(attrs || {}), preserveDrawingBuffer: true };
        return orig.call(this, type, attrs);
      };
    });
  }
  let clockFrozen: unknown, pendingImages = false, virtualNow = 0;
  const CLOCK_ORIGIN = 1000;
  // Virtual time starts at 0, is paused at CLOCK_ORIGIN (before any page script runs), and from then on moves only
  // through runFor(): load time, font and image waits cannot advance Date, timers or rAF.
  if (spec.clockMs !== undefined) { await page.clock.install({ time: 0 }); await page.clock.pauseAt(CLOCK_ORIGIN); }
  try {
    await page.goto(side.origin + spec.route, { waitUntil: 'load', timeout: 30000 });
    lap('loaded'); await inject(page);
    if (spec.clockMs !== undefined) {
      await page.evaluate(() => document.fonts.ready);
      const imagesDone = await page.waitForFunction(() => [...document.images].every(i => i.complete), null, { timeout: 8000, polling: 100 }).then(() => true, () => false);
      pendingImages = !imagesDone;
      await page.clock.runFor(spec.clockMs);
      clockFrozen = await page.evaluate((ms: number) => (globalThis as any).__mochiParity.freezeAnimations(ms), spec.clockMs);
      virtualNow = await page.evaluate(() => Date.now());
    }
    lap('injected');
    let settled = spec.clockMs !== undefined ? true : await settle(page, 1200); // clock states are fixed by the virtual clock; checked after the steps
    lap('settled=' + settled);
    let applied: Awaited<ReturnType<typeof applyMutation>> | undefined;
    if (opt.mutation?.before) applied = await applyMutation(page, opt.mutation);
    const focusOrder: Record<string, unknown>[] = [];
    let reached = 0;
    for (const step of spec.steps) {
      try { await runStep(page, step, opt, focusOrder, spec.clockMs !== undefined, side.label); } catch (e) { stepErrors.push(`${step.op}: ${(e as Error).message.split('\n')[0]}`); }
    }
    if (spec.steps.length && spec.clockMs === undefined) settled = (await settle(page, 600)) && settled;
    await inject(page);
    reached = await page.evaluate(() => Math.round(scrollY));
    if (opt.mutation && !opt.mutation.before) applied = await applyMutation(page, opt.mutation);
    lap('steps done');
    const collection = await page.evaluate((o: unknown) => (globalThis as any).__mochiParity.collect(o), { hashAssets: opt.hashAssets !== false });
    if (spec.clockMs !== undefined) {
      // animations created after the freeze (scroll/IntersectionObserver/class toggles) run on the real timeline
      const unfrozen: number = await page.evaluate(() => document.getAnimations().filter(a => a.playState === 'running').length);
      if (pendingImages) { settled = false; collection.limits.push('images were still loading when the virtual clock ran; this state is not fixed-time'); }
      if (Math.abs(virtualNow - (CLOCK_ORIGIN + spec.clockMs)) > 1) { settled = false; collection.limits.push(`virtual clock drifted: Date.now() was ${virtualNow}, expected ${CLOCK_ORIGIN + spec.clockMs}`); }
      if (unfrozen) { settled = false; collection.limits.push(`${unfrozen} animation(s) were running after the clock freeze (created after it); this state is not fixed-time`); }
    }
    lap('collected');
    return { collection, focusOrder, errors, blocked: [...blocked].sort(), stepErrors, settled, reachedScrollY: reached, clock: spec.clockMs !== undefined ? { ms: spec.clockMs, frozen: clockFrozen } : undefined, timingMs: Date.now() - t0, mutationChangedComputedStyle: applied?.changed, mutationRules: applied?.bp, mutationChangedProps: applied?.changedProps, mutationPixels: applied?.pixels };
  } finally {
    await context.close();
  }
}

async function runStep(page: any, step: Step, opt: RunOptions, focusOrder: Record<string, unknown>[], fixedClock: boolean, label: string) {
  const advance = async (ms: number) => { if (fixedClock) await page.clock.runFor(ms); else await page.waitForTimeout(ms); };
  switch (step.op) {
    case 'scroll': {
      const y = step.y ?? opt.scrollY?.[step.frac] ?? Math.round(step.frac * (await page.evaluate(() => document.documentElement.scrollHeight - innerHeight)));
      step.y = y; // resolved value is retained in the state record
      await scrollTo(page, y);
      return;
    }
    case 'wait': await advance(step.ms); return;
    case 'press': await page.keyboard.press(step.key); await advance(400); return;
    case 'tabs':
      for (let i = 0; i < step.count; i++) {
        await page.keyboard.press('Tab'); await advance(60);
        focusOrder.push(await page.evaluate(() => { const a = (globalThis as any).__mochiParity.activeElement(); delete a.path; return a; }));
      }
      return;
    default: {
      const el = await handleFor(page, step.target);
      if (!el) throw new Error(`${label}: target not found: ${desc(step.target)}`);
      if (step.op === 'hover') { await el.scrollIntoViewIfNeeded(); await el.hover({ timeout: 5000 }); await advance(700); }
      else if (step.op === 'click') { await el.scrollIntoViewIfNeeded(); await el.click({ timeout: 5000 }); await advance(900); }
      else if (step.op === 'focus') { await el.focus(); await advance(300); }
    }
  }
}

const STYLE_SNAPSHOT = `(() => { const out = []; for (const e of [document.documentElement, ...document.querySelectorAll('*')]) for (const pe of [null, '::before', '::after']) { const c = getComputedStyle(e, pe); const m = {}; for (let i = 0; i < c.length; i++) m[c[i]] = c.getPropertyValue(c[i]); out.push([(pe || '') + (e.tagName + (e.id ? '#' + e.id : '') + '.' + String(e.getAttribute('class') || '').split(' ')[0]), m]); } globalThis.__mochiSnap = out; return globalThis.__mochiParity.hash(JSON.stringify(out)); })()`;
export async function pixelProof(browser: any, side: Side, spec: StateSpec, opt: RunOptions, m: Mutation): Promise<'same' | 'changed' | 'unstable'> {
  // a dedicated page: the screenshots (and the style tag hiding canvas/video) never touch the run that is diffed
  const r = await runState(browser, side, spec, { ...opt, mutation: { ...m, pixelProof: true }, onlyProof: true });
  return r.mutationPixels ?? 'unstable';
}

async function applyMutation(page: any, m: Mutation): Promise<{ changed: boolean; bp?: { rewritten: number; unreadable: number }; changedProps?: string[]; pixels?: 'same' | 'changed' | 'unstable' }> {
  await inject(page);
  // Viewport-sized slices down the page (full-page captures fail on very tall pages). Scrolling reveals content, so
  // the sequence is walked once as a warm-up and only repeat passes are compared.
  const shot = async () => {
    const h = createHash('sha256');
    const { total, vh } = await page.evaluate(() => ({ total: Math.min(document.documentElement.scrollHeight, 40 * innerHeight), vh: innerHeight }));
    for (let y = 0; y < total; y += vh) {
      await page.evaluate((top: number) => window.scrollTo({ top, behavior: 'instant' as ScrollBehavior }), y);
      await page.waitForTimeout(120);
      h.update(await page.screenshot({ animations: 'disabled' }));
    }
    await page.evaluate(() => window.scrollTo({ top: 0, behavior: 'instant' as ScrollBehavior }));
    await page.waitForTimeout(200);
    return h.digest('hex');
  };
  let pixelsBefore: string | undefined, pixelsUnstable = false, hider: any;
  if (m.pixelProof) {
    hider = await page.addStyleTag({ content: 'canvas,video,iframe{visibility:hidden !important}' });
    await page.waitForTimeout(200);
    await shot(); // warm-up: scroll-triggered reveals happen here
    pixelsBefore = await shot();
    pixelsUnstable = pixelsBefore !== (await shot());
  }
  const before = await page.evaluate(STYLE_SNAPSHOT);
  await page.evaluate(() => { (globalThis as any).__mochiSnap0 = (globalThis as any).__mochiSnap; });
  if (m.css) await page.addStyleTag({ content: m.css });
  if (m.js) await page.evaluate(m.js);
  await page.waitForTimeout(150);
  const bp = await page.evaluate(() => (globalThis as any).__mochiBp);
  const afterHash = await page.evaluate(STYLE_SNAPSHOT);
  // which (element, property) pairs moved: lets a calibration blind spot be explained
  const changedProps: string[] = await page.evaluate(() => { const a = (globalThis as any).__mochiSnap0 as [string, Record<string, string>][], b = (globalThis as any).__mochiSnap as [string, Record<string, string>][]; const out: string[] = []; for (let i = 0; i < b.length && out.length < 40; i++) for (const k of Object.keys(b[i][1])) if (!a || !a[i] || a[i][1][k] !== b[i][1][k]) { out.push(`${b[i][0]} ${k}: ${a?.[i]?.[1][k]} -> ${b[i][1][k]}`); break; } return out; });
  let pixels: 'same' | 'changed' | 'unstable' | undefined;
  if (m.pixelProof) {
    pixels = pixelsUnstable ? 'unstable' : (await shot()) === pixelsBefore ? 'same' : 'changed';
    await hider.evaluate((n: Element) => n.remove());
  }
  return { changed: before !== afterHash, bp, changedProps, pixels };
}
