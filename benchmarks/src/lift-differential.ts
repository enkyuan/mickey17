import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

type Param = { from: string; name: string; type: string };
type LiftedFunction = {
  export: string;
  original: { name: string; line: number; kind: string; start: number; end: number; sha256: string };
  deps: string[];
  params: Param[];
  returns: string;
};
export type LiftManifest = {
  schema_version: number;
  source_sha256: string;
  module_sha256: string;
  functions: LiftedFunction[];
  unlifted: { name: string; line: number; reasons: string[] }[];
};
export type FunctionResult = { export: string; cases: number; mismatches: string[]; error?: string };
export type DifferentialReport = { functions: FunctionResult[]; passed: boolean };

const sha256 = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
const dense = Array.from({ length: 81 }, (_, i) => (i - 8) / 64);
const coarse = [-1, -0.25, 0, 0.001, 0.2, 0.5, 0.78, 0.999, 1, 1.5, 10, 0.3333333333333333, 2.5, -0.5, 0.75, 0.9, 0.1];
const edge = [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, -0];
const pointLists = [1, 2, 3, 5, 9].map((count) =>
  Array.from({ length: count }, (_, i) => ({ x: (i * 37) % 11 - 3.5, y: (i * i * 5 + 2) % 13 + 0.25 * i }))
);

const lookupTable = Array.from({ length: 81 }, (_, i) => Math.sin(i / 7) * 3 + i * i / 900);

/** Argument generators are chosen by declared TypeScript parameter type; an unknown type is a failure, not a skip. */
function domain(type: string): unknown[] | undefined {
  const normalized = type.replace(/\s+/g, '');
  if (normalized === 'number') return undefined;
  if (/^(readonly)?Point2\[\]$/.test(normalized)) return pointLists;
  if (normalized === 'readonlynumber[]|undefined') return [undefined, lookupTable];
  return [];
}
function* tuples(params: Param[]): Generator<unknown[]> {
  const lists = params.map((p) => {
    const special = domain(p.type);
    if (special === undefined) return params.length === 1 ? [...dense, ...edge] : [...coarse, ...edge];
    return special;
  });
  if (lists.some((list) => list.length === 0)) throw new Error(`no argument generator for ${params.map((p) => p.type).join(', ')}`);
  const walk = function* (index: number, prefix: unknown[]): Generator<unknown[]> {
    if (index === lists.length) {
      yield prefix;
      return;
    }
    for (const value of lists[index]) yield* walk(index + 1, [...prefix, value]);
  };
  yield* walk(0, []);
}
const same = (a: unknown, b: unknown): boolean => {
  if (typeof a === 'number' || typeof b === 'number') return Object.is(a, b);
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((v, i) => same(v, b[i]));
  if (a && b && typeof a === 'object' && typeof b === 'object') {
    const keys = Object.keys(a);
    return keys.length === Object.keys(b).length && keys.every((k) => same((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
  }
  return Object.is(a, b);
};
const describe = (value: unknown) => JSON.stringify(value, (_k, v) => (typeof v === 'number' && !Number.isFinite(v) ? String(v) : Object.is(v, -0) ? '-0' : v));

/** Evaluates the original function text with only its own transitive dependencies in scope. */
function originalFunction(manifest: LiftManifest, source: Uint8Array, target: LiftedFunction): (...args: unknown[]) => unknown {
  const byExport = new Map(manifest.functions.map((f) => [f.export, f]));
  const order: LiftedFunction[] = [];
  const visit = (fn: LiftedFunction) => {
    if (order.includes(fn)) return;
    for (const dep of fn.deps) visit(byExport.get(dep)!);
    order.push(fn);
  };
  visit(target);
  const text = (fn: LiftedFunction) => {
    const body = Buffer.from(source.subarray(fn.original.start, fn.original.end)).toString('utf8');
    return fn.original.kind === 'function-declaration' ? body : `var ${fn.original.name} = ${body};`;
  };
  const scope = new Set<string>();
  for (const fn of order) {
    if (scope.has(fn.original.name)) throw new Error(`original name ${fn.original.name} is reused inside the dependency closure of ${target.export}`);
    scope.add(fn.original.name);
  }
  return new Function(`${order.map(text).join('\n')}\nreturn ${target.original.name};`)() as (...args: unknown[]) => unknown;
}

export async function differential(manifestPath: string, modulePath: string, sourcePath: string, maxMismatches = 5): Promise<DifferentialReport> {
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as LiftManifest;
  const source = readFileSync(sourcePath);
  if (sha256(source) !== manifest.source_sha256) throw new Error('source does not match the lift manifest');
  const moduleBytes = readFileSync(modulePath);
  if (sha256(moduleBytes) !== manifest.module_sha256) throw new Error('lifted module does not match the lift manifest');
  const lifted = (await import(`${pathToFileURL(modulePath).href}?${manifest.module_sha256}`)) as Record<string, (...args: unknown[]) => unknown>;
  const functions: FunctionResult[] = [];
  for (const target of manifest.functions) {
    const result: FunctionResult = { export: target.export, cases: 0, mismatches: [] };
    functions.push(result);
    try {
      const span = Buffer.from(source.subarray(target.original.start, target.original.end));
      if (sha256(span) !== target.original.sha256) throw new Error('original span hash differs from the manifest');
      const original = originalFunction(manifest, source, target);
      const candidate = lifted[target.export];
      if (typeof candidate !== 'function') throw new Error('export missing from lifted module');
      for (const args of tuples(target.params)) {
        result.cases++;
        let expected: unknown, actual: unknown, expectedThrew = false, actualThrew = false;
        try { expected = original(...structuredClone(args)); } catch (e) { expectedThrew = true; expected = (e as Error).constructor.name; }
        try { actual = candidate(...structuredClone(args)); } catch (e) { actualThrew = true; actual = (e as Error).constructor.name; }
        if (expectedThrew !== actualThrew || !same(expected, actual)) {
          if (result.mismatches.length < maxMismatches) result.mismatches.push(`${target.export}(${args.map(describe).join(', ')}): original ${describe(expected)}, lifted ${describe(actual)}`);
        }
      }
    } catch (e) {
      result.error = (e as Error).message;
    }
  }
  return { functions, passed: functions.every((f) => !f.error && f.cases > 0 && f.mismatches.length === 0) };
}

if (import.meta.main) {
  const [manifest, module, source] = process.argv.slice(2);
  if (!manifest || !module || !source) throw new Error('usage: bun src/lift-differential.ts <manifest.json> <module.ts> <source.js>');
  const report = await differential(manifest, module, source);
  for (const f of report.functions) console.log(`${f.error ? 'ERROR' : f.mismatches.length ? 'DIFF ' : 'ok   '} ${f.export}: ${f.cases} cases${f.error ? `, ${f.error}` : ''}${f.mismatches.map((m) => `\n    ${m}`).join('')}`);
  console.log(report.passed ? 'differential: all lifted functions match the original' : 'differential: FAILED');
  process.exit(report.passed ? 0 : 1);
}
