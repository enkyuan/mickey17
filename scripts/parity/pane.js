/* Page-side helper for the Browser pane path (see docs/PARITY.md, "Browser pane driver").
 * Load from the sink, then call from javascript_tool:
 *   (0, eval)(await (await fetch('http://127.0.0.1:8765/pane.js')).text())
 *   await __mochiPane.capture('home.desktop1440.initial.ref')
 * Evaluates the collector (from the sink's /collect.js), waits for the visual signature to be
 * stable, collects, and POSTs { schema:'pane-1', context, collection } to the sink.
 * The pane has no reduced-motion or virtual-clock control: such states are headless only.
 */
(() => {
  const SINK = (document.currentScript && document.currentScript.src ? new URL(document.currentScript.src).origin : null) || globalThis.__mochiSink || 'http://127.0.0.1:8765';
  globalThis.__mochiPane = {
    sink: SINK,
    async settle(maxMs = 15000) {
      await document.fonts.ready;
      let last = '', stable = 0;
      const end = performance.now() + maxMs;
      while (performance.now() < end) {
        const sig = __mochiParity.signature() + '|' + [...document.images].filter(i => !i.complete).length;
        stable = sig === last ? stable + 1 : 0; last = sig;
        if (stable >= 4) return true;
        await new Promise(r => setTimeout(r, 300));
      }
      return false;
    },
    async capture(name, { hashAssets = true } = {}) {
      if (!globalThis.__mochiParity) (0, eval)(await (await fetch(SINK + '/collect.js')).text());
      const settled = await this.settle();
      const collection = await __mochiParity.collect({ hashAssets });
      const body = JSON.stringify({ schema: 'pane-1', name, settled, errors: [], blocked: [], stepErrors: [], focusOrder: [], reachedScrollY: Math.round(scrollY), timingMs: 0, collection });
      const res = await fetch(SINK + '/save/' + name + '.json', { method: 'POST', body });
      return { status: res.status, ...(await res.json().catch(() => ({}))), settled, elements: collection.els.length, scrollY: Math.round(scrollY), vw: innerWidth, vh: innerHeight };
    },
  };
  return 'pane helper installed';
})()
