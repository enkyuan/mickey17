import { b64urlDecode, b64urlEncode, canonicalJson, fromUtf8, utf8 } from "./bytes.ts";

export type SigAlg = "Ed25519" | "HMAC-SHA256";

export interface Verifier {
    readonly alg: SigAlg;
    verify(data: Uint8Array, sig: Uint8Array): Promise<boolean>;
}
export interface Signer extends Verifier {
    sign(data: Uint8Array): Promise<Uint8Array>;
}

const bs = (b: Uint8Array) => b as Uint8Array<ArrayBuffer>;

export async function generateEd25519(): Promise<{ signer: Signer; verifier: Verifier; publicKeyRaw: Uint8Array }> {
    const pair = (await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"])) as unknown as CryptoKeyPair;
    const publicKeyRaw = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
    const verifier = verifierFromKey(pair.publicKey);
    const signer: Signer = {
        alg: "Ed25519",
        verify: verifier.verify,
        async sign(data) {
            return new Uint8Array(await crypto.subtle.sign({ name: "Ed25519" }, pair.privateKey, bs(data)));
        },
    };
    return { signer, verifier, publicKeyRaw };
}

function verifierFromKey(key: CryptoKey): Verifier {
    return {
        alg: "Ed25519",
        async verify(data, sig) {
            if (sig.length !== 64) return false;
            try {
                return await crypto.subtle.verify({ name: "Ed25519" }, key, bs(sig), bs(data));
            } catch {
                return false;
            }
        },
    };
}

/** Verify-only: the executor holds this and therefore cannot mint a mandate. */
export async function ed25519Verifier(publicKeyRaw: Uint8Array): Promise<Verifier> {
    const key = await crypto.subtle.importKey("raw", bs(publicKeyRaw), { name: "Ed25519" }, false, ["verify"]);
    return verifierFromKey(key);
}

/** HMAC verification uses WebCrypto's verify, which compares the tag in constant time. */
export async function hmacSigner(raw: Uint8Array): Promise<Signer> {
    if (raw.length < 32) throw new RangeError("HMAC key must be at least 32 bytes");
    const key = await crypto.subtle.importKey("raw", bs(raw), { name: "HMAC", hash: "SHA-256" }, false, [
        "sign",
        "verify",
    ]);
    return {
        alg: "HMAC-SHA256",
        async sign(data) {
            return new Uint8Array(await crypto.subtle.sign("HMAC", key, bs(data)));
        },
        async verify(data, sig) {
            if (sig.length !== 32) return false;
            return crypto.subtle.verify("HMAC", key, bs(sig), bs(data));
        },
    };
}

const MAX_TOKEN_CHARS = 8192;

/**
 * Token = base64url(payload JSON) "." base64url(signature). The signature covers a
 * domain-separation string plus the payload segment, so a confirmation token can never
 * verify as a mandate (or the reverse). The algorithm is fixed by the verifier's key,
 * never read from the token (no algorithm-confusion).
 */
export async function signToken(domain: string, payload: unknown, signer: Signer): Promise<string> {
    const seg = b64urlEncode(utf8(canonicalJson(payload)));
    const sig = await signer.sign(utf8(`${domain}\n${seg}`));
    return `${seg}.${b64urlEncode(sig)}`;
}

export type OpenResult = { ok: true; payload: unknown } | { ok: false; code: "malformed" | "bad-signature" };

export async function openToken(domain: string, token: unknown, verifier: Verifier): Promise<OpenResult> {
    if (typeof token !== "string" || token.length === 0 || token.length > MAX_TOKEN_CHARS) {
        return { ok: false, code: "malformed" };
    }
    const parts = token.split(".");
    if (parts.length !== 2) return { ok: false, code: "malformed" };
    const [seg, sigSeg] = parts as [string, string];
    const sig = b64urlDecode(sigSeg);
    const body = b64urlDecode(seg);
    if (!sig || !body) return { ok: false, code: "malformed" };
    if (!(await verifier.verify(utf8(`${domain}\n${seg}`), sig))) return { ok: false, code: "bad-signature" };
    const text = fromUtf8(body);
    if (text === null) return { ok: false, code: "malformed" };
    try {
        return { ok: true, payload: JSON.parse(text) as unknown };
    } catch {
        return { ok: false, code: "malformed" };
    }
}
