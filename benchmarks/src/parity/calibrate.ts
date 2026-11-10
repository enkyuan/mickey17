// Calibration: how sensitive is the pipeline? The reference is loaded twice (an identical
// control), a known regression is injected into the second load, and the diff must report
// it. Misses are retained and fail the run. Blind-spot entries (expect 'miss') are the
// documented limits of a state kind. This measures the instrument, not the candidate.
// Every mutated capture and its diff is kept, so each detection can be audited.
import { existsSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { gateDifferences, gateStates, type GateState } from './facts';
import { launch, loadPlaywright, pixelProof, runState } from './driver';
import { breakpointMutation, MUTATIONS, type Mutation } from './mutations';
import { diffRuns, statusOf } from './pair';
import { write, writeGz } from './run';
import type { Inventory, StateSpec } from './state';

export interface CalibrationRow {
  mutation: string; dimension: string; state: string; scope: string; expect: 'detect' | 'miss'; detected: boolean; gateDetected: boolean;
  /** For undetected breakpoint shifts: did any resolved computed style of any element change at all? */
  computedStyleChanged?: boolean; pixels?: string; rulesRewritten?: number; changedProps?: string[]; unreadableSheets?: number; verdict: 'ok' | 'FAIL' | 'proven-no-op' | 'not-applicable' | 'unproven';
  baselineItems: number; mutatedItems: number; delta: number; dimensions: Record<string, number>; sample: string[]; note?: string; evidence: string;
}
const items = (r: ReturnType<typeof diffRuns>) => r.summary.majorItems + r.summary.refMissing + r.summary.candExtra;
/** The kinds of difference a diff contains: majors by property, plus missing / extra elements. */
const kinds = (r: ReturnType<typeof diffRuns>) => new Set<string>([...r.pairs.flatMap(p => p.items.filter(i => i.severity === 'major').map(i => i.prop)), ...r.pageFacts.filter(i => i.severity === 'major').map(i => i.prop), ...r.stateItems.map(i => i.prop), ...(r.summary.refMissing ? ['<missing element>'] : []), ...(r.summary.candExtra ? ['<extra element>'] : [])]);
/** Property kinds a mutation of a given dimension may legitimately surface. A new kind elsewhere is coincidence, not detection. */
const SEES: Record<string, string[]> = {
  styling: ['style.', 'effOpacity', 'effFx', 'effHidden', '<extra', '<missing', 'bgSha', 'maskSha', 'fact.'],
  typography: ['style.', 'bounds.', 'text', 'fact.'],
  layout: ['bounds.', 'style.', 'fact.scroll', 'fact.main', '<extra', '<missing', 'scroll.'],
  content: ['text', 'textHash', 'pseudo.content', '<extra', '<missing', 'fact.', 'accessibleName'],
  assets: ['<extra', '<missing', 'src', 'srcSha', 'bgSha', 'maskSha', 'nat', 'complete', 'style.backgroundImage', 'style.maskImage'],
  graphics: ['svg', 'vb', 'pixel', 'cw', 'ch', 'par', 'style.', 'bounds.', '<extra', '<missing', 'fact.count'],
  behavior: ['aria', 'tabindex', 'effHidden', 'inert', 'focus', 'step', 'style.', '<extra', '<missing', 'fact.', 'open', 'disabled', 'checked', 'value'],
  routing: ['href', 'target', 'rel', 'download'],
  semantics: ['tag', 'role', 'accessibleName', 'data-attributes', '<extra', '<missing', 'fact.landmarks', 'fact.count'],
  motion: ['style.transition', 'style.animation', 'style.willChange', 'fact.animations'],
  interaction: ['style.', 'focus.order'],
};
const plausible = (dimension: string, kind: string) => (SEES[dimension] ?? []).some(p => kind.startsWith(p));
const slug = (s: string) => s.replace(/[^a-z0-9.-]+/gi, '_');

export async function calibrate(inv: Inventory, reference: string, out: string, opts: { hashAssets: boolean; log?: (m: string) => void }) {
  if (existsSync(out)) throw new Error(`output directory exists: ${out}`);
  mkdirSync(join(out, 'evidence'), { recursive: true });
  const log = opts.log ?? console.error;
  const route = inv.routes[0].route;
  const pick = (f: (s: StateSpec) => boolean) => inv.states.find(s => s.route === route && f(s));
  const pickAny = (f: (s: StateSpec) => boolean) => inv.states.filter(s => f(s)); // candidates across routes, first route first
  const stateFor: Record<string, StateSpec | undefined> = {
    initial: pick(s => s.tags.includes('initial') && s.tags.includes('desktop') && !s.tags.includes('breakpoint')),
    hover: pick(s => s.tags.includes('hover')),
    keyboard: pick(s => s.tags.includes('keyboard') && s.tags.includes('desktop')),
    menu: pick(s => s.tags.includes('menu') && s.name.endsWith('.open')),
  };
  const pw = loadPlaywright(), browser = await launch(pw);
  const rows: CalibrationRow[] = [], notApplicable: { mutation: string; scope: string; reason: string }[] = [];
  const usedStates = new Set<string>(); // baselines of states that were only probed (inert menus) do not decide the outcome
  const baselines = new Map<string, { refRun: Awaited<ReturnType<typeof runState>>; items: number; detail: ReturnType<typeof diffRuns>; gateDiff: Set<string>; status: string }>();
  const allow = { allowedOrigins: [reference], hashAssets: opts.hashAssets, colorScheme: inv.colorScheme };
  const options = { siteOrigins: inv.siteOrigins };
  const baseline = async (spec: StateSpec) => {
    let b = baselines.get(spec.name);
    if (!b) {
      const s = structuredClone(spec);
      const refRun = await runState(browser, { origin: reference, label: 'ref' }, s, allow);
      const again = await runState(browser, { origin: reference, label: 'control' }, structuredClone(s), allow);
      const detail = diffRuns(refRun, again, options);
      const gsBase = gateStates(spec, refRun.collection, again.collection, detail, options, refRun.errors, again.errors, [refRun.settled, again.settled]);
      b = { refRun, items: items(detail), detail, gateDiff: gateDifferences(gsBase[0] as GateState, gsBase[1] as GateState), status: statusOf(detail) };
      baselines.set(spec.name, b);
      log(`baseline ${spec.name}: ${statusOf(detail)} (${b.items} items)`);
      writeGz(join(out, 'evidence', `baseline.${slug(spec.name)}.json.gz`), { reference: refRun, control: again, diff: detail });
    }
    return { spec: structuredClone(spec), b };
  };
  const trial = async (m: Mutation, spec: StateSpec, scope: string) => {
    try { await trialInner(m, spec, scope); } catch (e) {
      // a crash is a failed row, never a lost calibration
      const msg = (e as Error).message.split('\n')[0];
      rows.push({ mutation: m.name, dimension: m.dimension, state: spec.name, scope, expect: m.expect, detected: false, gateDetected: false, verdict: 'FAIL', baselineItems: 0, mutatedItems: 0, delta: 0, dimensions: {}, sample: ['crashed: ' + msg], note: m.note, evidence: '' });
      log(`FAIL         ${m.name} ${scope} crashed: ${msg}`);
    }
  };
  const trialInner = async (m: Mutation, spec: StateSpec, scope: string) => {
    const { spec: s, b } = await baseline(spec);
    usedStates.add(spec.name);
    if (m.requires) {
      const els = b.refRun.collection.els;
      const has = m.requires === 'pseudo' ? b.refRun.collection.pseudos.length > 0 : m.requires.startsWith('aria:') ? els.some(e => e.aria && m.requires!.slice(5) in e.aria) : els.some(e => e.tag === m.requires);
      if (!has) { notApplicable.push({ mutation: m.name, scope, reason: `${spec.name} has no ${m.requires}` }); return; }
    }
    // the diffed run never takes screenshots; the pixel proof (if needed) runs on its own page afterwards
    const mutated = await runState(browser, { origin: reference, label: 'mutated' }, s, { ...allow, mutation: { ...m, pixelProof: false } });
    const d = diffRuns(b.refRun, mutated, options);
    // detected == the mutated diff contains a kind of difference the identical control did not (the control may
    // carry timing noise, which then cannot hide a mutation unless the mutation produces exactly the noisy kind)
    // ... and, to rule out coincidence, one that the mutation's own dimension can produce
    const baseKinds = kinds(b.detail);
    const newKinds = [...kinds(d)].filter(k => !baseKinds.has(k));
    const detected = newKinds.some(k => plausible(m.dimension, k));
    // the same pair through the Rust-gate fact emitter: it must also see the regression
    const gs = gateStates(spec, b.refRun.collection, mutated.collection, d, options, b.refRun.errors, mutated.errors, [b.refRun.settled, mutated.settled], [{ focusOrder: b.refRun.focusOrder, stepErrors: b.refRun.stepErrors, reachedScrollY: b.refRun.reachedScrollY }, { focusOrder: mutated.focusOrder, stepErrors: mutated.stepErrors, reachedScrollY: mutated.reachedScrollY }]);
    const controlGate = b.gateDiff;
    const gateDetected = [...gateDifferences(gs[0], gs[1])].some(k => !controlGate.has(k));
    const all = [...d.pairs.flatMap(p => p.items.filter(i => i.severity === 'major').map(i => `${p.ref.tag} ${i.prop}: ${JSON.stringify(i.ref)?.slice(0, 40)} -> ${JSON.stringify(i.cand)?.slice(0, 40)}`)), ...d.pageFacts.filter(i => i.severity === 'major').map(i => `page ${i.prop}`), ...d.stateItems.map(i => `state ${i.prop}`), ...(d.summary.refMissing ? [`${d.summary.refMissing} missing`] : []), ...(d.summary.candExtra ? [`${d.summary.candExtra} extra`] : [])];
    const dims: Record<string, number> = {};
    for (const p of d.pairs) for (const i of p.items) if (i.severity === 'major') dims[i.dim] = (dims[i.dim] ?? 0) + 1;
    for (const i of [...d.pageFacts, ...d.stateItems]) if (i.severity === 'major') dims[i.dim] = (dims[i.dim] ?? 0) + 1;
    const evidence = join('evidence', `${slug(m.name)}.${slug(spec.name)}.json.gz`);
    writeGz(join(out, evidence), { mutation: m, state: s, mutated, diff: d, gateFactsDiffer: gateDetected });
    const isBp = scope === 'breakpoint';
    const proof = mutated.mutationChangedComputedStyle, rules = mutated.mutationRules;
    // the pixel oracle costs minutes per row and is unreliable on a loaded machine: opt-in only (MOCHI_PIXEL_PROOF=1)
    if (m.pixelProof && process.env.MOCHI_PIXEL_PROOF && !detected && rules && rules.rewritten > 0 && rules.unreadable === 0) mutated.mutationPixels = await pixelProof(browser, { origin: reference, label: 'proof' }, structuredClone(spec), allow, m);
    let verdict: CalibrationRow['verdict'];
    // a shift is a proven no-op only if rules were actually rewritten AND no resolved style (incl. ::before/::after) moved;
    // nothing rewritten (or unreadable sheets) means the shift was never applied: untested, which fails
    // no rule near this width exists in any stylesheet the route loads (and none was unreadable): the breakpoint belongs to another route
    // proven no-op needs an independent oracle: rules were rewritten and the rendered full page (canvas/video hidden)
    // is pixel-identical before and after. Style changes confined to uncollected wrappers with no pixel effect are
    // therefore fine; a pixel change nothing detected is a blind spot; an unstable page is unproven.
    if (isBp && !detected) verdict = rules && rules.rewritten === 0 && rules.unreadable === 0 ? 'not-applicable' : rules && rules.rewritten > 0 && rules.unreadable === 0 && mutated.mutationPixels === 'same' ? 'proven-no-op' : rules && rules.rewritten > 0 && rules.unreadable === 0 && mutated.mutationPixels === undefined ? 'unproven' : 'FAIL';
    else verdict = (m.expect === 'detect') === detected ? 'ok' : 'FAIL';
    // the gate must agree with the TS report for every expected detection
    if (verdict === 'ok' && m.expect === 'detect' && !gateDetected) verdict = 'FAIL';
    const row: CalibrationRow = { mutation: m.name, dimension: m.dimension, state: spec.name, scope, expect: m.expect, detected, gateDetected, computedStyleChanged: proof, pixels: mutated.mutationPixels, changedProps: mutated.mutationChangedProps?.slice(0, 6), rulesRewritten: rules?.rewritten, unreadableSheets: rules?.unreadable, verdict, baselineItems: b.items, mutatedItems: items(d), delta: items(d) - b.items, dimensions: dims, sample: all.slice(0, 3), note: m.note, evidence };
    rows.push(row);
    log(`${verdict.padEnd(12)} ${m.name.padEnd(26)} ${scope.padEnd(10)} expect=${m.expect} detected=${detected} gate=${gateDetected} (+${row.delta}) ${spec.name}`);
  };
  // Hover and keyboard states are only informative when the identical control is clean. If the first route's state
  // is noisy (the original differs from itself), use the first desktop state of any route whose control is clean and
  // record the noisy one; the noisy control is itself a finding about that state.
  const noisyControls: string[] = [];
  for (const [scope, tag] of [['keyboard', 'keyboard'], ['hover', 'hover']] as const) {
    for (const cand of pickAny(s => s.tags.includes(tag) && s.tags.includes('desktop')).slice(0, 6)) {
      const clean = (await baseline(cand)).b;
      if (clean.items === 0 && ['identical', 'minor_only'].includes(clean.status)) { stateFor[scope] = cand; break; }
      noisyControls.push(`${cand.name} (${clean.items} items)`);
    }
  }
  if (noisyControls.length) log(`states with a noisy identical control (not used for calibration): ${noisyControls.join(', ')}`);
  // A menu state can only calibrate click handling if the click changes the ORIGINAL at all. Take the first
  // `.open` state whose reference differs from the initial state of the same viewport; list the inert ones.
  const inertMenus: string[] = [];
  stateFor.menu = undefined; // no menu state is used unless one is shown to change the original
  const menuCandidates = inv.states.filter(s => s.route === route && s.tags.includes('menu') && s.name.endsWith('.open'));
  for (const cand of menuCandidates.slice(0, 8)) {
    const initial = inv.states.find(s => s.route === route && s.tags.includes('initial') && !s.tags.includes('breakpoint') && !s.tags.includes('custom') && s.viewport.join() === cand.viewport.join() && !s.reducedMotion);
    if (!initial) continue;
    const a = (await baseline(initial)).b.refRun, c = (await baseline(cand)).b.refRun;
    if (diffRuns(a, c, options).summary.pairsWithDifferences + diffRuns(a, c, options).summary.refMissing + diffRuns(a, c, options).summary.candExtra > 0) { stateFor.menu = cand; break; }
    inertMenus.push(cand.name);
  }
  if (inertMenus.length) log(`menu states where the click changes nothing in the original (cannot calibrate handlers): ${inertMenus.join(', ')}`);
  for (const m of MUTATIONS) {
    const scope = m.scope[0];
    const spec = stateFor[scope];
    if (!spec) { notApplicable.push({ mutation: m.name, scope, reason: `no ${scope} state in the inventory for ${route}` }); continue; }
    await trial(m, spec, scope);
  }
  // changed breakpoint: every breakpoint cluster of the first route, tested at the widths that straddle it
  const bpStates = inv.states.filter(s => s.route === route && s.tags.includes('breakpoint'));
  const clusters = inv.breakpoints.slice(0, 24);
  const byCluster: { px: number; rows: CalibrationRow[] }[] = [];
  for (const cluster of clusters) {
    const before = rows.length;
    // N-1, N, N+1: strict (`width < N`, `width > N`) and inclusive conditions switch on different sides of N
    const widths = [...new Set([Math.floor(cluster.px) - 1, Math.floor(cluster.px), Math.ceil(cluster.px), Math.ceil(cluster.px) + 1])];
    for (const w of widths) {
      const spec = bpStates.find(s => s.viewport[0] === w);
      if (spec) await trial(breakpointMutation(cluster.px), spec, 'breakpoint');
    }
    byCluster.push({ px: cluster.px, rows: rows.slice(before) });
  }
  await browser.close();
  const hard = rows.filter(r => r.scope !== 'breakpoint');
  const bpSummary = byCluster.map(c => ({ px: c.px, detected: c.rows.some(r => r.detected), provenNoOp: c.rows.length > 0 && c.rows.every(r => r.verdict === 'proven-no-op' || r.detected), notApplicable: c.rows.length > 0 && c.rows.every(r => r.verdict === 'not-applicable'), unproven: c.rows.some(r => r.verdict === 'unproven') && !c.rows.some(r => r.detected || r.verdict === 'FAIL'), failed: c.rows.some(r => r.verdict === 'FAIL'), tested: c.rows.length }));
  const summary = {
    schema: 1, reference, route, mutationsRun: rows.length, notApplicable, inertMenuStates: inertMenus, noisyControlStates: noisyControls,
    failedExpectations: rows.filter(r => r.verdict === 'FAIL').map(r => `${r.mutation}@${r.scope}:${r.state}`),
    breakpointShifts: { clusters: bpSummary.length, detected: bpSummary.filter(b => b.detected).length, provenNoOp: bpSummary.filter(b => !b.detected && !b.failed && !b.notApplicable && !b.unproven && b.tested > 0).map(b => b.px), notApplicable: bpSummary.filter(b => b.notApplicable).map(b => b.px), unproven: bpSummary.filter(b => b.unproven).map(b => b.px), untested: bpSummary.filter(b => b.tested === 0).map(b => b.px), failed: bpSummary.filter(b => b.failed).map(b => b.px) },
    baselines: [...baselines].filter(([name]) => usedStates.has(name)).map(([name, b]) => ({ state: name, items: b.items, status: b.status })),
    passed: false,
    caveat: 'Sensitivity of the instrument on this reference and these states. Not a measure of candidate accuracy. A breakpoint shift counts as a pass only if it is detected, or if rules were rewritten and the rendered page (viewport slices, canvas/video hidden, measured on a separate page) is pixel-identical before and after (a proven no-op on this route); a shift that changed pixels without a detected difference, an unstable page, or unreadable stylesheets fail. Detection is judged by kinds of difference plausible for the mutation dimension (a coarse table), not per-mutation expectations. Baselines above zero mean the original is nondeterministic at that state.',
  };
  summary.passed = summary.failedExpectations.length === 0 && summary.baselines.every(b => b.items === 0 && ['identical', 'minor_only'].includes(b.status)) && summary.breakpointShifts.detected > 0;
  write(join(out, 'calibration.json'), { summary, rows });
  const md = ['# Calibration', '', `Reference ${reference}, route ${route}. ${summary.passed ? '**PASSED**' : '**FAILED**'}: failed expectations ${summary.failedExpectations.join(', ') || 'none'}; baselines ${summary.baselines.map(b => `${b.state}=${b.items}`).join(', ')}.`, '', `Breakpoint shifts: ${summary.breakpointShifts.detected}/${summary.breakpointShifts.clusters} clusters detected; proven no-ops on this route (rules rewritten, full-page pixels identical before and after): ${summary.breakpointShifts.provenNoOp.join(', ') || 'none'}; not in any stylesheet this route loads: ${summary.breakpointShifts.notApplicable.join(', ') || 'none'}; no boundary state in the inventory: ${summary.breakpointShifts.untested.join(', ') || 'none'}; UNPROVEN (rules rewritten, nothing detected, no pixel oracle run: may be a no-op or a blind spot): ${summary.breakpointShifts.unproven.join(', ') || 'none'}; failed (pixels changed with nothing detected, an unstable page, or unreadable sheets): ${summary.breakpointShifts.failed.join(', ') || 'none'}.`, '', 'Every row links its mutated capture and diff under `evidence/`. "gate" is whether the Rust-gate fact emitter also saw the regression.', '', '| mutation | dimension | state kind | expect | detected | gate | verdict | Δitems | dimensions changed | sample |', '|---|---|---|---|---|---|---|---:|---|---|'];
  for (const r of rows) md.push(`| ${r.mutation} | ${r.dimension} | ${r.scope} | ${r.expect} | ${r.detected ? 'yes' : 'no'} | ${r.gateDetected ? 'yes' : 'no'} | ${r.verdict} | ${r.delta} | ${Object.entries(r.dimensions).map(([k, v]) => `${k}:${v}`).join(' ')} | ${(r.sample[0] ?? '').replace(/\|/g, '/')} |`);
  if (noisyControls.length) md.push('', `States whose identical control was NOT clean (the original differs from itself there), skipped for calibration: ${noisyControls.join(', ')}`);
  if (inertMenus.length) md.push('', `Menu states skipped because the click changes nothing in the original: ${inertMenus.join(', ')}`);
  if (notApplicable.length) md.push('', 'Not applicable (no matching state or target):', ...notApplicable.map(n => `- ${n.mutation} (${n.scope}): ${n.reason}`));
  md.push('', `> ${summary.caveat}`);
  write(join(out, 'CALIBRATION.md'), md.join('\n') + '\n');
  return { summary, rows };
}
