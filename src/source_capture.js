async (limits) => {
  window.__mochiCapturedNodes=[];
  const nodes = [], styles = [], sheets = [], fonts = [], resources = [], issues = [], surfaces = {}, animation_states = {};
  const styleIds = new Map(), seenSheets = new Set(), strings = [], stringIds = new Map();
  const internString = value => { if(stringIds.has(value)) return stringIds.get(value); const id=strings.length; if(!charge(value)) return null; strings.push(value); stringIds.set(value,id); return id; };
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

  const imageLoads=[...document.images].map(image=>{
    const previous=image.getAttribute('loading');image.loading='eager';
    const done=image.decode().catch(()=>{}).finally(()=>{if(previous===null)image.removeAttribute('loading');else image.setAttribute('loading',previous)});
    return done;
  });
  await Promise.race([Promise.all(imageLoads),new Promise(resolve=>setTimeout(resolve,5000))]);
  const animationDeadline=performance.now()+8000;
  for(let pass=0;pass<6 && performance.now()<animationDeadline;pass++) {
    const finite=document.getAnimations().filter(animation=>{
      const timing=animation.effect?.getComputedTiming();
      return ['running','pending'].includes(animation.playState) && timing && Number.isFinite(timing.endTime) && timing.endTime<=10000;
    });
    if(!finite.length)break;
    let timeout;
    try{await Promise.race([Promise.all(finite.map(animation=>animation.finished.catch(()=>{}))),new Promise(resolve=>{timeout=setTimeout(resolve,Math.max(0,animationDeadline-performance.now()))})])}
    finally{clearTimeout(timeout)}
    await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
  }
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
    const map = [];
    for(const [name,value] of Object.entries(style)) {const k=internString(name),v=internString(value);if(k===null||v===null)return null;map.push([k,v]);}
    if (!charge(map)) return null;
    const index = styles.length;
    styles.push(map); styleIds.set(key, index);
    return index;
  };
  const scopedCss = root => {
    const result=[];
    for(const sheet of [...(root.styleSheets || []),...(root.adoptedStyleSheets || [])]) {
      try { for(const rule of sheet.cssRules) {if(!bounded(rule.cssText))continue;result.push(rule.cssText);} }
      catch {issue('stylesheet_unavailable','Scoped stylesheet cannot be read');}
    }
    return result;
  };
  const omit = new Set(['script', 'style', 'noscript', 'template', 'meta', 'link']);
  const stack = document.documentElement ? [[document.documentElement, null, 0, 0, 0, false]] : [];
  if (!stack.length) issue('missing_document', 'Document has no root element');
  while (stack.length) {
    const [node, parent, depth, offsetX, offsetY, scopedChild] = stack.pop();
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
        bounds: {x: rect.x+offsetX, y: rect.y+offsetY, width: rect.width, height: rect.height}};
      const animated={};
      for(const animation of node.getAnimations?.()||[]) {
        if('animationName' in animation || 'transitionProperty' in animation || animation.effect?.target!==node || animation.effect?.pseudoElement) continue;
        for(const frame of animation.effect.getKeyframes()) for(const key of Object.keys(frame)) {
          if(['offset','computedOffset','easing','composite'].includes(key))continue;
          const property=key.replace(/[A-Z]/g,c=>'-'+c.toLowerCase());
          const value=getComputedStyle(node).getPropertyValue(property);
          if(value && bounded(value))animated[property]=value;
        }
      }
      if(Object.keys(animated).length && charge(animated))animation_states[nodes.length]=animated;
      const shadow = node.shadowRoot || window.__mochiShadowRoots?.get(node);
      if (shadow) {
        const css=scopedCss(shadow);
        surfaces[nodes.length]={kind:'shadow',mode:shadow.mode,css,base_url:node.baseURI,children:[],image:null,bounds:record.bounds};
      }
      if (['iframe','frame','canvas','video'].includes(node.localName)) {
        const hidden = getComputedStyle(node).display==='none' || (!rect.width && !rect.height);
        let child=null;
        if(['iframe','frame'].includes(node.localName)) {try{child=node.contentDocument;}catch{}}
        const computed=getComputedStyle(node);
        const left=(parseFloat(computed.borderLeftWidth)||0)+(parseFloat(computed.paddingLeft)||0),top=(parseFloat(computed.borderTopWidth)||0)+(parseFloat(computed.paddingTop)||0);
        const right=(parseFloat(computed.borderRightWidth)||0)+(parseFloat(computed.paddingRight)||0),bottom=(parseFloat(computed.borderBottomWidth)||0)+(parseFloat(computed.paddingBottom)||0);
        surfaces[nodes.length]={kind:hidden?'nonvisual':child?.documentElement?'frame':'raster',mode:'open',css:child?scopedCss(child):[],base_url:/^https?:/.test(child?.baseURI||'')?child.baseURI:node.baseURI,children:[],image:null,
          bounds:{x:rect.x+offsetX+left,y:rect.y+offsetY+top,width:Math.max(0,rect.width-left-right),height:Math.max(0,rect.height-top-bottom)}};
      }
    }
    if (!charge(record)) break;
    const index = nodes.length; nodes.push(record); window.__mochiCapturedNodes[index]=node;
    if(scopedChild && surfaces[parent]) surfaces[parent].children.push(index);
    if (node.nodeType === Node.ELEMENT_NODE) {
      const surface=surfaces[index];
      if(surface && !charge(surface)) break;
      const shadow=node.shadowRoot || window.__mochiShadowRoots?.get(node);
      let frame=null; if(surface?.kind==='frame') {try{frame=node.contentDocument.documentElement;}catch{}}
      const children=shadow?[...[...node.childNodes].map(n=>[n,false]),...[...shadow.childNodes].map(n=>[n,true])]:frame?[[frame,true]]:[...node.childNodes].map(n=>[n,false]);
      for(let i=children.length-1;i>=0;i--) stack.push([children[i][0],index,depth+1,frame?record.bounds.x+node.clientLeft:offsetX,frame?record.bounds.y+node.clientTop:offsetY,children[i][1]]);
    }
  }

  const queue = [...document.styleSheets, ...document.adoptedStyleSheets].map(sheet => [sheet, 0]);
  for (const link of document.querySelectorAll('link[rel~="stylesheet"]'))
    if (!link.disabled && !link.sheet) issue('stylesheet_unavailable', link.href || 'Stylesheet has not loaded');
  for (const image of document.images)
    if (image.getClientRects().length && (!image.complete || (image.currentSrc && !image.naturalWidth))) issue('image_unavailable', image.currentSrc || image.src);
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
      let rules;
      try { rules = sheet.cssRules; }
      catch(error) {
        const text = window.__mochiStylesheetResponses?.[sheet.href];
        if(typeof text!=='string' || /@import\b/i.test(text))throw error;
        const parsed = new CSSStyleSheet();parsed.replaceSync(text);rules=parsed.cssRules;
        record.text_source='devtools_loaded_stylesheet';
      }
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
  for(const collection of [surfaces,animation_states]) for(const id of Object.keys(collection)) if(Number(id)>=nodes.length) delete collection[id];
  const result = {schema_version: 1, url: location.href, metadata,
    viewport: {width: innerWidth, height: innerHeight}, color_scheme: matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
    state: 'initial', scope: 'light-dom-cssom-before-after',
    nodes, surfaces, animation_states, styles:{strings,maps:styles}, stylesheets: sheets, fonts, resources, screenshot: null,
    complete: issues.length === 0, issues: issues.map(item => ({...item, node: item.node !== null && item.node < nodes.length ? item.node : null}))};
  window.__mochiFramesFrozen=true;
  const output = JSON.stringify(result);
  if (encoder.encode(output).length > limits.bytes) throw new Error('Source evidence exceeds final byte budget');
  return output;
}
