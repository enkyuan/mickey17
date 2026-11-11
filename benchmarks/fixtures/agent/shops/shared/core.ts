// Transport-neutral domain of the fixture shops. Every stack maps HTTP onto
// `Shop.exec` and renders the `Result` its own way (JSON or HTML), so observable
// behavior (status codes, error codes, final order state) is identical by design
// and the HTTP layers are what the cross-stack test checks.
//
// Deterministic: ids come from a seeded PRNG (SHOP_SEED), time from a fixed clock
// (SHOP_CLOCK=fixed, the default). Payments are a test-mode stub: no card numbers,
// no provider, no network.
import { sign } from "./sign.js";

export const FIXED_NOW = Date.UTC(2026, 0, 15, 12, 0, 0);
export const SIGNATURE_TOLERANCE_MS = 5 * 60 * 1000;
export const TAX_RATE = 0.08;
export const CURRENCY = "USD";

export interface Product {
    sku: string;
    name: string;
    priceCents: number;
    stock: number;
    currency: string;
}
export const SEED_CATALOG: readonly Product[] = [
    { sku: "mug", name: "Stoneware mug", priceCents: 1200, stock: 10, currency: CURRENCY },
    { sku: "tote", name: "Canvas tote", priceCents: 1850, stock: 5, currency: CURRENCY },
    { sku: "kettle", name: "Enamel kettle", priceCents: 4500, stock: 2, currency: CURRENCY },
];
export const TEST_PAYMENT_METHODS = ["test_ok", "test_declined"] as const;

export interface Address {
    name: string;
    line1: string;
    city: string;
    region: string;
    postalCode: string;
    country: string;
}
export interface CartItem {
    sku: string;
    name: string;
    quantity: number;
    unitCents: number;
    lineCents: number;
}
export interface Rate {
    id: string;
    label: string;
    amountCents: number;
    etaDays: number;
}
export interface CartView {
    id: string;
    status: "open" | "ordered";
    items: CartItem[];
    address: Address | null;
    shippingRate: { id: string; label: string; amountCents: number } | null;
    subtotalCents: number;
    shippingCents: number;
    taxCents: number;
    totalCents: number;
    currency: string;
}
export interface OrderView {
    id: string;
    status: "confirmed";
    cartId: string;
    items: CartItem[];
    address: Address;
    shipping: { rateId: string; label: string; amountCents: number };
    subtotalCents: number;
    taxCents: number;
    totalCents: number;
    currency: string;
    placedAt: string;
}
export interface ErrorBody {
    error: { code: string; message: string; fields?: Record<string, string>; [k: string]: unknown };
}
export interface Result {
    status: number;
    body: unknown;
    headers?: Record<string, string>;
    session?: { sid: string; csrf: string };
}

export interface Ctx {
    sid?: string;
    csrf?: string;
    signature?: string;
    idempotencyKey?: string;
    rawBody?: string;
}

interface Session {
    sid: string;
    csrf: string;
}
interface Cart {
    id: string;
    sid: string;
    status: "open" | "ordered";
    items: { sku: string; quantity: number }[];
    address: Address | null;
    quote: Rate[] | null;
    rate: Rate | null;
}

export function mulberry32(seed: number): () => number {
    let a = seed >>> 0;
    return () => {
        a = (a + 0x6d2b79f5) >>> 0;
        let t = a;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

export interface ShopOptions {
    seed?: number;
    clock?: () => number;
}
export function optionsFromEnv(env: Record<string, string | undefined> = process.env): ShopOptions {
    return {
        seed: env.SHOP_SEED ? Number(env.SHOP_SEED) : 1,
        clock: env.SHOP_CLOCK === "real" ? () => Date.now() : () => FIXED_NOW,
    };
}

const err = (
    status: number,
    code: string,
    message: string,
    extra: Record<string, unknown> = {},
): Result => ({ status, body: { error: { code, message, ...extra } } });
const invalid = (fields: Record<string, string>): Result =>
    err(422, "validation_failed", "Some fields are invalid.", { fields });

export class Shop {
    readonly clock: () => number;
    private rng: () => number;
    private stock = new Map<string, number>();
    private sessions = new Map<string, Session>();
    private carts = new Map<string, Cart>();
    private orders = new Map<string, { sid: string; view: OrderView }>();
    private tokens = new Map<string, { method: string; used: boolean }>();
    private idem = new Map<string, { bodyText: string; result: Result }>();

    constructor(opts: ShopOptions = {}) {
        this.rng = mulberry32(opts.seed ?? 1);
        this.clock = opts.clock ?? (() => FIXED_NOW);
        for (const p of SEED_CATALOG) this.stock.set(p.sku, p.stock);
    }

    private hex(n: number) {
        let s = "";
        while (s.length < n) s += Math.floor(this.rng() * 0x100000000).toString(16).padStart(8, "0");
        return s.slice(0, n);
    }
    private product(sku: string) {
        return SEED_CATALOG.find((p) => p.sku === sku);
    }

    /** Operations the HTTP layers call. `name` selects the operation. */
    exec(name: string, ctx: Ctx, input: Record<string, unknown> = {}): Result {
        switch (name) {
            case "bootstrap":
                return this.bootstrap(ctx);
            case "tokenize":
                return this.tokenize(input);
            case "catalog":
                return { status: 200, body: { products: this.catalog() } };
        }
        const session = ctx.sid ? this.sessions.get(ctx.sid) : undefined;
        if (!session) return err(401, "session_required", "Open the shop first to start a session.");
        const mutating = name !== "rates" && name !== "order";
        if (mutating && (!ctx.csrf || ctx.csrf !== session.csrf))
            return err(403, "csrf_invalid", "Missing or invalid CSRF token.");
        switch (name) {
            case "createCart":
                return this.createCart(session);
            case "addItem":
                return this.addItem(session, input);
            case "setAddress":
                return this.setAddress(session, input);
            case "rates":
                return this.rates(session, input);
            case "selectRate":
                return this.selectRate(session, input);
            case "placeOrder":
                return this.placeOrder(session, ctx, input);
            case "order":
                return this.order(session, input);
        }
        return err(404, "unknown_operation", `No operation ${name}.`);
    }

    catalog(): Product[] {
        return SEED_CATALOG.map((p) => ({ ...p, stock: this.stock.get(p.sku) ?? 0 }));
    }

    /** Read access for server-side renderers (the server-rendered shop re-renders state). */
    session(sid?: string): { sid: string; csrf: string } | undefined {
        return sid ? this.sessions.get(sid) : undefined;
    }
    peek(sid: string | undefined, cartId: string): { cart: CartView; rates: Rate[] | null } | null {
        const s = this.session(sid);
        const c = this.carts.get(cartId);
        return s && c && c.sid === s.sid ? { cart: this.cartView(c), rates: c.quote } : null;
    }

    private bootstrap(ctx: Ctx): Result {
        const existing = ctx.sid ? this.sessions.get(ctx.sid) : undefined;
        if (existing) return { status: 200, body: { ok: true }, session: existing };
        const session = { sid: `sid_${this.hex(16)}`, csrf: `csrf_${this.hex(16)}` };
        this.sessions.set(session.sid, session);
        return { status: 200, body: { ok: true }, session };
    }

    /** Hosted-field-style test tokenizer. Takes a test method id, never card data. */
    private tokenize(input: Record<string, unknown>): Result {
        const method = input.method;
        if (typeof method !== "string" || !(TEST_PAYMENT_METHODS as readonly string[]).includes(method))
            return invalid({ method: `must be one of ${TEST_PAYMENT_METHODS.join(", ")}` });
        const token = `tok_${method}_${this.hex(8)}`;
        this.tokens.set(token, { method, used: false });
        return { status: 201, body: { token } };
    }

    private cartFor(session: Session, id: unknown): Cart | Result {
        const cart = typeof id === "string" ? this.carts.get(id) : undefined;
        if (!cart || cart.sid !== session.sid) return err(404, "cart_not_found", "No such cart.");
        return cart;
    }

    private cartView(cart: Cart): CartView {
        const items = cart.items.map((i) => {
            const p = this.product(i.sku)!;
            return { sku: i.sku, name: p.name, quantity: i.quantity, unitCents: p.priceCents, lineCents: p.priceCents * i.quantity };
        });
        const subtotalCents = items.reduce((s, i) => s + i.lineCents, 0);
        const shippingCents = cart.rate?.amountCents ?? 0;
        const taxCents = Math.round(subtotalCents * TAX_RATE);
        return {
            id: cart.id,
            status: cart.status,
            items,
            address: cart.address,
            shippingRate: cart.rate ? { id: cart.rate.id, label: cart.rate.label, amountCents: cart.rate.amountCents } : null,
            subtotalCents,
            shippingCents,
            taxCents,
            totalCents: subtotalCents + shippingCents + taxCents,
            currency: CURRENCY,
        };
    }

    private createCart(session: Session): Result {
        const cart: Cart = { id: `cart_${this.hex(8)}`, sid: session.sid, status: "open", items: [], address: null, quote: null, rate: null };
        this.carts.set(cart.id, cart);
        return { status: 201, body: { cart: this.cartView(cart) } };
    }

    private addItem(session: Session, input: Record<string, unknown>): Result {
        const cart = this.cartFor(session, input.cartId);
        if ("body" in cart) return cart;
        if (cart.status !== "open") return err(409, "cart_closed", "This cart has already been ordered.");
        const fields: Record<string, string> = {};
        const product = typeof input.sku === "string" ? this.product(input.sku) : undefined;
        if (!product) fields.sku = "unknown product";
        const q = input.quantity;
        const quantity = typeof q === "number" ? q : typeof q === "string" && /^-?\d+$/.test(q.trim()) ? Number(q) : NaN;
        if (!Number.isInteger(quantity) || quantity < 1 || quantity > 10)
            fields.quantity = "must be a whole number between 1 and 10";
        if (Object.keys(fields).length) return invalid(fields);
        const inCart = cart.items.find((i) => i.sku === product!.sku)?.quantity ?? 0;
        const available = (this.stock.get(product!.sku) ?? 0) - inCart;
        if (quantity > available)
            return err(409, "out_of_stock", `Only ${available} of ${product!.name} left.`, { sku: product!.sku, available });
        const line = cart.items.find((i) => i.sku === product!.sku);
        if (line) line.quantity += quantity;
        else cart.items.push({ sku: product!.sku, quantity });
        cart.quote = null;
        cart.rate = null;
        return { status: 200, body: { cart: this.cartView(cart) } };
    }

    private setAddress(session: Session, input: Record<string, unknown>): Result {
        const cart = this.cartFor(session, input.cartId);
        if ("body" in cart) return cart;
        if (cart.status !== "open") return err(409, "cart_closed", "This cart has already been ordered.");
        const a = (input.address ?? {}) as Record<string, unknown>;
        const str = (k: string) => (typeof a[k] === "string" ? (a[k] as string).trim() : "");
        const address: Address = {
            name: str("name"), line1: str("line1"), city: str("city"),
            region: str("region").toUpperCase(), postalCode: str("postalCode"), country: str("country").toUpperCase(),
        };
        const fields: Record<string, string> = {};
        if (!address.name || address.name.length > 80) fields.name = "required, at most 80 characters";
        if (!address.line1) fields.line1 = "required";
        if (!address.city) fields.city = "required";
        if (!/^[A-Z]{2}$/.test(address.region)) fields.region = "two-letter state code";
        if (!/^\d{5}$/.test(address.postalCode)) fields.postalCode = "five digits";
        if (address.country !== "US") fields.country = "only US is supported";
        if (Object.keys(fields).length) return invalid(fields);
        cart.address = address;
        cart.quote = null;
        cart.rate = null;
        return { status: 200, body: { cart: this.cartView(cart) } };
    }

    private rates(session: Session, input: Record<string, unknown>): Result {
        const cart = this.cartFor(session, input.cartId);
        if ("body" in cart) return cart;
        if (!cart.items.length) return err(409, "cart_empty", "Add an item first.");
        if (!cart.address) return err(409, "address_required", "Enter a shipping address first.");
        cart.quote = [
            { id: `rate_${this.hex(8)}`, label: "Standard", amountCents: 500, etaDays: 5 },
            { id: `rate_${this.hex(8)}`, label: "Express", amountCents: 1500, etaDays: 2 },
        ];
        return { status: 200, body: { rates: cart.quote } };
    }

    private selectRate(session: Session, input: Record<string, unknown>): Result {
        const cart = this.cartFor(session, input.cartId);
        if ("body" in cart) return cart;
        if (cart.status !== "open") return err(409, "cart_closed", "This cart has already been ordered.");
        const rate = cart.quote?.find((r) => r.id === input.rateId);
        if (!rate) return invalid({ rateId: "unknown or expired shipping rate" });
        cart.rate = rate;
        return { status: 200, body: { cart: this.cartView(cart) } };
    }

    private checkSignature(ctx: Ctx): Result | null {
        const m = /^v1\.(\d+)\.([0-9a-f]{14})$/.exec(ctx.signature ?? "");
        if (!m) return err(401, "invalid_signature", "Missing or malformed request signature.");
        const ts = Number(m[1]);
        if (Math.abs(this.clock() - ts) > SIGNATURE_TOLERANCE_MS)
            return err(401, "signature_expired", "Request signature timestamp is outside the allowed window.");
        if (sign(ts, ctx.rawBody ?? "") !== ctx.signature)
            return err(401, "invalid_signature", "Request signature does not match the body.");
        return null;
    }

    private placeOrder(session: Session, ctx: Ctx, input: Record<string, unknown>): Result {
        const bad = this.checkSignature(ctx);
        if (bad) return bad;
        if (!ctx.idempotencyKey) return err(400, "idempotency_key_required", "Idempotency-Key header is required.");
        const idemKey = `${session.sid}:${ctx.idempotencyKey}`;
        const prior = this.idem.get(idemKey);
        const bodyText = JSON.stringify([input.cartId, input.paymentToken]);
        if (prior) {
            if (prior.bodyText !== bodyText)
                return err(422, "idempotency_key_reuse", "Idempotency-Key was used with a different request.");
            return { ...prior.result, headers: { ...prior.result.headers, "Idempotent-Replayed": "true" } };
        }
        const result = this.placeOrderOnce(session, input);
        if (result.status === 201) this.idem.set(idemKey, { bodyText, result });
        return result;
    }

    private placeOrderOnce(session: Session, input: Record<string, unknown>): Result {
        const cart = this.cartFor(session, input.cartId);
        if ("body" in cart) return cart;
        if (cart.status !== "open") return err(409, "cart_closed", "This cart has already been ordered.");
        const missing: string[] = [];
        if (!cart.items.length) missing.push("items");
        if (!cart.address) missing.push("address");
        if (!cart.rate) missing.push("shippingRate");
        if (missing.length) return err(409, "cart_incomplete", "The cart is not ready to order.", { missing });
        const tok = typeof input.paymentToken === "string" ? this.tokens.get(input.paymentToken) : undefined;
        if (!tok || tok.used) return err(422, "invalid_payment_token", "Payment token is unknown or already used.");
        tok.used = true;
        if (tok.method === "test_declined")
            return err(402, "card_declined", "The test payment method was declined.", { declineCode: "generic_decline" });
        for (const i of cart.items)
            if ((this.stock.get(i.sku) ?? 0) < i.quantity)
                return err(409, "out_of_stock", `Only ${this.stock.get(i.sku)} of ${i.sku} left.`, { sku: i.sku, available: this.stock.get(i.sku) });
        for (const i of cart.items) this.stock.set(i.sku, this.stock.get(i.sku)! - i.quantity);
        cart.status = "ordered";
        const v = this.cartView(cart);
        const order: OrderView = {
            id: `ord_${this.hex(8)}`, status: "confirmed", cartId: cart.id, items: v.items, address: cart.address!,
            shipping: { rateId: cart.rate!.id, label: cart.rate!.label, amountCents: cart.rate!.amountCents },
            subtotalCents: v.subtotalCents, taxCents: v.taxCents, totalCents: v.totalCents, currency: CURRENCY,
            placedAt: new Date(this.clock()).toISOString(),
        };
        this.orders.set(order.id, { sid: session.sid, view: order });
        return { status: 201, body: { order } };
    }

    private order(session: Session, input: Record<string, unknown>): Result {
        const o = typeof input.orderId === "string" ? this.orders.get(input.orderId) : undefined;
        if (!o || o.sid !== session.sid) return err(404, "order_not_found", "No such order.");
        return { status: 200, body: { order: o.view } };
    }

    /** Test/inspection only: final state of the whole shop, for cross-stack equality. */
    snapshot() {
        return {
            stock: Object.fromEntries([...this.stock].sort()),
            carts: [...this.carts.values()].map((c) => ({ status: c.status, items: c.items, hasAddress: !!c.address, hasRate: !!c.rate })),
            orders: [...this.orders.values()].map((o) => ({ ...o.view })),
        };
    }
}
