// HTTP plumbing shared by the three shop servers. Handlers are plain
// `(Request) => Response` functions so tests and the recorder can call them
// in-process; `serve` is the only place that binds a socket.
import { join, normalize } from "node:path";
import { Shop, type Ctx, type Result } from "./core";

export type Handler = (req: Request) => Promise<Response>;

export function cookiesOf(req: Request): Record<string, string> {
    const out: Record<string, string> = {};
    for (const part of (req.headers.get("cookie") ?? "").split(";")) {
        const i = part.indexOf("=");
        if (i > 0) out[part.slice(0, i).trim()] = part.slice(i + 1).trim();
    }
    return out;
}

export function ctxOf(req: Request, rawBody = ""): Ctx {
    return {
        sid: cookiesOf(req).sid,
        csrf: req.headers.get("x-csrf-token") ?? undefined,
        signature: req.headers.get("x-signature") ?? undefined,
        idempotencyKey: req.headers.get("idempotency-key") ?? undefined,
        rawBody,
    };
}

export function sessionCookies(session: { sid: string; csrf: string }, csrfCookie: boolean): string[] {
    const out = [`sid=${session.sid}; Path=/; HttpOnly; SameSite=Lax`];
    if (csrfCookie) out.push(`csrf=${session.csrf}; Path=/; SameSite=Lax`);
    return out;
}

export function withCookies(res: Response, cookies: string[]): Response {
    for (const c of cookies) res.headers.append("set-cookie", c);
    return res;
}

export function jsonResponse(result: Result): Response {
    const headers = new Headers({ "content-type": "application/json; charset=utf-8", "cache-control": "no-store" });
    for (const [k, v] of Object.entries(result.headers ?? {})) headers.set(k, v);
    return new Response(JSON.stringify(result.body), { status: result.status, headers });
}

export const notFound = () =>
    jsonResponse({ status: 404, body: { error: { code: "not_found", message: "No such route." } } });
export const badRequest = (message: string) =>
    jsonResponse({ status: 400, body: { error: { code: "bad_request", message } } });

/** The hosted-field-style tokenization stub. Same wire in every stack. */
export async function paymentsStub(shop: Shop, req: Request): Promise<Response | null> {
    const url = new URL(req.url);
    if (url.pathname !== "/payments-stub/v1/tokens") return null;
    if (req.method !== "POST") return notFound();
    let input: Record<string, unknown>;
    try {
        input = (await req.json()) as Record<string, unknown>;
    } catch {
        return badRequest("Body must be JSON.");
    }
    return jsonResponse(shop.exec("tokenize", {}, input));
}

const TYPES: Record<string, string> = {
    ".js": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".html": "text/html; charset=utf-8",
    ".json": "application/json",
    ".map": "application/json",
};
export async function staticFile(dir: string, relative: string): Promise<Response | null> {
    const safe = normalize(relative).replace(/^(\.\.[/\\])+/, "");
    const file = Bun.file(join(dir, safe));
    if (safe.includes("..") || !(await file.exists())) return null;
    const ext = safe.slice(safe.lastIndexOf("."));
    return new Response(file, { headers: { "content-type": TYPES[ext] ?? "application/octet-stream" } });
}

export interface SpaConfig {
    shop: Shop;
    publicDir: string;
    csrfCookie: boolean;
    shell: (csrf: string) => string;
}

/** JSON API + static shell, used by the React and Svelte shops (identical wire). */
export function spaHandler(cfg: SpaConfig): Handler {
    const { shop } = cfg;
    return async (req) => {
        const url = new URL(req.url);
        const path = url.pathname;
        const stub = await paymentsStub(shop, req);
        if (stub) return stub;
        if (req.method === "GET" && path === "/") {
            const r = shop.exec("bootstrap", ctxOf(req));
            const res = new Response(cfg.shell(r.session!.csrf), {
                headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" },
            });
            return withCookies(res, sessionCookies(r.session!, cfg.csrfCookie));
        }
        if (req.method === "GET" && path.startsWith("/static/"))
            return (await staticFile(cfg.publicDir, path.slice("/static/".length))) ?? notFound();
        if (!path.startsWith("/api/")) return notFound();

        let raw = "";
        let input: Record<string, unknown> = {};
        if (req.method === "POST" || req.method === "PUT") {
            raw = await req.text();
            if (raw) {
                try {
                    input = JSON.parse(raw);
                } catch {
                    return badRequest("Body must be JSON.");
                }
            }
        }
        const ctx = ctxOf(req, raw);
        const seg = path.slice("/api/".length).split("/");
        const m = req.method;
        const op = (name: string, extra: Record<string, unknown> = {}) =>
            jsonResponse(shop.exec(name, ctx, { ...input, ...extra }));
        if (m === "GET" && path === "/api/products") return op("catalog");
        if (m === "POST" && path === "/api/carts") return op("createCart");
        if (seg[0] === "carts" && seg[1]) {
            const cartId = seg[1];
            if (m === "POST" && seg[2] === "items" && !seg[3]) return op("addItem", { cartId });
            if (m === "PUT" && seg[2] === "address" && !seg[3]) return op("setAddress", { cartId, address: input });
            if (m === "GET" && seg[2] === "shipping-rates" && !seg[3]) return op("rates", { cartId });
            if (m === "PUT" && seg[2] === "shipping-rate" && !seg[3]) return op("selectRate", { cartId });
            if (m === "POST" && seg[2] === "orders" && !seg[3]) return op("placeOrder", { cartId });
        }
        if (m === "GET" && seg[0] === "orders" && seg[1] && !seg[2]) return op("order", { orderId: seg[1] });
        return notFound();
    };
}

/** The only place that opens a socket. PORT overrides the stack's default. */
export function serve(handler: Handler, defaultPort: number, label: string) {
    const port = Number(process.env.PORT ?? defaultPort);
    const server = Bun.serve({ port, hostname: "127.0.0.1", fetch: handler });
    console.log(`${label} listening on http://127.0.0.1:${server.port}`);
    return server;
}
