// Server-render regression check for a built compatibility export. It imports the
// Nitro server bundle and calls its fetch handler with Requests; it binds no port
// and needs no browser, so it proves server HTML only. Hydration and viewport
// switching still need a real browser.
//
//   cd <export> && bun install --frozen-lockfile && bun run check && bun run build
//   bun benchmarks/tests/compat-ssr.ts <export> [--out <dir to keep the HTML>]
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const args = Bun.argv.slice(2);
const project = resolve(args[0] ?? "");
const outIndex = args.indexOf("--out");
const keep = outIndex >= 0 ? resolve(args[outIndex + 1]) : undefined;
if (!args[0]) throw Error("Usage: bun compat-ssr.ts EXPORT_DIR [--out DIR]");

const server = join(project, ".output/server/index.mjs");
const patched = readFileSync(server, "utf8").replace(
    "setupCloseHooks(serve(resolveServeOptions({ fetch: _fetch })));",
    "export const handler = _fetch;",
);
if (patched.includes("setupCloseHooks(serve("))
    throw Error("could not neutralize the Nitro listener; bundle layout changed");
const entry = join(project, ".output/server/index.ssr.mjs");
writeFileSync(entry, patched);
const { handler } = (await import(entry)) as {
    handler: (request: Request) => Promise<Response>;
};

const report = JSON.parse(readFileSync(join(project, ".mochi/export.json"), "utf8"));
const routes: string[] = report.captured_paths;
const sources = (function walk(dir: string): string[] {
    return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
        e.isDirectory() ? walk(join(dir, e.name)) : e.name.endsWith(".tsx") ? [join(dir, e.name)] : [],
    );
})(join(project, "src")).map((f) => readFileSync(f, "utf8"));
const sourceText = sources.join("\n");

const failures: string[] = [];
const expect = (ok: boolean, message: string) => ok || failures.push(message);
const get = (path: string, headers: Record<string, string> = {}) =>
    handler(new Request("http://check.test" + path, { headers }));
const toggled = (route: string) =>
    route === "/" ? route : route.endsWith("/") ? route.slice(0, -1) : route + "/";
const name = (route: string) => (route === "/" ? "index" : route.slice(1).replace(/\//g, "__"));
if (keep) mkdirSync(keep, { recursive: true });

let inert = 0;
for (const route of routes) {
    const response = await get(route);
    const html = await response.text();
    if (keep) writeFileSync(join(keep, name(route) + ".html"), html);
    expect(response.status === 200, `${route}: status ${response.status}`);
    inert += (html.match(/\sinert=""/g) ?? []).length;
    expect(!/\sinert="(false|true)"/.test(html), `${route}: inert must render as a bare attribute`);
    for (const img of html.match(/<img\b[^>]*>/g) ?? [])
        expect(
            /\s(src|srcset)="/i.test(img) || /\sdata-[a-z-]*src/i.test(img),
            `${route}: <img> rendered without src: ${img.slice(0, 120)}`,
        );
    // SVG data URIs may appear only where browsers load them as images: <img src>,
    // <img|source srcset>, and React's hoisted <link rel=preload imagesrcset>.
    for (const [, tag, attrs] of html.matchAll(/<([a-zA-Z][\w:-]*)\b([^>]*)>/g))
        for (const [, attr] of attrs.matchAll(/([^\s=]+)="[^"]*data:image\/svg/gi))
            expect(
                (tag === "img" && /^(src|srcset)$/i.test(attr)) ||
                    (tag === "source" && /^srcset$/i.test(attr)) ||
                    (tag === "link" && /^imagesrcset$/i.test(attr)),
                `${route}: SVG data URI in <${tag} ${attr}>`,
            );
    expect(!/<a\b[^>]*href="data:/i.test(html), `${route}: data: URI in an anchor`);
    expect(!/<a\b[^>]*\sping=/i.test(html), `${route}: <a ping> must not be emitted`);
    for (const [, tag, attrs] of html.matchAll(/<(animate|set|animateTransform|animateMotion)\b([^>]*)>/gi))
        expect(!/attributeName="\s*(xlink:|xml:)?(href|src|on[a-z]*|style|action|formaction|srcdoc|data|ping)\s*"/i.test(attrs), `${route}: <${tag}> animates a URL, handler or style attribute`);

    const withQuery = await get(route + "?mochi-ssr-check=1");
    expect(withQuery.status === 200, `${route}?query: status ${withQuery.status} (unknown query strings must serve the captured page)`);
    const slash = await get(toggled(route));
    expect(slash.status === 200, `${toggled(route)}: status ${slash.status} (trailing slash must be tolerated)`);
}
// Percent-encoded spellings of a captured path are decoded once: %xx works,
// %25xx (double encoding) and an encoded slash do not.
for (const route of routes.filter((r) => r.length > 2)) {
    const encoded = "/%" + route.charCodeAt(1).toString(16).toUpperCase().padStart(2, "0") + route.slice(2);
    expect((await get(encoded)).status === 200, `${encoded}: percent-encoded captured path must serve the page`);
    const doubled = "/%25" + route.charCodeAt(1).toString(16) + route.slice(2);
    expect((await get(doubled)).status === 404, `${doubled}: double-encoded path must stay a 404`);
    expect((await get(route.replace(/\/$/, "") + "%2F")).status === 404, `${route}: encoded slash must stay a 404`);
}
for (const path of ["/__mochi_never_captured__", "/__mochi_never_captured__?x=1", ...routes.flatMap((r) => [...(r.toUpperCase() !== r && !routes.includes(r.toUpperCase()) ? [r.toUpperCase()] : []), r.replace(/\/?$/, "/__mochi_extra")])]) {
    const missing = await get(path);
    expect(missing.status === 404, `uncaptured path ${path}: status ${missing.status}, expected 404`);
}

if (/'inert'\s*:\s*''/.test(sourceText))
    expect(inert > 0, "sources carry inert attributes but no server-rendered route emits inert=\"\"");

if (/ResponsiveCapture\s+widths=\{\[\s*\d+\s*,/.test(sourceText)) {
    const phone = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148";
    const desktop = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 Safari/605.1.15";
    const a = await get(routes[0], { "user-agent": phone });
    const b = await get(routes[0], { "user-agent": desktop });
    const hinted = await get(routes[0], { "sec-ch-ua-mobile": "?1", "user-agent": desktop });
    // Router dehydration stamps each match with a wall-clock `u:` value.
    const stable = (html: string) => html.replace(/\bu:\d{10,}/g, "u:0");
    const [phoneHtml, desktopHtml, hintedHtml] = (
        await Promise.all([a.text(), b.text(), hinted.text()])
    ).map(stable);
    // The dehydrated hint alone (`viewport:"mobile"`) makes the documents differ,
    // so compare the rendered markup without scripts as well.
    const markup = (html: string) => html.replace(/<script\b[\s\S]*?<\/script>/gi, "");
    expect(/viewport:"mobile"/.test(phoneHtml) && /viewport:"desktop"/.test(desktopHtml),
        "loader data must carry the server-chosen viewport so hydration renders the same variant");
    expect(hintedHtml === phoneHtml, "Sec-CH-UA-Mobile did not select the mobile variant");
    expect(/sec-ch-ua-mobile/i.test(a.headers.get("vary") ?? ""), "SSR variant selection must send Vary: Sec-CH-UA-Mobile");
    let rendered = 0;
    for (const route of routes) {
        const [p, d] = await Promise.all([get(route, { "user-agent": phone }), get(route, { "user-agent": desktop })]);
        if (markup(await p.text()) !== markup(await d.text())) rendered++;
    }
    expect(rendered > 0, "no route rendered different markup for mobile and desktop requests");
}

if (failures.length) {
    console.error(failures.map((f) => "FAIL " + f).join("\n"));
    process.exit(1);
}
console.log(`ok: ${routes.length} routes, inert=${inert}`);
process.exit(0);
