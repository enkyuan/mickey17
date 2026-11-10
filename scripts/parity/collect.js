/* Mochi parity collector, schema 2. A superset of the hardened Natural collector.
 *
 * Evaluate this file in the page (Playwright evaluate, Browser pane javascript_tool,
 * or the sink-served loader described in docs/PARITY.md). It evaluates to the API
 * object and also installs it as globalThis.__mochiParity:
 *
 *   await __mochiParity.collect(options)   // one full-document capture (JSON-safe)
 *   __mochiParity.find(descriptor)         // locate the element a state refers to
 *   __mochiParity.describe(element)        // stable descriptor of an element
 *   __mochiParity.freezeAnimations(ms)     // pause CSS/WAAPI animations at ms
 *   __mochiParity.activeElement()          // focus descriptor (+ :focus-visible)
 *   __mochiParity.signature()              // cheap hash of moving/transparent boxes: stable == settled
 *
 * The collector only reads the page. It never clicks, scrolls, creates canvas
 * contexts or edits styles; freezeAnimations is a separate, explicit call.
 * Nothing here is a screenshot comparison: everything is a resolved DOM/CSS fact.
 *
 * Coverage that a DOM read cannot give is reported in `limits`, never skipped
 * silently: unreadable canvas pixels, cross-origin asset bytes, shadow roots, iframes.
 */
(() => {
  if (globalThis.__mochiParity && globalThis.__mochiParity.version === 2) return globalThis.__mochiParity;
  const r2 = n => Math.round(n * 100) / 100;
  const SKIP = new Set(['script', 'style', 'link', 'noscript', 'template', 'meta', 'title', 'base', 'head', 'br', 'wbr', 'source', 'track', 'param']);
  const MEDIA = new Set(['img', 'svg', 'canvas', 'video', 'audio', 'iframe', 'object', 'embed', 'picture']);
  const CONTROL = new Set(['button', 'a', 'input', 'select', 'textarea', 'summary', 'details', 'dialog', 'label', 'option', 'form', 'fieldset', 'legend']);
  const LANDMARK = new Set(['h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'nav', 'main', 'header', 'footer', 'section', 'article', 'aside', 'ul', 'ol', 'li', 'table', 'tr', 'td', 'th', 'figure', 'figcaption', 'blockquote', 'pre', 'code']);
  const IMPLICIT_ROLE = { a: 'link', button: 'button', h1: 'heading', h2: 'heading', h3: 'heading', h4: 'heading', h5: 'heading', h6: 'heading', nav: 'navigation', main: 'main', header: 'banner', footer: 'contentinfo', aside: 'complementary', ul: 'list', ol: 'list', li: 'listitem', img: 'img', input: 'textbox', select: 'combobox', textarea: 'textbox', dialog: 'dialog', form: 'form', article: 'article', section: 'region', table: 'table', summary: 'button', details: 'group', figure: 'figure', svg: 'graphics-document' };
  const STYLE_GROUPS = {
    layout: ['display', 'position', 'float', 'boxSizing', 'overflowX', 'overflowY', 'zIndex', 'flexDirection', 'flexWrap', 'flexGrow', 'flexShrink', 'flexBasis', 'alignItems', 'alignSelf', 'alignContent', 'justifyContent', 'justifySelf', 'justifyItems', 'rowGap', 'columnGap', 'gridTemplateColumns', 'gridTemplateRows', 'gridAutoFlow', 'gridColumnStart', 'gridColumnEnd', 'gridRowStart', 'gridRowEnd', 'aspectRatio', 'paddingTop', 'paddingRight', 'paddingBottom', 'paddingLeft', 'marginTop', 'marginRight', 'marginBottom', 'marginLeft', 'objectFit', 'objectPosition', 'order'],
    typography: ['fontFamily', 'fontSize', 'fontWeight', 'fontStyle', 'fontVariationSettings', 'fontFeatureSettings', 'fontStretch', 'fontKerning', 'letterSpacing', 'wordSpacing', 'lineHeight', 'textAlign', 'textTransform', 'textDecorationLine', 'textDecorationStyle', 'textDecorationColor', 'textIndent', 'textWrap', 'whiteSpace', 'wordBreak', 'overflowWrap', 'textOverflow', 'textShadow', 'verticalAlign', 'hyphens', 'direction', 'writingMode', 'webkitTextStrokeWidth', 'webkitTextStrokeColor', 'webkitTextFillColor', 'textDecorationThickness', 'textUnderlineOffset', 'fontVariantCaps', 'fontVariantNumeric', 'fontVariantLigatures', 'fontVariantEastAsian', 'fontOpticalSizing', 'listStyleType', 'listStylePosition', 'listStyleImage'],
    styling: ['color', 'backgroundColor', 'backgroundImage', 'backgroundSize', 'backgroundPosition', 'backgroundRepeat', 'backgroundClip', 'backgroundBlendMode', 'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth', 'borderTopColor', 'borderRightColor', 'borderBottomColor', 'borderLeftColor', 'borderTopStyle', 'borderRightStyle', 'borderBottomStyle', 'borderLeftStyle', 'borderTopLeftRadius', 'borderTopRightRadius', 'borderBottomRightRadius', 'borderBottomLeftRadius', 'boxShadow', 'outlineWidth', 'outlineStyle', 'outlineColor', 'outlineOffset', 'opacity', 'visibility', 'transform', 'transformOrigin', 'clipPath', 'mixBlendMode', 'filter', 'backdropFilter', 'maskImage', 'maskSize', 'maskPosition', 'maskRepeat', 'fill', 'stroke', 'strokeWidth', 'fillOpacity', 'strokeOpacity', 'isolation', 'colorScheme', 'accentColor', 'caretColor', 'backgroundAttachment', 'backgroundOrigin', 'borderImageSource', 'borderImageSlice', 'translate', 'rotate', 'scale', 'perspective', 'transformStyle', 'contentVisibility', 'scrollbarWidth', 'scrollbarColor'],
    motion: ['transitionProperty', 'transitionDuration', 'transitionTimingFunction', 'transitionDelay', 'animationName', 'animationDuration', 'animationTimingFunction', 'animationDelay', 'animationIterationCount', 'animationDirection', 'animationFillMode', 'animationPlayState', 'willChange', 'scrollBehavior', 'scrollSnapType', 'scrollSnapAlign', 'overscrollBehavior', 'touchAction'],
    interaction: ['cursor', 'pointerEvents', 'userSelect'],
  };
  const TEXT_ONLY = new Set(STYLE_GROUPS.typography);
  const ALL_STYLE = [];
  for (const [g, list] of Object.entries(STYLE_GROUPS)) for (const p of list) ALL_STYLE.push([g, p]);

  const norm = s => (s || '').replace(/\s+/g, ' ').trim();
  // cyrb53: fast non-cryptographic 53-bit hash, enough to detect changes in long strings.
  const hash = (str, seed = 0) => {
    let h1 = 0xdeadbeef ^ seed, h2 = 0x41c6ce57 ^ seed;
    for (let i = 0; i < str.length; i++) { const ch = str.charCodeAt(i); h1 = Math.imul(h1 ^ ch, 2654435761); h2 = Math.imul(h2 ^ ch, 1597334677); }
    h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
    h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
    return (4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(16);
  };
  const hex = buf => [...new Uint8Array(buf)].map(b => b.toString(16).padStart(2, '0')).join('');
  // textContent minus <script>/<style>/<noscript>/<template> text, which the captured DOM snapshot does not contain either
  const textOfNode = n => n.nodeType === 3 ? n.textContent : n.nodeType === 1 && !/^(script|style|noscript|template)$/i.test(n.tagName) ? [...n.childNodes].map(textOfNode).join('') : '';
  const pathOf = e => { if (e === document.documentElement) return 'html'; const p = []; while (e && e.nodeType === 1 && e !== document.documentElement) { let s = e.tagName.toLowerCase(); if (e.id) s += '#' + e.id; p.unshift(s); e = e.parentElement; } return p.join('>'); };
  const ownText = e => norm([...e.childNodes].filter(n => n.nodeType === 3).map(n => n.textContent).join(''));
  const pageUrl = u => { // same-origin URLs become path+search; other origins stay absolute
    if (!u) return '';
    if (u.startsWith('data:')) return 'data:' + hash(u);
    try { const x = new URL(u, location.href); return x.origin === location.origin ? x.pathname + x.search : x.href; } catch { return u; }
  };
  const roleOf = e => e.getAttribute('role') || IMPLICIT_ROLE[e.tagName.toLowerCase()] || '';
  const nameOf = e => {
    const al = e.getAttribute('aria-label');
    if (al) return norm(al);
    const lb = e.getAttribute('aria-labelledby');
    if (lb) { const t = lb.split(/\s+/).map(id => document.getElementById(id)).filter(Boolean).map(n => norm(textOfNode(n))).join(' '); if (t) return t; }
    const alt = e.getAttribute('alt'); if (alt) return norm(alt);
    const title = e.getAttribute('title'); if (title) return norm(title);
    return norm(textOfNode(e)).slice(0, 120);
  };
  const DATA_RE = /^data-(base-ui|slot|state|open|closed|popup|checked|unchecked|selected|active|inactive|orientation|disabled|starting|ending|side|align|anchor|pressed|highlighted|expanded|panel|trigger|index|value|mochi|type|variant|size)/;
  // One predicate for describe() and find(): same tag, same role, same accessible name.
  // The nth match is returned only if it exists; a descriptor never resolves to a different element.
  const sameKind = (n, d) => n.tagName.toLowerCase() === d.tag && roleOf(n) === d.role && nameOf(n) === d.name;
  const describe = e => {
    if (!e || e.nodeType !== 1) return null;
    const d = { tag: e.tagName.toLowerCase(), role: roleOf(e), name: nameOf(e), text: norm(textOfNode(e)).slice(0, 80), path: pathOf(e) };
    d.nth = [...document.querySelectorAll(d.tag)].filter(n => sameKind(n, d)).indexOf(e);
    return d;
  };
  const find = d => {
    if (!d || !Number.isInteger(d.nth) || d.nth < 0) return null;
    return [...document.querySelectorAll(d.tag)].filter(n => sameKind(n, d))[d.nth] || null;
  };
  const activeElement = () => {
    const a = document.activeElement;
    if (!a || a === document.body || a === document.documentElement) return { tag: 'body', focusVisible: false };
    let fv = false; try { fv = a.matches(':focus-visible'); } catch { /* old engines */ }
    const cs = getComputedStyle(a);
    return { ...describe(a), focusVisible: fv, outline: `${cs.outlineStyle} ${cs.outlineWidth} ${cs.outlineColor}`, boxShadow: cs.boxShadow, tabindex: a.getAttribute('tabindex') };
  };
  // Visual signature used to decide when the page stopped moving: every element's box, the
  // opacity/transform/clip/filter of transformed ones, colors of transitioning ones, text length,
  // scroll position and the number of running finite animations. Not a proof of rest: an animation
  // that changes nothing recorded here (and whose delay exceeds the polling window) looks settled.
  const signature = () => {
    const parts = [Math.round(scrollY), document.documentElement.scrollHeight, (document.body.innerText || '').length];
    parts.push(document.getAnimations().filter(a => a.playState === 'running' && Number.isFinite(a.effect && a.effect.getComputedTiming ? a.effect.getComputedTiming().endTime : Infinity)).length);
    for (const e of document.body.querySelectorAll('*')) {
      const b = e.getBoundingClientRect();
      parts.push(Math.round(b.left * 10), Math.round(b.top * 10), Math.round(b.width * 10), Math.round(b.height * 10));
      const c = getComputedStyle(e);
      if (c.opacity !== '1' || c.transform !== 'none' || c.clipPath !== 'none' || c.filter !== 'none') parts.push(c.opacity, c.transform, c.clipPath, c.filter);
      if (c.transitionDuration !== '0s') parts.push(c.color, c.backgroundColor);
    }
    return hash(parts.join('|'));
  };
  const freezeAnimations = ms => {
    const out = { paused: 0, infinite: 0, skipped: 0 };
    for (const a of document.getAnimations()) {
      try {
        const t = a.effect && a.effect.getComputedTiming ? a.effect.getComputedTiming() : null;
        a.pause();
        if (t && Number.isFinite(t.endTime)) { a.currentTime = Math.min(ms, t.endTime); out.paused++; }
        else { a.currentTime = ms; out.infinite++; }
      } catch { out.skipped++; }
    }
    return out;
  };

  const collect = async (options = {}) => {
    const sx = scrollX, sy = scrollY;
    const limits = [];
    const styleTable = [], styleIndex = new Map();
    const assetJobs = new Map(); // url -> sha256/hash promise
    const hashUrl = url => {
      if (!url) return Promise.resolve(null);
      if (assetJobs.has(url)) return assetJobs.get(url);
      const job = (async () => {
        try {
          if (options.hashAssets === false) return null;
          const ctl = new AbortController(); const timer = setTimeout(() => ctl.abort(), 8000);
          const res = await fetch(url, { cache: 'force-cache', signal: ctl.signal }); clearTimeout(timer);
          if (!res.ok) return { error: 'http ' + res.status };
          const buf = await res.arrayBuffer();
          if (globalThis.crypto && crypto.subtle) return { sha: hex(await crypto.subtle.digest('SHA-256', buf)), bytes: buf.byteLength };
          return { sha: 'h' + hash(String.fromCharCode(...new Uint8Array(buf).slice(0, 65536))), bytes: buf.byteLength };
        } catch (e) { return { error: String(e && e.message || e).slice(0, 60) }; }
      })();
      assetJobs.set(url, job); return job;
    };
    const urlsIn = v => [...(v || '').matchAll(/url\((?:"([^"]*)"|'([^']*)'|([^)]*))\)/g)].map(m => m[1] ?? m[2] ?? m[3]).filter(Boolean);
    const abs = u => { if (u.startsWith('data:')) return u; try { return new URL(u, location.href).href; } catch { return u; } };

    const effCache = new Map();
    const effective = e => { // opacity product and clip/filter/mask/blend ancestors, hidden state
      if (effCache.has(e)) return effCache.get(e);
      const p = e.parentElement ? effective(e.parentElement) : { op: 1, fx: [], hid: false, inert: false };
      const c = getComputedStyle(e), fx = p.fx.slice();
      if (c.clipPath !== 'none') fx.push('clip:' + c.clipPath);
      if (c.filter !== 'none') fx.push('filter:' + c.filter);
      if (c.maskImage !== 'none' && c.maskImage) fx.push('mask');
      if (c.mixBlendMode !== 'normal') fx.push('blend:' + c.mixBlendMode);
      const v = { op: p.op * parseFloat(c.opacity), fx, hid: p.hid || c.visibility === 'hidden' || c.display === 'none', inert: p.inert || e.hasAttribute('inert') };
      effCache.set(e, v); return v;
    };

    const els = [];
    const pending = [];
    for (const e of [document.documentElement, document.body, ...document.body.querySelectorAll('*')]) {
      const tag = e.tagName.toLowerCase();
      if (SKIP.has(tag)) continue;
      if (e.closest('svg') && tag !== 'svg') continue;
      const cs = getComputedStyle(e), b = e.getBoundingClientRect();
      const own = ownText(e);
      const role = roleOf(e);
      const bw = parseFloat(cs.borderTopWidth) + parseFloat(cs.borderRightWidth) + parseFloat(cs.borderBottomWidth) + parseFloat(cs.borderLeftWidth);
      const paints = cs.backgroundColor !== 'rgba(0, 0, 0, 0)' || cs.backgroundImage !== 'none' || bw > 0 || cs.boxShadow !== 'none' || cs.outlineStyle !== 'none' && parseFloat(cs.outlineWidth) > 0;
      const effects = parseFloat(cs.opacity) !== 1 || cs.clipPath !== 'none' || cs.filter !== 'none' || cs.transform !== 'none' || cs.maskImage !== 'none' || cs.mixBlendMode !== 'normal' || cs.backdropFilter !== 'none';
      const interactive = CONTROL.has(tag) || e.hasAttribute('role') || e.hasAttribute('tabindex') || e.hasAttribute('contenteditable') || e.hasAttribute('aria-expanded') || e.hasAttribute('aria-controls');
      const dataAttrs = [...e.attributes].filter(a => DATA_RE.test(a.name));
      const root = e === document.documentElement || e === document.body;
      const include = root || own || MEDIA.has(tag) || interactive || LANDMARK.has(tag) || paints || effects; // data-* alone never adds an element: a lifted wrapper must not become an 'extra element'
      if (!include) continue;
      if (b.width === 0 && b.height === 0 && !own && !MEDIA.has(tag) && !interactive && !root) continue;
      const fullName = nameOf(e);
      if (own.length > 600) limits.push(`own text truncated to 600 chars (hash kept): ${pathOf(e)}`);
      const o = { tag, role, roleExplicit: e.hasAttribute('role'), textHash: own.length > 600 ? hash(own) : undefined, nameHash: fullName.length > 120 ? hash(fullName) : undefined, name: fullName.slice(0, 120), path: pathOf(e), cls: (e.getAttribute('class') || '').slice(0, 100), text: own.slice(0, 600), x: r2(b.left + sx), y: r2(b.top + sy), w: r2(b.width), h: r2(b.height) };
      // resolved style, deduplicated through a table
      const st = {};
      for (const [g, p] of ALL_STYLE) { if (TEXT_ONLY.has(p) && !own && !interactive) continue; st[p] = cs[p]; }
      const bgUrls = urlsIn(cs.backgroundImage), maskUrls = urlsIn(cs.maskImage);
      let key = JSON.stringify(st);
      if (bgUrls.length || maskUrls.length) { st.backgroundImage = cs.backgroundImage.replace(/url\([^)]*\)/g, 'url(#)'); st.maskImage = (cs.maskImage || '').replace(/url\([^)]*\)/g, 'url(#)'); key = JSON.stringify(st); }
      if (!styleIndex.has(key)) { styleIndex.set(key, styleTable.length); styleTable.push(st); }
      o.sty = styleIndex.get(key);
      const ef = effective(e);
      o.effOpacity = Math.round(ef.op * 1000) / 1000; o.effFx = ef.fx.join('|'); o.effHidden = ef.hid; o.inert = ef.inert;
      // ARIA and data attributes (all aria-*; data-* limited to component state/part names)
      const aria = {};
      for (const a of e.attributes) if (a.name.startsWith('aria-') && a.name !== 'aria-labelledby' && a.name !== 'aria-describedby' && a.name !== 'aria-controls') aria[a.name] = a.value;
      if (Object.keys(aria).length) o.aria = aria;
      if (e.hasAttribute('aria-controls') || e.hasAttribute('aria-labelledby') || e.hasAttribute('aria-describedby')) o.ariaRefs = ['aria-controls', 'aria-labelledby', 'aria-describedby'].filter(n => e.hasAttribute(n)).join(',');
      if (dataAttrs.length) { o.data = Object.fromEntries(dataAttrs.map(a => [a.name, a.value.slice(0, 60)]).sort()); if (dataAttrs.some(a => a.value.length > 60)) limits.push(`data-* value truncated to 60 chars: ${o.path}`); }
      if (e.hasAttribute('tabindex')) o.tabindex = e.getAttribute('tabindex');
      if (tag === 'a') { o.href = e.getAttribute('href'); o.hrefPath = pageUrl(e.href); o.target = e.getAttribute('target') || ''; o.rel = e.getAttribute('rel') || ''; o.download = e.hasAttribute('download'); }
      if (tag === 'button' || tag === 'input') { o.type = e.getAttribute('type') || (tag === 'button' ? 'submit' : 'text'); o.disabled = e.disabled === true; }
      if (tag === 'input' || tag === 'textarea' || tag === 'select') { o.placeholder = e.getAttribute('placeholder') || ''; o.required = e.required === true; o.checked = e.checked === true; if (e.type !== 'password') { o.value = String(e.value || '').slice(0, 80); if (String(e.value || '').length > 80) limits.push(`input value truncated to 80 chars: ${o.path}`); } }
      if (tag === 'details') o.open = e.open;
      if (tag === 'dialog') { o.open = e.open; o.modal = e.matches(':modal'); }
      if (tag === 'img') {
        o.src = pageUrl(e.currentSrc || e.src); o.srcsetPresent = e.hasAttribute('srcset'); o.alt = e.getAttribute('alt'); o.nat = e.naturalWidth + 'x' + e.naturalHeight; o.complete = e.complete && e.naturalWidth > 0; o.loading = e.getAttribute('loading') || '';
        pending.push([o, 'srcSha', abs(e.currentSrc || e.src)]);
      }
      if (tag === 'video' || tag === 'audio') { o.src = pageUrl(e.currentSrc || e.src); o.poster = pageUrl(e.poster || ''); o.paused = e.paused; o.currentTime = r2(e.currentTime); o.muted = e.muted; o.loop = e.loop; o.nat = (e.videoWidth || 0) + 'x' + (e.videoHeight || 0); }
      if (tag === 'iframe') { o.src = pageUrl(e.src); limits.push('iframe content not collected: ' + o.path); }
      if (e.shadowRoot) limits.push('open shadow root not collected: ' + o.path);
      if (tag === 'svg') {
        o.vb = e.getAttribute('viewBox') || ''; o.par = e.getAttribute('preserveAspectRatio') || '';
        o.svgAttrW = e.getAttribute('width') || ''; o.svgAttrH = e.getAttribute('height') || '';
        const shapes = [...e.querySelectorAll('*')];
        o.svgCount = shapes.length;
        const idMap = new Map(); const nid = v => v.replace(/#([\w:.-]+)/g, (_, id) => { if (!idMap.has(id)) idMap.set(id, 'i' + idMap.size); return '#' + idMap.get(id); });
        const ser = n => n.nodeType === 3 ? norm(n.textContent) : '<' + n.tagName.toLowerCase() + [...n.attributes].filter(a => a.name !== 'class' && a.name !== 'id').map(a => a.name + '=' + nid(a.value)).sort().join(' ') + '>' + [...n.childNodes].map(ser).join('') + '</>';
        o.svgHash = hash([...e.childNodes].map(ser).join(''));
        const pathNodes = shapes.filter(n => n.tagName.toLowerCase() === 'path');
        if (pathNodes.length > 400) limits.push(`svg path list truncated to 400 of ${pathNodes.length}: ${o.path}`);
        if (shapes.length > 120) limits.push(`svg paint hash covers the first 120 of ${shapes.length} shapes: ${o.path}`);
        o.paths = pathNodes.slice(0, 400).map(n => hash(n.getAttribute('d') || '') + ':' + (n.getAttribute('d') || '').length);
        o.svgPaint = hash(shapes.slice(0, 120).map(n => { const c = getComputedStyle(n); return [c.fill, c.stroke, c.strokeWidth, c.fillRule, c.fillOpacity, c.strokeOpacity, c.display, c.opacity, c.strokeDasharray, c.strokeDashoffset].join('/'); }).join(';'));
        o.svgAnimated = shapes.some(n => /^(animate|animateTransform|animateMotion|set)$/.test(n.tagName));
      }
      if (tag === 'canvas') {
        o.cw = e.width; o.ch = e.height;
        // Reading pixels never creates a context. WebGL canvases without preserveDrawingBuffer
        // read back blank outside their frame callback: that is recorded, not hidden.
        try {
          const url = e.toDataURL('image/png');
          o.pixelHash = hash(url); o.pixelBytes = url.length;
          const blank = (() => { try { const c2 = document.createElement('canvas'); c2.width = e.width; c2.height = e.height; return url === c2.toDataURL('image/png'); } catch { return null; } })();
          o.pixelBlank = blank;
          if (blank) limits.push('canvas pixels blank (WebGL without preserveDrawingBuffer, not yet drawn, or truly empty): ' + o.path);
        } catch (err) { o.pixelHash = null; o.pixelError = String(err && err.message || err).slice(0, 60); limits.push('canvas pixels unreadable: ' + o.path); }
      }
      for (const u of bgUrls) pending.push([o, 'bgSha', abs(u), true]);
      for (const u of maskUrls) pending.push([o, 'maskSha', abs(u), true]);
      els.push(o);
    }
    // pseudo-elements (content and box)
    const pseudos = [];
    for (const e of [document.documentElement, document.body, ...document.body.querySelectorAll('*')]) {
      if (e.closest('svg')) continue;
      for (const pe of ['::before', '::after', '::marker']) {
        let c; try { c = getComputedStyle(e, pe); } catch { continue; }
        if (!c || c.content === 'none' || c.content === 'normal' || c.display === 'none') continue;
        const b = e.getBoundingClientRect(), st = {};
        for (const p of ['content', 'display', 'position', 'width', 'height', 'color', 'backgroundColor', 'fontFamily', 'fontSize', 'fontWeight', 'opacity', 'transform', 'maskImage', 'borderTopWidth', 'borderRadius', 'boxShadow', 'top', 'left', 'right', 'bottom']) st[p] = c[p];
        st.backgroundImage = c.backgroundImage.replace(/url\([^)]*\)/g, 'url(#)');
        st.maskImage = (c.maskImage || '').replace(/url\([^)]*\)/g, 'url(#)');
        const o = { tag: pe, host: pathOf(e), pcontent: c.content, x: r2(b.left + sx), y: r2(b.top + sy), w: r2(parseFloat(c.width) || b.width), h: r2(parseFloat(c.height) || b.height), pst: st };
        for (const u of urlsIn(c.backgroundImage)) pending.push([o, 'bgSha', abs(u), true]);
        for (const u of urlsIn(c.maskImage)) pending.push([o, 'maskSha', abs(u), true]);
        pseudos.push(o);
      }
    }
    if (document.querySelector('[placeholder]')) limits.push('::placeholder styles are not collected');
    // asset bytes (same-origin or CORS-enabled); failures are recorded, not hidden
    const results = await Promise.all(pending.map(async ([o, key, url, multi]) => [o, key, multi, await hashUrl(url)]));
    let hashed = 0, unhashed = 0;
    for (const [o, key, multi, r] of results) {
      const v = r && r.sha ? r.sha : null;
      if (v) hashed++; else { unhashed++; if (r && r.error) o.assetError = r.error; }
      if (multi) { (o[key] ||= []).push(v || 'unhashed'); } else o[key] = v || 'unhashed';
    }
    if (unhashed) limits.push(`${unhashed} asset byte hashes unavailable (cross-origin without CORS, 404 or hashing off)`);

    // page-level facts
    const m = document.querySelector('main');
    const heads = [...document.querySelectorAll('h1,h2,h3,h4,h5,h6')].map(h => h.tagName.toLowerCase() + ':' + norm(h.innerText || h.textContent));
    const fontsDeclared = [...document.fonts].map(f => `${f.family.replace(/["']/g, '')}|${f.weight}|${f.style}|${f.stretch}|${hash(f.unicodeRange || '')}`).sort();
    const fontsLoaded = [...document.fonts].filter(f => f.status === 'loaded').map(f => `${f.family.replace(/["']/g, '')}|${f.weight}|${f.style}|${f.stretch}`).sort();
    const anim = document.getAnimations().map(a => { const t = a.effect && a.effect.getComputedTiming ? a.effect.getComputedTiming() : {}; const tg = a.effect && a.effect.target; return [a.animationName || a.transitionProperty || a.id || 'waapi', tg ? pathOf(tg).split('>').slice(-2).join('>') : '', a.playState, t.duration, t.iterations].join('|'); }).sort();
    const bodyCs = getComputedStyle(document.body), htmlCs = getComputedStyle(document.documentElement);
    const sheetCount = document.styleSheets.length;
    const facts = {
      'doc.title': document.title, 'doc.lang': document.documentElement.lang || '',
      'doc.dir': document.documentElement.dir || '',
      'meta.viewport': (document.querySelector('meta[name=viewport]') || {}).content || '', 'meta.description': (document.querySelector('meta[name=description]') || {}).content || '',
      'meta.themeColor': (document.querySelector('meta[name=theme-color]') || {}).content || '',
      'canonical': pageUrl((document.querySelector('link[rel=canonical]') || {}).href || ''),
      'scroll.docHeight': document.documentElement.scrollHeight, 'scroll.docWidth': document.documentElement.scrollWidth,
      'scroll.overflowX': document.documentElement.scrollWidth > innerWidth,
      'scroll.lock.bodyOverflow': bodyCs.overflow, 'scroll.lock.htmlOverflow': htmlCs.overflow, 'scroll.lock.bodyPaddingRight': bodyCs.paddingRight, 'scroll.lock.bodyPosition': bodyCs.position,
      'main.height': m ? r2(m.getBoundingClientRect().height) : null, 'main.width': m ? r2(m.getBoundingClientRect().width) : null,
      'count.img': document.images.length, 'count.svg': document.querySelectorAll('svg').length, 'count.canvas': document.querySelectorAll('canvas').length, 'count.video': document.querySelectorAll('video').length,
      'count.button': document.querySelectorAll('button,[role=button]').length, 'count.link': document.querySelectorAll('a').length, 'count.input': document.querySelectorAll('input,select,textarea').length,
      'count.inert': document.querySelectorAll('[inert]').length, 'count.dialogOpen': document.querySelectorAll('dialog[open],[role=dialog],[role=alertdialog]').length, 'count.ariaHidden': document.querySelectorAll('[aria-hidden=true]').length,
      'headings': heads, 'page.text': norm(m ? m.innerText || '' : document.body.innerText || ''), 'fonts.declared': fontsDeclared, 'info.fonts.loaded': fontsLoaded, 'animations.running': anim,
      'landmarks': [...document.querySelectorAll('header,nav,main,footer,aside,[role=banner],[role=navigation],[role=main],[role=contentinfo]')].map(n => roleOf(n)),
    };
    return {
      schema: 2, version: 2, collectedAt: new Date().toISOString(),
      context: { url: location.pathname + location.search, vw: innerWidth, vh: innerHeight, dpr: devicePixelRatio, scrollX: sx, scrollY: sy, reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches, colorScheme: matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light', hover: matchMedia('(hover: hover)').matches, pointer: matchMedia('(pointer: fine)').matches ? 'fine' : 'coarse', stylesheets: sheetCount, ua: navigator.userAgent },
      active: activeElement(), facts, styleTable, els, pseudos, limits: [...new Set(limits)], assets: { hashed, unhashed },
    };
  };
  const api = { version: 2, collect, find, describe, activeElement, freezeAnimations, signature, hash, STYLE_GROUPS };
  globalThis.__mochiParity = api;
  return api;
})()
