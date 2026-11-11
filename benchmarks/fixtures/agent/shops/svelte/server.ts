// Svelte shop server. Default port 5173 (the Vite/SvelteKit dev-server port); PORT overrides.
import { join } from "node:path";
import { Shop, optionsFromEnv, type ShopOptions } from "../shared/core";
import { serve, spaHandler } from "../shared/http";

export const DEFAULT_PORT = 5173;
// The CSRF token is NOT in the page: the client reads it from the `csrf` cookie.
const shell = (_csrf: string) => `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Mochi Shop (Svelte)</title>
</head>
<body>
<div id="root"></div>
<script type="module" src="/static/app.js"></script>
</body>
</html>
`;

export function createShop(opts: ShopOptions = optionsFromEnv()) {
    const shop = new Shop(opts);
    const handler = spaHandler({ shop, publicDir: join(import.meta.dir, "public"), csrfCookie: true, shell });
    return { shop, handler };
}

if (import.meta.main) serve(createShop().handler, DEFAULT_PORT, "svelte shop");
