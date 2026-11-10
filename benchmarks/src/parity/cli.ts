// bun benchmarks/src/parity/cli.ts <command>
//   inventory <capture-dir> --out inventory.json [--viewport 1000x800]...
//   run --inventory inventory.json --reference http://127.0.0.1:8000 --candidate http://127.0.0.1:3000 --out <new dir>
//       [--profile quick|core|standard|full] [--only a,b] [--workers 2] [--controls] [--no-hash-assets]
//       [--controls-all] [--force-webgl-readback] [--project <generated app dir>]
//   calibrate --inventory inventory.json --reference URL --out <new dir>
//   rediff <run dir> --out <new dir>     recompute summaries and report from retained raw captures
//   pair --ref ref.json[.gz] --cand cand.json[.gz] [--origins https://site] [--out report.json]
//       diff two saved captures, e.g. the files the Browser pane path posted to the sink
//   gate <run dir> --project <generated app dir> [--mochi path/to/mochi] --out <new dir>
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { buildInventory } from './inventory';
import { calibrate } from './calibrate';
import { rediff, runAll, write, type Profile } from './run';
import { writeReport } from './report';
import { gate } from './gate';
import { gunzipSync } from 'node:zlib';
import { diffRuns, statusOf } from './pair';
import type { Inventory } from './state';

const argv = process.argv.slice(2);
const cmd = argv.shift();
const flag = (n: string) => { const i = argv.indexOf('--' + n); return i < 0 ? undefined : argv[i + 1]; };
const has = (n: string) => argv.includes('--' + n);
const need = (n: string) => flag(n) ?? (() => { throw new Error(`missing --${n}`); })();
const inv = () => JSON.parse(readFileSync(need('inventory'), 'utf8')) as Inventory;

if (cmd === 'inventory') {
  const out = need('out');
  if (existsSync(out)) throw new Error('refusing to overwrite ' + out);
  const extraViewports = argv.flatMap((a, k) => (a === '--viewport' ? [argv[k + 1].split('x').map(Number) as [number, number]] : []));
  const i = buildInventory(argv[0], { extraViewports });
  writeFileSync(out, JSON.stringify(i, null, 1));
  console.log(`${i.routes.length} routes, ${i.states.length} states, ${i.breakpoints.length} breakpoints, ${i.gaps.length} gaps → ${out}`);
} else if (cmd === 'run') {
  const inventory = inv();
  const cfg = { inventory, reference: need('reference'), candidate: need('candidate'), out: need('out'), profile: (flag('profile') ?? 'standard') as Profile, only: flag('only')?.split(','), workers: +(flag('workers') ?? 2), hashAssets: !has('no-hash-assets'), forceWebglReadback: has('force-webgl-readback'), controls: has('controls') || has('controls-all'), controlsAll: has('controls-all'), project: flag('project') };
  const outcome = await runAll(cfg);
  const rep = writeReport(cfg.out, inventory, outcome);
  console.log(`${cfg.out}/REPORT.md  executed ${rep.totals.executed}/${rep.totals.scheduled}  ${JSON.stringify(rep.totals.byStatus)}`);
  const bad = Object.entries(rep.totals.byStatus).some(([k, n]) => n && k !== 'identical' && k !== 'minor_only');
  process.exit(outcome.failures.length || bad ? 1 : 0);
} else if (cmd === 'calibrate') {
  const r = await calibrate(inv(), need('reference'), need('out'), { hashAssets: !has('no-hash-assets') });
  console.log(r.summary.passed ? 'calibration passed' : 'calibration FAILED: ' + r.summary.failedExpectations.join(', '));
  process.exit(r.summary.passed ? 0 : 1);
} else if (cmd === 'rediff') {
  // rediff <run dir> --out <new dir>: same raw captures, current comparison code
  const o = await rediff(argv[0], need('out'));
  const rep = writeReport(need('out'), o.inventory, o);
  console.log(`${need('out')}/REPORT.md  ${JSON.stringify(rep.totals.byStatus)}`);
} else if (cmd === 'pair') {
  const load = (f: string) => JSON.parse((f.endsWith('.gz') ? gunzipSync(readFileSync(f)) : readFileSync(f)).toString());
  const r = diffRuns(load(need('ref')), load(need('cand')), { siteOrigins: flag('origins')?.split(',') ?? [] });
  const summary = { status: statusOf(r), ...r.summary, unsettled: r.unsettled, limits: r.limits };
  if (flag('out')) { if (existsSync(flag('out')!)) throw new Error('refusing to overwrite ' + flag('out')); writeFileSync(flag('out')!, JSON.stringify({ summary, pairs: r.pairs, missing: r.missing, extra: r.extra, pageFacts: r.pageFacts, stateItems: r.stateItems }, null, 1)); }
  console.log(JSON.stringify(summary, null, 1));
  process.exit(summary.status === 'identical' || summary.status === 'minor_only' ? 0 : 1);
} else if (cmd === 'gate') {
  process.exit(await gate(argv[0], need('project'), flag('mochi') ?? 'mochi', need('out')));
} else {
  console.error(readFileSync(import.meta.path, 'utf8').split('\n').slice(0, 10).join('\n'));
  process.exit(2);
}
void write;
