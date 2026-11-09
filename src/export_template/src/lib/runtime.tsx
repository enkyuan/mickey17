import { createContext, useContext, createElement, useLayoutEffect, useRef, useState, useSyncExternalStore, type JSX, type MouseEvent, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { notFound, useRouter, useRouterState } from '@tanstack/react-router'
import { createIsomorphicFn } from '@tanstack/react-start'
import { appendResponseHeader, getRequest } from '@tanstack/react-start/server'
import parseStyle from 'style-to-object'
import { assets, capturedRoutes } from './references'
import tokensCss from '@/styles/tokens.css?inline'
import { blockedAttribute, booleanAttributes, dataUriAllowed, decodePathname, firstVariantIndex, mobileRequest, pageFor, parseSrcset, safeAnimationAttributes, serializeSrcset, variantIndex } from './policy'
export { tokensCss }

export type Attributes = Record<string, string>
export type Control = { value?: string | null; checked?: boolean | null; selected?: boolean | null; open?: boolean | null } | null
export type PageData = {
  key: string
  document: { html: Attributes; body: Attributes }
  /** Set by selectPage during SSR only: which captured viewport variant the server rendered first. */
  viewport?: 'mobile' | 'desktop'
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

const localAssets = new Set(Object.values(assets))
export function resolve(value: string, base: string, navigation = false, svgImage = false) {
  if (localAssets.has(value.split('#')[0])) return value
  if (!navigation && (value === '' || value.startsWith('#'))) return value
  try {
    const url = new URL(value, base || 'http://localhost/')
    if (!['http:', 'https:', 'mailto:', 'tel:', 'data:'].includes(url.protocol)) return undefined
    if (url.protocol === 'data:' && !dataUriAllowed(value, svgImage)) return undefined
    const fragment = url.hash; url.hash = ''
    if (navigation && Object.hasOwn(capturedRoutes, url.href)) return capturedRoutes[url.href] + fragment
    if (Object.hasOwn(assets, url.href)) return assets[url.href] + fragment
    return url.href + fragment
  } catch { return undefined }
}

export function resolveSrcset(value: string, base: string) {
  return serializeSrcset(parseSrcset(value).flatMap(({ url, descriptor }) => {
    const resolved = resolve(url, base, false, true)
    return resolved === undefined ? [] : [{ url: resolved, descriptor }]
  }))
}

export function elementProps<T extends keyof JSX.IntrinsicElements>(tag: T, raw: Attributes, control: Control = null, base = ''): JSX.IntrinsicElements[T] {
  const out: Record<string, unknown> = {}
  for (const [name, value] of Object.entries(safeAnimationAttributes(tag, raw))) {
    if (blockedAttribute(name)) continue
    const key = aliases[name] ?? name
    if (['checked', 'selected', 'value'].includes(key)) continue
    if (['href', 'src', 'poster', 'xlinkHref', 'cite', 'background'].includes(key)) {
      const resolved = resolve(value, base, key === 'href' && ['a', 'area'].includes(tag), key === 'src' && tag === 'img')
      if (resolved !== undefined) out[key] = resolved
    } else if (key === 'srcSet') {
      // Only image contexts take srcset (and may carry SVG data URIs).
      const resolved = tag === 'img' || tag === 'source' ? resolveSrcset(value, base) : undefined
      if (resolved !== undefined) out.srcSet = resolved
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
  const location = requestedLocation(pathname, search)
  const page = pageFor(pages, location.pathname, location.search)
  if (!page) throw notFound()
  const viewport = requestedViewport()
  return viewport ? { ...page, viewport } : page
}
// Start canonicalizes SSR history query encoding. Read the untouched Request
// for source-state selection, and the opaque router search during navigation.
const requestedLocation = createIsomorphicFn()
  .server((_pathname: string, _search: string) => {
    const url = new URL(getRequest().url)
    // The raw pathname is still percent-encoded here (the client router has
    // already decoded its own). Decode once, or keep the raw path when ambiguous.
    return { pathname: decodePathname(url.pathname) ?? url.pathname, search: url.search }
  })
  .client((pathname: string, search: string) => ({ pathname, search }))
// The server cannot measure the browser window, so it picks the first captured
// viewport variant from the request: Sec-CH-UA-Mobile (sent by default by
// Chromium on secure origins) or, where absent, a mobile User-Agent. The choice
// travels to the client in loader data so hydration renders the same variant.
const requestedViewport = createIsomorphicFn()
  .server((): 'mobile' | 'desktop' => {
    const headers = getRequest().headers
    appendResponseHeader('Vary', 'Sec-CH-UA-Mobile, User-Agent')
    return mobileRequest(headers.get('sec-ch-ua-mobile'), headers.get('user-agent')) ? 'mobile' : 'desktop'
  })
  .client((): 'mobile' | 'desktop' | undefined => undefined)
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
// Which captured variant renders is a pure function of the window width (see
// variantIndex): the nearest capture by width, height only breaking exact ties.
// For 375/1440 captures, windows up to 907px get the 375 DOM at any height.
// SSR cannot measure the window, so it starts from the variant its request hints
// at (see requestedViewport); two cases then switch variants after hydration,
// which remounts the whole subtree, and are NOT verified in a browser: desktop
// browser windows narrower than the switch point, and tablets whose User-Agent
// looks like a desktop. Both variants are not rendered together because each is
// a full, differing DOM with its own ids and state.
export function ResponsiveCapture({widths,heights,children}:{widths:number[];heights:number[];children:ReactNode[]}) {
  const hint=useRouterState({select:state=>{for(const match of state.matches){const viewport=(match.loaderData as PageData|undefined)?.viewport;if(viewport)return viewport}return undefined}})
  const first=firstVariantIndex(widths,hint)
  const viewport=useSyncExternalStore(subscribeViewport,()=>`${window.innerWidth}x${window.innerHeight}`,()=>`${widths[first]}x${heights[first]}`)
  const [width,height]=viewport.split('x').map(Number)
  return children[variantIndex(widths,heights,width,height)]
}
