// Progressive enhancement for the server-rendered shop (htmx-style, ~70 lines, no framework).
//  - forms[data-enhance] are submitted with fetch; the CSRF token is moved from the hidden
//    `_csrf` field (page DOM) into the X-CSRF-Token header.
//  - the response is HTML: each top-level element with an id replaces the element with that id.
//  - [data-get][data-trigger=load] elements are fetched when they appear.
//  - forms[data-tokenize] first call the hosted-field test stub for a token.
//  - forms[data-idempotent] send an Idempotency-Key; forms[data-signed] send X-Signature.
import { sign } from "/static/sign.js";

function apply(markup) {
    const box = document.createElement("div");
    box.innerHTML = markup;
    for (const el of [...box.children]) {
        const target = el.id && document.getElementById(el.id);
        if (target) target.replaceWith(el);
    }
    activate(document);
}

async function activate(root) {
    for (const el of root.querySelectorAll("[data-get][data-trigger='load']")) {
        if (el.dataset.started) continue;
        el.dataset.started = "1";
        const res = await fetch(el.dataset.get, { headers: { "X-Requested-With": "enhance" } });
        apply(await res.text());
    }
}

async function submit(form) {
    const fd = new FormData(form);
    const csrf = fd.get("_csrf");
    fd.delete("_csrf");
    if (form.dataset.tokenize) {
        const method = fd.get("method");
        fd.delete("method");
        const t = await fetch(form.dataset.tokenize, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ method }),
        });
        fd.set("paymentToken", (await t.json()).token);
    }
    const body = new URLSearchParams([...fd].map(([k, v]) => [k, String(v)])).toString();
    const headers = { "content-type": "application/x-www-form-urlencoded", "X-CSRF-Token": csrf };
    if (form.hasAttribute("data-idempotent")) headers["Idempotency-Key"] = form.idempotencyKey ??= crypto.randomUUID();
    if (form.hasAttribute("data-signed")) headers["X-Signature"] = sign(Date.now(), body);
    const res = await fetch(form.getAttribute("action"), { method: "POST", headers, body });
    apply(await res.text());
}

document.addEventListener("submit", (e) => {
    const form = e.target;
    if (!form.hasAttribute || !form.hasAttribute("data-enhance")) return;
    e.preventDefault();
    submit(form);
});

if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", () => activate(document));
else activate(document);
