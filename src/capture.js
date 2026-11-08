(() => {
  const nodes = [], styles = [], styleIds = new Map();
  const attributes = ['id', 'class', 'role', 'type', 'name', 'href', 'src', 'alt', 'title',
    'aria-label', 'aria-labelledby', 'aria-describedby', 'aria-expanded', 'aria-pressed',
    'aria-checked', 'aria-haspopup', 'disabled', 'required', 'multiple', 'placeholder', 'for'];
  const properties = ['display', 'position', 'flex-direction', 'grid-template-columns', 'gap',
    'font-family', 'font-size', 'font-weight', 'line-height', 'color', 'background-color', 'border-radius'];
  const omit = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'LINK', 'META']);
  const stack = document.body ? [[document.body, null, 0]] : [];
  const encoder = new TextEncoder();
  const clip = (text, max) => {
    let prefix = text.slice(0, max);
    if (/[\uD800-\uDBFF]$/.test(prefix)) prefix = prefix.slice(0, -1);
    return prefix;
  };
  let truncated = false, bytes = 1024;
  while (stack.length) {
    const [element, parent, depth] = stack.pop();
    if (omit.has(element.localName.toUpperCase())) continue;
    if (nodes.length >= 20000 || depth > 256) { truncated = true; break; }
    const computed = getComputedStyle(element), style = {};
    for (const property of properties) {
      const value = computed.getPropertyValue(property);
      if (value.length > 1024) { truncated = true; break; }
      style[property] = value;
    }
    if (truncated) break;
    const styleKey = JSON.stringify(style);
    const freshStyle = !styleIds.has(styleKey);
    if (freshStyle && styles.length >= 4096) { truncated = true; break; }
    const attrs = {};
    for (const name of attributes) if (element.hasAttribute(name)) {
      const value = element.getAttribute(name);
      if (value.length > 1024) { truncated = true; break; }
      attrs[name] = value;
    }
    if (truncated || element.tagName.length > 128) { truncated = true; break; }
    const text = clip(Array.from(element.childNodes).filter(n => n.nodeType === Node.TEXT_NODE)
      .map(n => n.textContent).join(' ').replace(/\s+/g, ' ').trim(), 512);
    const textSlots = [];
    let position = 0;
    for (const child of element.childNodes) {
      if (child.nodeType === Node.ELEMENT_NODE && !omit.has(child.localName.toUpperCase())) position++;
      else if (child.nodeType === Node.TEXT_NODE && child.textContent.trim() && textSlots.at(-1) !== position) textSlots.push(position);
    }
    const record = {parent, tag: element.localName, namespace: element.namespaceURI || '', attributes: attrs, text, text_slots: textSlots,
      style: freshStyle ? styles.length : styleIds.get(styleKey)};
    const additional = encoder.encode(JSON.stringify(record)).length +
      (freshStyle ? encoder.encode(styleKey).length : 0);
    if (bytes + additional > 8 * 1024 * 1024 - 65536) { truncated = true; break; }
    bytes += additional;
    if (freshStyle) { styleIds.set(styleKey, styles.length); styles.push(style); }
    const index = nodes.length;
    nodes.push(record);
    for (let i = element.children.length - 1; i >= 0; i--) stack.push([element.children[i], index, depth + 1]);
  }
  return JSON.stringify({schema_version: 1, url: location.href,
    viewport: {width: innerWidth, height: innerHeight}, nodes, styles, truncated});
})()
