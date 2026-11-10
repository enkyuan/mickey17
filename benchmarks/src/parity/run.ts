// Orchestrates inventory states against a reference origin and a candidate origin,
// retains every raw capture, and writes per-state summaries plus an honest report.
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { readdirSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { COLLECT_JS, launch, loadPlaywright, runState } from './driver';
import { diffRuns, statusOf, type StateResult } from './pair';
import type { Options } from './diff';
import type { Inventory, SideRun, StateSpec } from './state';

export type Profile = 'quick' | 'core' | 'standard' | 'full';
export interface RunConfig {
  inventory: Inventory; reference: string; candidate: string; out: string;
  profile: Profile; only?: string[]; workers: number; hashAssets: boolean; forceWebglReadback: boolean; controls: boolean;
  /** Load the original twice in EVERY executed state (default: initial states only). */
  controlsAll?: boolean;
  /** Absolute path of a generated project; recorded for the Rust gate. */
  project?: string;
}
export const sha = (b: Buffer | string) => createHash('sha256').update(b).digest('hex');
export const write = (p: string, data: unknown) => writeFileSync(p, typeof data === 'string' ? data : JSON.stringify(data, null, 1));
export const writeGz = (p: string, data: unknown) => writeFileSync(p, gzipSync(Buffer.from(JSON.stringify(data)), { level: 6 }));

export function selectStates(inv: Inventory, profile: Profile, only?: string[]): { run: StateSpec[]; skipped: { name: string; reason: string }[] } {
  const firstRoute = inv.routes[0]?.route;
  const run: StateSpec[] = [], skipped: { name: string; reason: string }[] = [];
  const kept = new Map<string, number>();
  for (const s of inv.states) {
    let reason = '';
    if (only?.length && !only.some(o => s.name.includes(o))) reason = 'not selected by --only';
    else if (profile === 'quick' && (s.tags.includes('breakpoint') || !(s.tags.includes('initial') || s.tags.includes('reduced')) || s.phase === 'bottom')) reason = 'profile quick runs initial and reduced-motion initial states only';
    else if (profile === 'standard' && s.tags.includes('breakpoint') && !s.tags.includes('custom') && s.route !== firstRoute) reason = 'profile standard runs breakpoint boundaries for the first route only';
    else if (profile === 'core') {
      const t = s.tags;
      const viewportTag = t.includes('desktop') ? 'desktop' : t.includes('mobile') ? 'mobile' : 'other';
      const capped = ['menu', 'tab', 'hover'].find(k => t.includes(k));
      if (t.includes('breakpoint') && !t.includes('custom')) reason = 'profile core skips breakpoint boundary states (use standard)';
      else if (/scroll\.(down\.(25|75)|up\.(25|75))$/.test(s.name) || s.name.endsWith('reduced.bottom')) reason = 'profile core scripts scroll down 50/100 and up 50/0 only';
      else if (t.includes('entrance') && ![0, 1200].includes(s.clockMs ?? -1)) reason = 'profile core runs entrance times 0 and 1200 ms only';
      else if (capped) {
        const k = `${s.route}|${viewportTag}|${capped}`;
        const n = kept.get(k) ?? 0;
        if (n >= (capped === 'menu' ? 4 : 2)) reason = `profile core keeps the first ${capped === 'menu' ? 4 : 2} ${capped} states per route and viewport`;
        else kept.set(k, n + 1);
      }
    }
    if (reason) skipped.push({ name: s.name, reason }); else run.push(s);
  }
  return { run, skipped };
}

export async function runPair(browser: any, cfg: RunConfig, spec: StateSpec, options: Options) {
  const s = structuredClone(spec);
  const ro = { allowedOrigins: [cfg.reference], hashAssets: cfg.hashAssets, forceWebglReadback: cfg.forceWebglReadback, colorScheme: cfg.inventory.colorScheme };
  const co = { allowedOrigins: [cfg.candidate], hashAssets: cfg.hashAssets, forceWebglReadback: cfg.forceWebglReadback, colorScheme: cfg.inventory.colorScheme };
  const ref = await runState(browser, { origin: cfg.reference, label: 'ref' }, s, ro);
  const cand = await runState(browser, { origin: cfg.candidate, label: 'cand' }, s, co);
  let ref2: SideRun | undefined, cand2: SideRun | undefined;
  if (cfg.controlsAll || (spec.control && cfg.controls)) {
    ref2 = await runState(browser, { origin: cfg.reference, label: 'ref2' }, structuredClone(s), ro);
    cand2 = await runState(browser, { origin: cfg.candidate, label: 'cand2' }, structuredClone(s), co); // the candidate's own nondeterminism floor
  }
  const result = diffRuns(ref, cand, options);
  const noise = ref2 ? diffRuns(ref, ref2, options) : undefined;
  const candNoise = cand2 ? diffRuns(cand, cand2, options) : undefined;
  return { spec: s, ref, cand, ref2, cand2, result, noise, candNoise };
}

export interface StateSummary {
  name: string; route: string; viewport: [number, number]; reducedMotion: boolean; phase: string; tags: string[];
  status: ReturnType<typeof statusOf>; refElements: number; candElements: number; matched: number; refMissing: number; candExtra: number;
  legacyComparable: number; pairsWithDifferences: number; majorItems: number; minorItems: number; maxGeometryDelta: number;
  byDimension: Record<string, number>; minorRules: Record<string, number>; unsettled: string[]; stepErrors: { ref: string[]; cand: string[] };
  errors: { ref: string[]; cand: string[]; shared: string[] }; blocked: { ref: string[]; cand: string[] }; reachedScrollY: [number, number];
  top: { where: string; prop: string; ref: unknown; cand: unknown; dim: string; inNoise?: boolean }[]; missingSample: unknown[]; extraSample: unknown[];
  noise?: { status: string; majorItems: number; minorItems: number; props: string[] };
  /** The candidate loaded twice: its own nondeterminism floor (control states only). */
  candNoise?: { status: string; majorItems: number; minorItems: number; props: string[] }; resolvedSteps: unknown; timingMs: [number, number];
  limits: string[]; assets: { ref?: { hashed: number; unhashed: number }; cand?: { hashed: number; unhashed: number } };
}

export function summarize(spec: StateSpec, x: Awaited<ReturnType<typeof runPair>>): StateSummary {
  const { result: r, ref, cand } = x;
  const normErr = (e: string) => e.replace(/https?:\/\/(127\.0\.0\.1|localhost)(:\d+)?/g, 'ORIGIN');
  const candNorm = new Set(cand.errors.map(normErr)), refNorm = new Set(ref.errors.map(normErr));
  const shared = ref.errors.filter(e => candNorm.has(normErr(e)));
  const top = [...r.pairs.flatMap(p => p.items.filter(i => i.severity === 'major').map(i => ({ where: `${p.ref.tag} ${JSON.stringify(p.ref.text)} @${p.ref.x},${p.ref.y}`, prop: i.prop, ref: i.ref, cand: i.cand, dim: i.dim }))), ...r.pageFacts.filter(i => i.severity === 'major').map(i => ({ where: 'page', prop: i.prop, ref: i.ref, cand: i.cand, dim: i.dim })), ...r.stateItems.map(i => ({ where: 'state', prop: i.prop, ref: i.ref, cand: i.cand, dim: i.dim }))];
  // properties that also differ between two loads of the original in this very state are suspect
  const noiseProps = new Set(Object.keys(x.noise?.summary.byProperty ?? {}));
  for (const t of top) if (noiseProps.has(t.prop)) (t as { inNoise?: boolean }).inNoise = true;
  return {
    name: spec.name, route: spec.route, viewport: spec.viewport, reducedMotion: spec.reducedMotion, phase: spec.phase, tags: spec.tags,
    status: statusOf(r), refElements: r.summary.refElements, candElements: r.summary.candElements, matched: r.summary.matched, refMissing: r.summary.refMissing, candExtra: r.summary.candExtra,
    legacyComparable: r.legacyComparable, pairsWithDifferences: r.summary.pairsWithDifferences, majorItems: r.summary.majorItems, minorItems: r.summary.minorItems, maxGeometryDelta: r.summary.maxGeometryDelta,
    byDimension: r.summary.byDimension, minorRules: r.summary.minorRules, unsettled: r.unsettled, stepErrors: { ref: ref.stepErrors, cand: cand.stepErrors },
    errors: { ref: ref.errors.filter(e => !shared.includes(e)), cand: cand.errors.filter(e => !refNorm.has(normErr(e))), shared }, blocked: { ref: ref.blocked, cand: cand.blocked }, reachedScrollY: [ref.reachedScrollY, cand.reachedScrollY],
    top: top.slice(0, 40), missingSample: r.missing.slice(0, 15), extraSample: r.extra.slice(0, 15),
    candNoise: x.candNoise ? { status: statusOf(x.candNoise), majorItems: x.candNoise.summary.majorItems, minorItems: x.candNoise.summary.minorItems, props: Object.keys(x.candNoise.summary.byProperty).slice(0, 20) } : undefined,
    noise: x.noise ? { status: statusOf(x.noise), majorItems: x.noise.summary.majorItems, minorItems: x.noise.summary.minorItems, props: Object.keys(x.noise.summary.byProperty).slice(0, 20) } : undefined,
    resolvedSteps: x.spec.steps, timingMs: [ref.timingMs, cand.timingMs], limits: r.limits,
    assets: { ref: ref.collection.assets, cand: cand.collection.assets },
  };
}

export async function runAll(cfg: RunConfig, log: (m: string) => void = console.error) {
  if (existsSync(cfg.out)) throw new Error(`output directory exists: ${cfg.out} (reports are never overwritten)`);
  mkdirSync(join(cfg.out, 'states'), { recursive: true });
  const { run: states, skipped } = selectStates(cfg.inventory, cfg.profile, cfg.only);
  const options: Options = { siteOrigins: cfg.inventory.siteOrigins };
  const pw = loadPlaywright();
  const browser = await launch(pw);
  const startedAt = new Date().toISOString();
  write(join(cfg.out, 'inventory.json'), cfg.inventory);
  const runMeta = {
    schema: 1, startedAt, profile: cfg.profile, only: cfg.only ?? null, reference: cfg.reference, candidate: cfg.candidate, project: cfg.project ?? null,
    chromium: await browser.version(), collectorSha256: sha(readFileSync(COLLECT_JS)), inventorySha256: sha(readFileSync(join(cfg.out, 'inventory.json'))),
    pipelineSha256: sha(readdirSync(import.meta.dir).filter(f => f.endsWith('.ts')).sort().map(f => readFileSync(join(import.meta.dir, f), 'utf8')).join('\0')),
    options, hashAssets: cfg.hashAssets, forceWebglReadback: cfg.forceWebglReadback, workers: cfg.workers, scheduled: cfg.inventory.states.length, executed: states.length, skipped,
  };
  write(join(cfg.out, 'run.json'), runMeta);
  const summaries: StateSummary[] = [];
  const failures: { name: string; error: string }[] = [];
  let next = 0;
  const work = async () => {
    while (next < states.length) {
      const spec = states[next++];
      const dir = join(cfg.out, 'states', spec.name);
      mkdirSync(dir, { recursive: true });
      try {
        const x = await runPair(browser, cfg, spec, options);
        writeGz(join(dir, 'ref.json.gz'), x.ref); writeGz(join(dir, 'cand.json.gz'), x.cand);
        if (x.ref2) writeGz(join(dir, 'ref2.json.gz'), x.ref2);
        if (x.cand2) writeGz(join(dir, 'cand2.json.gz'), x.cand2);
        writeGz(join(dir, 'diff.json.gz'), x.result);
        const s = summarize(spec, x);
        write(join(dir, 'summary.json'), s);
        summaries.push(s);
        log(`${String(summaries.length + failures.length).padStart(3)}/${states.length} ${s.status.padEnd(10)} ${spec.name} major=${s.majorItems} minor=${s.minorItems} missing=${s.refMissing} extra=${s.candExtra}`);
      } catch (e) {
        const error = (e as Error).message.split('\n')[0];
        failures.push({ name: spec.name, error }); write(join(dir, 'error.json'), { error });
        log(`     ERROR      ${spec.name}: ${error}`);
      }
    }
  };
  await Promise.all(Array.from({ length: cfg.workers }, work));
  await browser.close();
  write(join(cfg.out, 'failures.json'), failures); // states that crashed are part of the record, never silently absent
  summaries.sort((a, b) => states.findIndex(s => s.name === a.name) - states.findIndex(s => s.name === b.name));
  return { runMeta, summaries, failures, finishedAt: new Date().toISOString() };
}

/** Recompute every summary and the report from the retained raw captures with the current diff code. */
export async function rediff(srcDir: string, outDir: string, log: (m: string) => void = console.error) {
  if (existsSync(outDir)) throw new Error('output directory exists: ' + outDir);
  const { readdirSync: ls } = await import('node:fs');
  const { gunzipSync } = await import('node:zlib');
  const meta = JSON.parse(readFileSync(join(srcDir, 'run.json'), 'utf8'));
  if (meta.rediffOf) throw new Error(`${srcDir} is itself a rediff (of ${meta.rediffOf}); it keeps no raw captures. Rediff the original run directory.`);
  const inventory = JSON.parse(readFileSync(join(srcDir, 'inventory.json'), 'utf8')) as Inventory;
  mkdirSync(join(outDir, 'states'), { recursive: true });
  const summaries: StateSummary[] = [];
  const failures: { name: string; error: string }[] = [];
  for (const name of ls(join(srcDir, 'states'))) {
    const d = join(srcDir, 'states', name);
    if (existsSync(join(d, 'error.json'))) failures.push({ name, error: JSON.parse(readFileSync(join(d, 'error.json'), 'utf8')).error });
    if (!existsSync(join(d, 'ref.json.gz'))) { if (!existsSync(join(d, 'error.json'))) failures.push({ name, error: 'incomplete state directory (no raw capture and no error record)' }); continue; }
    const load = (f: string) => JSON.parse(gunzipSync(readFileSync(join(d, f))).toString()) as SideRun;
    if (!existsSync(join(d, 'cand.json.gz'))) { failures.push({ name, error: 'incomplete state directory (reference capture without a candidate capture)' }); continue; }
    if (existsSync(join(d, 'error.json'))) continue; // recorded as a failure above; never also summarised
    const spec = inventory.states.find(s => s.name === name)!;
    const ref = load('ref.json.gz'), cand = load('cand.json.gz'), ref2 = existsSync(join(d, 'ref2.json.gz')) ? load('ref2.json.gz') : undefined;
    const result = diffRuns(ref, cand, meta.options), noise = ref2 ? diffRuns(ref, ref2, meta.options) : undefined;
    const cand2 = existsSync(join(d, 'cand2.json.gz')) ? load('cand2.json.gz') : undefined;
    const candNoise = cand2 ? diffRuns(cand, cand2, meta.options) : undefined;
    const sum = summarize(spec, { spec, ref, cand, ref2, cand2, result, noise, candNoise });
    mkdirSync(join(outDir, 'states', name), { recursive: true });
    write(join(outDir, 'states', name, 'summary.json'), sum); writeGz(join(outDir, 'states', name, 'diff.json.gz'), result);
    summaries.push(sum);
  }
  summaries.sort((a, b) => inventory.states.findIndex(s => s.name === a.name) - inventory.states.findIndex(s => s.name === b.name));
  write(join(outDir, 'inventory.json'), inventory);
  write(join(outDir, 'failures.json'), failures);
  write(join(outDir, 'run.json'), { ...meta, rediffOf: srcDir, rediffAt: new Date().toISOString(), pipelineSha256Rediff: sha(readdirSync(import.meta.dir).filter(f => f.endsWith('.ts')).sort().map(f => readFileSync(join(import.meta.dir, f), 'utf8')).join('\0')) });
  log(`rediffed ${summaries.length} states`);
  return { runMeta: meta, summaries, failures, finishedAt: new Date().toISOString(), inventory };
}
