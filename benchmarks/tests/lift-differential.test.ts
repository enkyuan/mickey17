import { createHash } from 'node:crypto';
import { cpSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, test } from 'bun:test';
import { differential, type LiftManifest } from '../src/lift-differential';

const fixture = join(import.meta.dir, '..', 'fixtures', 'lift');
const sha256 = (text: string) => createHash('sha256').update(text).digest('hex');

/** Copies the golden fixture so a test can alter it; hashes are recomputed so only behavior can fail. */
function workspace(alter?: { module?: (text: string) => string; source?: (text: string) => string }) {
  const dir = mkdtempSync(join(tmpdir(), 'mochi-lift-'));
  cpSync(fixture, dir, { recursive: true });
  const manifest = JSON.parse(readFileSync(join(dir, 'math.lift.json'), 'utf8')) as LiftManifest;
  if (alter?.module) {
    const next = alter.module(readFileSync(join(dir, 'math.ts'), 'utf8'));
    writeFileSync(join(dir, 'math.ts'), next);
    manifest.module_sha256 = sha256(next);
  }
  if (alter?.source) {
    const next = alter.source(readFileSync(join(dir, 'source.js'), 'utf8'));
    writeFileSync(join(dir, 'source.js'), next);
    manifest.source_sha256 = sha256(next);
  }
  writeFileSync(join(dir, 'math.lift.json'), JSON.stringify(manifest));
  return { manifest: join(dir, 'math.lift.json'), module: join(dir, 'math.ts'), source: join(dir, 'source.js') };
}
const run = (w: ReturnType<typeof workspace>) => differential(w.manifest, w.module, w.source);

describe('lifted functions versus the original source', () => {
  test('the committed fixture matches the original on every generated input', async () => {
    const report = await run(workspace());
    expect(report.passed).toBe(true);
    expect(report.functions.map((f) => f.export)).toEqual(['clamp01', 'mix', 'easeIn']);
    expect(report.functions.every((f) => f.cases > 80)).toBe(true);
  });

  test('a behavioral edit to the lifted module is detected', async () => {
    const report = await run(workspace({ module: (text) => text.replace('(end - start) * amount', '(end - start) * amount * 0.999') }));
    expect(report.passed).toBe(false);
    expect(report.functions.find((f) => f.export === 'mix')?.mismatches.length).toBeGreaterThan(0);
    expect(report.functions.find((f) => f.export === 'easeIn')?.mismatches.length).toBeGreaterThan(0);
  });

  test('an edge-case-only edit (NaN handling) is detected', async () => {
    const report = await run(workspace({ module: (text) => text.replace('Math.max(0, Math.min(1, value))', 'Number.isNaN(value) ? 0 : Math.max(0, Math.min(1, value))') }));
    expect(report.functions.find((f) => f.export === 'clamp01')?.mismatches.length).toBeGreaterThan(0);
  });

  test('a changed original is reported instead of compared', async () => {
    const report = await run(workspace({ source: (text) => text.replace('a + (b - a) * t', 'a - (b - a) * t') }));
    expect(report.passed).toBe(false);
    expect(report.functions.find((f) => f.export === 'mix')?.error).toContain('original span hash differs');
  });

  test('a type with no input generator fails rather than being skipped', async () => {
    const w = workspace();
    const manifest = JSON.parse(readFileSync(w.manifest, 'utf8')) as LiftManifest;
    manifest.functions[0].params[0].type = 'Unsupported';
    writeFileSync(w.manifest, JSON.stringify(manifest));
    const report = await run(w);
    expect(report.passed).toBe(false);
    expect(report.functions[0].error).toContain('no argument generator');
  });

  test('a module that does not match its manifest is rejected', async () => {
    const w = workspace();
    writeFileSync(w.module, readFileSync(w.module, 'utf8') + '\n// tampered\n');
    await expect(run(w)).rejects.toThrow('lifted module does not match');
  });
});

/** The context-taking fixture: a synthetic controller whose layout reads captured state, a DOM sample and a memo cache. */
function contextWorkspace(alter?: { module?: (text: string) => string; domain?: (text: string) => string; manifest?: (m: LiftManifest) => void }) {
  const dir = mkdtempSync(join(tmpdir(), 'mochi-lift-context-'));
  cpSync(join(fixture, 'context'), dir, { recursive: true });
  const manifest = JSON.parse(readFileSync(join(dir, 'layout.lift.json'), 'utf8')) as LiftManifest;
  if (alter?.module) {
    const next = alter.module(readFileSync(join(dir, 'layout.ts'), 'utf8'));
    writeFileSync(join(dir, 'layout.ts'), next);
    manifest.module_sha256 = sha256(next);
  }
  alter?.manifest?.(manifest);
  writeFileSync(join(dir, 'layout.lift.json'), JSON.stringify(manifest));
  if (alter?.domain) writeFileSync(join(dir, 'domain.ts'), alter.domain(readFileSync(join(dir, 'domain.ts'), 'utf8')));
  return { dir, manifest: join(dir, 'layout.lift.json'), module: join(dir, 'layout.ts'), source: join(dir, 'source.js'), domain: join(dir, 'domain.ts') };
}
const runContext = (w: ReturnType<typeof contextWorkspace>, domain: string | undefined = w.domain) => differential(w.manifest, w.module, w.source, { domain });
const failures = (report: Awaited<ReturnType<typeof runContext>>, name: string) => report.functions.find((f) => f.export === name);

describe('lifted functions that read captured state through an explicit context', () => {
  test('match the original on a viewport grid, a cache hit and a cache sweep', async () => {
    const report = await runContext(contextWorkspace());
    expect(report.functions.filter((f) => f.error || f.mismatches.length)).toEqual([]);
    expect(report.passed).toBe(true);
    expect(report.functions.map((f) => f.export)).toEqual(['clamp01', 'mix', 'fitScale', 'rowsFor', 'midline']);
    expect(failures(report, 'rowsFor')!.cases).toBeGreaterThan(200);
  });

  test('a context-taking function with no domain fails instead of being skipped', async () => {
    const w = contextWorkspace();
    const report = await differential(w.manifest, w.module, w.source);
    expect(report.passed).toBe(false);
    expect(failures(report, 'fitScale')!.error).toContain('supplies no contexts');
  });

  test('a changed formula in the extracted memo computation is detected', async () => {
    const report = await runContext(contextWorkspace({ module: (t) => t.replace('top - inputs.gap', 'top - inputs.gap - 1e-9') }));
    expect(report.passed).toBe(false);
    expect(failures(report, 'midline')!.mismatches.length).toBeGreaterThan(0);
    expect(failures(report, 'rowsFor')!.mismatches).toEqual([]);
  });

  test('a captured field hard-coded to its current value is detected by the perturbed contexts', async () => {
    const report = await runContext(contextWorkspace({ module: (t) => t.replace('inputs.height * inputs.bias', 'inputs.height * 0.5') }));
    expect(failures(report, 'fitScale')!.mismatches.length).toBeGreaterThan(0);
  });

  test('two fields wired to each other are detected', async () => {
    const report = await runContext(contextWorkspace({ module: (t) => t.replace('inputs.width * (inputs.width < 760', 'inputs.height * (inputs.width < 760') }));
    expect(failures(report, 'fitScale')!.mismatches.length).toBeGreaterThan(0);
  });

  test('a lifted function that mutates its context is detected', async () => {
    const report = await runContext(contextWorkspace({ module: (t) => t.replace('let zone = inputs.zone,', 'inputs.table[0][0] += 1; let zone = inputs.zone,') }));
    expect(failures(report, 'fitScale')!.mismatches.join('\n')).toContain('inputs mutated');
  });

  test('reordering or altering a returned Map is detected with exact comparison', async () => {
    const reordered = await runContext(contextWorkspace({ module: (t) => t.replace('[[1, rows[0]], [2, rows[1]]]', '[[2, rows[1]], [1, rows[0]]]') }));
    expect(failures(reordered, 'rowsFor')!.mismatches.length).toBeGreaterThan(0);
    const altered = await runContext(contextWorkspace({ module: (t) => t.replace('[2, rows[1]]]', '[2, rows[1].map((v) => v + Number.EPSILON * 1e6)]]') }));
    expect(failures(altered, 'rowsFor')!.mismatches.length).toBeGreaterThan(0);
  });

  test('an algebraically equivalent rewrite that differs by an ulp is not rounded away', async () => {
    const report = await runContext(contextWorkspace({ module: (t) => t.replace('start + (end - start) * amount', 'start * (1 - amount) + end * amount') }));
    expect(failures(report, 'mix')!.mismatches.length).toBeGreaterThan(0);
  });

  test('a sweep that would make the original read a stale cache is refused, not compared', async () => {
    const report = await runContext(contextWorkspace({ domain: (t) => t.replace('zone: zoneFor(height) };\n  }\n}\nexport const contexts', 'zone: { ...zoneFor(height), top: Math.random() } };\n  }\n}\nexport const contexts') }));
    expect(report.passed).toBe(false);
    expect(failures(report, 'rowsFor')!.error).toContain('stale');
  });

  test('a domain value missing a manifest field, or with the wrong kind, is refused instead of agreeing on NaN', async () => {
    const missing = await runContext(contextWorkspace({ domain: (t) => t.replaceAll(' gap: 12,', '').replaceAll('gap: 12,', '') }));
    expect(failures(missing, 'fitScale')!.error).toContain('manifest declares');
    const wrongKind = await runContext(contextWorkspace({ domain: (t) => t.replaceAll('bias: 0.5', 'bias: "0.5"') }));
    expect(wrongKind.passed).toBe(false);
  });

  test('a context field that never varies across the grid is refused unless declared constant', async () => {
    const report = await runContext(contextWorkspace({ domain: (t) => t.replace('const tables = [table, [[5, 6], [7, 8]], [[0, 9], [2.5, 11]]];', 'const tables = [table];') }));
    expect(report.passed).toBe(false);
    expect(failures(report, 'rowsFor')!.error).toContain('single value');
    const declared = await runContext(contextWorkspace({ domain: (t) => t.replace('const tables = [table, [[5, 6], [7, 8]], [[0, 9], [2.5, 11]]];', 'const tables = [table];') + "\nexport const constants: Domain['constants'] = { Inputs: ['table'] };\n" }));
    expect(declared.passed).toBe(true);
  });

  test('hard-coding a field that does vary is detected', async () => {
    const report = await runContext(contextWorkspace({ module: (t) => t.replace('inputs.table.map', '[[1, 2], [3, 4]].map') }));
    expect(failures(report, 'rowsFor')!.mismatches.length).toBeGreaterThan(0);
  });

  test('a sweep without a cache hit, a key change and a revisit is refused', async () => {
    const report = await runContext(contextWorkspace({ domain: (t) => t.replace('[[320, 568], [320, 568], [1440, 900], [320, 568], [760, 812], [760, 812], [1440, 900], [0, 0], [320, 568]]', '[[320, 568], [1440, 900], [760, 812]]') }));
    expect(failures(report, 'rowsFor')!.error).toContain('cache hit');
  });

  test('a memoizing function with no sweep fails', async () => {
    const report = await runContext(contextWorkspace({ domain: (t) => t.replace(/export const sweeps.*\n/, '') }));
    expect(failures(report, 'rowsFor')!.error).toContain('no sweep');
  });

  test('a manifest field name that does not exist in the domain is a mismatch, not a skip', async () => {
    const report = await runContext(contextWorkspace({ manifest: (m) => { m.contexts!.Inputs.fields[0].name = 'missing'; } }));
    expect(report.passed).toBe(false);
  });
});

/** Hazard fixture: multi-line literals, function expressions, assignment patterns, a dependency cycle, strings, booleans and array holes. */
function hazardWorkspace(alterModule?: (text: string) => string) {
  const dir = mkdtempSync(join(tmpdir(), 'mochi-lift-hazards-'));
  cpSync(join(fixture, 'hazards'), dir, { recursive: true });
  const manifest = JSON.parse(readFileSync(join(dir, 'hazards.lift.json'), 'utf8')) as LiftManifest;
  if (alterModule) {
    const next = alterModule(readFileSync(join(dir, 'hazards.ts'), 'utf8'));
    writeFileSync(join(dir, 'hazards.ts'), next);
    manifest.module_sha256 = sha256(next);
  }
  writeFileSync(join(dir, 'hazards.lift.json'), JSON.stringify(manifest));
  return { manifest: join(dir, 'hazards.lift.json'), module: join(dir, 'hazards.ts'), source: join(dir, 'source.js') };
}
const runHazards = (w: ReturnType<typeof hazardWorkspace>) => differential(w.manifest, w.module, w.source);
const resultOf = (report: Awaited<ReturnType<typeof runHazards>>, name: string) => report.functions.find((f) => f.export === name)!;

/** A one-function workspace written by hand, so the checker can be tested on lifts the Rust lifter would refuse. */
function handWorkspace(original: string, lifted: string, params = [{ from: 'e', name: 'e', type: 'number' }], callLimitMs?: number) {
  const dir = mkdtempSync(join(tmpdir(), 'mochi-lift-hand-'));
  const source = `${original}\n`;
  // The function is the declarator's initializer: `const g = <this>;`.
  const start = source.indexOf('= ') + 2;
  const end = source.lastIndexOf(';');
  const module = `${lifted}\n`;
  writeFileSync(join(dir, 'source.js'), source);
  writeFileSync(join(dir, 'g.ts'), module);
  const manifest: LiftManifest = {
    schema_version: 1,
    source_sha256: sha256(source),
    module_sha256: sha256(module),
    functions: [{ export: 'g', original: { name: 'g', line: 1, kind: 'arrow-declarator', start, end, sha256: sha256(source.slice(start, end)) }, deps: [], params, returns: 'number' }],
    unlifted: [],
  };
  writeFileSync(join(dir, 'g.lift.json'), JSON.stringify(manifest));
  return differential(join(dir, 'g.lift.json'), join(dir, 'g.ts'), join(dir, 'source.js'), { callLimitMs });
}

describe('checker hardening', () => {
  test('literals with line breaks, function expressions, assignment patterns, cycles, strings and holes all match', async () => {
    const report = await runHazards(hazardWorkspace());
    expect(report.functions.filter((f) => f.error || f.mismatches.length)).toEqual([]);
    expect(report.passed).toBe(true);
    expect(report.functions.map((f) => f.export)).toEqual(['describe', 'joinParts', 'factorial', 'isEven', 'isOdd', 'spread', 'label', 'holes']);
  });

  test('re-indenting a template literal changes its value and is detected', async () => {
    const report = await runHazards(hazardWorkspace((t) => t.replace('\n\t indented line', '\n   indented line')));
    expect(resultOf(report, 'describe').mismatches.length).toBeGreaterThan(0);
  });

  test('an array hole replaced by undefined is detected', async () => {
    const report = await runHazards(hazardWorkspace((t) => t.replace('[1, , 3]', '[1, undefined, 3]')));
    expect(resultOf(report, 'holes').mismatches.length).toBeGreaterThan(0);
  });

  test('a wrong branch for integers larger than the old unit-range domain is detected', async () => {
    const report = await runHazards(hazardWorkspace((t) => t.replace('n <= 150', 'n <= 80')));
    expect(resultOf(report, 'factorial').mismatches.length).toBeGreaterThan(0);
  });

  test('a string and a boolean mutation are detected', async () => {
    const report = await runHazards(hazardWorkspace((t) => t.replace('text.toUpperCase()', 'text.toLowerCase()')));
    expect(resultOf(report, 'label').mismatches.length).toBeGreaterThan(0);
  });

  test('mutually recursive functions do not hang the closure walk', async () => {
    const report = await runHazards(hazardWorkspace());
    expect(resultOf(report, 'isEven').cases).toBeGreaterThan(50);
  });

  test('a function whose original never returns normally is not a pass', async () => {
    const report = await handWorkspace('const g = (e) => { throw new Error("x"); };', 'export function g(e: number): number { throw new Error("x"); }');
    expect(report.passed).toBe(false);
    expect(report.functions[0].returned).toBe(0);
  });

  test('thrown errors with different messages, or non-Error throws, are told apart', async () => {
    const otherMessage = await handWorkspace('const g = (e) => { if (e > 0.5) throw new Error("x"); return e; };', 'export function g(e: number): number { if (e > 0.5) throw new Error("y"); return e; }');
    expect(otherMessage.functions[0].mismatches.length).toBeGreaterThan(0);
    const primitive = await handWorkspace('const g = (e) => { if (e > 0.5) throw null; return e; };', 'export function g(e: number): number { if (e > 0.5) throw undefined; return e; }');
    expect(primitive.functions[0].mismatches.length).toBeGreaterThan(0);
    const same = await handWorkspace('const g = (e) => { if (e > 0.5) throw null; return e; };', 'export function g(e: number): number { if (e > 0.5) throw null; return e; }');
    expect(same.passed).toBe(true);
  });

  test('the original runs strict and cannot leak globals into the checking realm', async () => {
    const leak = await handWorkspace('const g = (e) => { Math.__leak = 1; return e; };', 'export function g(e: number): number { return e; }');
    expect(leak.passed).toBe(true);
    expect((Math as unknown as Record<string, unknown>).__leak).toBeUndefined();
    const sloppy = await handWorkspace('const g = (e) => { undeclaredVariable = e; return e; };', 'export function g(e: number): number { undeclaredVariable = e; return e; }');
    // Strict mode: both sides throw a ReferenceError, so nothing returns normally and the function cannot pass.
    expect(sloppy.passed).toBe(false);
  });

  test('module state in the lifted code that changes results across cases is exposed', async () => {
    const stateful = await handWorkspace('const g = (e) => e;', 'let calls = 0;\nexport function g(e: number): number { calls++; return calls > 50 ? e + 1 : e; }');
    // State that builds up across the cases of one function is exposed (and the original span was really evaluated).
    expect(stateful.functions[0].error).toBeUndefined();
    expect(stateful.functions[0].returned).toBeGreaterThan(100);
    expect(stateful.functions[0].mismatches.length).toBeGreaterThan(0);
  });
  test('an input on which the original never terminates is reported, not hung on', async () => {
    const report = await handWorkspace('const g = (e) => { while (e > 0.9) {} return e; };', 'export function g(e: number): number { return e; }', undefined, 100);
    expect(report.passed).toBe(false);
    expect(report.functions[0].mismatches.join('\n')).toContain('did not finish');
  }, 30000);

  test('key order and frozenness of returned objects are compared', async () => {
    const order = await handWorkspace('const g = (e) => ({ a: e, b: 2 });', 'export function g(e: number) { return { b: 2, a: e }; }');
    expect(order.functions[0].mismatches.length).toBeGreaterThan(0);
    const frozen = await handWorkspace('const g = (e) => Object.freeze({ a: e });', 'export function g(e: number) { return { a: e }; }');
    expect(frozen.functions[0].mismatches.length).toBeGreaterThan(0);
  });
});

describe('checker round three', () => {
  test('a lifted function that mutates an ordinary argument the original leaves alone is detected', async () => {
    const report = await handWorkspace('const g = (e) => e;', 'export function g(e: number): number { return e; }', [{ from: 'e', name: 'e', type: 'readonly Point2[]' }]);
    expect(report.passed).toBe(true);
    const mutating = await handWorkspace('const g = (p) => p.length;', 'export function g(p: { x: number }[]): number { p[0].x += 1; return p.length; }', [{ from: 'p', name: 'p', type: 'readonly Point2[]' }]);
    expect(mutating.passed).toBe(false);
    expect(mutating.functions[0].mismatches.join('\n')).toContain('arguments afterwards');
    const both = await handWorkspace('const g = (p) => { p[0].x += 1; return p.length; };', 'export function g(p: { x: number }[]): number { p[0].x += 1; return p.length; }', [{ from: 'p', name: 'p', type: 'readonly Point2[]' }]);
    expect(both.passed).toBe(true);
  });

  test('a function whose original throws on almost every case is not a pass', async () => {
    const report = await handWorkspace('const g = (e) => { if (e !== 0.5) throw new Error("x"); return e; };', 'export function g(e: number): number { if (e !== 0.5) throw new Error("x"); return e; }');
    expect(report.functions[0].returned).toBeGreaterThan(0);
    expect(report.passed).toBe(false);
  });
});
