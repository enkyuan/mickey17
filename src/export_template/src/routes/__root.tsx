import '@/styles/tailwind.css'
import { createRootRoute, HeadContent, Outlet, Scripts } from '@tanstack/react-router'
import { elementProps } from '../lib/runtime'
import { useDocument } from '../lib/document'

export const Route = createRootRoute({
  head: () => ({ meta: [{ charSet: 'utf-8' }, { name: 'viewport', content: 'width=device-width, initial-scale=1' }] }),
  component: Document,
  notFoundComponent: () => <main><h1>Page not captured</h1></main>,
})

// Captured `action`, `formaction` and `on*` attributes are removed by the
// exporter and no backend was captured, so there is nothing for a submit to do.
// Left alone, the browser would reload the page with every field value in the
// query string. This handler cancels the submit and says so once in the console.
let announced = false
function formSubmissionDisabled(event: { preventDefault(): void }) {
  event.preventDefault()
  if (!announced) {
    announced = true
    console.info('This is a static Mochi compatibility export: form submission is disabled because no backend was captured.')
  }
}

function Document() {
  const doc = useDocument()
  return <html {...elementProps('html', doc?.html ?? {}, null, doc?.base)}>
    <head><HeadContent /></head>
    <body {...elementProps('body', doc?.body ?? {}, null, doc?.base)} onSubmitCapture={formSubmissionDisabled}>
      <Outlet /><Scripts />
    </body>
  </html>
}
