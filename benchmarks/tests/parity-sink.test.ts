import { describe, expect, test } from 'bun:test';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createSink } from '../../scripts/parity/sink';

// Needs to bind a loopback port. Where binding is not permitted (some sandboxes) the
// tests are reported as skipped with that reason, never silently passed.
let server: ReturnType<typeof createSink> | undefined, reason = '';
const dir = mkdtempSync(join(tmpdir(), 'parity-sink-'));
try { server = createSink({ dir, port: 0 }); } catch (e) { reason = (e as Error).message; }
const t = server ? test : test.skip;
if (!server) console.warn('parity sink tests skipped: cannot bind loopback port: ' + reason);
const url = (p: string) => `http://127.0.0.1:${server!.port}${p}`;

describe('parity sink', () => {
  t('stores JSON once, refuses overwrite, bad names, non-JSON, and serves the collector with CORS', async () => {
    const ok = await fetch(url('/save/state.ref.json'), { method: 'POST', body: JSON.stringify({ a: 1 }) });
    expect(ok.status).toBe(200);
    expect(ok.headers.get('access-control-allow-origin')).toBeNull(); // no Origin header: no CORS grant
    const cross = await fetch(url('/list'), { headers: { origin: 'https://evil.example' } });
    expect(cross.headers.get('access-control-allow-origin')).toBeNull();
    const local = await fetch(url('/list'), { headers: { origin: 'http://127.0.0.1:8100' } });
    expect(local.headers.get('access-control-allow-origin')).toBe('http://127.0.0.1:8100');
    expect(JSON.parse(readFileSync(join(dir, 'state.ref.json'), 'utf8'))).toEqual({ a: 1 });
    expect((await fetch(url('/save/state.ref.json'), { method: 'POST', body: '{}' })).status).toBe(409);
    expect((await fetch(url('/save/..%2Fescape.json'), { method: 'POST', body: '{}' })).status).toBe(404);
    expect((await fetch(url('/save/.hidden'), { method: 'POST', body: '{}' })).status).toBe(404);
    expect((await fetch(url('/save/bad.json'), { method: 'POST', body: 'not json' })).status).toBe(400);
    expect((await fetch(url('/collect.js'))).headers.get('content-type')).toContain('javascript');
    expect(await (await fetch(url('/list'))).json()).toEqual(['state.ref.json']);
    expect((await fetch(url('/save/evil.json'), { method: 'POST', body: '{}', headers: { origin: 'https://evil.example' } })).status).toBe(403);
    server!.stop(true);
  });
});
