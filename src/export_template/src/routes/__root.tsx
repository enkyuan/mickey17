import '@/styles/tailwind.css'
import { createRootRoute, HeadContent, Outlet, Scripts, useRouterState } from '@tanstack/react-router'
import { elementProps, type PageData } from '../lib/runtime'

export const Route = createRootRoute({
  head: () => ({ meta: [{ charSet: 'utf-8' }, { name: 'viewport', content: 'width=device-width, initial-scale=1' }] }),
  component: Document,
  notFoundComponent: () => <main><h1>Page not captured</h1></main>,
})

function Document() {
  const page = useRouterState({ select: state => [...state.matches].reverse()
    .map(match => match.loaderData as PageData | undefined).find(data => data?.document) })
  return <html {...elementProps('html', page?.document.html ?? {}, null, page?.base)}>
    <head><HeadContent /></head>
    <body {...elementProps('body', page?.document.body ?? {}, null, page?.base)} onSubmitCapture={event => event.preventDefault()}>
      <Outlet /><Scripts />
    </body>
  </html>
}
