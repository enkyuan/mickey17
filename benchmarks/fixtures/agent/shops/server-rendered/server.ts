// Server-rendered shop: HTML fragments over form posts, no client framework.
// Default port 8080 (no framework default exists); PORT overrides.
import { join } from "node:path";
import { Shop, optionsFromEnv, type CartView, type ErrorBody, type OrderView, type Rate, type ShopOptions } from "../shared/core";
import { ctxOf, notFound, paymentsStub, serve, sessionCookies, staticFile, withCookies, type Handler } from "../shared/http";
import { alertFragment, catalogFragment, orderStatusFragment, page, ratesFragment, workspace, type RenderState } from "./render";

export const DEFAULT_PORT = 8080;
const html = (status: number, body: string, headers: Record<string, string> = {}) =>
    new Response(body, { status, headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", ...headers } });

export function createShop(opts: ShopOptions = optionsFromEnv()) {
    const shop = new Shop(opts);
    const sharedDir = join(import.meta.dir, "..", "shared");
    const publicDir = join(import.meta.dir, "public");

    const handler: Handler = async (req) => {
        const url = new URL(req.url);
        const path = url.pathname;
        const m = req.method;
        const stub = await paymentsStub(shop, req);
        if (stub) return stub;

        const state = (sid: string | undefined, over: Partial<RenderState> = {}): RenderState => ({
            csrf: shop.session(sid)?.csrf ?? "",
            products: shop.catalog(),
            cart: null,
            rates: null,
            order: null,
            error: null,
            ...over,
        });

        if (m === "GET" && path === "/") {
            const r = shop.exec("bootstrap", ctxOf(req));
            return withCookies(html(200, page(state(r.session!.sid))), sessionCookies(r.session!, false));
        }
        if (m === "GET" && path === "/static/enhance.js") return (await staticFile(publicDir, "enhance.js")) ?? notFound();
        if (m === "GET" && path === "/static/sign.js") return (await staticFile(sharedDir, "sign.js")) ?? notFound();

        const raw = m === "POST" ? await req.text() : "";
        const form = new URLSearchParams(raw);
        const ctx = ctxOf(req, raw);
        ctx.csrf ??= form.get("_csrf") ?? undefined; // no-JS fallback: hidden field instead of header
        if (path.slice(1).split("/").includes("")) return html(404, "<p>Not found.</p>"); // empty or trailing segments are not routes
        const seg = path.split("/").filter(Boolean);

        let op = "";
        let input: Record<string, unknown> = {};
        let cartId: string | undefined;
        if (m === "GET" && path === "/catalog") op = "catalog";
        else if (m === "POST" && path === "/carts") op = "createCart";
        else if (seg[0] === "carts" && seg[1]) {
            cartId = seg[1];
            if (m === "POST" && seg[2] === "items" && !seg[3]) (op = "addItem"), (input = { cartId, sku: form.get("sku"), quantity: form.get("quantity") });
            else if (m === "POST" && seg[2] === "address" && !seg[3]) {
                const address: Record<string, string> = {};
                for (const [k, v] of form) if (k !== "_csrf") address[k] = v;
                (op = "setAddress"), (input = { cartId, address });
            } else if (m === "GET" && seg[2] === "shipping-rates" && !seg[3]) (op = "rates"), (input = { cartId });
            else if (m === "POST" && seg[2] === "shipping-rate" && !seg[3]) (op = "selectRate"), (input = { cartId, rateId: form.get("rate") });
            else if (m === "POST" && seg[2] === "orders" && !seg[3]) (op = "placeOrder"), (input = { cartId, paymentToken: form.get("paymentToken") });
        } else if (m === "GET" && seg[0] === "orders" && seg[1] && !seg[2]) op = "order", (input = { orderId: seg[1] });
        if (!op) return html(404, "<p>Not found.</p>");

        const result = shop.exec(op, ctx, input);
        const body = result.body as Record<string, any>;
        const error: ErrorBody["error"] | null = result.status >= 400 ? body.error : null;
        const headers = result.headers ?? {};

        if (op === "catalog") return html(200, catalogFragment(state(ctx.sid)), headers);
        if (op === "order")
            return html(result.status, error ? alertFragment(error) : orderStatusFragment(body.order as OrderView), headers);
        if (op === "rates") {
            if (error) return html(result.status, alertFragment(error), headers);
            const peek = shop.peek(ctx.sid, cartId!)!;
            return html(200, ratesFragment(state(ctx.sid, { cart: peek.cart }), body.rates as Rate[]), headers);
        }
        // Mutating operations return the whole state-dependent workspace.
        if (op === "createCart" && !error) cartId = (body.cart as CartView).id;
        const peek = cartId ? shop.peek(ctx.sid, cartId) : null;
        const st = state(ctx.sid, {
            cart: peek?.cart ?? null,
            rates: peek?.rates ?? null,
            error,
            order: op === "placeOrder" && !error ? (body.order as OrderView) : null,
            values: op === "setAddress" && error ? (input.address as Record<string, string>) : undefined,
            loadRates: op === "setAddress" && !error,
        });
        return html(result.status, workspace(st), headers);
    };
    return { shop, handler };
}

if (import.meta.main) serve(createShop().handler, DEFAULT_PORT, "server-rendered shop");
