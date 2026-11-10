// Calibration controls injected into the candidate page after it has settled.
// `expect: 'detect'` mutations MUST turn a matching pair into a reported difference;
// a miss fails calibration and is retained. `expect: 'miss'` entries document blind
// spots of a given state kind: they prove a state class has teeth by being invisible
// to the others (a hover-only rule is invisible without a hover state).
export interface Mutation {
  name: string;
  dimension: string;
  css?: string;
  js?: string;
  expect: 'detect' | 'miss';
  /** State kinds in which the expectation holds (see StateSpec.tags). */
  scope: string[];
  note?: string;
  /** The reference must contain this (a tag, or `aria:<attribute>`) or the mutation is reported not applicable. */
  requires?: string;
  /** Apply before the state's interaction steps (needed to break a handler the steps then exercise). */
  before?: boolean;
  /** Take viewport-sliced screenshots (canvas/video hidden) before and after: an independent oracle for "nothing visible changed". */
  pixelProof?: boolean;
}
const ANY = ['initial'];
export const MUTATIONS: Mutation[] = [
  { name: 'opacity', dimension: 'styling', css: 'h1,h2,h3{opacity:0 !important}', expect: 'detect', scope: ANY },
  { name: 'wrapper-opacity', dimension: 'styling', css: 'main > *:first-child, body > div:first-child > *:first-child{opacity:.5 !important}', expect: 'detect', scope: ANY },
  { name: 'font-variation', dimension: 'typography', css: 'h1,h2{font-variation-settings:"wght" 700 !important}', expect: 'detect', scope: ANY },
  { name: 'letter-spacing', dimension: 'typography', css: 'p{letter-spacing:0.05em !important}', expect: 'detect', scope: ANY },
  { name: 'text-align', dimension: 'typography', css: 'p{text-align:center !important}', expect: 'detect', scope: ANY },
  { name: 'image-src', requires: 'img', dimension: 'assets', js: `const i=[...document.images].find(x=>x.naturalWidth>40);if(i){i.removeAttribute('srcset');i.src="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'/>"}`, expect: 'detect', scope: ANY },
  { name: 'background-color', dimension: 'styling', css: 'section,article,main > div{background-color:#eee !important}', expect: 'detect', scope: ANY },
  { name: 'background-image', dimension: 'assets', css: 'main *:nth-child(3), section{background-image:linear-gradient(red,blue) !important}', expect: 'detect', scope: ANY },
  { name: 'border', dimension: 'styling', css: 'section,article{border:3px solid red !important}', expect: 'detect', scope: ANY },
  { name: 'radius', dimension: 'styling', css: 'section,article,img{border-radius:20px !important}', expect: 'detect', scope: ANY },
  { name: 'shadow', dimension: 'styling', css: 'section,article,button{box-shadow:0 0 9px #000 !important}', expect: 'detect', scope: ANY },
  { name: 'text-shadow', dimension: 'typography', css: 'p,h1,h2{text-shadow:0 0 3px red !important}', expect: 'detect', scope: ANY },
  { name: 'injected-text', dimension: 'content', js: `const d=document.createElement('div');d.textContent='extra injected';(document.querySelector('main')||document.body).appendChild(d)`, expect: 'detect', scope: ANY },
  { name: 'late-text-edit', dimension: 'content', js: `const p=[...document.querySelectorAll('p,h1,h2,h3,li,span')].find(x=>x.firstChild&&x.firstChild.nodeType===3&&x.firstChild.textContent.trim().length>20);if(p)p.firstChild.textContent=p.firstChild.textContent.replace(/e/,'E')`, expect: 'detect', scope: ANY },
  { name: 'removed-section', dimension: 'content', js: `const s=[...document.querySelectorAll('main > *, main section, body > div > section')].filter(x=>x.getBoundingClientRect().height>100)[1];if(s)s.remove()`, expect: 'detect', scope: ANY },
  { name: 'hidden-menu', dimension: 'behavior', css: 'nav,[role=menu],[role=dialog],[role=tablist],[aria-haspopup],button[aria-expanded]{visibility:hidden !important}', expect: 'detect', scope: ANY },
  { name: 'hide-svg', requires: 'svg', dimension: 'graphics', css: 'svg{display:none !important}', expect: 'detect', scope: ANY },
  { name: 'svg-path', requires: 'svg', dimension: 'graphics', js: `const p=document.querySelector('svg path[d]');if(p)p.setAttribute('d',p.getAttribute('d')+' M0 0L1 1')`, expect: 'detect', scope: ANY },
  { name: 'svg-viewbox', requires: 'svg', dimension: 'graphics', js: `const s=document.querySelector('svg[viewBox]');if(s)s.setAttribute('viewBox','0 0 10 10')`, expect: 'detect', scope: ANY },
  { name: 'svg-fill', requires: 'svg', dimension: 'graphics', css: 'svg path{fill:red !important}', expect: 'detect', scope: ANY },
  { name: 'canvas-size', requires: 'canvas', dimension: 'graphics', js: `const c=document.querySelector('canvas');if(c)c.width=c.width+7`, expect: 'detect', scope: ANY, note: 'Only on routes with a canvas; recorded as not-applicable elsewhere.' },
  { name: 'aria-label', requires: 'aria:aria-label', dimension: 'behavior', js: `const e=document.querySelector('[aria-label]');if(e)e.setAttribute('aria-label','X')`, expect: 'detect', scope: ANY },
  { name: 'aria-hidden', dimension: 'behavior', js: `const e=document.querySelectorAll('img,a,button')[3];if(e)e.setAttribute('aria-hidden','true')`, expect: 'detect', scope: ANY },
  { name: 'aria-expanded', requires: 'aria:aria-expanded', dimension: 'behavior', js: `const e=document.querySelector('[aria-expanded]');if(e)e.setAttribute('aria-expanded',e.getAttribute('aria-expanded')==='true'?'false':'true')`, expect: 'detect', scope: ANY },
  { name: 'link-href', dimension: 'routing', js: `const a=[...document.querySelectorAll('a[href]')].find(x=>x.getAttribute('href').startsWith('/'));if(a)a.setAttribute('href','/wrong')`, expect: 'detect', scope: ANY },
  { name: 'heading-tag', dimension: 'semantics', js: `const h=document.querySelector('h2');if(h){const p=document.createElement('p');p.className=h.className;p.textContent=h.textContent;h.replaceWith(p)}`, expect: 'detect', scope: ANY },
  { name: 'pseudo-content', dimension: 'content', css: 'h2::after{content:"WRONG";position:absolute}', expect: 'detect', scope: ANY },
  { name: 'transition', dimension: 'motion', css: 'a,button{transition:all 3s !important}', expect: 'detect', scope: ANY },
  { name: 'shift-2px', dimension: 'layout', js: `const d=document.createElement('div');d.style.cssText='height:2px;margin:0;padding:0;border:0';document.body.prepend(d)`, expect: 'detect', scope: ANY, note: 'An empty, unpainted 2px block at the top of body: no collected element changes its own style, so only the geometry check can see the shift.' },
  { name: 'body-background', dimension: 'styling', css: 'body{background:#f0f !important}', expect: 'detect', scope: ANY, note: 'The page colour usually lives on html/body, which carry no text.' },
  { name: 'html-background', dimension: 'styling', css: 'html{background:#0c0 !important}', expect: 'detect', scope: ANY },
  { name: 'explicit-role', dimension: 'semantics', js: `const d=[...document.querySelectorAll('div')].find(x=>x.textContent.trim().length>10);if(d)d.setAttribute('role','group')`, expect: 'detect', scope: ANY, note: 'Same tag with an added role attribute: the container-retag equivalence must not cover it.' },
  { name: 'pseudo-mask', requires: 'pseudo', dimension: 'graphics', css: '*::before,*::after{mask-image:linear-gradient(red,blue) !important;-webkit-mask-image:linear-gradient(red,blue) !important}', expect: 'detect', scope: ANY },
  { name: 'hover-style', dimension: 'interaction', css: 'a:hover,button:hover{color:red !important;background:blue !important}', expect: 'miss', scope: ANY, note: 'Invisible unless a hover state is captured. Detected in tag "hover" states.' },
  { name: 'hover-style', dimension: 'interaction', css: 'a:hover,button:hover{color:red !important;background:blue !important}', expect: 'detect', scope: ['hover'] },
  { name: 'keyboard-focus-ring', dimension: 'behavior', css: ':focus-visible{outline:4px solid red !important}', expect: 'miss', scope: ANY, note: 'Invisible until something has focus. Detected in tag "keyboard" states.' },
  { name: 'keyboard-focus-ring', dimension: 'behavior', css: ':focus-visible{outline:4px solid red !important}', expect: 'detect', scope: ['keyboard'] },
  { name: 'keyboard-focus-shadow', dimension: 'behavior', css: ':focus-visible{box-shadow:0 0 0 4px red !important;outline:none !important}', expect: 'detect', scope: ['keyboard'] },
  { name: 'listener-removed', requires: 'aria:aria-expanded', dimension: 'behavior', js: `for(const t of ['pointerdown','pointerup','mousedown','mouseup','click','keydown','keyup','touchstart','touchend'])window.addEventListener(t,e=>{if(e.target&&e.target.closest&&e.target.closest('[aria-expanded]')){e.stopImmediatePropagation();e.preventDefault()}},true)`, expect: 'miss', scope: ANY, note: 'Handlers cannot be read from the DOM (a capture-phase listener swallows the click, so delegated and direct handlers both break); detected only when the interaction state is run.' },
  { name: 'listener-removed', requires: 'aria:aria-expanded', dimension: 'behavior', before: true, js: `for(const t of ['pointerdown','pointerup','mousedown','mouseup','click','keydown','keyup','touchstart','touchend'])window.addEventListener(t,e=>{if(e.target&&e.target.closest&&e.target.closest('[aria-expanded]')){e.stopImmediatePropagation();e.preventDefault()}},true)`, expect: 'detect', scope: ['menu'] },
];
/**
 * Shift every media-query width within 1.5 px of `px` the way that changes behavior at a viewport of exactly
 * `px`: min-width rules move up (the rule turns off), max-width rules move down (the rule turns off).
 * Rules inside @import-ed sheets, @media/@supports nesting and `media=` attributes of sheets are visited;
 * unreadable (cross-origin) sheets are counted. The page-side counters let calibration tell "nothing to shift"
 * (untested) from "shifted, nothing recorded changed" (a proven no-op).
 */
export function breakpointMutation(px: number): Mutation {
  return {
    name: `changed-breakpoint-${px}`, dimension: 'layout', expect: 'detect', scope: ['breakpoint'], pixelProof: true,
    js: `const PX=${px};const conv=(v,u)=>u==='px'?v:v*16;const stats={rewritten:0,unreadable:0};const near=n=>Math.abs(n-PX)<=2;
// lower bounds (min-width, width >= N, width > N, N <= width, N < width) move up; upper bounds move down, so the rule turns off around PX
const shift=t=>t.replace(/(min|max)-width:\\s*([\\d.]+)(px|rem|em)/g,(m,k,v,u)=>{const n=conv(parseFloat(v),u);if(!near(n))return m;return k+'-width:'+(k==='min'?n+400:Math.max(n-400,1))+'px'})
 .replace(/width\\s*(>=|<=|>|<)\\s*([\\d.]+)(px|rem|em)/g,(m,op,v,u)=>{const n=conv(parseFloat(v),u);if(!near(n))return m;return 'width '+op+' '+(op[0]==='>'?n+400:Math.max(n-400,1))+'px'})
 .replace(/([\\d.]+)(px|rem|em)\\s*(<=|>=|<|>)\\s*width/g,(m,v,u,op)=>{const n=conv(parseFloat(v),u);if(!near(n))return m;return (op[0]==='<'?n+400:Math.max(n-400,1))+'px '+op+' width'});
const mediaOf=list=>{if(list.mediaText){const b=list.mediaText,a=shift(b);if(a!==b){list.mediaText=a;stats.rewritten++}}};
const visited=new Set();
const sheetWalk=sheet=>{if(visited.has(sheet))return;visited.add(sheet);if(sheet.media)mediaOf(sheet.media);let rules;try{rules=sheet.cssRules}catch{stats.unreadable++;return}walk(rules)};
const walk=list=>{for(const r of list){if(r.styleSheet)sheetWalk(r.styleSheet);if(r.media)mediaOf(r.media);if(r.cssRules)walk(r.cssRules)}};
for(const sheet of document.styleSheets)sheetWalk(sheet);
if(document.adoptedStyleSheets)for(const sheet of document.adoptedStyleSheets)sheetWalk(sheet);
globalThis.__mochiBp=stats`,
  };
}
