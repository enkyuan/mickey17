// HTML rendering for the server-rendered shop. No client framework: the server
// returns HTML fragments, and public/enhance.js swaps them in by element id
// (htmx-style). Every top-level fragment carries data-fragment="<id>".
import type { Address, CartItem, CartView, ErrorBody, OrderView, Product, Rate } from "../shared/core";

export const esc = (s: unknown) =>
    String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const attrs = (o: Record<string, unknown>) =>
    Object.entries(o)
        .filter(([, v]) => v !== undefined && v !== null && v !== false)
        .map(([k, v]) => (v === true ? ` ${k}` : ` ${k}="${esc(v)}"`))
        .join("");
export const money = (c: number) => `$${(c / 100).toFixed(2)}`;

export interface RenderState {
    csrf: string;
    products: Product[];
    cart: CartView | null;
    rates: Rate[] | null;
    order: OrderView | null;
    error: ErrorBody["error"] | null;
    values?: Record<string, string>;
    loadRates?: boolean;
}

const csrfField = (csrf: string) => `<input type="hidden" name="_csrf" value="${esc(csrf)}">`;
const fieldError = (st: RenderState, name: string) =>
    st.error?.fields?.[name] ? `<span data-field-error="${esc(name)}">${esc(st.error.fields[name])}</span>` : "";

export function alertFragment(error: RenderState["error"]): string {
    if (!error) return `<div id="alert" data-fragment="alert"></div>`;
    const { code, message, fields, ...extra } = error;
    // field errors are also summarized in the alert, so they are visible whether or not the form is on screen
    const summary = Object.entries(fields ?? {}).map(([k, v]) => ` <span data-field-error="${esc(k)}">${esc(v)}</span>`).join("");
    const x: Record<string, string> = {};
    for (const [k, v] of Object.entries(extra)) x[`data-x-${k.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase())}`] = JSON.stringify(v);
    return `<p id="alert" role="alert" data-fragment="alert"${attrs({ "data-error": code, ...x })} data-message="${esc(message)}">${esc(message)}${summary}</p>`;
}

export function catalogFragment(st: RenderState): string {
    const open = st.cart?.status === "open" ? st.cart : null;
    const items = st.products
        .map((p) => {
            const form = open
                ? `<form data-enhance method="post" action="/carts/${esc(open.id)}/items" data-add-form>${csrfField(st.csrf)}<input type="hidden" name="sku" value="${esc(p.sku)}">`
                : `<form data-add-form>`;
            return `<li${attrs({ "data-sku": p.sku, "data-price": p.priceCents, "data-stock": p.stock, "data-currency": p.currency })}>
<h3>${esc(p.name)}</h3> <span>${money(p.priceCents)}</span> <span>${p.stock} in stock</span>
${form}<label>Quantity <input name="quantity" value="1" inputmode="numeric">${st.cart ? fieldError(st, "quantity") : ""}</label>
<button type="submit" data-action="add"${open ? "" : " disabled"}>Add to cart</button></form>
</li>`;
        })
        .join("\n");
    return `<section id="catalog" data-fragment="catalog" aria-label="Catalog"><ul>\n${items}\n</ul></section>`;
}

const addrAttrs = (a: Address | null) =>
    a ? { "data-addr-name": a.name, "data-addr-line1": a.line1, "data-addr-city": a.city, "data-addr-region": a.region, "data-addr-postal-code": a.postalCode, "data-addr-country": a.country } : {};
const lineItems = (items: CartItem[]) =>
    `<ul>${items.map((i) => `<li${attrs({ "data-line-sku": i.sku, "data-line-name": i.name, "data-line-qty": i.quantity, "data-line-unit-cents": i.unitCents, "data-line-cents": i.lineCents })}>${i.quantity} × ${esc(i.name)} — ${money(i.lineCents)}</li>`).join("")}</ul>`;

export function cartFragment(st: RenderState): string {
    const c = st.cart;
    if (!c)
        return `<section id="cart" data-fragment="cart" aria-label="Cart"><form data-enhance method="post" action="/carts">${csrfField(st.csrf)}<button id="start-cart" type="submit">Start a cart</button></form></section>`;
    const a = attrs({
        "data-cart-id": c.id, "data-cart-status": c.status, "data-subtotal-cents": c.subtotalCents, "data-shipping-cents": c.shippingCents,
        "data-tax-cents": c.taxCents, "data-total-cents": c.totalCents, "data-currency": c.currency,
        "data-rate-id": c.shippingRate?.id, "data-rate-label": c.shippingRate?.label, "data-rate-cents": c.shippingRate?.amountCents,
        ...addrAttrs(c.address),
    });
    return `<section id="cart" data-fragment="cart" aria-label="Cart"><div id="cart-summary"${a}>
${lineItems(c.items)}
<dl><dt>Subtotal</dt><dd>${money(c.subtotalCents)}</dd><dt>Shipping</dt><dd>${money(c.shippingCents)}</dd><dt>Tax</dt><dd>${money(c.taxCents)}</dd><dt>Total</dt><dd>${money(c.totalCents)}</dd></dl>
</div></section>`;
}

export function ratesFragment(st: RenderState, rates: Rate[]): string {
    const c = st.cart!;
    const radios = rates
        .map((r) => `<label><input type="radio" name="rate" value="${esc(r.id)}"${attrs({ "data-rate-label": r.label, "data-rate-cents": r.amountCents, "data-rate-eta": r.etaDays, checked: c.shippingRate?.id === r.id })}> ${esc(r.label)} — ${money(r.amountCents)} (${r.etaDays} days)</label>`)
        .join("\n");
    return `<form id="rates" data-fragment="rates" data-enhance method="post" action="/carts/${esc(c.id)}/shipping-rate" aria-label="Shipping rates">${csrfField(st.csrf)}
${radios}${fieldError(st, "rateId")}
<button id="save-rate" type="submit">Use this rate</button></form>`;
}

const orderAttrs = (o: OrderView) =>
    attrs({
        "data-order-id": o.id, "data-status": o.status, "data-cart-id": o.cartId, "data-subtotal-cents": o.subtotalCents,
        "data-tax-cents": o.taxCents, "data-total-cents": o.totalCents, "data-currency": o.currency, "data-placed-at": o.placedAt,
        "data-rate-id": o.shipping.rateId, "data-rate-label": o.shipping.label, "data-rate-cents": o.shipping.amountCents, ...addrAttrs(o.address),
    });

export function orderStatusFragment(o: OrderView): string {
    return `<p id="order-status" data-fragment="order-status"${orderAttrs(o)}>Status: ${o.status}, total ${money(o.totalCents)}</p>\n${lineItems(o.items)}`;
}

function flowFragment(st: RenderState): string {
    const c = st.cart;
    const parts: string[] = [];
    if (c && c.items.length > 0 && c.status === "open") {
        const v = (n: string, d = "") => esc(st.values?.[n] ?? c.address?.[n as keyof Address] ?? d);
        const f = (n: string, label: string, d = "") => `<label>${label} <input name="${n}" value="${v(n, d)}">${fieldError(st, n)}</label>`;
        parts.push(`<form id="address-form" data-enhance method="post" action="/carts/${esc(c.id)}/address" aria-label="Shipping address">${csrfField(st.csrf)}
${f("name", "Full name")}${f("line1", "Address")}${f("city", "City")}${f("region", "State")}${f("postalCode", "ZIP")}${f("country", "Country", "US")}
<button id="save-address" type="submit">Save address</button></form>`);
        if (st.rates) parts.push(ratesFragment(st, st.rates));
        else if (c.address && st.loadRates)
            parts.push(`<div id="rates" data-fragment="rates" data-get="/carts/${esc(c.id)}/shipping-rates" data-trigger="load">Loading rates…</div>`);
    }
    if (c?.shippingRate && c.status === "open")
        parts.push(`<form id="payment" data-enhance data-tokenize="/payments-stub/v1/tokens" data-idempotent data-signed method="post" action="/carts/${esc(c.id)}/orders" aria-label="Payment (test mode)">${csrfField(st.csrf)}
<label>Test payment method <select id="pay-method" name="method"><option value="test_ok" selected>Test: approved</option><option value="test_declined">Test: declined</option></select></label>
<button id="pay" type="submit">Pay ${money(c.totalCents)}</button></form>`);
    if (st.order)
        parts.push(`<section id="confirmation" data-fragment="confirmation"${orderAttrs(st.order)} aria-label="Order confirmation"><h2>Thank you</h2><p>Order ${esc(st.order.id)} is confirmed.</p>
${lineItems(st.order.items)}
<p id="order-status" data-get="/orders/${esc(st.order.id)}" data-trigger="load">Loading status…</p></section>`);
    return `<div id="flow" data-fragment="flow">\n${parts.join("\n")}\n</div>`;
}

/** The full state-dependent set of fragments every mutating response carries. */
export function workspace(st: RenderState): string {
    return [alertFragment(st.error), catalogFragment(st), cartFragment(st), flowFragment(st)].join("\n");
}

export function page(st: RenderState): string {
    return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Mochi Shop (server-rendered)</title>
</head>
<body>
<main>
<h1>Mochi Shop</h1>
<div id="alert" data-fragment="alert"></div>
<section id="catalog" data-fragment="catalog" data-get="/catalog" data-trigger="load" aria-label="Catalog">Loading catalog…</section>
${cartFragment(st)}
<div id="flow" data-fragment="flow"></div>
</main>
<script type="module" src="/static/enhance.js"></script>
</body>
</html>
`;
}
