// Server-render routes through a built TanStack Start project without opening a port.
// usage (inside the project, after `bun run build`): bun ssr-check.mjs / /pay ...
// Exit 1 when a route is not 200, its HTML is implausibly small, or two routes render the same
// visible text (a child route swallowed by its parent still returns 200 with a title).
// Evidence for "renders on the server", not for hydration or for what a browser shows.
const mod = await import(process.cwd() + '/.output/server/_ssr/ssr.mjs')
const handler = mod.default ?? mod
const fetchFn = handler.fetch ?? handler
const visibleText = (html) =>
  html
    .replace(/<script[\s\S]*?<\/script>/g, ' ')
    .replace(/<style[\s\S]*?<\/style>/g, ' ')
    .replace(/<head[\s\S]*?<\/head>/g, ' ')
    .replace(/<[^>]+>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
let failed = 0
const seen = new Map()
for (const path of process.argv.slice(2)) {
  const res = await fetchFn(new Request('http://localhost' + path))
  const html = await res.text()
  const title = (html.match(/<title>([^<]*)<\/title>/) || [])[1]
  const text = visibleText(html)
  console.log(res.status, path, 'bytes=' + html.length, 'text=' + text.length, 'title=' + JSON.stringify(title), 'img=' + (html.match(/<img /g) || []).length)
  if (res.status !== 200 || html.length < 1000) failed++
  if (seen.has(text)) {
    console.log('  SAME VISIBLE TEXT AS', seen.get(text))
    failed++
  }
  seen.set(text, path)
}
process.exit(failed ? 1 : 0)
