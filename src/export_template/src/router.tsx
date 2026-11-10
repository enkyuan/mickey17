import { createRouter } from '@tanstack/react-router'
import * as React from 'react'
import { installOwnerGuard } from './lib/policy'
import { routeTree } from './routeTree.gen'

// Recovered programs may bundle a production React renderer that shares this
// app's dev React; see installOwnerGuard. Dev only, no effect in production.
if (import.meta.env.DEV) {
  const key = '__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE'
  const react = React as unknown as Record<string, { A?: unknown } | undefined> & { default?: Record<string, { A?: unknown } | undefined> }
  installOwnerGuard(react[key] ?? react.default?.[key])
}


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
