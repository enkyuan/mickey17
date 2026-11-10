// Feed a finished run into `mochi check-parity` (src/parity.rs): the strict, contract-bound,
// source-fingerprinted gate. Alignment comes from the run's saved element pairing; facts are
// canonicalised only by the named equivalences in diff.ts. Gaps stay in the contract, so the
// gate can report them but never pass while any exist.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { gunzipSync } from 'node:zlib';
import { gateStates, type GateState } from './facts';
import { sha, write } from './run';
import type { Inventory, SideRun } from './state';
import type { Result } from './diff';

const gz = <T>(p: string): T => JSON.parse(gunzipSync(readFileSync(p)).toString());

export async function gate(runDir: string, project: string, mochi: string, out: string): Promise<number> {
  if (existsSync(out)) throw new Error('output exists: ' + out);
  mkdirSync(out, { recursive: true });
  const inv = JSON.parse(readFileSync(join(runDir, 'inventory.json'), 'utf8')) as Inventory;
  const meta = JSON.parse(readFileSync(join(runDir, 'run.json'), 'utf8'));
  const fp = spawnSync(mochi, ['fingerprint-project', resolve(project)], { encoding: 'utf8' });
  if (fp.status !== 0) throw new Error('fingerprint-project failed: ' + fp.stderr);
  const projectSha = fp.stdout.trim();
  const refStates: Record<string, GateState> = {}, candStates: Record<string, GateState> = {}, cstates: Record<string, unknown> = {};
  const rawDir: string = meta.rediffOf ?? runDir; // a rediff directory keeps no raw captures of its own
  for (const name of readdirSync(join(runDir, 'states'))) {
    const d = join(runDir, 'states', name), raw = join(rawDir, 'states', name);
    if (!existsSync(join(d, 'summary.json'))) continue;
    const spec = inv.states.find(s => s.name === name)!;
    const ref = gz<SideRun>(join(raw, 'ref.json.gz')), cand = gz<SideRun>(join(raw, 'cand.json.gz')), diff = gz<Result>(join(d, 'diff.json.gz'));
    // every console/page error from either side reaches the Rust gate, which fails any state with errors
    const ex = (x: SideRun) => ({ focusOrder: x.focusOrder, stepErrors: x.stepErrors, reachedScrollY: x.reachedScrollY });
    const [r, c] = gateStates(spec, ref.collection, cand.collection, diff, meta.options, ref.errors, cand.errors, [ref.settled, cand.settled], [ex(ref), ex(cand)]);
    refStates[name] = r; candStates[name] = c;
    const tol: Record<string, number> = {};
    for (const a of r.inventory) if (a !== 'page') for (const k of ['x', 'y', 'width', 'height']) tol[`${a}.bounds.${k}`] = 1;
    cstates[name] = { route: spec.route, viewport: spec.viewport, reduced_motion: spec.reducedMotion, phase: spec.phase, inventory: r.inventory, tolerances: tol };
  }
  // src/parity.rs requires every route to have desktop and mobile (<768 px) initial, scroll and bottom states plus a
  // reduced-motion state. Without them check-parity refuses the input and writes no report, so say so here instead.
  const summarisedNames = new Set(Object.keys(candStates));
  const missingMatrix: string[] = [];
  for (const route of inv.routes.map(r => r.route)) for (const mobile of [false, true]) {
    const at = (f: (s: (typeof inv.states)[number]) => boolean) => inv.states.some(s => summarisedNames.has(s.name) && s.route === route && (s.viewport[0] < 768) === mobile && f(s));
    for (const phase of ['initial', 'scroll', 'bottom']) if (!at(s => !s.reducedMotion && s.phase === phase)) missingMatrix.push(`${route} ${mobile ? 'mobile' : 'desktop'} ${phase}`);
    if (!at(s => s.reducedMotion)) missingMatrix.push(`${route} ${mobile ? 'mobile' : 'desktop'} reduced-motion`);
  }
  if (missingMatrix.length) {
    write(join(out, 'gate-not-run.json'), { reason: 'the executed states do not meet the minimum matrix check-parity requires (use --profile core or larger)', missing: missingMatrix });
    console.log(`check-parity not run: ${missingMatrix.length} required state kinds missing (see gate-not-run.json)`);
    return 2;
  }
  const referenceBytes = JSON.stringify({ schema_version: 1, project_sha256: null, states: refStates });
  write(join(out, 'reference.json'), referenceBytes);
  write(join(out, 'candidate.json'), JSON.stringify({ schema_version: 1, project_sha256: projectSha, states: candStates }));
  const summarised = summarisedNames;
  const skipped = new Set<string>(meta.skipped.map((k: { name: string }) => k.name));
  const crashed = inv.states.filter(s => !summarised.has(s.name) && !skipped.has(s.name)).map(s => `state failed to run: ${s.name}`);
  const gaps = [...inv.gaps.map(g => `[${g.area}] ${g.detail}`), ...meta.skipped.map((k: { name: string; reason: string }) => `state not executed: ${k.name} (${k.reason})`), ...crashed,
    ...Object.entries(candStates).filter(([n]) => (JSON.parse(readFileSync(join(runDir, 'states', n, 'summary.json'), 'utf8')).status ?? '') === 'not_exercised').map(([n]) => `state not exercised (interaction target missing on the reference): ${n}`)];
  write(join(out, 'contract.json'), JSON.stringify({ schema_version: 1, reference_sha256: sha(referenceBytes), project_sha256: projectSha, routes: inv.routes.map(r => r.route), states: cstates, gaps }));
  const res = spawnSync(mochi, ['check-parity', resolve(project), '--contract', join(out, 'contract.json'), '--reference', join(out, 'reference.json'), '--candidate', join(out, 'candidate.json'), '--out', join(out, 'check-parity-report.json')], { encoding: 'utf8' });
  write(join(out, 'check-parity.log'), `exit ${res.status}\n${res.stdout}\n${res.stderr}`);
  console.log(`check-parity exit ${res.status}: ${(res.stdout + res.stderr).trim().split('\n')[0]}`);
  return res.status ?? 2;
}
