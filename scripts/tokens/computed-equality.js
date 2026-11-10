/*
 * Per-element computed-style equality harness for the token rewrite.
 *
 * In a browser (preview pane, `browse eval`, DevTools): load this file, then
 *   const before = await mochiTokenEquality.snapshot();   // original project
 *   const after  = await mochiTokenEquality.snapshot();   // tokenized project, same viewport
 *   mochiTokenEquality.compare(before, after);
 * Snapshots are plain JSON, so they can be saved from one page load and compared in
 * another: `bun scripts/tokens/computed-equality.js a.json b.json [tokens.manifest.json]` prints
 * the same report and exits 1 when anything differs. Pass the manifest so custom properties the
 * central file defines site-wide are accepted as additions (`options.introduced`).
 *
 * Rules (set these before navigating, identically for both runs): same viewport and device
 * pixel ratio, same prefers-color-scheme / reduced-motion emulation, same fonts, no pointer
 * over the page. The harness records every computed property of every element and of
 * ::before / ::after that generate content; nothing is sampled or excluded unless listed in
 * `ignore`. Elements are matched by structural path (tag and nth-of-type from the root, plus
 * id where present), so a rewrite that preserves the DOM compares one-to-one.
 */
(function (root) {
  'use strict';

  const DEFAULT_IGNORE = []; // keep empty: ignoring a property must be an explicit decision

  function pathOf(el) {
    const parts = [];
    for (let n = el; n && n.nodeType === 1; n = n.parentElement) {
      let index = 1;
      for (let s = n.previousElementSibling; s; s = s.previousElementSibling) {
        if (s.tagName === n.tagName) index++;
      }
      const id = n.id ? '#' + n.id : '';
      parts.push(n.tagName.toLowerCase() + id + ':' + index);
    }
    return parts.reverse().join('/');
  }

  function readStyle(style, ignore, only) {
    const out = {};
    const names = only || Array.from({ length: style.length }, (_, i) => style[i]);
    for (const name of names) {
      if (ignore.has(name)) continue;
      out[name] = style.getPropertyValue(name);
    }
    return out;
  }

  async function settle() {
    if (document.fonts && document.fonts.ready) await document.fonts.ready;
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  }

  async function snapshot(options) {
    const opts = options || {};
    const ignore = new Set(opts.ignore || DEFAULT_IGNORE);
    const only = opts.properties || null;
    await settle();
    const elements = {};
    const walk = (scope, prefix) => {
      for (const el of scope.querySelectorAll('*')) {
        // Shadow trees restart their paths at the shadow root; the host path keeps keys unique.
        const key = prefix + pathOf(el);
        elements[key] = readStyle(getComputedStyle(el), ignore, only);
        for (const pseudo of ['::before', '::after']) {
          const style = getComputedStyle(el, pseudo);
          const content = style.getPropertyValue('content');
          if (content && content !== 'none' && content !== 'normal') {
            elements[key + pseudo] = readStyle(style, ignore, only);
          }
        }
        if (el.shadowRoot && el.shadowRoot.mode === 'open') walk(el.shadowRoot, key + '::shadow/');
      }
    };
    walk(document, '');
    return {
      version: 1,
      url: location.href,
      viewport: { width: innerWidth, height: innerHeight, dpr: devicePixelRatio },
      colorScheme: matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
      reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
      elementCount: Object.keys(elements).length,
      elements,
    };
  }

  const PX = /^-?\d*\.?\d+px$/;
  function same(a, b, tolerance) {
    if (a === b) return true;
    if (PX.test(a) && PX.test(b)) return Math.abs(parseFloat(a) - parseFloat(b)) <= tolerance;
    return false;
  }

  function compare(before, after, options) {
    const tolerance = (options && options.tolerancePx) || 0; // exact by default
    // Custom properties the central tokens file defines site-wide (manifest.defined_properties).
    // After the rewrite they can appear in computed styles of elements that did not have them
    // before; that is expected, a changed value of a property both sides have is not.
    const introduced = new Set((options && options.introduced) || []);
    const report = {
      context: {
        sameViewport: JSON.stringify(before.viewport) === JSON.stringify(after.viewport),
        sameColorScheme: before.colorScheme === after.colorScheme,
        sameReducedMotion: before.reducedMotion === after.reducedMotion,
      },
      elementsBefore: before.elementCount,
      elementsAfter: after.elementCount,
      missingInAfter: [],
      extraInAfter: [],
      propertiesCompared: 0,
      differences: [],
    };
    for (const key of Object.keys(before.elements)) {
      const a = before.elements[key];
      const b = after.elements[key];
      if (!b) {
        report.missingInAfter.push(key);
        continue;
      }
      for (const name of new Set([...Object.keys(a), ...Object.keys(b)])) {
        if (a[name] === undefined && introduced.has(name)) {
          report.introducedProperties = (report.introducedProperties || 0) + 1;
          continue;
        }
        report.propertiesCompared++;
        if (!same(a[name], b[name], tolerance)) {
          report.differences.push({ element: key, property: name, before: a[name], after: b[name] });
        }
      }
    }
    for (const key of Object.keys(after.elements)) {
      if (!before.elements[key]) report.extraInAfter.push(key);
    }
    report.equal =
      report.differences.length === 0 &&
      report.missingInAfter.length === 0 &&
      report.extraInAfter.length === 0 &&
      report.context.sameViewport &&
      report.context.sameColorScheme &&
      report.context.sameReducedMotion;
    return report;
  }

  const api = { snapshot, compare, settle };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  root.mochiTokenEquality = api;

  // Command line: compare two saved snapshots.
  if (typeof require !== 'undefined' && typeof module !== 'undefined' && require.main === module) {
    if (process.argv.length < 4) {
      console.error('usage: computed-equality.js before.json after.json [tokens.manifest.json]');
      process.exit(2);
    }
    const fs = require('fs');
    const [a, b] = process.argv.slice(2, 4).map((f) => JSON.parse(fs.readFileSync(f, 'utf8')));
    // Optional third argument: tokens.manifest.json (its defined_properties are expected additions).
    const manifest = process.argv[4] ? JSON.parse(fs.readFileSync(process.argv[4], 'utf8')) : null;
    const report = compare(a, b, {
      tolerancePx: Number(process.env.TOLERANCE_PX || 0),
      introduced: manifest ? manifest.defined_properties : [],
    });
    console.log(JSON.stringify({ ...report, differences: report.differences.slice(0, 50) }, null, 2));
    process.exit(report.equal ? 0 : 1);
  }
})(typeof globalThis !== 'undefined' ? globalThis : this);
