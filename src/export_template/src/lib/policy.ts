// Pure decisions the runtime makes about captured content. This file has no
// imports so it can be unit-tested directly (benchmarks/tests/compat-runtime.test.ts).

// React 19 only reflects these as present attributes when given `true`; the
// captured empty string (`inert=""`) is falsy and the attribute would be dropped.
export const booleanAttributes = new Set(['disabled', 'required', 'multiple', 'readOnly', 'autoFocus', 'hidden',
  'open', 'checked', 'selected', 'controls', 'loop', 'muted', 'noValidate', 'formNoValidate', 'reversed', 'allowFullScreen',
  'inert', 'default'])

// data: URIs are accepted only as images. Raster images are accepted wherever a
// URL attribute is resolved. SVG is a document type that can carry <script>,
// event handlers and <foreignObject>, so it is accepted only where the browser
// loads it in secure image mode (no scripts, no external loads, no interaction):
// `<img src>` and `<img|source srcset>` (`svgImage`). It is never accepted for
// <a href>, <iframe src>, <embed>/<object>, <use href> or <script src>, where it
// would run as a document or be referenced as live content. Inline-style url()
// stays raster-only. Tested in benchmarks/tests/compat-runtime.test.ts and benchmarks/tests/compat-ssr.ts.
export const rasterDataImage = /^data:image\/(png|jpeg|gif|webp|avif)[;,]/i
export const svgDataImage = /^data:image\/svg\+xml[;,]/i
export const dataUriAllowed = (value: string, svgImage = false) =>
  rasterDataImage.test(value) || (svgImage && svgDataImage.test(value))

const srcsetSpace = /[ \t\n\f\r]/
// WHATWG srcset parsing: a URL is a run of non-whitespace characters (so commas
// inside data: URLs survive); only trailing commas end a candidate without
// descriptors, and descriptors end at a comma outside parentheses. Parentheses
// do not nest: the first `)` closes them, exactly as browsers tokenize, so a
// value such as `a.png 1x((,), http://x/b.png` cannot hide a candidate here that
// a browser would then load.
export function parseSrcset(value: string): Array<{ url: string; descriptor: string }> {
  const out: Array<{ url: string; descriptor: string }> = []
  let i = 0
  while (i < value.length) {
    while (i < value.length && (srcsetSpace.test(value[i]) || value[i] === ',')) i++
    if (i >= value.length) break
    let start = i
    while (i < value.length && !srcsetSpace.test(value[i])) i++
    let url = value.slice(start, i), descriptor = ''
    if (url.endsWith(',')) url = url.replace(/,+$/, '')
    else {
      start = i
      for (let parens = false; i < value.length; i++) {
        const c = value[i]
        if (parens) { if (c === ')') parens = false }
        else if (c === '(') parens = true
        else if (c === ',') break
      }
      descriptor = value.slice(start, i).replace(/^[ \t\n\f\r]+|[ \t\n\f\r]+$/g, '').replace(/[ \t\n\f\r]+/g, ' ')
      if (i < value.length) i++ // the comma that ended this candidate's descriptors
    }
    if (url) out.push({ url, descriptor })
  }
  return out
}

// Only width (`100w`), density (`1.5x`) and future-compat height (`50h`, with a
// width) descriptors are valid; browsers drop any other candidate. Re-emitting
// only valid descriptors keeps commas and parentheses out of the output.
export function srcsetDescriptorValid(descriptor: string) {
  if (!descriptor) return true
  const tokens = descriptor.split(' ')
  const w = tokens.filter(t => /^\d+w$/.test(t)), h = tokens.filter(t => /^\d+h$/.test(t))
  const x = tokens.filter(t => /^(\d+(\.\d+)?|\.\d+)([eE][+-]?\d+)?x$/.test(t))
  return w.length + h.length + x.length === tokens.length && w.length <= 1 && h.length <= 1 && x.length <= 1
    && !(w.length && x.length) && !(h.length && !w.length) && !w.some(t => /^0+w$/.test(t))
}

// Serializes resolved candidates, keeping only those a browser would parse back
// as exactly that URL and descriptor (e.g. a resolved URL that ends in a comma
// would otherwise turn its descriptor into a new candidate).
export function serializeSrcset(candidates: Array<{ url: string; descriptor: string }>) {
  const kept = candidates.flatMap(({ url, descriptor }) => {
    if (!srcsetDescriptorValid(descriptor)) return []
    const text = descriptor ? `${url} ${descriptor}` : url
    const back = parseSrcset(text)
    return back.length === 1 && back[0].url === url && back[0].descriptor === descriptor ? [text] : []
  })
  return kept.length ? kept.join(', ') : undefined
}

// Captured keys are literal path+query states. Match the exact state first, then
// the same path with the opposite trailing slash, then the path without the
// query. Unknown query strings and `/pay/` vs `/pay` therefore serve the
// captured page instead of a 404; paths that were never captured still 404.
export function pageFor<T>(pages: Record<string, T>, pathname: string, search: string): T | undefined {
  const toggled = pathname.length > 1 ? (pathname.endsWith('/') ? pathname.slice(0, -1) : pathname + '/') : pathname
  for (const key of [pathname + search, toggled + search, pathname, toggled]) if (Object.hasOwn(pages, key)) return pages[key]
  return undefined
}

// The server cannot measure the browser window, so SSR picks the first captured
// viewport variant from the request: Sec-CH-UA-Mobile (sent by default by
// Chromium on secure origins) or, where absent, a mobile User-Agent.
export const mobileRequest = (hint: string | null, userAgent: string | null) =>
  hint ? hint.trim() === '?1' : /Mobi|Android|iPhone|iPod/i.test(userAgent ?? '')

// Which captured variant (an index into widths/heights) renders for a window.
// Selection is by width alone; height only breaks an exact width tie, so
// resizing just the height (devtools, mobile URL bars) never swaps and remounts
// the page. With captures at 375 and 1440 the switch is at 907.5px: windows up
// to 907px get the 375 DOM. Shared by ResponsiveCapture and the document
// attributes so both always agree.
export function variantIndex(widths: number[], heights: number[], width: number, height: number) {
  let best = 0
  for (let i = 1; i < widths.length; i++) {
    const closer = Math.abs(widths[i] - width) - Math.abs(widths[best] - width)
    // A window exactly midway between two different widths keeps the earlier
    // (narrower: captures are sorted by width) capture; height decides only
    // between captures of equal width.
    if (closer < 0 || (closer === 0 && widths[i] === widths[best] && Math.abs(heights[i] - height) < Math.abs(heights[best] - height))) best = i
  }
  return best
}
// The variant SSR (and hydration) starts from: the smallest capture for mobile
// requests, the largest otherwise.
export const firstVariantIndex = (widths: number[], hint: 'mobile' | 'desktop' | undefined) =>
  hint === 'mobile' ? widths.reduce((best, width, i) => (width < widths[best] ? i : best), 0) : widths.length - 1

// SMIL can rewrite any attribute of its parent, e.g.
// `<a><set attributeName="href" to="javascript:..."/></a>`, bypassing every
// per-attribute filter. Animation targeting URL, event-handler or style
// attributes is dropped (together with its value attributes); presentation and
// geometry animation (opacity, transform, d, cx, ...) is kept.
export const animationTags = new Set(['animate', 'set', 'animateTransform', 'animateMotion'])
const animationValues = ['to', 'values', 'from', 'by']
export function unsafeAnimationTarget(attributeName: string) {
  const name = attributeName.trim().toLowerCase().replace(/^(xlink|xml):/, '')
  return /^(on|href$|src$|srcdoc$|action$|formaction$|data$|ping$|style$|poster$|background$|cite$)/.test(name)
}
export function safeAnimationAttributes(tag: string, raw: Record<string, string>): Record<string, string> {
  if (!animationTags.has(tag)) return raw
  // SVG attribute names are case-sensitive, so a capture can carry both
  // `attributename` and `attributeName`; every spelling must be safe.
  const targets = Object.entries(raw).filter(([name]) => name.toLowerCase() === 'attributename')
  if (!targets.some(([, value]) => unsafeAnimationTarget(value))) return raw
  return Object.fromEntries(Object.entries(raw).filter(([name]) => name.toLowerCase() !== 'attributename' && !animationValues.includes(name.toLowerCase())))
}

// Attributes that are never emitted: event handlers and executable or
// side-effecting attributes (`ping` makes the browser POST to a captured URL on
// every click).
export const blockedAttributes = new Set(['srcdoc', 'is', 'dangerouslySetInnerHTML', 'innerHTML', 'action', 'formaction', 'formAction', 'autoplay', 'nonce', 'integrity', 'ping'])
export const blockedAttribute = (name: string) => /^on/i.test(name) || blockedAttributes.has(name) || name.toLowerCase() === 'ping'

// Decodes a raw request pathname exactly once. Returns undefined for anything
// ambiguous: malformed sequences, an encoded slash, backslash, `?` or `#`, control
// characters (including NUL), or dot segments. Callers fall back to the raw path,
// which then simply does not match a captured page.
export function decodePathname(pathname: string): string | undefined {
  const segments: string[] = []
  for (const raw of pathname.split('/')) {
    let segment: string
    try { segment = decodeURIComponent(raw) } catch { return undefined }
    if (/[\\/?#\u0000-\u001f\u007f]/.test(segment) || segment === '.' || segment === '..') return undefined
    segments.push(segment)
  }
  return segments.join('/')
}
