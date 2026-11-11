// Rebuilds the committed client bundles: react/public/app.js and svelte/public/app.js.
// Needs react, react-dom and svelte resolvable from this directory (see package.json).
//   bun build.ts
import { join } from "node:path";
import { compile } from "svelte/compiler";

const here = import.meta.dir;
const sveltePlugin: Bun.BunPlugin = {
    name: "svelte",
    setup(b) {
        b.onLoad({ filter: /\.svelte$/ }, async ({ path }) => {
            const { js } = compile(await Bun.file(path).text(), { filename: path, generate: "client" });
            return { contents: js.code, loader: "js" };
        });
    },
};

for (const [stack, entry] of [["react", "app.jsx"], ["svelte", "main.js"]] as const) {
    const out = await Bun.build({
        entrypoints: [join(here, stack, entry)],
        outdir: join(here, stack, "public"),
        naming: "app.js",
        target: "browser",
        format: "esm",
        minify: true,
        define: { "process.env.NODE_ENV": '"production"' },
        plugins: stack === "svelte" ? [sveltePlugin] : [],
    });
    if (!out.success) {
        for (const l of out.logs) console.error(String(l));
        process.exit(1);
    }
    console.log(stack, out.outputs.map((o) => `${o.path} ${o.size}B`).join(", "));
}
