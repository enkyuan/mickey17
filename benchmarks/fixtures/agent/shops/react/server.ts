// React shop server. Default port 3000 (the usual React dev-server port); PORT overrides.
import { join } from "node:path";
import { Shop, optionsFromEnv, type ShopOptions } from "../shared/core";
import { serve, spaHandler } from "../shared/http";

export const DEFAULT_PORT = 3000;
const shell = (csrf: string) => `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="csrf-token" content="${csrf}">
<title>Mochi Shop (React)</title>
</head>
<body>
<div id="root"></div>
<script type="module" src="/static/app.js"></script>
</body>
</html>
`;

export function createShop(opts: ShopOptions = optionsFromEnv()) {
    const shop = new Shop(opts);
    const handler = spaHandler({ shop, publicDir: join(import.meta.dir, "public"), csrfCookie: false, shell });
    return { shop, handler };
}

if (import.meta.main) serve(createShop().handler, DEFAULT_PORT, "react shop");
