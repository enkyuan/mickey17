import type { Probe } from "./schema";
export function measurement(probes: Probe[], side: "reference" | "candidate") {
    return `(async()=>{
 await document.fonts.ready;
 const probes=${JSON.stringify(probes)},facts={},errors=[];
 const text=e=>(e.innerText??e.textContent??'').replace(/\\s+/g,' ').trim();
 for(const p of probes){try{
  const nodes=document.querySelectorAll(p.${side});if(nodes.length!==1)throw Error('selector count '+nodes.length);
  const e=nodes[0],r=e.getBoundingClientRect(),s=getComputedStyle(e),key=p.property;let v;
  if(key.startsWith('bounds.'))v=r[key.slice(7)];
  else if(key.startsWith('style.')){v=s.getPropertyValue(key.slice(6));if(v==='')throw Error('empty/unsupported CSS property');}
  else if(key.startsWith('has-attribute.'))v=e.hasAttribute(key.slice(14));
  else if(key==='value')v=e.value;
  else if(key==='validity.valid')v=e.validity.valid;
  else if(key==='visible')v=e.checkVisibility({opacityProperty:true,visibilityProperty:true});
  else if(key.startsWith('attribute.'))v=e.getAttribute(key.slice(10))??'';
  else if(key==='text')v=text(e);
  else if(key==='focused')v=document.activeElement===e;
  else if(key==='image.loaded')v=e.complete&&e.naturalWidth>0;
  else if(key==='image.width')v=e.naturalWidth;
  else if(key==='image.height')v=e.naturalHeight;
  else if(key==='path.length')v=e.getTotalLength();
  else if(key==='path.d')v=e.getAttribute('d');
  else if(key==='children.count')v=e.children.length;
  else if(key==='document.images.loaded')v=[...document.images].every(i=>i.complete&&i.naturalWidth>0);
  else if(key==='document.images.count')v=document.images.length;
  else if(key==='document.svg.count')v=document.querySelectorAll('svg').length;
  else if(key==='document.canvas.count')v=document.querySelectorAll('canvas').length;
  else if(key==='document.controls.count')v=document.querySelectorAll('button,input,select,textarea,[role=button]').length;
  else if(key==='document.path')v=location.pathname;
  else if(key==='document.overflow')v=document.documentElement.scrollWidth>innerWidth;
  else throw Error('unsupported property '+key);
  if(!['string','number','boolean'].includes(typeof v)||typeof v==='number'&&!Number.isFinite(v))throw Error('invalid measured value');
  facts[p.id]=v;
 }catch(error){errors.push(p.id+': '+error.message);}}
 const longTasks=[];let observer;
 try {observer=new PerformanceObserver(list=>{for(const e of list.getEntries())longTasks.push({startMs:e.startTime,durationMs:e.duration})});observer.observe({type:'longtask',buffered:true});}catch{}
 const frames=[];let last=performance.now();for(let i=0;i<30;i++){await new Promise(requestAnimationFrame);const t=performance.now();frames.push(t-last);last=t;}
 observer?.disconnect();
 const navigation=performance.getEntriesByType('navigation')[0];
 return {facts,errors,context:{url:location.href,viewport:[innerWidth,innerHeight],dpr:devicePixelRatio,reducedMotion:matchMedia('(prefers-reduced-motion:reduce)').matches,userAgent:navigator.userAgent},performance:{frames,frameSampleScope:'30 idle frames after actions; not interaction latency or animation fidelity',longTasks:observer?longTasks:null,domNodes:document.querySelectorAll('*').length,resourceCount:performance.getEntriesByType('resource').length,transferBytes:performance.getEntriesByType('resource').reduce((n,r)=>n+r.transferSize,0),loadMs:navigation?.loadEventEnd,domContentLoadedMs:navigation?.domContentLoadedEventEnd,heapBytes:performance.memory?.usedJSHeapSize??null},inventory:{headings:[...document.querySelectorAll('h1,h2,h3')].map(text),controls:document.querySelectorAll('button,input,select,textarea,[role=button]').length,images:document.images.length,canvases:document.querySelectorAll('canvas').length,svg:document.querySelectorAll('svg').length}};
 })()`;
}
