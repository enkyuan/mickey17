// React shop. CSRF token: read from <meta name="csrf-token"> (page -> header).
import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { sign } from "../shared/sign.js";

const money = (c) => `$${(c / 100).toFixed(2)}`;
const csrf = () => document.querySelector('meta[name="csrf-token"]').content;

async function call(method, path, body, extraHeaders = {}) {
    const raw = body === undefined ? undefined : JSON.stringify(body);
    const headers = { ...extraHeaders };
    if (raw !== undefined) headers["content-type"] = "application/json";
    if (method !== "GET" && path.startsWith("/api/")) headers["X-CSRF-Token"] = csrf();
    const res = await fetch(path, { method, headers, body: raw });
    return { ok: res.ok, status: res.status, data: await res.json() };
}

function Field({ name, label, error, ...rest }) {
    return (
        <label>
            {label}
            <input name={name} {...rest} />
            {error ? <span data-field-error={name}>{error}</span> : null}
        </label>
    );
}

function App() {
    const [products, setProducts] = useState([]);
    const [cart, setCart] = useState(null);
    const [rates, setRates] = useState(null);
    const [order, setOrder] = useState(null);
    const [orderStatus, setOrderStatus] = useState(null);
    const [error, setError] = useState(null);
    const idempotencyKey = useRef(null);

    useEffect(() => {
        call("GET", "/api/products").then((r) => setProducts(r.data.products));
    }, []);

    const fail = (r) => {
        setError(r.data.error);
        return false;
    };
    const clear = () => setError(null);
    const fields = error?.fields ?? {};

    async function startCart() {
        const r = await call("POST", "/api/carts", {});
        if (!r.ok) return fail(r);
        clear();
        setCart(r.data.cart);
    }
    async function addItem(e, sku) {
        e.preventDefault();
        const quantity = new FormData(e.currentTarget).get("quantity"); // sent as typed
        const r = await call("POST", `/api/carts/${cart.id}/items`, { sku, quantity });
        if (!r.ok) return fail(r);
        clear();
        setCart(r.data.cart);
    }
    async function saveAddress(e) {
        e.preventDefault();
        const address = Object.fromEntries(new FormData(e.currentTarget));
        const r = await call("PUT", `/api/carts/${cart.id}/address`, address);
        if (!r.ok) return fail(r);
        clear();
        setCart(r.data.cart);
        const q = await call("GET", `/api/carts/${cart.id}/shipping-rates`);
        if (!q.ok) return fail(q);
        setRates(q.data.rates);
    }
    async function saveRate(e) {
        e.preventDefault();
        const rateId = new FormData(e.currentTarget).get("rate");
        const r = await call("PUT", `/api/carts/${cart.id}/shipping-rate`, { rateId });
        if (!r.ok) return fail(r);
        clear();
        setCart(r.data.cart);
        idempotencyKey.current = crypto.randomUUID();
    }
    async function pay(e) {
        e.preventDefault();
        const method = new FormData(e.currentTarget).get("method");
        // Hosted-field-style tokenization (test stub): a test method id in, a fake token out.
        const t = await call("POST", "/payments-stub/v1/tokens", { method });
        if (!t.ok) return fail(t);
        const raw = JSON.stringify({ paymentToken: t.data.token });
        const r = await call("POST", `/api/carts/${cart.id}/orders`, { paymentToken: t.data.token }, {
            "Idempotency-Key": idempotencyKey.current,
            "X-Signature": sign(Date.now(), raw),
        });
        if (!r.ok) return fail(r);
        clear();
        setOrder(r.data.order);
        const s = await call("GET", `/api/orders/${r.data.order.id}`);
        setOrderStatus(s.data.order);
    }

    return (
        <main>
            <h1>Mochi Shop</h1>
            {error ? (
                <p role="alert" data-error={error.code}>
                    {error.message}
                </p>
            ) : null}
            <section id="catalog" aria-label="Catalog">
                <ul>
                    {products.map((p) => (
                        <li key={p.sku} data-sku={p.sku}>
                            <h3>{p.name}</h3>
                            <span data-price={p.priceCents}>{money(p.priceCents)}</span>{" "}
                            <span data-stock={p.stock}>{p.stock} in stock</span>
                            <form data-add-form onSubmit={(e) => addItem(e, p.sku)}>
                                <Field name="quantity" label="Quantity" defaultValue="1" inputMode="numeric" error={error?.fields && cart ? fields.quantity : null} />
                                <button type="submit" data-action="add" disabled={!cart || cart.status !== "open"}>
                                    Add to cart
                                </button>
                            </form>
                        </li>
                    ))}
                </ul>
            </section>
            <section id="cart" aria-label="Cart">
                {!cart ? (
                    <button id="start-cart" type="button" onClick={startCart}>
                        Start a cart
                    </button>
                ) : (
                    <div id="cart-summary" data-cart-id={cart.id} data-cart-status={cart.status}>
                        <ul>
                            {cart.items.map((i) => (
                                <li key={i.sku} data-line-sku={i.sku} data-line-qty={i.quantity}>
                                    {i.quantity} × {i.name} — {money(i.lineCents)}
                                </li>
                            ))}
                        </ul>
                        <dl>
                            <dt>Subtotal</dt><dd data-subtotal-cents={cart.subtotalCents}>{money(cart.subtotalCents)}</dd>
                            <dt>Shipping</dt><dd data-shipping-cents={cart.shippingCents}>{money(cart.shippingCents)}</dd>
                            <dt>Tax</dt><dd data-tax-cents={cart.taxCents}>{money(cart.taxCents)}</dd>
                            <dt>Total</dt><dd data-total-cents={cart.totalCents}>{money(cart.totalCents)}</dd>
                        </dl>
                    </div>
                )}
            </section>
            {cart && cart.items.length > 0 && cart.status === "open" ? (
                <form id="address-form" onSubmit={saveAddress} aria-label="Shipping address">
                    <Field name="name" label="Full name" error={fields.name} />
                    <Field name="line1" label="Address" error={fields.line1} />
                    <Field name="city" label="City" error={fields.city} />
                    <Field name="region" label="State" error={fields.region} />
                    <Field name="postalCode" label="ZIP" error={fields.postalCode} />
                    <Field name="country" label="Country" defaultValue="US" error={fields.country} />
                    <button id="save-address" type="submit">Save address</button>
                </form>
            ) : null}
            {rates && cart?.status === "open" ? (
                <form id="rates" onSubmit={saveRate} aria-label="Shipping rates">
                    {rates.map((r) => (
                        <label key={r.id}>
                            <input type="radio" name="rate" value={r.id} data-rate-label={r.label} />
                            {r.label} — {money(r.amountCents)} ({r.etaDays} days)
                        </label>
                    ))}
                    {fields.rateId ? <span data-field-error="rateId">{fields.rateId}</span> : null}
                    <button id="save-rate" type="submit">Use this rate</button>
                </form>
            ) : null}
            {cart?.shippingRate && cart.status === "open" ? (
                <form id="payment" onSubmit={pay} aria-label="Payment (test mode)">
                    <label>
                        Test payment method
                        <select id="pay-method" name="method" defaultValue="test_ok">
                            <option value="test_ok">Test: approved</option>
                            <option value="test_declined">Test: declined</option>
                        </select>
                    </label>
                    <button id="pay" type="submit">Pay {money(cart.totalCents)}</button>
                </form>
            ) : null}
            {order ? (
                <section id="confirmation" data-order-id={order.id} aria-label="Order confirmation">
                    <h2>Thank you</h2>
                    <p>Order {order.id} is confirmed.</p>
                    {orderStatus ? (
                        <p id="order-status" data-status={orderStatus.status} data-total-cents={orderStatus.totalCents}>
                            Status: {orderStatus.status}, total {money(orderStatus.totalCents)}
                        </p>
                    ) : null}
                </section>
            ) : null}
        </main>
    );
}

createRoot(document.getElementById("root")).render(<App />);
