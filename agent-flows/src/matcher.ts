// Value relations: how a target value can be a function of a source value.
//
//   target == prefix + transform(source) + suffix
//
// The transform is one of a fixed set of encodings and prefix/suffix are literal text
// (`Bearer `, a cookie attribute). A relation is only used when it holds with the same
// transform, prefix and suffix in every recorded run; the classifier enforces that.

import { isEvidence, isWeakEvidence } from "./evidence.ts";
import { cmp, decodeComponentStrict } from "./util.ts";

/** Order is the tie-break order of relations (`Identity` first). */
export const TRANSFORMS = [
    "Identity",
    "UrlEncode",
    "FormEncode",
    "JsonEscape",
    "UrlDecode",
    "JsonUnescape",
    "Base64",
    "Base64NoPad",
    "Base64Url",
    "Base64UrlNoPad",
    "Base64Decode",
] as const;
export type Transform = (typeof TRANSFORMS)[number];

const NAMES: Record<Transform, string> = {
    Identity: "exact",
    UrlEncode: "url-encoded",
    FormEncode: "form-encoded",
    JsonEscape: "json-escaped",
    UrlDecode: "url-decoded",
    JsonUnescape: "json-unescaped",
    Base64: "base64",
    Base64NoPad: "base64-nopad",
    Base64Url: "base64url",
    Base64UrlNoPad: "base64url-nopad",
    Base64Decode: "base64-decoded",
};

export const transformName = (t: Transform): string => NAMES[t];
export const transformIndex = (t: Transform): number => TRANSFORMS.indexOf(t);

/** Decoding transforms: the result is the distinctive plain value. */
export const isDecode = (t: Transform): boolean => t === "UrlDecode" || t === "JsonUnescape" || t === "Base64Decode";

const printable = (s: string): boolean => s.length > 0 && !/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(s);

const b64 = (s: string, urlSafe: boolean, pad: boolean): string => {
    let out = Buffer.from(s, "utf8").toString(urlSafe ? "base64url" : "base64");
    if (urlSafe && pad) while (out.length % 4) out += "=";
    if (!urlSafe && !pad) out = out.replace(/=+$/, "");
    return out;
};

/** Canonical (strict) base64 decode: the input must be exactly what the encoder would produce. */
function b64Decode(s: string): string | null {
    for (const [urlSafe, pad] of [
        [false, true],
        [false, false],
        [true, true],
        [true, false],
    ] as const) {
        const alphabet = urlSafe ? /^[A-Za-z0-9_-]*=*$/ : /^[A-Za-z0-9+/]*=*$/;
        if (!alphabet.test(s)) continue;
        let bytes: Buffer;
        try {
            bytes = Buffer.from(s, urlSafe ? "base64url" : "base64");
        } catch {
            continue;
        }
        let text: string;
        try {
            text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
        } catch {
            continue;
        }
        if (b64(text, urlSafe, pad) === s && printable(text)) return text;
    }
    return null;
}

const FORM_EXTRA: Record<string, string> = { "!": "%21", "'": "%27", "(": "%28", ")": "%29", "~": "%7E" };

/** Apply `t` to `s`; null when it does not apply or would not change the value. */
export function apply(t: Transform, s: string): string | null {
    let out: string;
    switch (t) {
        case "Identity":
            return s;
        case "UrlEncode":
            try {
                out = encodeURIComponent(s);
            } catch {
                return null; // lone surrogate
            }
            break;
        case "FormEncode":
            try {
                out = encodeURIComponent(s).replace(/[!'()~]/g, (c) => FORM_EXTRA[c]!).replace(/%20/g, "+");
            } catch {
                return null;
            }
            break;
        case "JsonEscape":
            out = JSON.stringify(s).slice(1, -1);
            break;
        case "UrlDecode":
            if (!s.includes("%")) return null;
            {
                const d = decodeComponentStrict(s);
                if (d === null) return null;
                out = d;
            }
            break;
        case "JsonUnescape":
            if (!s.includes("\\")) return null;
            try {
                out = JSON.parse(`"${s}"`) as string;
            } catch {
                return null;
            }
            break;
        case "Base64":
            out = b64(s, false, true);
            break;
        case "Base64NoPad":
            out = b64(s, false, false);
            break;
        case "Base64Url":
            out = b64(s, true, true);
            break;
        case "Base64UrlNoPad":
            out = b64(s, true, false);
            break;
        case "Base64Decode": {
            const d = b64Decode(s);
            if (d === null) return null;
            out = d;
            break;
        }
    }
    return out !== s && out !== "" ? out : null;
}

/** The distinctive value of a (transform, source) pair: the source for encoders, the decoded result for decoders. */
export const plainOf = (t: Transform, source: string, transformed: string): string => (isDecode(t) ? transformed : source);

export interface Rel {
    transform: Transform;
    prefix: string;
    suffix: string;
}

export const relIsExact = (r: Rel): boolean => r.prefix === "" && r.suffix === "";
export const relKey = (r: Rel): string => `${transformIndex(r.transform)}\u0000${r.prefix}\u0000${r.suffix}`;
export const relCmp = (a: Rel, b: Rel): number =>
    transformIndex(a.transform) - transformIndex(b.transform) || cmp(a.prefix, b.prefix) || cmp(a.suffix, b.suffix);

/** Human-readable label recorded as an edge's `via`. */
export function relVia(r: Rel): string {
    if (relIsExact(r)) return transformName(r.transform);
    return r.transform === "Identity" ? "substring" : `${transformName(r.transform)}+substring`;
}

/** Exact relations only (target == transform(source)), used for run parameters. */
export function exactTransforms(target: string, source: string, requireEvidence: boolean): Transform[] {
    return TRANSFORMS.filter((t) => {
        const v = apply(t, source);
        return v !== null && v === target && (!requireEvidence || isEvidence(plainOf(t, source, v)));
    });
}

/**
 * Every relation `target == prefix + transform(source) + suffix` with an evidence-grade plain
 * value. Exact matches carry empty prefix and suffix; a substring match needs `target` strictly
 * longer than the transformed source.
 */
export function relations(target: string, source: string): Rel[] {
    const out: Rel[] = [];
    for (const t of TRANSFORMS) {
        const v = apply(t, source);
        if (v === null || !isWeakEvidence(plainOf(t, source, v))) continue;
        if (v === target) out.push({ transform: t, prefix: "", suffix: "" });
        else if (target.length > v.length) {
            const i = target.indexOf(v);
            if (i >= 0) out.push({ transform: t, prefix: target.slice(0, i), suffix: target.slice(i + v.length) });
        }
    }
    return out.sort(relCmp);
}

/** Whether the relation through `t` from `source` rests on a value that passes the strong evidence gate. */
export function strongRelation(t: Transform, source: string, transformed: string): boolean {
    return isEvidence(plainOf(t, source, transformed));
}

/** Candidate sources `s` such that `transform(s) == target`, found by inverting the transform on `target`. */
export function preimages(target: string): [Transform, string][] {
    const candidates = new Set<string>([target]);
    for (const t of ["UrlDecode", "JsonUnescape", "Base64Decode", "UrlEncode", "JsonEscape", "Base64", "Base64Url"] as const) {
        const c = apply(t, target);
        if (c !== null) candidates.add(c);
    }
    const out = new Map<string, [Transform, string]>();
    for (const c of [...candidates].sort(cmp)) {
        for (const t of TRANSFORMS) {
            const v = apply(t, c);
            if (v !== null && v === target && isWeakEvidence(plainOf(t, c, v))) out.set(`${t}\u0000${c}`, [t, c]);
        }
    }
    return [...out.values()].sort((a, b) => transformIndex(a[0]) - transformIndex(b[0]) || cmp(a[1], b[1]));
}
