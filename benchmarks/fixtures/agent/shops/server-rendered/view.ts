// Reads the server-rendered shop's HTML fragments back into the canonical JSON views
// the other stacks return, so the cross-stack test can compare observable behavior.
// Only data-* attributes the renderer emits are used; no DOM implementation needed.
import type { Address, CartItem, CartView, ErrorBody, OrderView, Product, Rate } from "../shared/core";

type Attrs = Record<string, string>;
const unesc = (s: string) => s.replace(/&quot;/g, '"').replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");

export function tags(html: string): { name: string; attrs: Attrs; end: number }[] {
    const out: { name: string; attrs: Attrs; end: number }[] = [];
    for (const m of html.matchAll(/<([a-zA-Z][\w-]*)((?:\s+[\w:-]+(?:="[^"]*")?)*)\s*\/?>/g)) {
        const attrs: Attrs = {};
        for (const a of m[2].matchAll(/([\w:-]+)(?:="([^"]*)")?/g)) attrs[a[1]] = a[2] === undefined ? "" : unesc(a[2]);
        out.push({ name: m[1].toLowerCase(), attrs, end: (m.index ?? 0) + m[0].length });
    }
    return out;
}

/** Top-level fragments by data-fragment name. */
export function fragments(html: string): Record<string, string> {
    const starts = [...html.matchAll(/<[^<>]*\bdata-fragment="([^"]+)"/g)].map((m) => ({ name: m[1], at: m.index! }));
    const out: Record<string, string> = {};
    starts.forEach((s, i) => (out[s.name] = html.slice(s.at, starts[i + 1]?.at ?? html.length)));
    return out;
}

const num = (s: string | undefined) => Number(s);
function lines(html: string): CartItem[] {
    return tags(html)
        .filter((t) => "data-line-sku" in t.attrs)
        .map((t) => ({
            sku: t.attrs["data-line-sku"], name: t.attrs["data-line-name"], quantity: num(t.attrs["data-line-qty"]),
            unitCents: num(t.attrs["data-line-unit-cents"]), lineCents: num(t.attrs["data-line-cents"]),
        }));
}
function address(a: Attrs): Address | null {
    return "data-addr-name" in a
        ? { name: a["data-addr-name"], line1: a["data-addr-line1"], city: a["data-addr-city"], region: a["data-addr-region"], postalCode: a["data-addr-postal-code"], country: a["data-addr-country"] }
        : null;
}

export function parseCart(html: string | undefined): CartView | null {
    const t = html && tags(html).find((x) => x.attrs.id === "cart-summary");
    if (!t) return null;
    const a = t.attrs;
    return {
        id: a["data-cart-id"], status: a["data-cart-status"] as CartView["status"], items: lines(html!), address: address(a),
        shippingRate: a["data-rate-id"] ? { id: a["data-rate-id"], label: a["data-rate-label"], amountCents: num(a["data-rate-cents"]) } : null,
        subtotalCents: num(a["data-subtotal-cents"]), shippingCents: num(a["data-shipping-cents"]), taxCents: num(a["data-tax-cents"]),
        totalCents: num(a["data-total-cents"]), currency: a["data-currency"],
    };
}

export function parseOrder(html: string): OrderView {
    const a = tags(html).find((x) => "data-order-id" in x.attrs)!.attrs;
    return {
        id: a["data-order-id"], status: a["data-status"] as OrderView["status"], cartId: a["data-cart-id"], items: lines(html), address: address(a)!,
        shipping: { rateId: a["data-rate-id"], label: a["data-rate-label"], amountCents: num(a["data-rate-cents"]) },
        subtotalCents: num(a["data-subtotal-cents"]), taxCents: num(a["data-tax-cents"]), totalCents: num(a["data-total-cents"]),
        currency: a["data-currency"], placedAt: a["data-placed-at"],
    };
}

export function parseRates(html: string): Rate[] {
    return tags(html)
        .filter((t) => t.attrs.type === "radio")
        .map((t) => ({ id: t.attrs.value, label: t.attrs["data-rate-label"], amountCents: num(t.attrs["data-rate-cents"]), etaDays: num(t.attrs["data-rate-eta"]) }));
}

export function parseCatalog(html: string): Product[] {
    return html.split(/(?=<li )/).filter((s) => s.startsWith("<li ")).map((chunk) => {
        const a = tags(chunk)[0].attrs;
        return { sku: a["data-sku"], name: unesc(/<h3>([^<]*)<\/h3>/.exec(chunk)![1]), priceCents: num(a["data-price"]), stock: num(a["data-stock"]), currency: a["data-currency"] };
    });
}

export function parseError(html: string): ErrorBody["error"] | null {
    const alert = tags(html).find((t) => t.attrs.id === "alert" && "data-error" in t.attrs);
    if (!alert) return null;
    const error: ErrorBody["error"] = { code: alert.attrs["data-error"], message: alert.attrs["data-message"] };
    for (const [k, v] of Object.entries(alert.attrs))
        if (k.startsWith("data-x-")) error[k.slice(7).replace(/-([a-z])/g, (_, c) => c.toUpperCase())] = JSON.parse(v);
    const fields: Record<string, string> = {};
    for (const m of html.matchAll(/<span data-field-error="([^"]+)">([^<]*)<\/span>/g)) fields[unesc(m[1])] = unesc(m[2]);
    if (Object.keys(fields).length) error.fields = fields;
    return error;
}
