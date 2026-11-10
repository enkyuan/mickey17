import { describe, expect, test } from 'bun:test';
import { findChromium, loadPlaywright } from '../src/parity/driver';
import { runExample, verdict, VARIANTS } from '../src/parity/example';

// Real Chromium over the offline fixtures (no sockets). Reported as skipped, with the reason,
// when Playwright or a browser binary is missing or cannot start (for example inside a sandbox).
let reason = '';
try { loadPlaywright(); if (!findChromium()) reason = 'no Chromium binary (set MOCHI_CHROME)'; } catch (e) { reason = (e as Error).message.split('\n')[0]; }
let outcome: Awaited<ReturnType<typeof runExample>> | undefined;
if (!reason) {
  try { outcome = await runExample(undefined, () => {}); } catch (e) { reason = 'browser did not start: ' + (e as Error).message.split('\n')[0]; }
}
const t = outcome ? test : test.skip;
if (!outcome) console.warn('parity browser tests skipped: ' + reason);

describe('collector + driver on offline fixtures', () => {
  t('markup-only changes are accepted in every state', () => {
    for (const [state, status] of Object.entries(outcome!.candidate)) expect(['identical', 'minor_only'], state).toContain(status);
  });
  for (const v of VARIANTS) t(`${v.name}: rejected in ${v.seenIn}${v.unseenIn ? `, invisible in ${v.unseenIn}` : ''}`, () => {
    const r = outcome!.variants[v.name];
    expect(r.seenIn).toBe('different');
    expect(r.items.some(i => i.startsWith(v.expect)), r.items.join(', ')).toBe(true);
    if (v.unseenIn) expect(['identical', 'minor_only']).toContain(r.unseenIn!);
  });
  t('verdict() has no problems', () => expect(verdict(outcome!)).toEqual([]));
});
