import { createRouter } from '@tanstack/react-router'
import { routeTree } from './routeTree.gen'


export function getRouter() {
  return createRouter({
    routeTree, scrollRestoration: true, caseSensitive: true, trailingSlash: 'preserve',
    // Captured query states are opaque. The default JSON-style parser can lose
    // repeated keys and normalize + versus %20 before the loader sees them.
    parseSearch: raw => ({ _mochiRaw: raw }),
    stringifySearch: search => typeof search._mochiRaw === 'string' ? search._mochiRaw : '',
  })
}

declare module '@tanstack/react-router' {
  interface Register { router: ReturnType<typeof getRouter> }
}
