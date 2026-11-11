<!-- Svelte 5 shop. CSRF token: read from the `csrf` cookie (cookie -> header). -->
<script>
    import { onMount } from "svelte";
    import { sign } from "../shared/sign.js";

    const money = (c) => `$${(c / 100).toFixed(2)}`;
    const csrf = () => /(?:^|;\s*)csrf=([^;]+)/.exec(document.cookie)?.[1] ?? "";

    async function call(method, path, body, extraHeaders = {}) {
        const raw = body === undefined ? undefined : JSON.stringify(body);
        const headers = { ...extraHeaders };
        if (raw !== undefined) headers["content-type"] = "application/json";
        if (method !== "GET" && path.startsWith("/api/")) headers["X-CSRF-Token"] = csrf();
        const res = await fetch(path, { method, headers, body: raw });
        return { ok: res.ok, status: res.status, data: await res.json() };
    }

    let products = $state([]);
    let cart = $state(null);
    let rates = $state(null);
    let order = $state(null);
    let orderStatus = $state(null);
    let error = $state(null);
    let idempotencyKey = null;
    const fields = $derived(error?.fields ?? {});

    onMount(async () => {
        products = (await call("GET", "/api/products")).data.products;
    });

    function fail(r) {
        error = r.data.error;
        return false;
    }

    async function startCart() {
        const r = await call("POST", "/api/carts", {});
        if (!r.ok) return fail(r);
        error = null;
        cart = r.data.cart;
    }
    async function addItem(e, sku) {
        e.preventDefault();
        const quantity = Number(new FormData(e.currentTarget).get("quantity"));
        const r = await call("POST", `/api/carts/${cart.id}/items`, { sku, quantity });
        if (!r.ok) return fail(r);
        error = null;
        cart = r.data.cart;
    }
    async function saveAddress(e) {
        e.preventDefault();
        const address = Object.fromEntries(new FormData(e.currentTarget));
        const r = await call("PUT", `/api/carts/${cart.id}/address`, address);
        if (!r.ok) return fail(r);
        error = null;
        cart = r.data.cart;
        const q = await call("GET", `/api/carts/${cart.id}/shipping-rates`);
        if (!q.ok) return fail(q);
        rates = q.data.rates;
    }
    async function saveRate(e) {
        e.preventDefault();
        const rateId = new FormData(e.currentTarget).get("rate");
        const r = await call("PUT", `/api/carts/${cart.id}/shipping-rate`, { rateId });
        if (!r.ok) return fail(r);
        error = null;
        cart = r.data.cart;
        idempotencyKey = crypto.randomUUID();
    }
    async function pay(e) {
        e.preventDefault();
        const method = new FormData(e.currentTarget).get("method");
        // Hosted-field-style tokenization (test stub): a test method id in, a fake token out.
        const t = await call("POST", "/payments-stub/v1/tokens", { method });
        if (!t.ok) return fail(t);
        const raw = JSON.stringify({ paymentToken: t.data.token });
        const r = await call("POST", `/api/carts/${cart.id}/orders`, { paymentToken: t.data.token }, {
            "Idempotency-Key": idempotencyKey,
            "X-Signature": sign(Date.now(), raw),
        });
        if (!r.ok) return fail(r);
        error = null;
        order = r.data.order;
        orderStatus = (await call("GET", `/api/orders/${order.id}`)).data.order;
    }
</script>

<main>
    <h1>Mochi Shop</h1>
    {#if error}
        <p role="alert" data-error={error.code}>{error.message}</p>
    {/if}
    <section id="catalog" aria-label="Catalog">
        <ul>
            {#each products as p (p.sku)}
                <li data-sku={p.sku}>
                    <h3>{p.name}</h3>
                    <span data-price={p.priceCents}>{money(p.priceCents)}</span>
                    <span data-stock={p.stock}>{p.stock} in stock</span>
                    <form data-add-form onsubmit={(e) => addItem(e, p.sku)}>
                        <label>
                            Quantity
                            <input name="quantity" value="1" inputmode="numeric" />
                            {#if cart && fields.quantity}<span data-field-error="quantity">{fields.quantity}</span>{/if}
                        </label>
                        <button type="submit" data-action="add" disabled={!cart || cart.status !== "open"}>Add to cart</button>
                    </form>
                </li>
            {/each}
        </ul>
    </section>
    <section id="cart" aria-label="Cart">
        {#if !cart}
            <button id="start-cart" type="button" onclick={startCart}>Start a cart</button>
        {:else}
            <div id="cart-summary" data-cart-id={cart.id} data-cart-status={cart.status}>
                <ul>
                    {#each cart.items as i (i.sku)}
                        <li data-line-sku={i.sku} data-line-qty={i.quantity}>{i.quantity} × {i.name} — {money(i.lineCents)}</li>
                    {/each}
                </ul>
                <dl>
                    <dt>Subtotal</dt><dd data-subtotal-cents={cart.subtotalCents}>{money(cart.subtotalCents)}</dd>
                    <dt>Shipping</dt><dd data-shipping-cents={cart.shippingCents}>{money(cart.shippingCents)}</dd>
                    <dt>Tax</dt><dd data-tax-cents={cart.taxCents}>{money(cart.taxCents)}</dd>
                    <dt>Total</dt><dd data-total-cents={cart.totalCents}>{money(cart.totalCents)}</dd>
                </dl>
            </div>
        {/if}
    </section>
    {#if cart && cart.items.length > 0 && cart.status === "open"}
        <form id="address-form" onsubmit={saveAddress} aria-label="Shipping address">
            {#each [["name", "Full name"], ["line1", "Address"], ["city", "City"], ["region", "State"], ["postalCode", "ZIP"], ["country", "Country"]] as [n, label] (n)}
                <label>
                    {label}
                    <input name={n} value={n === "country" ? "US" : ""} />
                    {#if fields[n]}<span data-field-error={n}>{fields[n]}</span>{/if}
                </label>
            {/each}
            <button id="save-address" type="submit">Save address</button>
        </form>
    {/if}
    {#if rates && cart?.status === "open"}
        <form id="rates" onsubmit={saveRate} aria-label="Shipping rates">
            {#each rates as r (r.id)}
                <label>
                    <input type="radio" name="rate" value={r.id} data-rate-label={r.label} />
                    {r.label} — {money(r.amountCents)} ({r.etaDays} days)
                </label>
            {/each}
            {#if fields.rateId}<span data-field-error="rateId">{fields.rateId}</span>{/if}
            <button id="save-rate" type="submit">Use this rate</button>
        </form>
    {/if}
    {#if cart?.shippingRate && cart.status === "open"}
        <form id="payment" onsubmit={pay} aria-label="Payment (test mode)">
            <label>
                Test payment method
                <select id="pay-method" name="method">
                    <option value="test_ok" selected>Test: approved</option>
                    <option value="test_declined">Test: declined</option>
                </select>
            </label>
            <button id="pay" type="submit">Pay {money(cart.totalCents)}</button>
        </form>
    {/if}
    {#if order}
        <section id="confirmation" data-order-id={order.id} aria-label="Order confirmation">
            <h2>Thank you</h2>
            <p>Order {order.id} is confirmed.</p>
            {#if orderStatus}
                <p id="order-status" data-status={orderStatus.status} data-total-cents={orderStatus.totalCents}>
                    Status: {orderStatus.status}, total {money(orderStatus.totalCents)}
                </p>
            {/if}
        </section>
    {/if}
</main>
