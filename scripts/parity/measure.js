/** Run in the reference/candidate browser after applying the same interaction trace.
 * Set globalThis.mochiParityProbe = { route, viewport, reduced_motion, anchors }.
 * Anchors map reviewed semantic names to selectors that uniquely identify elements.
 * This collector reads the DOM; it does not change layout or pause site animations.
 */
(async () => {
  const input = globalThis.mochiParityProbe;
  if (!input || !input.anchors || Object.keys(input.anchors).length === 0) throw new Error('Missing parity probe configuration');
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  if (location.pathname !== input.route || innerWidth !== input.viewport[0] || innerHeight !== input.viewport[1] || reduced !== input.reduced_motion) throw new Error('Parity browser context mismatch');
  await document.fonts.ready;
  const properties = ['display', 'position', 'fontFamily', 'fontSize', 'fontWeight', 'lineHeight', 'letterSpacing', 'color', 'backgroundColor', 'borderRadius', 'padding', 'gap', 'overflow', 'visibility', 'opacity', 'transform'];
  const normalize = text => text.replace(/\s+/g, ' ').trim();
  const measure = () => {
    const facts = {}, errors = [], inventory = Object.keys(input.anchors).sort();
    for (const [key, selector] of Object.entries(input.anchors)) {
      const matches = document.querySelectorAll(selector);
      if (matches.length !== 1) { errors.push(`${key}: expected one element, found ${matches.length}`); continue; }
      const node = matches[0], rect = node.getBoundingClientRect(), style = getComputedStyle(node);
      for (const name of ['x','y','width','height']) facts[`${key}.bounds.${name}`] = rect[name];
      // Text includes hidden panel content where it remains in the DOM. Missing content is not waived.
      facts[`${key}.text`] = normalize(node.textContent || '');
      for (const property of properties) facts[`${key}.style.${property}`] = style[property];
      for (const attribute of ['role','aria-label','aria-expanded','aria-selected','aria-pressed','aria-hidden','href','type','disabled']) {
        facts[`${key}.attribute.${attribute}`] = node.getAttribute(attribute) ?? '';
        facts[`${key}.attribute.${attribute}.present`] = node.hasAttribute(attribute);
      }
      if (node instanceof HTMLImageElement) {
        facts[`${key}.image.loaded`] = node.complete && node.naturalWidth > 0;
        facts[`${key}.image.aspectRatio`] = node.naturalHeight ? node.naturalWidth / node.naturalHeight : 0;
      }
      if (node instanceof SVGPathElement) {
        facts[`${key}.path.d`] = node.getAttribute('d') ?? '';
        facts[`${key}.path.length`] = node.getTotalLength();
        facts[`${key}.path.strokeDashoffset`] = style.strokeDashoffset;
      }
    }
    return { route: input.route, viewport: input.viewport, reduced_motion: reduced, inventory, facts, errors, settled: false };
  };
  // Consecutive identical observations are required. Persistent motion must be
  // captured at an explicit source-supported clock/state by the trace runner.
  let previous = '', stable = 0, state;
  const deadline = performance.now() + 5000;
  while (performance.now() < deadline) {
    state = measure();
    const signature = JSON.stringify(state.facts);
    stable = signature === previous ? stable + 1 : 0;
    previous = signature;
    if (stable >= 5) { state.settled = true; break; }
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  return state;
})()
