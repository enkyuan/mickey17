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
