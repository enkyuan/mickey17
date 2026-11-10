// State and route inventory derived from a saved capture (never hand-written).
// Sources: manifest.json (routes, external hosts), reconstruction/pages/*.json (the
// captured DOM per viewport: controls, roles, graphics, document height) and the
// captured CSS (media-query breakpoints). Anything detected that has no scripted
// state becomes an explicit gap instead of silently shrinking the matrix.
import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { IMPLICIT_ROLE } from './groups';
import type { Descriptor, Gap, Inventory, StateSpec, Step } from './state';

interface Node { parent: number; kind: string; tag?: string; value?: string; attributes?: { name: string; value: string }[]; bounds?: { x: number; y: number; width: number; height: number } }
interface PageEvidence { viewport: { width: number; height: number }; nodes: Node[] }

const attr = (n: Node, name: string) => n.attributes?.find(a => a.name === name)?.value;
const norm = (s: string) => s.replace(/\s+/g, ' ').trim();
export const slug = (s: string) => s.replace(/[^a-z0-9]+/gi, '-').replace(/^-|-$/g, '').toLowerCase() || 'x';
const routeSlug = (r: string) => (r === '/' ? 'home' : slug(r));

const IMPLICIT = IMPLICIT_ROLE; // the collector's table, so descriptors resolve to the same role at run time

function descriptors(page: PageEvidence) {
  const kids = new Map<number, number[]>();
  page.nodes.forEach((n, i) => { if (n.parent >= 0) (kids.get(n.parent) ?? kids.set(n.parent, []).get(n.parent)!).push(i); });
  const textOf = (i: number): string => {
    const n = page.nodes[i];
    if (n.kind === 'text') return n.value ?? '';
    return (kids.get(i) ?? []).map(textOf).join(''); // textContent semantics: no separator is invented
  };
  const byId = new Map<string, number>();
  page.nodes.forEach((n, i) => { const id = attr(n, 'id'); if (id && !byId.has(id)) byId.set(id, i); });
  const nameOf = (n: Node, i: number): string => {
    const al = attr(n, 'aria-label'); if (al) return norm(al);
    const lb = attr(n, 'aria-labelledby');
    if (lb) { const t = lb.split(/\s+/).map(id => byId.get(id)).filter((x): x is number => x !== undefined).map(x => norm(textOf(x))).join(' '); if (t) return t; }
    const alt = attr(n, 'alt'); if (alt) return norm(alt);
    const title = attr(n, 'title'); if (title) return norm(title);
    return norm(textOf(i)).slice(0, 120);
  };
  // nth counts every element of the same tag, role and name in document order, exactly as the runtime find() does
  const seen = new Map<string, number>();
  const nthOf = new Map<number, number>();
  page.nodes.forEach((n, i) => {
    if (n.kind !== 'element' || !n.tag) return;
    const key = `${n.tag}|${attr(n, 'role') || IMPLICIT[n.tag] || ''}|${nameOf(n, i)}`;
    const k = seen.get(key) ?? 0; seen.set(key, k + 1); nthOf.set(i, k);
  });
  const out: { kind: string; descriptor: Descriptor; expanded?: string; haspopup?: string; controls?: string; bounds?: Node['bounds']; idx: number }[] = [];
  page.nodes.forEach((n, i) => {
    if (n.kind !== 'element' || !n.tag) return;
    const role = attr(n, 'role') || IMPLICIT[n.tag] || '';
    const interactive = ['a', 'button', 'summary', 'input', 'select', 'textarea', 'dialog'].includes(n.tag) || attr(n, 'role') !== undefined || attr(n, 'aria-expanded') !== undefined;
    if (!interactive) return;
    const full = norm(textOf(i));
    const name = nameOf(n, i);
    const nth = nthOf.get(i) ?? 0;
    const kind = n.tag === 'summary' ? 'disclosure' : role === 'tab' ? 'tab' : attr(n, 'aria-haspopup') && attr(n, 'aria-haspopup') !== 'false' ? 'popup-trigger' : attr(n, 'aria-expanded') !== undefined ? 'disclosure' : n.tag === 'a' ? 'link' : role === 'dialog' || role === 'tablist' ? role : n.tag;
    out.push({ kind, descriptor: { tag: n.tag, role, name, text: full.slice(0, 80), nth }, expanded: attr(n, 'aria-expanded'), haspopup: attr(n, 'aria-haspopup'), controls: attr(n, 'aria-controls'), bounds: n.bounds, idx: i });
  });
  return out;
}

function mediaBreakpoints(siteDir: string) {
  const files: string[] = [];
  const notes: string[] = [];
  const features = new Set<string>();
  const walk = (d: string, depth: number) => {
    if (depth > 8 || files.length > 400) { notes.push('breakpoint scan stopped at depth 8 or 400 files'); return; }
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p, depth + 1);
      else if (e.name.endsWith('.css') || e.name.endsWith('.html')) { if (statSync(p).size < 8_000_000) files.push(p); else notes.push(`${e.name} is 8 MB or larger and was not scanned for media queries`); }
    }
  };
  walk(siteDir, 0);
  const found = new Map<string, { px: number; feature: string; files: Set<string> }>();
  for (const f of files) {
    let css = readFileSync(f, 'utf8');
    if (f.endsWith('.html')) {
      if (/<link[^>]+rel=["']stylesheet["'][^>]+media=["'](?!all)[^"']+["']/.test(css)) notes.push(`${f.split('/site/')[1] ?? f} has <link media=…> stylesheets whose conditions are not scanned`);
      css = [...css.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map(m => m[1]).join('\n'); // inline <style> only
    }
    for (const [pat, name] of [[/@container/, 'container queries'], [/\((?:min|max)-height/, 'height media queries'], [/orientation\s*:/, 'orientation media queries'], [/hover\s*:\s*none|pointer\s*:\s*coarse|any-pointer/, 'touch/pointer media queries (hover:none, pointer:coarse)'], [/prefers-color-scheme\s*:\s*light/, 'light color scheme'], [/prefers-contrast|forced-colors/, 'contrast / forced-colors queries'], [/@media[^{]*print/, 'print styles']] as [RegExp, string][]) if (pat.test(css)) features.add(name);
    for (const m of css.matchAll(/@media[^{]*\{/g)) {
      const q = m[0];
      const add = (feature: string, v: string, unit: string) => {
        const px = Math.round(parseFloat(v) * (unit === 'px' ? 1 : 16) * 100) / 100;
        const k = `${feature}:${px}`;
        (found.get(k) ?? found.set(k, { px, feature, files: new Set() }).get(k)!).files.add(f.split('/site/')[1] ?? f);
      };
      for (const n of q.matchAll(/(min|max)-width:\s*([\d.]+)(px|em|rem)/g)) add(n[1] + '-width', n[2], n[3]);
      for (const n of q.matchAll(/([\d.]+)(px|em|rem)\s*(<=|>=|<|>)\s*width/g)) add(n[3].startsWith('<') ? 'min-width' : 'max-width', n[1], n[2]); // reversed range syntax: 768px <= width
      for (const n of q.matchAll(/width\s*(>=|<=|>|<)\s*([\d.]+)(px|em|rem)/g)) add(n[1].startsWith('>') ? 'min-width' : 'max-width', n[2], n[3]);
    }
  }
  return { list: [...found.values()].sort((a, b) => a.px - b.px).map(b => ({ px: b.px, feature: b.feature, files: [...b.files].slice(0, 3) })), notes, features };
}

export interface InventoryOptions {
  scrollFractions?: number[];
  hoverCount?: number;
  tabCount?: number;
  clockTimes?: number[];
  /** Cluster breakpoints closer than this many px (max-width:767 and min-width:768 are one boundary). */
  breakpointCluster?: number;
  breakpointLimit?: number;
  /** Extra viewports to script (e.g. the ~1000px width where a layout was known to break): initial, mid and bottom scroll. */
  extraViewports?: [number, number][];
}

export function buildInventory(captureDir: string, opts: InventoryOptions = {}): Inventory {
  const manifestPath = join(captureDir, 'manifest.json');
  const manifestBytes = readFileSync(manifestPath);
  const manifest = JSON.parse(manifestBytes.toString());
  const siteDir = join(captureDir, 'site');
  const gaps: Gap[] = [];
  const fracs = opts.scrollFractions ?? [0.25, 0.5, 0.75, 1];
  const hoverCount = opts.hoverCount ?? 4, tabCount = opts.tabCount ?? 12;
  const clockTimes = opts.clockTimes ?? [0, 400, 1200, 3000];

  let colorScheme: 'dark' | 'light' = 'dark';
  const siteOrigins = [...new Set<string>((manifest.pages as { final_url: string }[]).map(p => new URL(p.final_url).origin))];
  const routes = (manifest.pages as { final_url: string; title: string; source_evidence?: string[] }[]).map(p => ({ route: new URL(p.final_url).pathname, title: p.title, evidence: p.source_evidence ?? [] }));
  if (new Set(routes.map(r => r.route)).size !== routes.length) throw new Error('duplicate routes in manifest');

  // evidence per route and viewport
  const evidence = new Map<string, Map<string, PageEvidence>>();
  const viewportSet = new Map<string, [number, number]>();
  for (const r of routes) {
    const per = new Map<string, PageEvidence>();
    for (const rel of r.evidence) {
      const m = /-(\d+)x(\d+)\.json$/.exec(rel);
      const file = join(captureDir, rel);
      if (!m || !existsSync(file)) { gaps.push({ id: `evidence-missing:${r.route}:${rel}`, area: 'inventory', detail: `evidence file ${rel} not found; its controls are not enumerated` }); continue; }
      const page = JSON.parse(readFileSync(file, 'utf8')) as PageEvidence & { color_scheme?: string };
      if (page.color_scheme === 'light' || page.color_scheme === 'dark') colorScheme = page.color_scheme;
      per.set(`${m[1]}x${m[2]}`, page);
      viewportSet.set(`${m[1]}x${m[2]}`, [+m[1], +m[2]]);
    }
    evidence.set(r.route, per);
  }
  const viewports = [...viewportSet.values()].sort((a, b) => b[0] - a[0]).map(size => ({ name: size[0] >= 768 ? 'desktop' : 'mobile', size, reason: 'captured viewport in reconstruction/pages' }));
  // the capture's mobile evidence is 375 wide but real phones are 390; keep the captured size as the contract

  // breakpoints
  const scan = mediaBreakpoints(siteDir);
  for (const n of scan.notes) gaps.push({ id: 'css-scan:' + n.slice(0, 40), area: 'responsive', detail: n });
  for (const f of scan.features) gaps.push({ id: 'css-feature:' + f, area: 'responsive', detail: `the captured CSS uses ${f}; no state varies this condition` });
  const outOfRange = scan.list.filter(b => b.px < 320 || b.px > 2400);
  if (outOfRange.length) gaps.push({ id: 'breakpoints-out-of-range', area: 'responsive', detail: `width breakpoints outside 320-2400 px are not scripted: ${outOfRange.map(b => `${b.feature}:${b.px}`).join(', ')}` });
  const bps = scan.list.filter(b => b.px >= 320 && b.px <= 2400);
  const cluster = opts.breakpointCluster ?? 2;
  const clusters: { px: number; feature: string; files: string[] }[] = [];
  for (const b of bps) {
    const last = clusters[clusters.length - 1];
    if (last && b.px - last.px <= cluster) { last.files = [...new Set([...last.files, ...b.files])].slice(0, 3); continue; }
    clusters.push({ ...b });
  }
  const limit = opts.breakpointLimit ?? 24;
  if (clusters.length > limit) gaps.push({ id: 'breakpoints-truncated', area: 'responsive', detail: `${clusters.length} breakpoint clusters found, first ${limit} scripted: ${clusters.slice(limit).map(c => c.px).join(', ')}` });

  const controls: Inventory['controls'] = {};
  const graphics: Inventory['graphics'] = {};
  const states: StateSpec[] = [];
  const add = (s: Omit<StateSpec, 'tags'> & { tags: string[] }) => { if (states.some(x => x.name === s.name)) throw new Error('duplicate state ' + s.name); states.push(s); };
  const sFrac = (f: number): Step => ({ op: 'scroll', frac: f });
  const fq = (f: number) => String(Math.round(f * 100));

  for (const { route } of routes) {
    const rs = routeSlug(route);
    const per = evidence.get(route)!;
    const counts: Record<string, number> = {};
    const routeControls: { kind: string; descriptor: Descriptor }[] = [];
    for (const [vp, page] of per) {
      const d = descriptors(page);
      const vpName = +vp.split('x')[0] >= 768 ? 'desktop' : 'mobile';
      for (const t of ['canvas', 'svg', 'img', 'video', 'picture', 'iframe', 'input', 'select', 'textarea', 'form', 'audio']) counts[`${vpName}.${t}`] = page.nodes.filter(n => n.tag === t).length;
      for (const c of d) routeControls.push({ kind: `${vpName}:${c.kind}`, descriptor: c.descriptor });
    }
    graphics[route] = counts;
    controls[route] = routeControls.filter(c => !c.kind.endsWith(':link')); // link targets are compared as attributes; no state follows a link

    for (const [vp, page] of per) {
      const [w, h] = vp.split('x').map(Number) as [number, number];
      const vpName = w >= 768 ? 'desktop' : 'mobile';
      const base = `${rs}.${vpName}${w}`;
      const size: [number, number] = [w, h];
      add({ name: `${base}.initial`, route, viewport: size, reducedMotion: false, phase: 'initial', steps: [], control: true, tags: ['initial', vpName] });
      add({ name: `${base}.reduced.initial`, route, viewport: size, reducedMotion: true, phase: 'initial', steps: [], tags: ['reduced', vpName] });
      const down: Step[] = [];
      for (const f of fracs) {
        down.push(sFrac(f));
        add({ name: `${base}.scroll.down.${fq(f)}`, route, viewport: size, reducedMotion: false, phase: f === 1 ? 'bottom' : 'scroll', steps: [...down], tags: ['scroll', vpName, f === 1 ? 'bottom' : 'down'] });
      }
      add({ name: `${base}.reduced.bottom`, route, viewport: size, reducedMotion: true, phase: 'bottom', steps: [sFrac(1)], tags: ['reduced', 'scroll', vpName] });
      const upFracs = [...fracs].reverse().slice(1).concat([0]);
      for (const f of upFracs) add({ name: `${base}.scroll.up.${fq(f)}`, route, viewport: size, reducedMotion: false, phase: 'scroll', steps: [...down, ...upFracs.slice(0, upFracs.indexOf(f) + 1).map(sFrac)], tags: ['scroll', 'reverse', vpName] });

      // keyboard
      add({ name: `${base}.keyboard.tab${tabCount}`, route, viewport: size, reducedMotion: false, phase: 'interaction', steps: [{ op: 'tabs', count: tabCount }], tags: ['keyboard', vpName] });
      // controls detected in this viewport's capture
      const d = descriptors(page);
      const seenNames = new Set<string>();
      let hovers = 0;
      for (const c of d) {
        const label = slug(c.descriptor.name || c.descriptor.text || c.kind) + (c.descriptor.nth ? '-' + c.descriptor.nth : '');
        if (c.kind === 'tab') add({ name: `${base}.tab.${label}`, route, viewport: size, reducedMotion: false, phase: 'interaction', steps: [{ op: 'click', target: c.descriptor }], tags: ['tab', 'interaction', vpName] });
        else if (c.kind === 'disclosure' || c.kind === 'popup-trigger') {
          const nm = `${base}.${c.kind === 'disclosure' ? 'menu' : 'popup'}.${label}`;
          add({ name: `${nm}.open`, route, viewport: size, reducedMotion: false, phase: 'interaction', steps: [{ op: 'click', target: c.descriptor }], tags: ['menu', 'interaction', vpName] });
          add({ name: `${nm}.escape`, route, viewport: size, reducedMotion: false, phase: 'interaction', steps: [{ op: 'click', target: c.descriptor }, { op: 'press', key: 'Escape' }], tags: ['menu', 'interaction', vpName] });
          add({ name: `${nm}.toggle-twice`, route, viewport: size, reducedMotion: false, phase: 'interaction', steps: [{ op: 'click', target: c.descriptor }, { op: 'click', target: c.descriptor }], tags: ['menu', 'interaction', vpName] });
        } else if (vpName === 'desktop' && (c.kind === 'link' || c.kind === 'button') && c.bounds && c.bounds.width > 0 && c.bounds.y < 4000 && hovers < hoverCount) {
          const k = c.descriptor.name || c.descriptor.text;
          if (!k || seenNames.has(k)) continue;
          seenNames.add(k); hovers++;
          add({ name: `${base}.hover.${label}`, route, viewport: size, reducedMotion: false, phase: 'interaction', steps: [{ op: 'hover', target: c.descriptor }], tags: ['hover', 'interaction', vpName] });
        }
        if (c.kind === 'dialog' || c.descriptor.tag === 'dialog') gaps.push({ id: `dialog:${route}:${vpName}`, area: 'dialog', detail: `${route}: role=dialog present in captured DOM at ${vp} (${c.descriptor.name || 'unnamed'}); its opening trigger is not statically derivable, so modal/inert/scroll-lock is only checked when a popup-trigger state opens it` });
      }
      const unscripted = new Map<string, number>();
      for (const c of d) if (c.kind !== 'disclosure' && c.kind !== 'popup-trigger' && c.descriptor.role && !['link', 'button', 'dialog', 'tab', 'tablist'].includes(c.descriptor.role)) unscripted.set(c.descriptor.role, (unscripted.get(c.descriptor.role) ?? 0) + 1);
      if (unscripted.size) gaps.push({ id: `unscripted-roles:${route}:${vpName}`, area: 'interaction', detail: `${route}: controls with roles ${[...unscripted].map(([r, n]) => `${r}×${n}`).join(', ')} at ${vp} are listed but no state activates them` });
      if (!d.some(c => c.kind === 'tab') && !d.some(c => c.kind === 'disclosure' || c.kind === 'popup-trigger')) gaps.push({ id: `no-controls:${route}:${vpName}`, area: 'interaction', detail: `${route}: no tab/disclosure/popup controls detected in the capture at ${vp}; no menu, tab or accordion state scripted` });
    }
    // fixed-clock entrance animations (desktop evidence viewport of this route)
    const desk = [...per.keys()].map(k => k.split('x').map(Number) as [number, number]).sort((a, b) => b[0] - a[0])[0];
    if (desk) for (const t of clockTimes) add({ name: `${rs}.desktop${desk[0]}.entrance.t${t}`, route, viewport: desk, reducedMotion: false, phase: 'initial', steps: [], clockMs: t, tags: ['entrance', 'clock', 'desktop'] });
    // breakpoint boundaries (initial state only)
    // N-1, N, N+1 straddle both `min-width:N` (on from N) and `max-width:N` (off from N+1)
    for (const bp of clusters.slice(0, limit)) for (const w of [...new Set([Math.floor(bp.px) - 1, Math.floor(bp.px), Math.ceil(bp.px), Math.ceil(bp.px) + 1])]) {
      if (w < 320) continue;
      if (states.some(s => s.route === route && s.viewport[0] === w && s.tags.includes('breakpoint'))) continue;
      add({ name: `${rs}.bp${w}.initial`, route, viewport: [w, 900], reducedMotion: false, phase: 'initial', steps: [], tags: ['breakpoint', 'initial'] });
    }
    for (const [w, h] of opts.extraViewports ?? []) {
      add({ name: `${rs}.vp${w}x${h}.initial`, route, viewport: [w, h], reducedMotion: false, phase: 'initial', steps: [], tags: ['viewport', 'initial', 'custom'] });
      add({ name: `${rs}.vp${w}x${h}.scroll.down.50`, route, viewport: [w, h], reducedMotion: false, phase: 'scroll', steps: [sFrac(0.5)], tags: ['viewport', 'scroll', 'custom'] });
      add({ name: `${rs}.vp${w}x${h}.scroll.down.100`, route, viewport: [w, h], reducedMotion: false, phase: 'bottom', steps: [sFrac(1)], tags: ['viewport', 'scroll', 'bottom', 'custom'] });
    }
    for (const [k, v] of Object.entries(counts)) {
      if (k.endsWith('.canvas') && v > 0) gaps.push({ id: `canvas:${route}:${k}`, area: 'graphics', detail: `${route}: ${v} canvas element(s) in the captured DOM at ${k.split('.')[0]}: pixels are only comparable if WebGL readback is forced and the animation is frozen at a source-supported time; shader uniforms/draw calls are not read` });
      if (k.endsWith('.video') && v > 0) gaps.push({ id: `video:${route}:${k}`, area: 'graphics', detail: `${route}: ${v} video element(s): frame content is not compared (only src, poster, paused, size)` });
      if (/\.(input|select|textarea)$/.test(k) && v > 0) gaps.push({ id: `form:${route}:${k}`, area: 'forms', detail: `${route}: ${v} ${k.split('.')[1]} control(s): validation, submission and autofill behavior are not exercised (the runner never submits forms)` });
      if (k.endsWith('.iframe') && v > 0) gaps.push({ id: `iframe:${route}:${k}`, area: 'embedding', detail: `${route}: ${v} iframe(s): embedded documents are not collected` });
    }
  }
  const ext: string[] = manifest.external_hosts ?? [];
  if (ext.length) gaps.push({ id: 'external-hosts', area: 'network', detail: `requests to ${ext.filter(Boolean).join(', ')} are blocked on both sides; behavior depending on them is not compared` });
  gaps.push(
    { id: 'mobile-emulation', area: 'responsive', detail: 'mobile states are narrow desktop viewports (no touch, isMobile or device-pixel-ratio emulation): hover/pointer media queries and touch gestures are not exercised; hover states are scripted on desktop only and for at most 4 controls per route' },
    { id: 'color-scheme', area: 'styling', detail: `one color scheme only (the captured one, ${colorScheme}); the other scheme and forced-colors rendering are not compared` },
    { id: 'font-bytes', area: 'typography', detail: 'font files are compared by family/weight/style and the resulting text metrics, not by byte content' },
    { id: 'candidate-only-requests', area: 'network', detail: 'third-party hosts are blocked on both sides, so a candidate that adds or drops a request to one is not detected' },
    { id: 'pointer-driven', area: 'interaction', detail: 'pointermove/drag/touch gestures and cursor-following effects are not scripted; only hover on a bounded set of controls is' },
    { id: 'smooth-scroll-curves', area: 'scroll', detail: 'scroll positions are reached with wheel steps and compared after settling; per-frame scroll-linked values between positions are not sampled' },
    { id: 'js-animation-on-pane', area: 'animation', detail: 'fixed-clock states require the headless driver (Playwright clock); the Browser pane path compares real-time settled states only' },
    { id: 'whole-states-only', area: 'coverage', detail: 'states are independent page loads; sequences longer than the scripted steps, and state carried across routes, are not covered' },
  );
  return {
    schema: 1, colorScheme,
    generatedFrom: { capture: captureDir, manifestSha256: createHash('sha256').update(manifestBytes).digest('hex'), generator: 'benchmarks/src/parity/inventory.ts' },
    siteOrigins, routes, viewports, breakpoints: clusters, controls, graphics, states, gaps,
  };
}
