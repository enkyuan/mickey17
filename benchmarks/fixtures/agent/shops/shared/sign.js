// The one client-computed request value in every shop: a request signature.
// Pure and deterministic (cyrb53 over "<timestamp>.<body>"), so it is liftable.
// Served unchanged to the browser; the server verifies with the same function.
// NOT a security mechanism: the salt is public. It exists to be a value with no
// source in any earlier response.
const SALT = "mochi-shop-fixture-v1";

function cyrb53(str, seed = 0) {
    let h1 = 0xdeadbeef ^ seed;
    let h2 = 0x41c6ce57 ^ seed;
    for (let i = 0; i < str.length; i++) {
        const ch = str.charCodeAt(i);
        h1 = Math.imul(h1 ^ ch, 2654435761);
        h2 = Math.imul(h2 ^ ch, 1597334677);
    }
    h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
    h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
    return 4294967296 * (2097151 & h2) + (h1 >>> 0);
}

/** `v1.<timestamp ms>.<14 hex digits>`, sent as the X-Signature header. */
export function sign(timestampMs, body) {
    const hash = cyrb53(`${SALT}.${timestampMs}.${body}`).toString(16).padStart(14, "0");
    return `v1.${timestampMs}.${hash}`;
}
