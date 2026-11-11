// HTTP-level clients for the three wires, used by the cross-stack test. They issue exactly the
// requests a page's own client JS issues (same paths, methods, headers, body encodings) and
// reduce each response to a canonical { status, view }. Transport is injected: an in-process
// `handler(Request)` (no socket) or real `fetch` against a running server.
import { FIXED_NOW, type Address } from "./core";
import { sign } from "./sign.js";
import { fragments, parseCart, parseCatalog, parseError, parseOrder, parseRates } from "../server-rendered/view";

export type Transport = (req: Request) => Promise<Response>;
export interface Obs {
    status: number;
    view: any;
    headers: Record<string, string>;
}
export interface PlaceOpts {
    key?: string | null;
    signature?: string | null;
    csrf?: string | null;
    now?: number;
}
export interface ShopClient {
    readonly stack: string;
    csrf: string | null;
    open(): Promise<void>;
    catalog(): Promise<Obs>;
    createCart(): Promise<Obs>;
    add(cartId: string, sku: string, quantity: string): Promise<Obs>;
    address(cartId: string, a: Partial<Address>): Promise<Obs>;
    rates(cartId: string): Promise<Obs>;
    chooseRate(cartId: string, rateId: string | null): Promise<Obs>;
    tokenize(method: string): Promise<Obs>;
    placeOrder(cartId: string, token: string, opts?: PlaceOpts): Promise<Obs>;
    order(id: string): Promise<Obs>;
}

const BASE = "http://shop.test";

abstract class Base {
    protected jar = new Map<string, string>();
    /** Token the client sends; tests set it to null/another value to exercise CSRF enforcement. */
    csrf: string | null = "";
    constructor(protected send: Transport, protected base = BASE, protected now = FIXED_NOW) {}

    protected async http(method: string, path: string, init: { headers?: Record<string, string>; body?: string } = {}) {
        const headers = new Headers(init.headers);
        if (this.jar.size) headers.set("cookie", [...this.jar].map(([k, v]) => `${k}=${v}`).join("; "));
        const res = await this.send(new Request(this.base + path, { method, headers, body: init.body }));
        for (const c of res.headers.getSetCookie?.() ?? []) {
            const [pair] = c.split(";");
            const i = pair.indexOf("=");
            this.jar.set(pair.slice(0, i).trim(), pair.slice(i + 1).trim());
        }
        const text = await res.text();
        const hdr: Record<string, string> = {};
        res.headers.forEach((v, k) => (hdr[k] = v));
        return { status: res.status, text, headers: hdr, type: res.headers.get("content-type") ?? "" };
    }
    protected stamp(raw: string, o: PlaceOpts): string | undefined {
        return o.signature === null ? undefined : (o.signature ?? sign(o.now ?? this.now, raw));
    }
    /** Hosted-field test stub: identical JSON wire in every stack, no CSRF, no session. */
    async tokenize(method: string): Promise<Obs> {
        const r = await this.http("POST", "/payments-stub/v1/tokens", { headers: { "content-type": "application/json" }, body: JSON.stringify({ method }) });
        return { status: r.status, view: JSON.parse(r.text), headers: r.headers };
    }
}

/** React and Svelte: JSON API. `csrfFrom` is where each stack's client reads the token. */
export class JsonClient extends Base implements ShopClient {
    constructor(readonly stack: string, private csrfFrom: "meta" | "cookie", send: Transport, base?: string, now?: number) {
        super(send, base, now);
    }
    async open() {
        const r = await this.http("GET", "/");
        this.csrf = this.csrfFrom === "meta" ? /<meta name="csrf-token" content="([^"]+)"/.exec(r.text)![1] : this.jar.get("csrf")!;
    }
    private async api(method: string, path: string, body?: unknown, extra: Record<string, string> = {}, csrf: string | null = this.csrf): Promise<Obs> {
        const headers: Record<string, string> = { ...extra };
        if (body !== undefined) headers["content-type"] = "application/json";
        if (method !== "GET" && csrf !== null) headers["x-csrf-token"] = csrf;
        const r = await this.http(method, path, { headers, body: body === undefined ? undefined : JSON.stringify(body) });
        return { status: r.status, view: JSON.parse(r.text), headers: r.headers };
    }
    catalog() { return this.api("GET", "/api/products"); }
    createCart() { return this.api("POST", "/api/carts", {}); }
    add(cartId: string, sku: string, quantity: string) { return this.api("POST", `/api/carts/${cartId}/items`, { sku, quantity }); }
    address(cartId: string, a: Partial<Address>) { return this.api("PUT", `/api/carts/${cartId}/address`, a); }
    rates(cartId: string) { return this.api("GET", `/api/carts/${cartId}/shipping-rates`); }
    chooseRate(cartId: string, rateId: string | null) { return this.api("PUT", `/api/carts/${cartId}/shipping-rate`, { rateId }); }
    placeOrder(cartId: string, token: string, o: PlaceOpts = {}) {
        const body = { paymentToken: token };
        const extra: Record<string, string> = {};
        const sig = this.stamp(JSON.stringify(body), o);
        if (sig) extra["x-signature"] = sig;
        if (o.key !== null) extra["idempotency-key"] = o.key ?? "key-default";
        return this.api("POST", `/api/carts/${cartId}/orders`, body, extra, o.csrf === undefined ? this.csrf : o.csrf);
    }
    order(id: string) { return this.api("GET", `/api/orders/${id}`); }
}

/** Server-rendered: form-encoded posts, HTML fragments back; the CSRF token is a hidden field. */
export class SsrClient extends Base implements ShopClient {
    readonly stack = "server-rendered";
    async open() {
        const r = await this.http("GET", "/");
        this.csrf = /name="_csrf" value="([^"]+)"/.exec(r.text)![1];
    }
    private view(r: { status: number; text: string; headers: Record<string, string> }, ok: (html: string, f: Record<string, string>) => any): Obs {
        const f = fragments(r.text);
        const error = r.status >= 400 ? parseError(r.text) : null;
        return { status: r.status, view: error ? { error } : ok(r.text, f), headers: r.headers };
    }
    private async form(path: string, fields: [string, string][], extra: Record<string, string> = {}, csrf: string | null = this.csrf) {
        const body = new URLSearchParams(fields).toString();
        const headers: Record<string, string> = { "content-type": "application/x-www-form-urlencoded", ...extra };
        if (csrf !== null) headers["x-csrf-token"] = csrf;
        return { r: await this.http("POST", path, { headers, body }), body };
    }
    async catalog() { return this.view(await this.http("GET", "/catalog"), (h) => ({ products: parseCatalog(h) })); }
    async createCart() { return this.view((await this.form("/carts", [])).r, (_h, f) => ({ cart: parseCart(f.cart) })); }
    async add(cartId: string, sku: string, quantity: string) {
        return this.view((await this.form(`/carts/${cartId}/items`, [["sku", sku], ["quantity", quantity]])).r, (_h, f) => ({ cart: parseCart(f.cart) }));
    }
    async address(cartId: string, a: Partial<Address>) {
        return this.view((await this.form(`/carts/${cartId}/address`, Object.entries(a) as [string, string][])).r, (_h, f) => ({ cart: parseCart(f.cart) }));
    }
    async rates(cartId: string) { return this.view(await this.http("GET", `/carts/${cartId}/shipping-rates`), (h) => ({ rates: parseRates(h) })); }
    async chooseRate(cartId: string, rateId: string | null) {
        return this.view((await this.form(`/carts/${cartId}/shipping-rate`, rateId === null ? [] : [["rate", rateId]])).r, (_h, f) => ({ cart: parseCart(f.cart) }));
    }
    async placeOrder(cartId: string, token: string, o: PlaceOpts = {}) {
        const body = new URLSearchParams([["paymentToken", token]]).toString();
        const extra: Record<string, string> = {};
        const sig = this.stamp(body, o);
        if (sig) extra["x-signature"] = sig;
        if (o.key !== null) extra["idempotency-key"] = o.key ?? "key-default";
        const { r } = await this.form(`/carts/${cartId}/orders`, [["paymentToken", token]], extra, o.csrf === undefined ? this.csrf : o.csrf);
        return this.view(r, (_h, f) => ({ order: parseOrder(f.confirmation) }));
    }
    async order(id: string) { return this.view(await this.http("GET", `/orders/${id}`), (h) => ({ order: parseOrder(h) })); }
}

// ---- scenario runner shared by tests and anything else that drives a ShopClient ----
import { SCENARIOS, type ScenarioName } from "./scenarios";

export interface Step extends Obs {
    step: string;
}
export async function runScenario(c: ShopClient, name: ScenarioName, runIndex: number): Promise<Step[]> {
    const run = SCENARIOS[name].runs[runIndex];
    const out: Step[] = [];
    const rec = (step: string, o: Obs) => (out.push({ step, ...o }), o);
    await c.open();
    rec("catalog", await c.catalog());
    const cartId = rec("create-cart", await c.createCart()).view.cart.id as string;
    if (name === "validation-error") rec("add-item", await c.add(cartId, run.sku, run.badQuantity!));
    rec("add-item", await c.add(cartId, run.sku, run.quantity));
    if (name === "out-of-stock") return out;
    if (name === "validation-error") rec("set-address", await c.address(cartId, { ...run.address, ...run.badAddress }));
    rec("set-address", await c.address(cartId, run.address));
    const rates = rec("shipping-rates", await c.rates(cartId)).view.rates as { id: string; label: string }[];
    rec("select-shipping", await c.chooseRate(cartId, rates.find((r) => r.label === run.rate)!.id));
    const method = name === "declined" ? "test_declined" : "test_ok";
    const token = rec("tokenize-payment", await c.tokenize(method)).view.token as string;
    const placed = rec("place-order", await c.placeOrder(cartId, token, { key: `idem-${run.seed}` }));
    if (placed.status === 201) rec("order-status", await c.order(placed.view.order.id));
    return out;
}
