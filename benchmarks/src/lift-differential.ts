import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import vm from 'node:vm';

type Param = { from: string; name: string; type: string };
type ContextField = { from: string; name: string; type: string };
type LiftedFunction = {
  export: string;
  original: { name: string; line: number; kind: string; start: number; end: number; sha256: string };
  deps: string[];
  params: Param[];
  returns: string;
  context?: { type: string; param: string; reads: string[] };
  memo?: { cache: string; compute: string; key_fields: string[]; coupled: { cache: string; sampler: string }[] };
  memo_read?: { cache: string; via: string; compute: string };
};
export type LiftManifest = {
  schema_version: number;
  source_sha256: string;
  module_sha256: string;
  contexts?: Record<string, { fields: ContextField[] }>;
  functions: LiftedFunction[];
  unlifted: { name: string; line: number; reasons: string[] }[];
};
/** A context value: one explicit value for every field of a context type. */
export type ContextValue = Record<string, unknown>;
/**
 * Supplies real captured values for each context type. `contexts` is a grid of independent
 * values; `sweeps` are ordered sequences replayed through ONE original closure so its memo
 * cache is exercised across hits, key changes and revisits.
 */
export type Domain = {
  contexts: Record<string, () => Iterable<ContextValue>>;
  sweeps?: Record<string, () => Iterable<ContextValue>>;
  /** Fields that are one fixed real value across the whole grid, by context type. Any other field must vary. */
  constants?: Record<string, string[]>;
};
export type Options = { domain?: string; maxMismatches?: number; callLimitMs?: number };
export type FunctionResult = { export: string; cases: number; returned: number; threw?: number; mismatches: string[]; error?: string };
/** A function whose original throws on more than this share of cases was barely compared. */
export const MAX_THROW_SHARE = 0.75;
export type DifferentialReport = { functions: FunctionResult[]; passed: boolean };

const sha256 = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
const dense = Array.from({ length: 81 }, (_, i) => (i - 8) / 64);
const coarse = [-1, -0.25, 0, 0.001, 0.2, 0.5, 0.78, 0.999, 1, 1.5, 10, 0.3333333333333333, 2.5, -0.5, 0.75, 0.9, 0.1];
const integers = [2, 3, 4, 5, 7, 8, 16, 64, 100, 1000, -2, -10, 2 ** 31, 2 ** 53];
const edge = [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, -0];
const pointLists = [1, 2, 3, 5, 9].map((count) =>
  Array.from({ length: count }, (_, i) => ({ x: (i * 37) % 11 - 3.5, y: (i * i * 5 + 2) % 13 + 0.25 * i }))
);

const lookupTable = Array.from({ length: 81 }, (_, i) => Math.sin(i / 7) * 3 + i * i / 900);

/** Argument generators are chosen by declared TypeScript parameter type; an unknown type is a failure, not a skip. */
function domain(type: string): unknown[] | undefined {
  const normalized = type.replace(/\s+/g, '');
  if (normalized === 'number') return undefined;
  if (normalized === 'string') return ['', 'a', 'Hello, World', 'ÅÄö—😀', ' \t\n', '0', 'NaN'];
  if (normalized === 'boolean') return [false, true];
  if (/^(readonly)?Point2\[\]$/.test(normalized)) return pointLists;
  if (normalized === 'readonlynumber[]|undefined') return [undefined, lookupTable];
  return [];
}
function* tuples(params: Param[]): Generator<unknown[]> {
  const lists = params.map((p) => {
    const special = domain(p.type);
    if (special === undefined) return params.length === 1 ? [...dense, ...coarse, ...integers, ...edge] : [...coarse, 2, 3, 4, 8, 64, ...edge];
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
const tagOf = (x: object) => Object.prototype.toString.call(x);
/** Realm-independent: the originals run in a separate vm context, so prototypes are compared by tag and constructor name. */
const protoName = (x: object) => {
  const proto = Object.getPrototypeOf(x);
  return proto === null ? 'null' : String((proto as { constructor?: { name?: string } }).constructor?.name);
};
const same = (a: unknown, b: unknown): boolean => {
  if (typeof a !== typeof b) return false;
  if (typeof a === 'number') return Object.is(a, b);
  if (typeof a !== 'object' || a === null || b === null) return Object.is(a, b);
  const tag = tagOf(a);
  if (tag !== tagOf(b as object) || protoName(a) !== protoName(b as object)) return false;
  if (a instanceof Date || tag === '[object Date]') return Object.is((a as Date).getTime(), (b as Date).getTime());
  if (tag === '[object Map]') {
    const left = [...(a as Map<unknown, unknown>).entries()], right = [...(b as Map<unknown, unknown>).entries()];
    return left.length === right.length && left.every(([k, v], i) => same(k, right[i][0]) && same(v, right[i][1]));
  }
  if (tag === '[object Set]') {
    const left = [...(a as Set<unknown>)], right = [...(b as Set<unknown>)];
    return left.length === right.length && left.every((v, i) => same(v, right[i]));
  }
  // Own keys include array indices (so holes differ), `length`, non-index properties and symbols.
  const left = Reflect.ownKeys(a), right = Reflect.ownKeys(b as object);
  if (left.length !== right.length) return false;
  if (Object.isFrozen(a) !== Object.isFrozen(b) || Object.isSealed(a) !== Object.isSealed(b) || Object.isExtensible(a) !== Object.isExtensible(b)) return false;
  // Key order is observable (`Object.keys`, JSON, spread), so it is compared too.
  return left.every((k, i) => k === right[i] && same((a as Record<PropertyKey, unknown>)[k], (b as Record<PropertyKey, unknown>)[k]));
};
type Outcome = { threw: false; value: unknown } | { threw: true; kind: string; message: string };
const outcome = (run: () => unknown): Outcome => {
  try {
    return { threw: false, value: run() };
  } catch (e) {
    if (e !== null && typeof e === 'object') {
      const err = e as { name?: unknown; message?: unknown; code?: unknown; constructor?: { name?: unknown } };
      if (err.code === 'ERR_SCRIPT_EXECUTION_TIMEOUT') return { threw: true, kind: 'Timeout', message: `did not finish within ${CALL_LIMIT_MS} ms` };
      return { threw: true, kind: String(err.constructor?.name ?? err.name), message: String(err.message) };
    }
    return { threw: true, kind: `thrown ${typeof e}`, message: String(e) };
  }
};
/** Engine-generated TypeError/ReferenceError messages quote the source text, which renaming legitimately changes. */
const quotesSource = (kind: string) => kind === 'TypeError' || kind === 'ReferenceError';
const sameOutcome = (a: Outcome, b: Outcome): boolean => {
  if ((a.threw && a.kind === 'Timeout') || (b.threw && b.kind === 'Timeout')) return false; // not comparable
  if (a.threw && b.threw) return a.kind === b.kind && (quotesSource(a.kind) || a.message === b.message);
  if (!a.threw && !b.threw) return same(a.value, b.value);
  return false;
};
const shown = (o: Outcome) => (o.threw ? `throws ${o.kind}: ${o.message}` : o.value);
const describe = (value: unknown): string =>
  String(
    JSON.stringify(value, (_k, v) => {
      if (typeof v === 'number') return Number.isFinite(v) && !Object.is(v, -0) ? v : Object.is(v, -0) ? '-0' : String(v);
      if (v === undefined) return 'undefined';
      if (v !== null && typeof v === 'object') {
        const tag = Object.prototype.toString.call(v);
        if (tag === '[object Map]') return { Map: [...(v as Map<unknown, unknown>).entries()] };
        if (tag === '[object Set]') return { Set: [...(v as Set<unknown>)] };
      }
      return v;
    }),
  ).slice(0, 400);

/** Ordered dependency closure of one lifted function, in the order the original text must be declared. */
function closureOf(manifest: LiftManifest, target: LiftedFunction): LiftedFunction[] {
  const byExport = new Map(manifest.functions.map((f) => [f.export, f]));
  const order: LiftedFunction[] = [];
  const seen = new Set<LiftedFunction>();
  const visit = (fn: LiftedFunction) => {
    if (seen.has(fn)) return;
    seen.add(fn); // before recursing: mutually recursive functions are one closure
    for (const dep of fn.deps) {
      const next = byExport.get(dep);
      if (!next) throw new Error(`${fn.export} depends on ${dep}, which the manifest does not list`);
      visit(next);
    }
    order.push(fn);
  };
  visit(target);
  const scope = new Set<string>();
  for (const fn of order) {
    if (scope.has(fn.original.name)) throw new Error(`original name ${fn.original.name} is reused inside the dependency closure of ${target.export}`);
    scope.add(fn.original.name);
  }
  return order;
}
const originalText = (source: Uint8Array, fn: LiftedFunction) => {
  const body = Buffer.from(source.subarray(fn.original.start, fn.original.end)).toString('utf8');
  return fn.original.kind === 'function-declaration' ? body : `var ${fn.original.name} = ${body};`;
};

/**
 * Runs original text the way the controller runs it: strict (it is a module), in a fresh realm so a
 * write to a global (`Math.x = 1`) cannot leak into the lifted code or the next case.
 */
function strictScope(body: string): { value: unknown; context: vm.Context } {
  const context = vm.createContext({});
  return { value: vm.runInContext(`(function () {\n"use strict";\n${body}\n})()`, context), context };
}
/** Every call runs as its own script so a non-terminating input is cut off instead of hanging the checker. */
let CALL_LIMIT_MS = 2000;
const callScript = new vm.Script('__result = __fn.apply(undefined, __args)');
function limited(context: vm.Context, fn: (...args: unknown[]) => unknown): (...args: unknown[]) => unknown {
  return (...args) => {
    Object.assign(context, { __fn: fn, __args: args, __result: undefined });
    callScript.runInContext(context, { timeout: CALL_LIMIT_MS });
    return (context as { __result: unknown }).__result;
  };
}
const lifterContext = vm.createContext({});

/** Evaluates the original function text with only its own transitive dependencies in scope. */
function originalFunction(manifest: LiftManifest, source: Uint8Array, target: LiftedFunction): (...args: unknown[]) => unknown {
  const order = closureOf(manifest, target);
  const { value, context } = strictScope(`${order.map((fn) => originalText(source, fn)).join('\n')}\nreturn ${target.original.name};`);
  return limited(context, value as (...args: unknown[]) => unknown);
}

/** The original, with each captured variable bound to the value its context field carries. */
type BoundOriginal = { call: (...args: unknown[]) => unknown; set: (context: ContextValue) => void };

const isIdentifier = (name: string) => /^[A-Za-z_$][\w$]*$/.test(name);
/**
 * Builds the ORIGINAL function text in a fresh scope where every captured variable named by the
 * context fields is a `var`, each `name()` field is a function returning the sampled value, and
 * every memo cache the closure can touch starts as `null`, exactly as in the controller. `set`
 * rebinds the captured values (as the controller's resize handler would) without touching caches.
 */
function bindOriginal(manifest: LiftManifest, source: Uint8Array, target: LiftedFunction): BoundOriginal {
  const usage = target.context;
  if (!usage) {
    const fn = originalFunction(manifest, source, target);
    return { call: fn, set: () => {} };
  }
  const fields = manifest.contexts?.[usage.type]?.fields;
  if (!fields) throw new Error(`manifest declares no context ${usage.type}`);
  const order = closureOf(manifest, target);
  const names = new Set(order.map((fn) => fn.original.name));
  const roots = new Map<string, { tree: Map<string, unknown> | null; field?: ContextField }>();
  const callRoots: { root: string; field: ContextField }[] = [];
  for (const field of fields) {
    const call = field.from.endsWith('()');
    const parts = (call ? field.from.slice(0, -2) : field.from).split('.');
    if (!parts.every(isIdentifier)) throw new Error(`context field ${field.from} is not a bindable path`);
    if (names.has(parts[0])) throw new Error(`captured name ${parts[0]} collides with a function in the closure of ${target.export}`);
    if (call) callRoots.push({ root: parts[0], field });
    else if (parts.length === 1) roots.set(parts[0], { tree: null, field });
    else {
      let entry = roots.get(parts[0]);
      if (!entry) roots.set(parts[0], (entry = { tree: new Map() }));
      let node = entry.tree!;
      for (const part of parts.slice(1, -1)) {
        if (!node.has(part)) node.set(part, new Map());
        node = node.get(part) as Map<string, unknown>;
      }
      node.set(parts[parts.length - 1], field);
    }
  }
  const expr = (node: unknown): string =>
    node instanceof Map ? `{${[...node.entries()].map(([k, v]) => `${JSON.stringify(k)}: ${expr(v)}`).join(', ')}}` : `__ctx[${JSON.stringify((node as ContextField).name)}]`;
  const caches = new Set<string>();
  for (const fn of order) {
    if (fn.memo) {
      caches.add(fn.memo.cache);
      for (const c of fn.memo.coupled) caches.add(c.cache);
    }
    if (fn.memo_read) caches.add(fn.memo_read.cache);
  }
  for (const cache of caches) if (names.has(cache) || roots.has(cache)) throw new Error(`cache ${cache} collides with another captured name`);
  const declarations = [...roots.keys(), ...caches].map((n) => `var ${n}${caches.has(n) ? ' = null' : ''};`);
  const samplers = callRoots.map(({ root, field }) => `function ${root}() { return __ctx[${JSON.stringify(field.name)}]; }`);
  const assignments = [...roots.entries()].map(([root, { tree, field }]) => `${root} = ${tree ? expr(tree) : expr(field)};`);
  const body = [
    'var __ctx;',
    ...declarations,
    ...samplers,
    ...order.map((fn) => originalText(source, fn)),
    `return { call: ${target.original.name}, set: function (c) { __ctx = c; ${assignments.join(' ')} } };`,
  ].join('\n');
  const { value, context } = strictScope(body);
  const bound = value as BoundOriginal;
  return { call: limited(context, bound.call), set: bound.set };
}

const GRID_LIMIT = 20000;
/** A domain value must carry exactly the manifest's fields, and number fields must hold numbers: a missing field would make both sides compute NaN and agree. */
function checkContext(type: string, fields: ContextField[], value: ContextValue) {
  const names = fields.map((f) => f.name);
  const keys = Object.keys(value);
  if (keys.length !== names.length || !names.every((n) => n in value)) throw new Error(`a ${type} value has fields [${keys.join(', ')}], but the manifest declares [${names.join(', ')}]`);
  for (const f of fields) {
    const kind = f.type.trim();
    if ((kind === 'number' || kind === 'string' || kind === 'boolean') && typeof value[f.name] !== kind) throw new Error(`${type}.${f.name} must be a ${kind}, got ${describe(value[f.name])}`);
    if (value[f.name] === undefined || value[f.name] === null) throw new Error(`${type}.${f.name} is ${value[f.name]}`);
  }
}
function* contextGrid(domain: Domain | undefined, type: string, fields: ContextField[]): Generator<ContextValue> {
  const make = domain?.contexts[type];
  if (!make) throw new Error(`the domain supplies no contexts for ${type}; a context type with no input generator fails rather than being skipped`);
  let count = 0;
  for (const value of make()) {
    if (++count > GRID_LIMIT) throw new Error(`the domain for ${type} exceeds ${GRID_LIMIT} contexts`);
    checkContext(type, fields, value);
    yield value;
  }
  if (count === 0) throw new Error(`the domain for ${type} is empty`);
}
const memoOf = (manifest: LiftManifest, target: LiftedFunction) => closureOf(manifest, target).filter((fn) => fn.memo || fn.memo_read);

export async function differential(manifestPath: string, modulePath: string, sourcePath: string, options: Options | number = {}): Promise<DifferentialReport> {
  const { domain: domainPath, maxMismatches = 5, callLimitMs = 2000 } = typeof options === 'number' ? { maxMismatches: options } : options;
  CALL_LIMIT_MS = callLimitMs;
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as LiftManifest;
  const source = readFileSync(sourcePath);
  if (sha256(source) !== manifest.source_sha256) throw new Error('source does not match the lift manifest');
  const moduleBytes = readFileSync(modulePath);
  if (sha256(moduleBytes) !== manifest.module_sha256) throw new Error('lifted module does not match the lift manifest');
  // Import the bytes that were hashed (not whatever is on disk now), once per checked function so
  // module-level state in the lifted code cannot carry over from one function's cases to the next.
  const verified = join(mkdtempSync(join(tmpdir(), 'mochi-lift-verified-')), basename(modulePath));
  writeFileSync(verified, moduleBytes);
  let instance = 0;
  const load = async () => (await import(`${pathToFileURL(verified).href}?${manifest.module_sha256}-${instance++}`)) as Record<string, (...args: unknown[]) => unknown>;
  const domain = domainPath ? ((await import(pathToFileURL(resolve(domainPath)).href)) as Domain) : undefined;
  const functions: FunctionResult[] = [];
  for (const target of manifest.functions) {
    const result: FunctionResult = { export: target.export, cases: 0, returned: 0, mismatches: [] };
    functions.push(result);
    const record = (args: unknown[], expected: unknown, actual: unknown, note = '') => {
      if (result.mismatches.length < maxMismatches) result.mismatches.push(`${target.export}(${args.map(describe).join(', ')})${note}: original ${describe(expected)}, lifted ${describe(actual)}`);
    };
    /** Runs the original and the lifted function once and compares them, including whether the lifted one altered its inputs. */
    const compare = (call: (...args: unknown[]) => unknown, candidate: (...args: unknown[]) => unknown, args: unknown[], context?: ContextValue, note = '') => {
      result.cases++;
      const expectedArgs = structuredClone(args);
      const lastArgs = context ? [...structuredClone(args), structuredClone(context)] : structuredClone(args);
      const expected = outcome(() => call(...expectedArgs));
      const pristine = context ? structuredClone(context) : undefined;
      const actual = outcome(() => limited(lifterContext, candidate)(...lastArgs));
      if (expected.threw) result.threw = (result.threw ?? 0) + 1;
      if (expected.threw && expected.kind === 'Timeout') {
        record(args, `original ${expected.message}`, 'not run', note);
        return;
      }
      if (!expected.threw) result.returned++;
      if (context && !same(lastArgs[lastArgs.length - 1], pristine)) record(args, 'inputs unchanged', 'inputs mutated by the lifted function', note);
      else if (!sameOutcome(expected, actual)) record(args, shown(expected), shown(actual), note);
      // Whatever the original does to its arguments, the lifted function must do the same.
      else if (!same(expectedArgs, lastArgs.slice(0, args.length))) record(args, `arguments afterwards ${describe(expectedArgs)}`, `arguments afterwards ${describe(lastArgs.slice(0, args.length))}`, note);
    };
    try {
      const span = Buffer.from(source.subarray(target.original.start, target.original.end));
      if (sha256(span) !== target.original.sha256) throw new Error('original span hash differs from the manifest');
      const candidate = (await load())[target.export];
      if (typeof candidate !== 'function') throw new Error('export missing from lifted module');
      const usage = target.context;
      if (!usage) {
        const original = originalFunction(manifest, source, target);
        for (const args of tuples(target.params)) compare(original, candidate, args);
        continue;
      }
      // A fresh original closure per context value: every call starts from an empty cache.
      const distinct = new Map<string, Set<string>>();
      for (const context of contextGrid(domain, usage.type, manifest.contexts?.[usage.type]?.fields ?? [])) {
        for (const [name, value] of Object.entries(context)) {
          const seen = distinct.get(name) ?? distinct.set(name, new Set()).get(name)!;
          if (seen.size < 2) seen.add(describe(value));
        }
        const bound = bindOriginal(manifest, source, target);
        bound.set(structuredClone(context));
        for (const args of tuples(target.params)) {
          compare(bound.call, candidate, args, context, ` @ ${describe(context)}`);
          // A second call on the same closure is served from the original's cache.
          if (memoOf(manifest, target).length) compare(bound.call, candidate, args, context, ` @ ${describe(context)} (cached)`);
        }
      }
      // A field that never varies cannot show that the lifted code really reads it.
      const constants = new Set(domain?.constants?.[usage.type] ?? []);
      for (const field of manifest.contexts?.[usage.type]?.fields ?? []) {
        if (!constants.has(field.name) && (distinct.get(field.name)?.size ?? 0) < 2) throw new Error(`context field ${usage.type}.${field.name} takes a single value across the grid; vary it or list it in the domain's constants`);
      }
      // One closure replayed through a sequence of contexts: cache hits, key changes and revisits.
      if (memoOf(manifest, target).length) {
        const make = domain?.sweeps?.[usage.type];
        if (!make) throw new Error(`${target.export} reaches a memo cache but the domain supplies no sweep for ${usage.type}`);
        const keyFields = memoOf(manifest, target).flatMap((fn) => fn.memo?.key_fields ?? []);
        const bound = bindOriginal(manifest, source, target);
        const seen = new Map<string, string>();
        let steps = 0, hits = 0, changes = 0, revisits = 0;
        let previous: string | undefined;
        const visited = new Set<string>();
        for (const context of make()) {
          steps++;
          checkContext(usage.type, manifest.contexts?.[usage.type]?.fields ?? [], context);
          const key = JSON.stringify(keyFields.map((f) => describe(context[f])));
          const whole = describe(context);
          if (keyFields.length && seen.has(key) && seen.get(key) !== whole) throw new Error(`sweep for ${usage.type} revisits key ${key} with different inputs; the original's cache would be stale, so the sweep does not model a coherent controller`);
          seen.set(key, whole);
          if (key === previous) hits++;
          else {
            if (previous !== undefined) changes++;
            if (visited.has(key)) revisits++;
          }
          previous = key;
          visited.add(key);
          bound.set(structuredClone(context));
          for (const args of tuples(target.params)) compare(bound.call, candidate, args, context, ` @ sweep step ${steps}`);
        }
        if (steps < 2) throw new Error(`the sweep for ${usage.type} has fewer than two steps`);
        if (keyFields.length && (hits === 0 || changes === 0 || revisits === 0)) throw new Error(`the sweep for ${usage.type} must contain a cache hit, a key change and a revisit (saw ${hits}, ${changes}, ${revisits})`);
      }
    } catch (e) {
      result.error = (e as Error).message;
    }
  }
  // A function whose original never returned normally on any case was never really compared.
  return { functions, passed: functions.every((f) => !f.error && f.cases > 0 && f.returned > 0 && (f.threw ?? 0) <= MAX_THROW_SHARE * f.cases && f.mismatches.length === 0) };
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  const flag = args.indexOf('--domain');
  const domain = flag >= 0 ? args.splice(flag, 2)[1] : undefined;
  const [manifest, module, source] = args;
  if (!manifest || !module || !source) throw new Error('usage: bun src/lift-differential.ts <manifest.json> <module.ts> <source.js> [--domain <domain.ts>]');
  const report = await differential(manifest, module, source, { domain });
  for (const f of report.functions) console.log(`${f.error ? 'ERROR' : f.mismatches.length ? 'DIFF ' : f.returned === 0 || (f.threw ?? 0) > MAX_THROW_SHARE * f.cases ? 'NONE ' : 'ok   '} ${f.export}: ${f.cases} cases (${f.returned} returned)${f.error ? `, ${f.error}` : ''}${f.mismatches.map((m) => `\n    ${m}`).join('')}`);
  console.log(report.passed ? 'differential: all lifted functions match the original' : 'differential: FAILED');
  process.exit(report.passed ? 0 : 1);
}
