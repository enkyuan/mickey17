async (limits) => {
  const nodes = [], styles = [], sheets = [], fonts = [], resources = [], issues = [];
  const styleIds = new Map(), seenSheets = new Set();
  const encoder = new TextEncoder();
  let bytes = 1024, ruleCount = 0, rootRuleCount = 0;
  const issue = (code, detail, node = null) => {
    if (issues.length < 32) issues.push({code, detail: Array.from(String(detail)).slice(0, 256).join(''), node});
  };
  const charge = value => {
    const size = encoder.encode(JSON.stringify(value)).length + 2;
    if (bytes + size > limits.bytes - 65536) { issue('byte_limit', 'Source evidence byte budget reached'); return false; }
    bytes += size;
    return true;
  };
  const bounded = value => {
    if (encoder.encode(value).length > limits.value_bytes) {
      issue('value_limit', 'A text, attribute, or CSS value exceeds the per-value budget');
      return false;
    }
    return true;
  };
  let timer;
  try {
    await Promise.race([document.fonts.ready, new Promise(resolve => { timer = setTimeout(resolve, 3000); })]);
  } finally { clearTimeout(timer); }
  if (document.fonts.status !== 'loaded') issue('fonts_pending', 'Font readiness deadline exceeded');

  const metadata = {title: document.title, base_url: document.baseURI,
    lang: document.documentElement?.lang || '', doctype: document.doctype?.name || null,
    canonical: document.querySelector('link[rel="canonical"]')?.getAttribute('href') || null,
    meta: []};
  for (const element of document.querySelectorAll('meta')) {
    const record = {name: element.getAttribute('name'), property: element.getAttribute('property'),
      http_equiv: element.getAttribute('http-equiv'), content: element.getAttribute('content') || ''};
    if (!bounded(record.content) || !charge(record)) break;
    metadata.meta.push(record);
    if (metadata.meta.length >= 256) { issue('metadata_limit', 'Metadata record limit reached'); break; }
  }
  if (![metadata.title, metadata.base_url, metadata.lang, metadata.canonical || ''].every(bounded) || !charge({...metadata, meta: []})) {
    throw new Error('Document metadata exceeds source evidence limits');
  }
  const intern = computed => {
    const style = {};
    if (computed.length > 2048) { issue('style_limit', 'Too many computed properties'); return null; }
    for (const name of computed) {
      const value = computed.getPropertyValue(name);
      if (!bounded(name) || !bounded(value)) return null;
      style[name] = value;
    }
    const key = JSON.stringify(style);
    if (styleIds.has(key)) return styleIds.get(key);
    if (styles.length >= limits.styles) { issue('style_limit', 'Distinct style budget reached'); return null; }
    if (!charge(style)) return null;
    const index = styles.length;
    styles.push(style); styleIds.set(key, index);
    return index;
  };
  const omit = new Set(['script', 'style', 'noscript', 'template', 'meta', 'link']);
  const stack = document.documentElement ? [[document.documentElement, null, 0]] : [];
  if (!stack.length) issue('missing_document', 'Document has no root element');
  while (stack.length) {
    const [node, parent, depth] = stack.pop();
    if (node.nodeType !== Node.ELEMENT_NODE && node.nodeType !== Node.TEXT_NODE) continue;
    if (node.nodeType === Node.ELEMENT_NODE && omit.has(node.localName.toLowerCase())) continue;
    if (nodes.length >= limits.nodes) { issue('node_limit', 'Node budget reached'); break; }
    if (depth > limits.depth) { issue('depth_limit', 'DOM depth budget reached'); break; }
    let record;
    if (node.nodeType === Node.TEXT_NODE) {
      if (!bounded(node.data)) break;
      record = {parent, kind: 'text', value: node.data};
    } else {
      if (node.localName.length > 128 || (node.namespaceURI || '').length > 1024) {
        issue('value_limit', 'Element name or namespace exceeds source limits'); break;
      }
      const attributes = [];
      if (node.attributes.length > 256) { issue('attribute_limit', 'Attribute count budget reached'); break; }
      for (const attr of node.attributes) {
        if (node.localName === 'input' && ['password', 'file'].includes(node.type) && attr.name === 'value' && attr.value) {
          issue('sensitive_value', 'Sensitive input value omitted', nodes.length); continue;
        }
        if (!bounded(attr.name) || !bounded(attr.value)) break;
        attributes.push({name: attr.name, namespace: attr.namespaceURI, value: attr.value});
      }
      if (attributes.length !== node.attributes.length && !['password', 'file'].includes(node.type)) break;
      const style = intern(getComputedStyle(node));
      if (style === null) break;
      const pseudo = {};
      for (const name of ['::before', '::after']) {
        const computed = getComputedStyle(node, name);
        if (!['none', 'normal'].includes(computed.content)) {
          const id = intern(computed);
          if (id !== null) pseudo[name] = id;
        }
      }
      const rect = node.getBoundingClientRect();
      let control = null;
      if (['input', 'textarea', 'select', 'option', 'details'].includes(node.localName)) {
        const sensitive = node.localName === 'input' && ['password', 'file'].includes(node.type);
        if (sensitive && node.value) issue('sensitive_value', 'Sensitive live control value omitted', nodes.length);
        control = {value: sensitive && node.value ? null : ('value' in node ? String(node.value) : null),
          checked: typeof node.checked === 'boolean' ? node.checked : null,
          selected: typeof node.selected === 'boolean' ? node.selected : null,
          open: typeof node.open === 'boolean' ? node.open : null};
        if (control.value !== null && !bounded(control.value)) break;
      }
      record = {parent, kind: 'element', tag: node.localName, namespace: node.namespaceURI || '',
        attributes, style, pseudo, control,
        bounds: {x: rect.x, y: rect.y, width: rect.width, height: rect.height}};
      if (node.shadowRoot) issue('shadow_document', 'Open shadow root contents are not captured', nodes.length);
      if (['iframe', 'frame', 'canvas', 'video'].includes(node.localName))
        issue('embedded_content', `${node.localName} internals are not captured`, nodes.length);
    }
    if (!charge(record)) break;
    const index = nodes.length; nodes.push(record);
    if (node.nodeType === Node.ELEMENT_NODE)
      for (let i = node.childNodes.length - 1; i >= 0; i--) stack.push([node.childNodes[i], index, depth + 1]);
  }

  const queue = [...document.styleSheets, ...document.adoptedStyleSheets].map(sheet => [sheet, 0]);
  for (const link of document.querySelectorAll('link[rel~="stylesheet"]'))
    if (!link.disabled && !link.sheet) issue('stylesheet_unavailable', link.href || 'Stylesheet has not loaded');
  for (const image of document.images)
    if (!image.complete || (image.currentSrc && !image.naturalWidth)) issue('image_unavailable', image.currentSrc || image.src);
  for (let cursor = 0; cursor < queue.length; cursor++) {
    const [sheet, importDepth] = queue[cursor];
    if (seenSheets.has(sheet)) continue;
    seenSheets.add(sheet);
    if (sheets.length >= 128) { issue('stylesheet_limit', 'Stylesheet count budget reached'); break; }
    const record = {href: sheet.href, base_url: sheet.href || document.baseURI,
      disabled: sheet.disabled, media: sheet.media?.mediaText || '', rules: [], readable: true};
    const index = sheets.length;
    if (!charge({...record, rules: []})) break;
    sheets.push(record);
    try {
      const rules = sheet.cssRules;
      for (const rule of rules) {
        if (rootRuleCount >= limits.rules) { issue('rule_limit', 'CSS rule count budget reached'); break; }
        const text = rule.cssText;
        if (!bounded(text) || !charge(text)) break;
        record.rules.push(text); rootRuleCount++;
      }
      // Preserve groups verbatim above; traverse once for font descriptors and imports.
      const pending = Array.from(rules, rule => [rule, 0]);
      while (pending.length) {
        const [rule, depth] = pending.pop();
        if (++ruleCount > limits.rules) { issue('rule_limit', 'CSS rule count budget reached'); break; }
        if (depth > limits.depth) { issue('depth_limit', 'CSS grouping depth budget reached'); break; }
        if (rule.type === CSSRule.FONT_FACE_RULE) {
          const descriptors = {};
          for (const name of rule.style) descriptors[name] = rule.style.getPropertyValue(name);
          const font = {stylesheet: index, descriptors};
          if (!Object.values(descriptors).every(bounded) || !charge(font)) break;
          fonts.push(font);
        }
        if (rule.styleSheet) {
          if (importDepth >= 32 || queue.length >= 256) issue('stylesheet_limit', 'CSS import budget reached');
          else queue.push([rule.styleSheet, importDepth + 1]);
        }
        if (rule.cssRules) for (const child of rule.cssRules) pending.push([child, depth + 1]);
      }
    } catch (_) {
      record.readable = false;
      issue('stylesheet_unavailable', sheet.href || 'Inaccessible stylesheet');
    }
    if (ruleCount > limits.rules) break;
  }
  for (const font of document.fonts)
    if (font.status === 'error') issue('font_unavailable', font.family);
  const seenResources = new Set();
  for (const resource of performance.getEntriesByType('resource')) {
    if (seenResources.has(resource.name)) continue;
    seenResources.add(resource.name);
    const record = {url: resource.name, initiator: resource.initiatorType,
      response_status: typeof resource.responseStatus === 'number' ? resource.responseStatus : null};
    if (record.response_status >= 400 && ['link', 'css', 'img'].includes(record.initiator))
      issue('resource_unavailable', resource.name);
    if (!bounded(record.url) || !charge(record)) break;
    if (resources.length >= 10000) { issue('resource_limit', 'Resource observation budget reached'); break; }
    resources.push(record);
  }
  const result = {schema_version: 1, url: location.href, metadata,
    viewport: {width: innerWidth, height: innerHeight}, color_scheme: matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
    state: 'initial', scope: 'light-dom-cssom-before-after',
    nodes, styles, stylesheets: sheets, fonts, resources, screenshot: null,
    complete: issues.length === 0, issues: issues.map(item => ({...item, node: item.node !== null && item.node < nodes.length ? item.node : null}))};
  const output = JSON.stringify(result);
  if (encoder.encode(output).length > limits.bytes) throw new Error('Source evidence exceeds final byte budget');
  return output;
}
