/**
 * Page-text hygiene (D8). Strips what a human never sees but a model reads, and flags
 * (without silently rewriting) look-alike letters. Pure and deterministic.
 */

export type FlagKind =
    | "zero-width"
    | "bidi-control"
    | "unicode-tag"
    | "variation-selector"
    | "format-char"
    | "control-char"
    | "line-separator"
    | "compat-normalized"
    | "mixed-script"
    | "confusable-word"
    | "long-token"
    | "truncated";

export interface Flag {
    readonly kind: FlagKind;
    readonly count: number;
}

export interface SanitizeResult {
    readonly text: string;
    readonly flags: readonly Flag[];
    /**
     * Text that was hidden inside Unicode tag characters ("ASCII smuggling"), decoded for
     * audit only. Page-origin: never place it in a tool description or any surface field.
     */
    readonly quarantined: readonly string[];
}

export interface SanitizeOptions {
    readonly maxChars?: number;
    /** Replace confusable letters in flagged tokens by their Latin look-alike. */
    readonly foldConfusables?: boolean;
}

const DEFAULT_IGNORABLE = /\p{Default_Ignorable_Code_Point}/u;
const EMOJI = /\p{Extended_Pictographic}/u;
const ZERO_WIDTH = new Set([0x2800, 0x17b4, 0x17b5, 0x200b, 0x200c, 0x200d, 0x2060, 0xfeff, 0x180e, 0x00ad, 0x2061, 0x2062, 0x2063, 0x2064, 0x034f, 0x115f, 0x1160, 0x3164, 0xffa0]);
const BIDI = new Set([0x200e, 0x200f, 0x061c, 0x202a, 0x202b, 0x202c, 0x202d, 0x202e, 0x2066, 0x2067, 0x2068, 0x2069]);

/** Cyrillic/Greek letters that render like Latin ones. Not exhaustive; flags, not proofs. */
const CONFUSABLES: Readonly<Record<string, string>> = {
    "а": "a", "е": "e", "о": "o", "р": "p", "с": "c", "х": "x", "у": "y", "і": "i", "ј": "j", "ѕ": "s", "ԁ": "d", "ԛ": "q", "ԝ": "w", "һ": "h", "ӏ": "l",
    "А": "A", "В": "B", "Е": "E", "К": "K", "М": "M", "Н": "H", "О": "O", "Р": "P", "С": "C", "Т": "T", "Х": "X", "І": "I", "Ј": "J", "Ѕ": "S",
    "ο": "o", "ν": "v", "ρ": "p", "α": "a", "ι": "i", "κ": "k", "τ": "t", "υ": "u", "χ": "x",
    "Α": "A", "Β": "B", "Ε": "E", "Ζ": "Z", "Η": "H", "Ι": "I", "Κ": "K", "Μ": "M", "Ν": "N", "Ο": "O", "Ρ": "P", "Τ": "T", "Υ": "Y", "Χ": "X",
};

/** Scripts that have Latin look-alikes. Han/Hiragana/Katakana/Hangul/Bopomofo count as one (CJK text mixes them legitimately). */
const SCRIPT_TESTS: ReadonlyArray<readonly [string, RegExp]> = [
    ["Latin", /\p{Script=Latin}/u], ["Cyrillic", /\p{Script=Cyrillic}/u], ["Greek", /\p{Script=Greek}/u], ["Armenian", /\p{Script=Armenian}/u],
    ["Cherokee", /\p{Script=Cherokee}/u], ["Georgian", /\p{Script=Georgian}/u], ["Hebrew", /\p{Script=Hebrew}/u], ["Arabic", /\p{Script=Arabic}/u],
    ["Coptic", /\p{Script=Coptic}/u], ["Lisu", /\p{Script=Lisu}/u], ["Canadian_Aboriginal", /\p{Script=Canadian_Aboriginal}/u],
    ["Old_Italic", /\p{Script=Old_Italic}/u], ["Runic", /\p{Script=Runic}/u], ["Tifinagh", /\p{Script=Tifinagh}/u], ["Lycian", /\p{Script=Lycian}/u],
    ["Carian", /\p{Script=Carian}/u], ["Ogham", /\p{Script=Ogham}/u], ["Gothic", /\p{Script=Gothic}/u], ["Glagolitic", /\p{Script=Glagolitic}/u],
    ["Thai", /\p{Script=Thai}/u], ["Devanagari", /\p{Script=Devanagari}/u], ["Ethiopic", /\p{Script=Ethiopic}/u],
    ["CJK", /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}\p{Script=Bopomofo}]/u],
];
function scriptOf(ch: string): string | null {
    if (ch < "\u0080") return /[A-Za-z]/.test(ch) ? "Latin" : null;
    for (const [name, re] of SCRIPT_TESTS) if (re.test(ch)) return name;
    return null;
}
const LATIN = /\p{Script=Latin}/u;
const MAX_RAW_CHARS = 1_000_000;
const TOKEN = /[\p{L}\p{M}\p{N}_'-]+/gu;

function bump(m: Map<FlagKind, number>, k: FlagKind, n = 1): void {
    m.set(k, (m.get(k) ?? 0) + n);
}

function stripInvisible(input: string, flags: Map<FlagKind, number>, tags: number[][]): string {
    let out = "";
    let prevCp = -1; // last code point appended to `out` (kept so emoji-selector checks are O(1))
    let tagRun: number[] = [];
    const flushTags = () => {
        if (tagRun.length) tags.push(tagRun);
        tagRun = [];
    };
    const chars = Array.from(input);
    for (let i = 0; i < chars.length; i++) {
        const ch = chars[i]!;
        const cp = ch.codePointAt(0)!;
        if (cp >= 0xe0000 && cp <= 0xe007f) {
            bump(flags, "unicode-tag");
            tagRun.push(cp);
            continue;
        }
        flushTags();
        if (cp === 0x0a || cp === 0x09) {
            out += ch;
            prevCp = cp;
        } else if (cp === 0x0d || cp === 0x0b || cp === 0x0c || cp === 0x85 || cp === 0x2028 || cp === 0x2029) {
            bump(flags, "line-separator");
            out += "\n";
            prevCp = 0x0a;
        } else if (cp < 0x20 || cp === 0x7f || (cp >= 0x80 && cp <= 0x9f)) {
            bump(flags, "control-char");
        } else if (BIDI.has(cp)) {
            bump(flags, "bidi-control");
        } else if (ZERO_WIDTH.has(cp)) {
            bump(flags, "zero-width");
        } else if ((cp >= 0xfe00 && cp <= 0xfe0d) || (cp >= 0xe0100 && cp <= 0xe01ef)) {
            bump(flags, "variation-selector");
        } else if (cp === 0xfe0e || cp === 0xfe0f) {
            // emoji presentation selectors are only legitimate right after an emoji,
            // or right after a keycap base ("1" + FE0F + U+20E3)
            const next = chars[i + 1]?.codePointAt(0);
            if ((prevCp >= 0 && EMOJI.test(String.fromCodePoint(prevCp))) || next === 0x20e3) {
                out += ch;
                prevCp = cp;
            } else bump(flags, "variation-selector");
        } else if (DEFAULT_IGNORABLE.test(ch)) {
            bump(flags, "zero-width");
        } else if (cp === 0xfffc || cp === 0xfffd || (cp >= 0xfff9 && cp <= 0xfffb) || /\p{Cf}/u.test(ch) || /\p{Co}/u.test(ch) || /\p{Cn}/u.test(ch) || /\p{Cs}/u.test(ch)) {
            bump(flags, "format-char");
        } else {
            out += ch;
            prevCp = cp;
        }
    }
    flushTags();
    return out;
}

export function sanitizePageText(input: string, opts: SanitizeOptions = {}): SanitizeResult {
    const flags = new Map<FlagKind, number>();
    const tagRuns: number[][] = [];
    // Hard bound on work, applied before any per-character pass.
    let raw = input;
    if (raw.length > MAX_RAW_CHARS) {
        let cut = MAX_RAW_CHARS;
        const hi = raw.charCodeAt(cut - 1);
        if (hi >= 0xd800 && hi <= 0xdbff) cut -= 1; // never split a surrogate pair
        bump(flags, "truncated", raw.length - cut);
        raw = raw.slice(0, cut);
    }
    let text = stripInvisible(raw, flags, tagRuns);

    const folded = text.normalize("NFKC");
    if (folded !== text) {
        bump(flags, "compat-normalized");
        text = stripInvisible(folded, flags, tagRuns);
    }

    const max = opts.maxChars ?? 20_000;
    const cps = Array.from(text);
    if (cps.length > max) {
        bump(flags, "truncated", cps.length - max);
        text = cps.slice(0, max).join("");
    }

    // Look-alike detection per token.
    let result = "";
    let last = 0;
    for (const m of text.matchAll(TOKEN)) {
        const tok = m[0];
        const start = m.index ?? 0;
        result += text.slice(last, start);
        last = start + tok.length;
        if (tok.length > 200) bump(flags, "long-token");
        const scripts = new Set<string>();
        let hasLatin = false;
        let allMapped = true;
        let anyMapped = false;
        for (const ch of tok) {
            const sc = scriptOf(ch);
            if (sc) scripts.add(sc);
            if (sc === "Latin") hasLatin = true;
            if (CONFUSABLES[ch] !== undefined) anyMapped = true;
            else if (!/[\p{N}_'-]/u.test(ch) && !LATIN.test(ch)) allMapped = false;
        }
        let flagged = false;
        // Latin embedded in CJK or Thai running text (no spaces between words) is ordinary.
        const benign = [...scripts].every((s) => s === "Latin" || s === "CJK" || s === "Thai");
        if (scripts.size > 1 && !benign) {
            bump(flags, "mixed-script");
            flagged = true;
        } else if (anyMapped && allMapped && !hasLatin) {
            bump(flags, "confusable-word");
            flagged = true;
        }
        result += flagged && opts.foldConfusables ? Array.from(tok, (c) => CONFUSABLES[c] ?? c).join("") : tok;
    }
    result += text.slice(last);
    text = result;

    const quarantined = tagRuns
        .map((run) => run.map((cp) => (cp >= 0xe0020 && cp <= 0xe007e ? String.fromCharCode(cp - 0xe0000) : "")).join("").slice(0, 200))
        .filter((s) => s.length > 0);

    const order: FlagKind[] = ["zero-width", "bidi-control", "unicode-tag", "variation-selector", "format-char", "control-char", "line-separator", "compat-normalized", "mixed-script", "confusable-word", "long-token", "truncated"];
    return {
        text,
        flags: order.filter((k) => flags.has(k)).map((kind) => ({ kind, count: flags.get(kind)! })),
        quarantined,
    };
}
