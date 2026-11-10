// bun test scripts/tokens/computed-equality.test.js
// Covers compare() and the command line. snapshot() needs a real DOM and is exercised in a browser.
import { expect, test } from 'bun:test';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const harness = require('./computed-equality.js');

const snap = (elements, extra = {}) => ({
  version: 1,
  url: 'https://t.test/',
  viewport: { width: 1440, height: 900, dpr: 1 },
  colorScheme: 'dark',
  reducedMotion: false,
  elementCount: Object.keys(elements).length,
  elements,
  ...extra,
});

const base = () => ({ 'html:1/body:1/div#a:1': { color: 'rgb(17, 17, 17)', 'padding-top': '16px' } });

test('identical snapshots are equal', () => {
  const r = harness.compare(snap(base()), snap(base()));
  expect(r.equal).toBe(true);
  expect(r.propertiesCompared).toBe(2);
});

test('a changed value, a missing element and an extra element are all reported', () => {
  const changed = base();
  changed['html:1/body:1/div#a:1'].color = 'rgb(34, 34, 34)';
  const r = harness.compare(snap(base()), snap(changed));
  expect(r.equal).toBe(false);
  expect(r.differences).toEqual([
    { element: 'html:1/body:1/div#a:1', property: 'color', before: 'rgb(17, 17, 17)', after: 'rgb(34, 34, 34)' },
  ]);
  expect(harness.compare(snap(base()), snap({})).missingInAfter.length).toBe(1);
  expect(harness.compare(snap({}), snap(base())).extraInAfter.length).toBe(1);
});

test('a property present on one side only is a difference', () => {
  const after = base();
  delete after['html:1/body:1/div#a:1']['padding-top'];
  expect(harness.compare(snap(base()), snap(after)).differences[0].property).toBe('padding-top');
});

test('pixel tolerance is opt-in and only applies to px values', () => {
  const near = base();
  near['html:1/body:1/div#a:1']['padding-top'] = '16.01px';
  expect(harness.compare(snap(base()), snap(near)).equal).toBe(false);
  expect(harness.compare(snap(base()), snap(near), { tolerancePx: 0.02 }).equal).toBe(true);
  const colour = base();
  colour['html:1/body:1/div#a:1'].color = 'rgb(17, 17, 18)';
  expect(harness.compare(snap(base()), snap(colour), { tolerancePx: 5 }).equal).toBe(false);
});

test('properties the central file introduces are accepted, changed shared properties are not', () => {
  const after = base();
  after['html:1/body:1/div#a:1']['--color-ink'] = '#111';
  expect(harness.compare(snap(base()), snap(after)).equal).toBe(false);
  expect(harness.compare(snap(base()), snap(after), { introduced: ['--color-ink'] }).equal).toBe(true);
  after['html:1/body:1/div#a:1'].color = 'red';
  const r = harness.compare(snap(base()), snap(after), { introduced: ['--color-ink'] });
  expect(r.equal).toBe(false);
  expect(r.differences.length).toBe(1);
});

test('runs compared in different environments are not equal', () => {
  const r = harness.compare(snap(base()), snap(base(), { colorScheme: 'light' }));
  expect(r.context.sameColorScheme).toBe(false);
  expect(r.equal).toBe(false);
  expect(harness.compare(snap(base()), snap(base(), { viewport: { width: 375, height: 812, dpr: 1 } })).equal).toBe(false);
});

test('command line exits 1 on a difference and 0 when equal', () => {
  const dir = mkdtempSync(join(tmpdir(), 'tokens-harness-'));
  const write = (name, value) => {
    const file = join(dir, name);
    writeFileSync(file, JSON.stringify(value));
    return file;
  };
  const a = write('a.json', snap(base()));
  const b = write('b.json', snap(base()));
  const changed = base();
  changed['html:1/body:1/div#a:1'].color = 'red';
  const c = write('c.json', snap(changed));
  const script = join(import.meta.dir, 'computed-equality.js');
  expect(spawnSync('bun', [script, a, b]).status).toBe(0);
  expect(spawnSync('bun', [script, a, c]).status).toBe(1);
  // The pipeline ships the same file as tokens.equality-harness.js; it must behave identically.
  const shipped = join(dir, 'tokens.equality-harness.js');
  copyFileSync(script, shipped);
  expect(spawnSync('bun', [shipped, a, b]).status).toBe(0);
  expect(spawnSync('bun', [shipped, a, c]).status).toBe(1);
});
