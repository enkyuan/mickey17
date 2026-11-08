import { createContext, useContext, createElement, useLayoutEffect, useRef, useState, useSyncExternalStore, type JSX, type MouseEvent, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { notFound, useRouter } from '@tanstack/react-router'
import { createIsomorphicFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'
import parseStyle from 'style-to-object'
import { assets, capturedRoutes } from './references'
import tokensCss from '@/styles/tokens.css?inline'
export { tokensCss }

export type Attributes = Record<string, string>
export type Control = { value?: string | null; checked?: boolean | null; selected?: boolean | null; open?: boolean | null } | null
export type PageData = {
  key: string
  document: { html: Attributes; body: Attributes }
  title: string
  meta: Array<{ name?: string; property?: string; content: string }>
  css: string
  base: string
}
export const SourceContext = createContext('')
export const useSource = () => useContext(SourceContext)

const aliases: Record<string, string> = {
  class: 'className', for: 'htmlFor', tabindex: 'tabIndex', readonly: 'readOnly',
  maxlength: 'maxLength', minlength: 'minLength', colspan: 'colSpan', rowspan: 'rowSpan',
  autofocus: 'autoFocus', autocomplete: 'autoComplete', spellcheck: 'spellCheck',
  contenteditable: 'contentEditable', acceptcharset: 'acceptCharset',
  'accept-charset': 'acceptCharset', 'http-equiv': 'httpEquiv',
  srcset: 'srcSet', crossorigin: 'crossOrigin', usemap: 'useMap',
  'xlink:href': 'xlinkHref', 'xml:lang': 'xmlLang', 'xmlns:xlink': 'xmlnsXlink',
  'stroke-width': 'strokeWidth', 'stroke-linecap': 'strokeLinecap',
  'stroke-linejoin': 'strokeLinejoin', 'fill-rule': 'fillRule',
  'clip-rule': 'clipRule', 'clip-path': 'clipPath', 'fill-opacity': 'fillOpacity',
  'stroke-opacity': 'strokeOpacity', 'stop-color': 'stopColor', 'stop-opacity': 'stopOpacity',
}
const booleanAttributes = new Set(['disabled', 'required', 'multiple', 'readOnly', 'autoFocus', 'hidden',
  'open', 'checked', 'selected', 'controls', 'loop', 'muted', 'noValidate', 'formNoValidate', 'reversed', 'allowFullScreen'])

const localAssets = new Set(Object.values(assets))
function resolve(value: string, base: string, navigation = false) {
  if (localAssets.has(value.split('#')[0])) return value
  if (!navigation && (value === '' || value.startsWith('#'))) return value
  try {
    const url = new URL(value, base || 'http://localhost/')
    if (!['http:', 'https:', 'mailto:', 'tel:', 'data:'].includes(url.protocol)) return undefined
    if (url.protocol === 'data:' && !/^data:image\/(png|jpeg|gif|webp|avif);/i.test(value)) return undefined
    const fragment = url.hash; url.hash = ''
    if (navigation && Object.hasOwn(capturedRoutes, url.href)) return capturedRoutes[url.href] + fragment
    if (Object.hasOwn(assets, url.href)) return assets[url.href] + fragment
    return url.href + fragment
  } catch { return undefined }
}

export function elementProps<T extends keyof JSX.IntrinsicElements>(tag: T, raw: Attributes, control: Control = null, base = ''): JSX.IntrinsicElements[T] {
  const out: Record<string, unknown> = {}
  for (const [name, value] of Object.entries(raw)) {
    if (/^on/i.test(name) || ['srcdoc', 'is', 'dangerouslySetInnerHTML', 'innerHTML', 'action', 'formaction', 'formAction', 'autoplay', 'nonce', 'integrity'].includes(name)) continue
    const key = aliases[name] ?? name
    if (['checked', 'selected', 'value'].includes(key)) continue
    if (['href', 'src', 'poster', 'xlinkHref', 'cite', 'background'].includes(key)) {
      const resolved = resolve(value, base, key === 'href' && ['a', 'area'].includes(tag))
      if (resolved !== undefined) out[key] = resolved
    } else if (key === 'srcSet') {
      // The exporter reports this omission; selecting srcset candidates requires
      // a dedicated parser, especially for comma-containing data URLs.
      continue
    } else if (key === 'style') {
      let style: Record<string,string> = {}
      try { style = parseStyle(value) ?? {} } catch { /* Captured invalid declarations remain an export diagnostic. */ }
      out.style = Object.fromEntries(Object.entries(style)
        .map(([k,v]) => [k.startsWith('--') ? k : k.replace(/^-ms-/, 'ms-').replace(/-([a-z])/g, (_,c: string) => c.toUpperCase()),
          v.replace(/\s*!important\s*$/i, '').replace(/url\(\s*(?:"([^"]*)"|'([^']*)'|([^\s()]*))\s*\)/gi, (_match,a:string,b:string,c:string) => `url(${JSON.stringify(resolve(a ?? b ?? c,base) ?? '')})`)]))
      if (Object.values(style).some(value => /!important\s*$/i.test(value))) out.ref = (node: HTMLElement | null) => {
        if (node) for (const [name,value] of Object.entries(style)) if (/!important\s*$/i.test(value)) node.style.setProperty(name,value.replace(/\s*!important\s*$/i,''),'important')
      }
    } else if (booleanAttributes.has(key)) out[key] = true
    else if (key !== 'value' && key !== 'checked' && key !== 'selected') out[key] = value
  }
  if (tag === 'input' || tag === 'textarea' || tag === 'select') {
    if (control?.value != null) out.defaultValue = control.value
    else if (raw.value !== undefined) out.defaultValue = raw.value
    if (tag === 'input') out.defaultChecked = control?.checked ?? Object.hasOwn(raw, 'checked')
  } else if ((tag === 'button' || tag === 'option') && raw.value !== undefined) out.value = raw.value
  if (tag === 'details' && control?.open != null) out.open = control.open
  return out as JSX.IntrinsicElements[T]
}

export function Anchor({ raw, children }: { raw: Attributes; children: ReactNode }) {
  const base = useSource(); const router = useRouter()
  const props = elementProps('a', raw, null, base)
  function navigate(event: MouseEvent<HTMLAnchorElement>) {
    const href = props.href
    if (!href?.startsWith('/') || href.startsWith('//') || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || props.download !== undefined || (props.target && props.target !== '_self')) return
    event.preventDefault(); router.history.push(href)
  }
  return <a {...props} onClick={navigate}>{children}</a>
}
export function selectPage(pages: Record<string, PageData>, pathname: string, search: string): PageData {
  const page = pages[requestedKey(pathname, search)]
  if (!page) throw notFound()
  return page
}
// Start canonicalizes SSR history query encoding. Read the untouched Request
// for source-state selection, and the opaque router search during navigation.
const requestedKey = createIsomorphicFn()
  .server((_pathname: string, _search: string) => {
    const url = new URL(getRequest().url)
    return url.pathname + url.search
  })
  .client((pathname: string, search: string) => pathname + search)
// Match SSR/client cache identities despite Start's normalization. Every URL
// navigation still reloads its literal state (shouldReload: true).
export function queryKey(search: Record<string, unknown>) {
  return new URLSearchParams(String(search._mochiRaw ?? '')).toString()
}
export const capturedCss=(value:string)=>value.replace(/<(?=\/style)/gi,'\\3c ')
export function pageHead(page: PageData | undefined) {
  return page ? { meta: [{ title: page.title }, ...page.meta], styles: [{ children: (page.css + '\n' + tokensCss).replace(/<(?=\/style)/gi, '\\3c ') }] } : {}
}

// Scene pixels are an explicit editable initial-state asset, not recovered drawing code.
export function CapturedSurface({tag,raw,image}:{tag:string;raw:Attributes;image:string}) {
  const base=useSource(),canvas=useRef<HTMLCanvasElement>(null),[ready,setReady]=useState(false)
  useLayoutEffect(()=>{
    if(tag!=='canvas')return
    setReady(false)
    const frame=new Image()
    frame.onload=()=>{
      const node=canvas.current,context=node?.getContext('2d')
      if(node&&context){context.clearRect(0,0,node.width,node.height);context.drawImage(frame,0,0,node.width,node.height);setReady(true)}
    }
    frame.src=image
    return ()=>{frame.onload=null}
  },[tag,image])
  if(tag==='iframe'||tag==='frame') return <iframe {...elementProps('iframe',raw,null,base)} sandbox="" srcDoc={`<style>html,body{margin:0;width:100%;height:100%;overflow:hidden}img{width:100%;height:100%}</style><img src="${image}">`} />
  const props=elementProps('canvas',raw,null,base)
  return createElement(tag,{...props,
    ref:tag==='canvas'?(node:HTMLCanvasElement|null)=>{canvas.current=node;if(typeof props.ref==='function')props.ref(node)}:props.ref,
    style:ready?props.style:{...props.style,backgroundImage:`url("${image}")`,backgroundSize:'100% 100%',backgroundOrigin:'content-box',backgroundClip:'content-box',backgroundRepeat:'no-repeat'},
    'data-mochi-surface':'static-snapshot','data-mochi-surface-ready':tag!=='canvas'||ready?'true':'false'})
}
export function ShadowHost({tag,raw,mode,css,light,children}:{tag:string;raw:Attributes;mode:'open'|'closed';css:string;light:ReactNode;children:ReactNode}) {
  const base=useSource(),host=useRef<HTMLElement>(null),[root,setRoot]=useState<ShadowRoot|null>(null)
  useLayoutEffect(()=>{if(host.current&&!root)setRoot(host.current.shadowRoot??host.current.attachShadow({mode}))},[mode,root])
  return createElement(tag,{...elementProps('div',raw,null,base),ref:host},light,root?createPortal(<><style>{capturedCss(css)}</style>{children}</>,root):null)
}
const subscribeViewport=(notify:()=>void)=>{window.addEventListener('resize',notify);return ()=>window.removeEventListener('resize',notify)}
export function ResponsiveCapture({widths,heights,children}:{widths:number[];heights:number[];children:ReactNode[]}) {
  const last=widths.length-1
  const viewport=useSyncExternalStore(subscribeViewport,()=>`${window.innerWidth}x${window.innerHeight}`,()=>`${widths[last]}x${heights[last]}`)
  const [width,height]=viewport.split('x').map(Number)
  const distance=(i:number)=>Math.abs(widths[i]-width)*10+Math.abs(heights[i]-height)
  const index=widths.reduce((best,_value,i)=>distance(i)<distance(best)?i:best,0)
  return children[index]
}
