// Sink server for the Browser pane path. The pane cannot write files, and a collection is
// too large to read back through a tool result, so the page POSTs it here.
//   bun scripts/parity/sink.ts --port 8765 --dir .mochi/parity-runs/pane-1
//   GET  /collect.js, /pane.js     the collector and the page-side helper (so the page can load them)
//   POST /save/<name>.json         stores the body; an existing name is refused (409)
//   GET  /list                     names saved so far
// Loopback only. Names are [A-Za-z0-9._-]+ and bodies are capped, so a page cannot write elsewhere.
import { existsSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

export function createSink({ dir, port = 8765 }: { dir: string; port?: number }) {
  const root = resolve(dir);
  const here = import.meta.dir;
  mkdirSync(root, { recursive: true });
  const corsFor = (req: Request): Record<string, string> => {
    // only pages served from a loopback origin (the original / generated app under test) may use the sink
    let allowed = false;
    try { const o = new URL(req.headers.get('origin') ?? ''); allowed = ['127.0.0.1', 'localhost', '[::1]'].includes(o.hostname); } catch { /* no Origin: same-origin or non-browser */ }
    return allowed ? { 'access-control-allow-origin': req.headers.get('origin')!, vary: 'origin', 'access-control-allow-methods': 'GET,POST,OPTIONS', 'access-control-allow-headers': 'content-type', 'access-control-allow-private-network': 'true' } : {};
  };
  return Bun.serve({
    hostname: '127.0.0.1', port, maxRequestBodySize: 64 * 1024 * 1024,
    async fetch(req) {
      const url = new URL(req.url);
      const cors = corsFor(req);
      // CORS only protects reads; a text/plain POST needs no preflight, so refuse writes from non-loopback origins outright
      const origin = req.headers.get('origin');
      if (origin && !cors['access-control-allow-origin']) return new Response('forbidden origin', { status: 403 });
      if (req.method === 'OPTIONS') return new Response(null, { status: 204, headers: cors });
      if (req.method === 'GET' && (url.pathname === '/collect.js' || url.pathname === '/pane.js')) return new Response(Bun.file(join(here, url.pathname.slice(1))), { headers: { ...cors, 'content-type': 'text/javascript' } });
      if (req.method === 'GET' && url.pathname === '/list') return Response.json(readdirSync(root), { headers: cors });
      const m = /^\/save\/([A-Za-z0-9._-]{1,120})$/.exec(url.pathname);
      if (req.method === 'POST' && m && !m[1].startsWith('.')) {
        const file = join(root, m[1]);
        if (existsSync(file)) return new Response('exists', { status: 409, headers: cors });
        const body = await req.text();
        try { JSON.parse(body); } catch { return new Response('not json', { status: 400, headers: cors }); }
        writeFileSync(file, body, { flag: 'wx' });
        return Response.json({ saved: m[1], bytes: body.length }, { headers: cors });
      }
      return new Response('not found', { status: 404, headers: cors });
    },
  });
}

if (import.meta.main) {
  const arg = (n: string, d: string) => { const i = process.argv.indexOf('--' + n); return i < 0 ? d : process.argv[i + 1]; };
  const s = createSink({ dir: arg('dir', '.mochi/parity-runs/pane'), port: +arg('port', '8765') });
  console.error(`parity sink on http://127.0.0.1:${s.port} -> ${resolve(arg('dir', '.mochi/parity-runs/pane'))}`);
}
