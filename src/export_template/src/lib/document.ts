// Document-level data that belongs to a captured route: head links, structured
// data, and the <html>/<body> attributes of each captured viewport. Page content
// itself lives in src/features/pages. Nothing here runs captured scripts.
import { useSyncExternalStore } from 'react'
import { useRouterState } from '@tanstack/react-router'
import { pageHead, type Attributes, type PageData } from './runtime'
import { firstVariantIndex, variantIndex } from './policy'

export type DocumentLink = { rel: string; href: string; type?: string; sizes?: string; media?: string; color?: string }
/** `<html>`/`<body>` attributes as captured at one viewport, ordered by (width, height). */
export type DocumentVariant = { width: number; height: number; html: Attributes; body: Attributes }
export type DocumentPage = PageData & {
  links?: DocumentLink[]
  /** JSON-LD structured data, serialized once by the exporter. */
  scripts?: Array<{ type: 'application/ld+json'; children: string }>
  /** Present only when the captured viewports disagree on <html>/<body> attributes. */
  documents?: DocumentVariant[]
}

export function documentHead(page: DocumentPage | undefined) {
  return page ? { ...pageHead(page), links: page.links ?? [], scripts: page.scripts ?? [] } : {}
}

const subscribeViewport = (notify: () => void) => {
  window.addEventListener('resize', notify)
  return () => window.removeEventListener('resize', notify)
}

/**
 * Attributes for <html> and <body>. A route captured at several viewports can
 * carry different document state per viewport (for example a desktop-only
 * `padding-bottom` on <body>), so this selects the variant the same way
 * ResponsiveCapture selects page content: SSR renders the variant implied by the
 * request (mobile hint -> smallest capture, otherwise largest), and the client
 * then follows the nearest captured viewport to the real window.
 */
export function useDocument(): { html: Attributes; body: Attributes; base: string } | undefined {
  const page = useRouterState({
    select: state => [...state.matches].reverse()
      .map(match => match.loaderData as DocumentPage | undefined).find(data => data?.document),
  })
  const variants = page?.documents ?? []
  const widths = variants.map(variant => variant.width), heights = variants.map(variant => variant.height)
  const first = firstVariantIndex(widths, page?.viewport)
  const viewport = useSyncExternalStore(
    subscribeViewport,
    () => `${window.innerWidth}x${window.innerHeight}`,
    () => `${widths[first]}x${heights[first]}`,
  )
  if (!page) return undefined
  if (!variants.length) return { ...page.document, base: page.base }
  const [width, height] = viewport.split('x').map(Number)
  const { html, body } = variants[variantIndex(widths, heights, width, height)]
  return { html, body, base: page.base }
}
