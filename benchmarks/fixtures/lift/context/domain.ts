import type { ContextValue, Domain } from '../../../src/lift-differential';

const table = [[1, 2], [3, 4]];
const widths = [0, 1, 63, 64, 320, 759, 760, 1024, 1440];
const heights = [0, 300, 812, 1018];
/** Sampled DOM values, derived from the viewport the way the controller would see them. */
const zoneFor = (height: number) => ({ span: Math.max(1, height / 10), top: height * 0.1 + 16 });

function* grid(): Generator<ContextValue> {
  for (const width of widths) {
    for (const height of heights) {
      for (const zone of [zoneFor(height), { span: 3, top: -5 }, { span: 0, top: 1e9 }]) {
        yield { width, height, table, bias: 0.5, gap: 12, size: 90, zone };
      }
    }
  }
}
/** Not real values: perturbs the constants so a field wired to the wrong place, or hard-coded, is visible. */
function* wiring(): Generator<ContextValue> {
  const tables = [table, [[5, 6], [7, 8]], [[0, 9], [2.5, 11]]];
  for (const [bias, gap, size] of [[0.3, 7, 60], [0.5, 12, 145], [0.7, 0, 90]]) {
    for (const [width, height] of [[320, 568], [1440, 900]]) {
      for (const t of tables) yield { width, height, table: t, bias, gap, size, zone: zoneFor(height) };
    }
  }
}
/** Revisits viewports in a shuffled order so cache hits and key changes both occur. */
function* sweep(): Generator<ContextValue> {
  for (const [width, height] of [[320, 568], [320, 568], [1440, 900], [320, 568], [760, 812], [760, 812], [1440, 900], [0, 0], [320, 568]]) {
    yield { width, height, table, bias: 0.5, gap: 12, size: 90, zone: zoneFor(height) };
  }
}
export const contexts: Domain['contexts'] = { Inputs: () => [...grid(), ...wiring()] };
export const sweeps: Domain['sweeps'] = { Inputs: sweep };
