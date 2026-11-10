// One state: diff two driver runs (collector output plus focus order, errors, settle).
import { diffCollections, focusSig, type Collection, type DiffItem, type Options, type Result } from './diff';
import type { SideRun } from './state';

export interface StateResult extends Result {
  unsettled: string[];
  /** Steps that failed on the REFERENCE: the state was not exercised, whatever the candidate did. */
  notExercised: string[];
  stateItems: DiffItem[];
  /** Elements the legacy Natural collector would also have listed (text, media, controls, paints). */
  legacyComparable: number;
}
const sameFocus = (a: Record<string, unknown>, b: Record<string, unknown>) => JSON.stringify(focusSig(a)) === JSON.stringify(focusSig(b));

/** Elements the hardened Natural collector (natural-harness/collect.js `v` list) would list: text, media, controls or a painted box, plus pseudo-elements. Reproduces its 585 / 575 / 192 counts on the natural captures. */
const legacyCount = (c: Collection): number => {
  const skip = new Set(['script', 'style', 'link', 'noscript', 'template', 'path', 'circle', 'rect', 'g', 'defs', 'clippath', 'lineargradient', 'stop', 'mask', 'use', 'ellipse', 'line', 'polygon', 'polyline', 'filter', 'br', 'picture', 'source', 'html', 'body']);
  const px = (v?: string) => parseFloat(v ?? '0') || 0;
  let n = c.pseudos.length;
  for (const e of c.els) {
    if (skip.has(e.tag)) continue;
    const st = c.styleTable[e.sty ?? -1] ?? {};
    const paints = (st.backgroundColor ?? 'rgba(0, 0, 0, 0)') !== 'rgba(0, 0, 0, 0)' || (st.backgroundImage ?? 'none') !== 'none' || px(st.borderTopWidth) + px(st.borderRightWidth) + px(st.borderBottomWidth) + px(st.borderLeftWidth) > 0 || (st.boxShadow ?? 'none') !== 'none';
    const keep = e.text || ['img', 'svg', 'canvas', 'video'].includes(e.tag) || ['button', 'a', 'input', 'select', 'textarea'].includes(e.tag) || e.role === 'button' || paints;
    if (keep && !(e.w === 0 && e.h === 0 && !e.text)) n++;
  }
  return n;
};

export function diffRuns(ref: SideRun, cand: SideRun, options: Options): StateResult {
  const r = diffCollections(ref.collection, cand.collection, options);
  const items: DiffItem[] = [];
  const add = (dim: DiffItem['dim'], prop: string, a: unknown, b: unknown) => items.push({ dim, prop, ref: a, cand: b, severity: 'major' });
  const n = Math.max(ref.focusOrder.length, cand.focusOrder.length);
  for (let i = 0; i < n; i++) {
    const a = ref.focusOrder[i], b = cand.focusOrder[i];
    if (!a || !b || !sameFocus(a, b)) add('behavior', `focus.order[${i + 1}]`, a ? `${a.tag} "${a.name}" focus-visible=${a.focusVisible} outline=${a.outline} shadow=${a.boxShadow}` : null, b ? `${b.tag} "${b.name}" focus-visible=${b.focusVisible} outline=${b.outline} shadow=${b.boxShadow}` : null);
  }
  // A failed step is never a pass. On the candidate alone it is a behavioral difference; on the reference
  // (inventory target not found / not interactable) the state was not exercised, even if the candidate fails alike.
  if (!ref.stepErrors.length) for (const e of cand.stepErrors) add('behavior', 'step.failed', null, e);
  if (ref.reachedScrollY !== cand.reachedScrollY) add('layout', 'scroll.reachedY', ref.reachedScrollY, cand.reachedScrollY);
  // errors embed the server origin (127.0.0.1:8100 vs :3101): compare them with the origin removed
  const normErr = (e: string) => e.replace(/https?:\/\/(127\.0\.0\.1|localhost)(:\d+)?/g, 'ORIGIN');
  const refErrs = new Set(ref.errors.map(normErr));
  for (const e of cand.errors) if (!refErrs.has(normErr(e))) add('behavior', 'error.candidate-only', null, e);
  const out = { ...r, unsettled: [] as string[], notExercised: [...ref.stepErrors], stateItems: items, legacyComparable: legacyCount(ref.collection) };
  if (!ref.settled) out.unsettled.push('reference');
  if (!cand.settled) out.unsettled.push('candidate');
  r.summary.majorItems += items.length;
  for (const it of items) { r.summary.byDimension[it.dim] = (r.summary.byDimension[it.dim] ?? 0) + 1; r.summary.byProperty[it.prop] = (r.summary.byProperty[it.prop] ?? 0) + 1; }
  if (r.status !== 'different' && (items.length || out.unsettled.length)) out.status = items.length ? 'different' : r.status;
  return out;
}
export const statusOf = (r: StateResult): 'identical' | 'minor_only' | 'different' | 'unsettled' | 'not_exercised' =>
  r.notExercised.length ? 'not_exercised' : r.status === 'different' ? 'different' : r.unsettled.length ? 'unsettled' : r.status;
