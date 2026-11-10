// REPORT.md / report.json for a run directory. Counts only; no blended score.
import { join } from 'node:path';
import type { Inventory } from './state';
import { write, type StateSummary } from './run';

export interface RunOutcome { runMeta: Record<string, any>; summaries: StateSummary[]; failures: { name: string; error: string }[]; finishedAt: string }

export function buildReport(inv: Inventory, o: RunOutcome) {
  const S = o.summaries;
  const byStatus: Record<string, number> = {};
  for (const s of S) byStatus[s.status] = (byStatus[s.status] ?? 0) + 1;
  const dims: Record<string, number> = {}, rules: Record<string, number> = {}, props: Record<string, number> = {};
  for (const s of S) {
    for (const [k, v] of Object.entries(s.byDimension)) dims[k] = (dims[k] ?? 0) + v;
    for (const [k, v] of Object.entries(s.minorRules)) rules[k] = (rules[k] ?? 0) + v;
    for (const t of s.top) props[t.prop] = (props[t.prop] ?? 0) + 1;
  }
  const sum = (f: (s: StateSummary) => number) => S.reduce((a, s) => a + f(s), 0);
  const ref = sum(s => s.refElements), clean = sum(s => s.matched - s.pairsWithDifferences);
  const blocked = [...new Set(S.flatMap(s => [...s.blocked.ref, ...s.blocked.cand]))].sort();
  const limits = [...new Set(S.flatMap(s => s.limits))];
  const executed = new Set(S.map(s => s.name));
  const notRun: { name: string; reason: string }[] = [...o.runMeta.skipped.map((k: { name: string; reason: string }) => k), ...o.failures.map(f => ({ name: f.name, reason: 'runtime failure: ' + f.error }))];
  const accounted = S.length + notRun.length;
  const accounting = accounted === inv.states.length ? 'ok' : `MISMATCH: ${S.length} summarised + ${notRun.length} not run = ${accounted}, but ${inv.states.length} states were scheduled`;
  const noisy = S.filter(s => s.noise && (s.noise.majorItems || s.noise.minorItems));
  const candNoisy = S.filter(s => s.candNoise && (s.candNoise.majorItems || s.candNoise.minorItems));
  return {
    schema: 1, scope: 'Resolved DOM/CSS facts of the declared states only. Not a screenshot or perceptual comparison; no claim about unobserved states, routes, browsers or GPUs.',
    run: o.runMeta, finishedAt: o.finishedAt,
    totals: {
      scheduled: inv.states.length, executed: S.length, failedToRun: o.failures.length, byStatus,
      referenceElements: ref, elementsWithoutDifference: clean, elementsWithDifferences: sum(s => s.pairsWithDifferences), refMissing: sum(s => s.refMissing), candExtra: sum(s => s.candExtra),
      legacyComparableElements: sum(s => s.legacyComparable), majorItems: sum(s => s.majorItems), minorItems: sum(s => s.minorItems), maxGeometryDelta: Math.max(0, ...S.map(s => s.maxGeometryDelta)),
      byDimension: dims, minorRules: rules,
    },
    noiseFloor: { controlStates: S.filter(s => s.noise).length, noisyStates: noisy.map(s => ({ name: s.name, ...s.noise })), candidateControlStates: S.filter(s => s.candNoise).length, candidateNoisyStates: candNoisy.map(s => ({ name: s.name, ...s.candNoise })) },
    accounting, blockedHosts: blocked, runtimeLimits: limits, inventoryGaps: inv.gaps, notRun, executedNames: [...executed],
  };
}

export function renderMarkdown(inv: Inventory, rep: ReturnType<typeof buildReport>, S: StateSummary[]): string {
  const t = rep.totals, L: string[] = [];
  L.push(`# Parity report`, '', `Reference \`${rep.run.reference}\` (original) vs candidate \`${rep.run.candidate}\`. Profile **${rep.run.profile}**. Chromium ${rep.run.chromium}. ${rep.run.startedAt} to ${rep.finishedAt}.`, '', `> ${rep.scope}`, '');
  L.push(`## Result`, '', `States: ${t.executed} executed of ${t.scheduled} scheduled (${t.failedToRun} failed to run, ${rep.notRun.length - t.failedToRun} not selected). ` + Object.entries(t.byStatus).map(([k, v]) => `${k}: ${v}`).join(', ') + '.', '');
  if (rep.accounting !== 'ok') L.push(`**State accounting ${rep.accounting}**`, '');
  L.push(`Elements compared across executed states: ${t.referenceElements} reference elements; ${t.elementsWithoutDifference} paired without any difference, ${t.elementsWithDifferences} paired with differences, ${t.refMissing} missing in candidate, ${t.candExtra} extra in candidate. Major items ${t.majorItems}, minor (named equivalence) items ${t.minorItems}. Largest geometry delta ${t.maxGeometryDelta}px (tolerance 1px). Of these, ${t.legacyComparableElements} meet the hardened Natural collector's inclusion rule (text, media, control or painted box, plus pseudo-elements), which reproduces its element counts.`, '');
  L.push('Differences by dimension: ' + (Object.entries(t.byDimension).map(([k, v]) => `${k} ${v}`).join(', ') || 'none') + '.', '');
  L.push('Named minor equivalences applied (counts, never silent): ' + (Object.entries(t.minorRules).map(([k, v]) => `${k} ${v}`).join(', ') || 'none') + '.', '');
  L.push(`## States`, '', '| state | status | ref | matched | missing | extra | major | minor | max Δpx | dimensions |', '|---|---|---:|---:|---:|---:|---:|---:|---:|---|');
  for (const s of S) L.push(`| ${s.name} | ${s.status} | ${s.refElements} | ${s.matched} | ${s.refMissing} | ${s.candExtra} | ${s.majorItems} | ${s.minorItems} | ${s.maxGeometryDelta} | ${Object.entries(s.byDimension).map(([k, v]) => `${k}:${v}`).join(' ')} |`);
  L.push('', '## Largest differences (first state per distinct property)', '');
  const seen = new Set<string>();
  for (const s of S) for (const d of s.top) { if (seen.has(d.prop) || seen.size > 40) continue; seen.add(d.prop); L.push(`- \`${d.prop}\` (${d.dim})${d.inNoise ? ' [also differs between two loads of the original]' : ''} in ${s.name}: ${d.where}: ${JSON.stringify(d.ref)?.slice(0, 80)} → ${JSON.stringify(d.cand)?.slice(0, 80)}`); }
  L.push('', '## Nondeterminism control (reference vs reference)', '');
  L.push(rep.noiseFloor.controlStates ? `${rep.noiseFloor.controlStates} control states; ${rep.noiseFloor.noisyStates.length} showed differences between two loads of the same original: ${rep.noiseFloor.noisyStates.map(n => `${n.name} (${n.props?.join(', ')})`).join('; ') || 'none'}.` : 'Not run (use --controls). Differences below may include nondeterminism.', '');
  L.push(`Candidate loaded twice: ${rep.noiseFloor.candidateControlStates} control states; ${rep.noiseFloor.candidateNoisyStates.length} showed differences between two loads of the candidate: ${rep.noiseFloor.candidateNoisyStates.map(n => `${n.name} (${n.props?.join(', ')})`).join('; ') || 'none'}.`, '');
  L.push('## Errors, blocked hosts', '', `Third-party hosts blocked on both sides: ${rep.blockedHosts.join(', ') || 'none'}.`, '');
  const grouped = new Map<string, { side: string; n: number }>();
  for (const st of S) for (const [side, list] of [['reference-only', st.errors.ref], ['candidate-only', st.errors.cand], ['both sides', st.errors.shared]] as const) for (const e of list) { const k = `${side}: ${e.split('\n')[0].replace(/127\.0\.0\.1:\d+/g, 'HOST')}`; grouped.set(k, { side, n: (grouped.get(k)?.n ?? 0) + 1 }); }
  for (const [k, v] of [...grouped].slice(0, 30)) L.push(`- ${v.n} state(s), ${k}`);
  if (!grouped.size) L.push('- none');
  L.push('', '## Coverage gaps (explicit)', '');
  for (const g of rep.inventoryGaps) L.push(`- [${g.area}] ${g.detail}`);
  for (const l of rep.runtimeLimits) L.push(`- [collector] ${l}`);
  L.push('', `### Scheduled states not executed (${rep.notRun.length})`, '');
  const reasons: Record<string, string[]> = {};
  for (const k of rep.notRun) (reasons[k.reason] ??= []).push(k.name);
  for (const [r, n] of Object.entries(reasons)) L.push(`- ${r}: ${n.length} (${n.slice(0, 6).join(', ')}${n.length > 6 ? ', …' : ''})`);
  return L.join('\n') + '\n';
}

export function writeReport(dir: string, inv: Inventory, o: RunOutcome) {
  const rep = buildReport(inv, o);
  write(join(dir, 'report.json'), rep);
  write(join(dir, 'REPORT.md'), renderMarkdown(inv, rep, o.summaries));
  return rep;
}
