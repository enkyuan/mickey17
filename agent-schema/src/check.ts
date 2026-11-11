// Invariants the wire format cannot express (it has no refinements). They run in code
// after parsing: cross-references, uniqueness, hash, coverage, money and secret rules,
// and rules tied to the mode. Each issue carries a stable `code` so fixtures assert the
// reason, not the wording.
import { z } from "zod";
import type { Action, Input } from "./action";
import type { ConfirmationRequest, ConfirmationToken, Mandate, Receipt } from "./authority";
import type { OperationResult } from "./result";
import type { Flow, FlowStep } from "./flow";
import { canonicalize, computeHash } from "./hash";
import { EFFECT_ORDER, EVIDENCE_ORDER, effectRank, requiresConfirmation } from "./policy";
import type { Effect, Tier } from "./policy";
import { SCHEMA_MAJOR, SCHEMA_MINOR } from "./primitives";
import { AgentSurface } from "./surface";
import type { Coverage } from "./surface";
import type { DependencyEdge, RequestSpec, ValueClass } from "./values";

export type IssuePath = (string | number)[];

export interface CheckIssue {
    path: IssuePath;
    code: string;
    message: string;
}

export const CHECK_CODES = [
    "hash_mismatch",
    "duplicate_id",
    "dangling_reference",
    "hidden_not_opted_in",
    "mode_forbids_tier",
    "mode_forbids_effect",
    "mode_forbids_flows",
    "gates_forbid_read_only",
    "read_only_requires_auth",
    "evidence_not_ordered",
    "coverage_mismatch",
    "coverage_not_canonical",
    "flow_effect_mismatch",
    "step_effect_mismatch",
    "dependency_not_earlier",
    "dependency_missing_step",
    "edge_needs_flow",
    "undeclared_input",
    "payment_effect_mismatch",
    "idempotency_key_missing",
    "idempotency_key_not_in_request",
    "time_order",
    "invalid_timestamp",
    "cap_order",
    "currency_mismatch",
    "total_mismatch",
    "tier_duplicated",
    "secret_constant",
    "card_number_constant",
    "password_not_credential",
    "card_input_forbidden",
    "sensitive_in_url",
    "page_text_in_surface",
    "ill_formed_string",
    "parent_cycle",
    "unclassified_has_value",
    "constraint_order",
    "enum_without_options",
    "schema_version_newer",
    "unsupported_tier_mismatch",
    "receipt_incomplete",
    "receipt_tier_effect",
    "amount_not_financial",
    "receipt_result_mismatch",
    "payment_token_not_financial",
    "human_gate_not_financial",
    "secret_in_url",
    "gates_unchecked",
] as const;
export type CheckCode = (typeof CHECK_CODES)[number];

const issues = () => {
    const list: CheckIssue[] = [];
    const add = (path: IssuePath, code: CheckCode, message: string) => list.push({ path, code, message });
    return { list, add };
};
type Add = ReturnType<typeof issues>["add"];

function duplicates<T>(items: readonly T[], key: (t: T) => string): number[] {
    const seen = new Set<string>();
    const dups: number[] = [];
    items.forEach((item, i) => {
        const k = key(item);
        if (seen.has(k)) dups.push(i);
        seen.add(k);
    });
    return dups;
}

// ---------------------------------------------------------------------------
// Schema version
// ---------------------------------------------------------------------------

export type VersionCheck = { ok: true } | { ok: false; reason: "malformed" | "major" | "minor" };

/** Reader rule: accept the same major and a minor no newer than this package's. */
export function checkSchemaVersion(version: string): VersionCheck {
    const m = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.exec(version);
    if (!m) return { ok: false, reason: "malformed" };
    if (Number(m[1]) !== SCHEMA_MAJOR) return { ok: false, reason: "major" };
    if (Number(m[2]) > SCHEMA_MINOR) return { ok: false, reason: "minor" };
    return { ok: true };
}

// ---------------------------------------------------------------------------
// Generic scan: ill-formed strings, timestamps, page text copied into surface text
// ---------------------------------------------------------------------------

/** Shared-run lengths: 8 consecutive words, or 16 consecutive characters of unspaced script (CJK, Thai). */
const WORD_RUN = 8;
const CHAR_RUN = 16;
/** A whole page string shorter than a run still counts when it is a real sentence: 3 to 7 words and 20+ characters. */
const WHOLE_MIN_WORDS = 3;
const WHOLE_MIN_CHARS = 20;
/** Candidates kept per 3-word key, so a flood of identical openings cannot make matching expensive. */
const WHOLE_PER_KEY = 64;
const UNSPACED = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Thai}\p{Script=Lao}\p{Script=Khmer}\p{Script=Myanmar}]/u;

/** Case-fold, NFKC, drop format/default-ignorable characters, and turn everything that is not a letter or digit into a space. */
const fold = (t: string): string => t.normalize("NFKC").toLowerCase().replace(/[\p{Cf}­]/gu, "").replace(/[^\p{L}\p{N}]+/gu, " ").trim();

/** 52-bit FNV-style hash of a string, so shingle sets hold numbers, not strings. */
function hash52(text: string): number {
    let h1 = 0x811c9dc5;
    let h2 = 0x01000193;
    for (let i = 0; i < text.length; i++) {
        const c = text.charCodeAt(i);
        h1 = Math.imul(h1 ^ c, 0x01000193) >>> 0;
        h2 = Math.imul(h2 + c, 0x85ebca6b) >>> 0;
    }
    return (h1 & 0x1fffff) * 0x100000000 + h2;
}

/** Hashes of every run of WORD_RUN words (spaced scripts) and CHAR_RUN characters (unspaced scripts) in `text`. */
function shingles(text: string, out: Set<number>, whole?: Map<string, string[]>) {
    const folded = fold(text);
    if (folded === "") return;
    if (whole) {
        const w = folded.split(" ");
        if (w.length >= WHOLE_MIN_WORDS && w.length < WORD_RUN && folded.length >= WHOLE_MIN_CHARS) {
            const key = w.slice(0, 3).join(" ");
            const list = whole.get(key);
            if (!list) whole.set(key, [folded]);
            else if (list.length < WHOLE_PER_KEY && !list.includes(folded)) list.push(folded);
        }
    }
    const unspaced: string[] = [];
    const wordsOnly = folded.replace(new RegExp(UNSPACED.source, "gu"), (m) => {
        unspaced.push(m);
        return " ";
    });
    const words = wordsOnly.split(" ").filter(Boolean);
    for (let i = 0; i + WORD_RUN <= words.length; i++) out.add(hash52(`w${words.slice(i, i + WORD_RUN).join(" ")}`));
    const chars = [...unspaced.join("")];
    for (let i = 0; i + CHAR_RUN <= chars.length; i++) out.add(hash52(`c${chars.slice(i, i + CHAR_RUN).join("")}`));
}

/**
 * Scans any document. Flags (1) ill-formed UTF-16 (a lone surrogate cannot round-trip through
 * UTF-8 JSON), (2) timestamps that are not real instants (keys ending `_at`, and a top-level `at`),
 * and (3) surface-authored text that shares a run of 8 words (16 characters of unspaced script) with
 * page text, or contains a whole short page sentence (3-7 words, 20+ characters), after case-folding and with punctuation, whitespace and format characters ignored
 * (D8 property: page text never appears in a surface-origin field). Page strings shorter than a run
 * never count, so ordinary headings do not trip it. Cost is linear in the text: every window is hashed once.
 * It catches copying, including a sentence lifted out of a longer paragraph and quoting or punctuation
 * changes; it does not catch paraphrase. A tripwire, not a proof.
 */
function scan(doc: unknown, add: Add) {
    const page = new Set<number>();
    const pageWhole = new Map<string, string[]>();
    const surface: { path: IssuePath; value: string }[] = [];
    const walk = (n: unknown, path: IssuePath) => {
        if (typeof n === "string") {
            if (!n.isWellFormed()) add(path, "ill_formed_string", "string contains a lone surrogate");
            return;
        }
        if (Array.isArray(n)) return n.forEach((x, i) => walk(x, [...path, i]));
        if (!n || typeof n !== "object") return;
        const o = n as Record<string, unknown>;
        if (typeof o.value === "string" && (o.origin === "page" || o.origin === "surface")) {
            if (o.origin === "page") shingles(o.value, page, pageWhole);
            else surface.push({ path: [...path, "value"], value: o.value });
        }
        for (const [k, v] of Object.entries(o)) {
            if ((k.endsWith("_at") || (k === "at" && path.length === 0)) && typeof v === "string" && /^[0-9]{4}-/.test(v)) {
                const t = new Date(v);
                if (Number.isNaN(t.getTime()) || `${t.toISOString().slice(0, 19)}Z` !== v) {
                    add([...path, k], "invalid_timestamp", `${v} is not a real instant`);
                }
            }
            walk(v, [...path, k]);
        }
    };
    walk(doc, []);
    if (page.size === 0 && pageWhole.size === 0) return;
    for (const s of surface) {
        const mine = new Set<number>();
        shingles(s.value, mine);
        let hit = [...mine].some((h) => page.has(h));
        if (!hit && pageWhole.size > 0) {
            const folded = ` ${fold(s.value)} `;
            const words = folded.trim().split(" ");
            const seen = new Set<string>();
            for (let i = 0; i + 3 <= words.length && !hit; i++) {
                const key = words.slice(i, i + 3).join(" ");
                if (seen.has(key)) continue;
                seen.add(key);
                hit = pageWhole.get(key)?.some((c) => folded.includes(` ${c} `)) ?? false;
            }
        }
        if (hit) add(s.path, "page_text_in_surface", "surface-authored text repeats page text");
    }
}

// ---------------------------------------------------------------------------
// Secrets and instruments (D7, D11)
// ---------------------------------------------------------------------------

// --- name and value detectors (exported for tests; heuristics, documented in the README) ---

/** Split a name into lowercase tokens: camelCase, `_`, `-`, `.`, `[`, `]`, `/` all separate. */
export const nameTokens = (name: string): string[] =>
    name
        .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
        .toLowerCase()
        .split(/[^a-z0-9]+/)
        .filter(Boolean);

const SECRET_WORDS = new Set(["password", "passwd", "pwd", "passcode", "secret", "jwt", "bearer", "credential", "credentials", "sid", "sess", "cvv", "cvc", "ccv"]);
/** A token word counts as a credential unless the name says it is about the token, not the secret (token_type, max_tokens...). */
const NOT_A_SECRET = new Set(["type", "max", "min", "count", "limit", "length", "ttl", "hint", "provider", "policy", "timeout", "duration", "expires", "lifetime", "method", "terms", "reference", "date", "enabled", "required", "page", "next", "cursor", "continuation", "sync", "cart", "checkout"]);

/** True when a header, query, form, cookie or JSON-pointer name denotes a credential. */
export function isSecretName(name: string): boolean {
    const segments = name.split("/").filter(Boolean);
    const last = [...segments].reverse().find((x) => !/^[0-9]+$/.test(x)) ?? name;
    const t = nameTokens(last);
    if (t.some((w) => NOT_A_SECRET.has(w))) return false; // password_hint, token_type, max_tokens, session_length...
    if (t.some((w) => SECRET_WORDS.has(w))) return true;
    if (t.length === 1 && (t[0] === "pin" || t[0] === "otp")) return true;
    const pair = (a: string, b: string) => t.some((w, i) => w === a && t[i + 1] === b);
    if (pair("api", "key") || pair("access", "key") || pair("private", "key") || pair("secret", "key") || t.includes("apikey")) return true;
    return t.some((w) => w === "token" || w === "auth" || w === "authorization" || w === "session" || w === "signature" || w === "sig");
}

/** Constant values that are credentials whatever the field is called. */
const SECRET_SHAPE =
    /^bearer\s+[A-Za-z0-9._~+/=-]{16,}|^basic\s+[A-Za-z0-9+/=]{12,}$|\bsk_(live|test)_[A-Za-z0-9]+|\brk_(live|test)_[A-Za-z0-9]+|^eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*$|^AKIA[0-9A-Z]{16}$|\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}|\bxox[abprs]-[A-Za-z0-9-]{10,}|\bAIza[0-9A-Za-z_-]{20,}|-----BEGIN [A-Z ]*PRIVATE KEY-----/i;
export const isSecretValue = (v: string): boolean => SECRET_SHAPE.test(v);

const CARD_NAME_TOKENS = new Set(["cvv", "cvv2", "cvc", "cvc2", "ccv", "cid"]);
/** Card-ish input or field names: number/num/no after card or cc, credit card, pan next to card, cvv, expiry month/year. */
export function isCardName(name: string): boolean {
    const t = nameTokens(name.split("/").filter(Boolean).pop() ?? name);
    if (t.some((w) => CARD_NAME_TOKENS.has(w))) return true;
    const joined = t.join("_");
    if (/(^|_)(card|cc)_?(number|num|no|pan)\d*($|_)/.test(joined) || /(^|_)(cardnumber|ccnum|cardnum)\d*($|_)/.test(joined)) return true;
    if (/(^|_)credit_?card($|_)/.test(joined) || /(^|_)security_code($|_)/.test(joined) || /(^|_)primary_account_number($|_)/.test(joined)) return true;
    if (/(^|_)(card|cc)_exp/.test(joined) || /(^|_)(exp|expiry|expiration)_(month|year|mm|yy|yyyy)($|_)/.test(joined)) return true;
    return t.includes("card") && t.includes("pan");
}

/** Issuer prefixes that make a Luhn-valid 15-19 digit run plausibly a payment card. */
function looksLikeCard(digits: string): boolean {
    if (!/^[0-9]{15,19}$/.test(digits) || !luhn(digits)) return false;
    return /^(4|5[0-8]|2[2-7]|3[047-9]|6)/.test(digits);
}

const CARD_WORDS = new Set(["card", "cards", "credit", "cc", "pan"]);
/** True when a name or URL contains a whole word that denotes a payment card (not a substring: `company`, `span`). */
const hasCardWord = (text: string): boolean => nameTokens(text).some((w) => CARD_WORDS.has(w));

/** Card number in a constant: grouped like a card, or in a field that is named like one. */
export function isCardConstant(value: string, fieldName: string): boolean {
    const grouped = /^[0-9]{4}([ .-])[0-9]{4}\1[0-9]{4}\1[0-9]{3,4}(\1[0-9]{1,3})?$/.test(value.trim());
    const digits = value.replace(/[ .-]/g, "");
    return looksLikeCard(digits) && (grouped || isCardName(fieldName) || hasCardWord(fieldName));
}

function luhn(digits: string): boolean {
    let sum = 0;
    let alt = false;
    for (let i = digits.length - 1; i >= 0; i--) {
        let d = digits.charCodeAt(i) - 48;
        if (alt) {
            d *= 2;
            if (d > 9) d -= 9;
        }
        sum += d;
        alt = !alt;
    }
    return sum % 10 === 0;
}

/** Credential or payment inputs must say so: by name (password, secret, api key, bearer/access tokens, PAN-like), not only by label. */
function sensitiveNameNeedsLabel(inp: Input): boolean {
    if (inp.sensitivity === "credential" || inp.sensitivity === "payment-token") return false;
    if (inp.type === "password") return true;
    const t = nameTokens(inp.name);
    if (t.some((w) => NOT_A_SECRET.has(w))) return false; // payment_method, payment_terms, session_date, auth_method, otp_enabled
    if (t.some((w) => ["password", "passwd", "pwd", "passcode", "secret", "jwt", "credential", "credentials", "bearer"].includes(w))) return true;
    const pair = (a: string, b: string) => t.some((w, i) => w === a && t[i + 1] === b);
    if (pair("api", "key") || pair("access", "key") || pair("private", "key") || pair("secret", "key") || t.includes("apikey")) return true;
    if (t.includes("token") && t.some((w) => ["auth", "access", "id", "refresh", "bearer", "payment", "pay", "card", "api"].includes(w))) return true;
    if (t.length === 1 && (t[0] === "token" || t[0] === "pin" || t[0] === "otp")) return true;
    // payment together with token / intent / nonce is a payment credential; payment_method, payment_terms are not.
    return t.includes("payment") && t.some((w) => w === "token" || w === "intent" || w === "nonce" || w === "credential");
}

/** decodeURIComponent that never throws: page-controlled strings may hold invalid escapes. */
function safeDecode(text: string): string {
    try {
        return decodeURIComponent(text);
    } catch {
        return text;
    }
}

/** Rules for a URL or URL template found anywhere (requests, routes, hrefs, navigation targets). */
function checkUrl(path: IssuePath, url: string, add: Add) {
    let bad = false;
    if (/\/\/[^/\s@:]+:[^/\s@]*@/.test(url)) bad = true; // userinfo (user part excludes ":" so the match cannot backtrack quadratically)
    const qi = url.indexOf("?");
    if (qi >= 0) {
        for (const pair of url.slice(qi + 1).split(/[&;]/)) {
            const [name = "", value = ""] = pair.split("=");
            if (/^\{[^}]*\}$/.test(value)) continue; // a template placeholder, not a value
            if (name && isSecretName(safeDecode(name))) bad = true;
        }
    }
    if (isSecretValue(url) || url.split(/[/?&=;#]/).some((seg) => isSecretValue(seg))) bad = true;
    if (bad) add(path, "secret_in_url", "a credential must not appear in a URL");
    const stripped = url.replace(/%20|[ .-]/g, "");
    for (const run of stripped.match(/[0-9]{15,19}/g) ?? []) {
        if (looksLikeCard(run) && (hasCardWord(url) || /[0-9]{4}[ .-][0-9]{4}[ .-][0-9]{4}/.test(url))) {
            add(path, "card_number_constant", "URL contains what looks like a payment card number");
            break;
        }
    }
}

function checkInputs(path: IssuePath, inputs: readonly Input[], add: Add) {
    duplicates(inputs, (i) => i.name).forEach((i) => add([...path, i, "name"], "duplicate_id", "duplicate input name"));
    inputs.forEach((inp, i) => {
        const p = [...path, i];
        if (isCardName(inp.name)) add([...p, "name"], "card_input_forbidden", "card data never passes through the surface; use a payment-token input");
        if (sensitiveNameNeedsLabel(inp)) add([...p, "sensitivity"], "password_not_credential", "a credential, secret or payment input must be sensitivity credential or payment-token");
        if (inp.min !== undefined && inp.max !== undefined && inp.min > inp.max) add([...p, "min"], "constraint_order", "min exceeds max");
        if (inp.min_length !== undefined && inp.max_length !== undefined && inp.min_length > inp.max_length) add([...p, "min_length"], "constraint_order", "min_length exceeds max_length");
        if (inp.type === "enum" && (!inp.options || inp.options.length === 0)) add([...p, "options"], "enum_without_options", "an enum input lists its options");
    });
}

function edgesOf(value: ValueClass): { edge: DependencyEdge; sub: IssuePath }[] {
    if (value.class === "derived") return [{ edge: value.edge, sub: ["edge"] }];
    if (value.class === "client-computed" && value.resolution.kind === "lift") {
        return value.resolution.args.map((edge, j) => ({ edge, sub: ["resolution", "args", j] }));
    }
    return [];
}

/** Rules every request obeys, in or out of a flow. `effect` is the effect of the action or step that sends it. */
function checkRequest(path: IssuePath, request: RequestSpec, inputs: readonly Input[], effect: Effect, add: Add, onEdge: (edge: DependencyEdge, p: IssuePath) => void) {
    const byName = new Map(inputs.map((i) => [i.name, i]));
    checkUrl([...path, "url", "value"], request.url.value, add);
    request.values.forEach((rv, i) => {
        const vp = [...path, "values", i, "value"];
        const v = rv.value;
        const inUrl = rv.at.in === "query" || rv.at.in === "path";
        if (v.class === "input") {
            const inp = byName.get(v.input);
            if (!inp) add([...vp, "input"], "undeclared_input", `input ${v.input} is not declared`);
            else {
                if ((inp.sensitivity === "credential" || inp.sensitivity === "payment-token") && inUrl) {
                    add([...path, "values", i, "at", "in"], "sensitive_in_url", "a credential or payment token must not be placed in the URL");
                }
                if (inp.sensitivity === "payment-token" && effect !== "financial") {
                    add([...vp, "input"], "payment_token_not_financial", "a request that carries a payment token is financial");
                }
            }
        }
        if (v.class === "constant") {
            const raw = String(v.value.value);
            if (isSecretName(rv.at.name) || isSecretValue(raw)) {
                add([...vp, "value"], "secret_constant", "a credential or session value must be supplied at run time, not embedded");
            }
            if (isCardConstant(raw, rv.at.name)) add([...vp, "value"], "card_number_constant", "constant looks like a payment card number");
        }
        if (v.class === "derived" && inUrl) {
            const src = v.edge.source;
            const secretSource =
                (src.kind === "cookie" && isSecretName(src.name)) ||
                (src.kind === "storage" && isSecretName(src.key)) ||
                (src.kind === "response" && (src.at.in === "set-cookie" || (src.at.in === "header" && isSecretName(src.at.name))));
            if (secretSource) add([...path, "values", i, "at", "in"], "sensitive_in_url", "a credential taken from a cookie, storage or response must not be placed in the URL");
        }
        if (v.class === "client-computed" && v.resolution.kind === "requires_browser" && v.resolution.reason === "tokenization" && effect !== "financial") {
            add([...vp, "resolution", "reason"], "payment_token_not_financial", "payment tokenisation belongs to a financial request");
        }
        edgesOf(v).forEach(({ edge, sub }) => onEdge(edge, [...vp, ...sub]));
    });
}

// ---------------------------------------------------------------------------
// Evidence and money rules shared by actions and flow steps
// ---------------------------------------------------------------------------

function evidenceRules(path: IssuePath, evidence: readonly { source: string }[], add: Add) {
    let last = -1;
    evidence.forEach((e, i) => {
        const rank = EVIDENCE_ORDER.indexOf(e.source as (typeof EVIDENCE_ORDER)[number]);
        if (rank < last) add([...path, i, "source"], "evidence_not_ordered", "evidence must be ordered declared, observed, derived, inferred");
        last = Math.max(last, rank);
    });
}

function idempotencyRules(
    path: IssuePath,
    effect: Effect,
    idem: { idempotent: boolean; key?: { in: string; name: string } | undefined },
    requests: readonly RequestSpec[],
    add: Add,
) {
    if (effect === "financial" && idem.key === undefined) {
        add([...path, "idempotency"], "idempotency_key_missing", "a money-moving call carries an idempotency key (D11)");
    }
    if (idem.key !== undefined) {
        const key = idem.key;
        const found = requests.some((r) => r.values.some((rv) => rv.at.in === key.in && rv.at.name === key.name && rv.value.class === "input"));
        if (!found) add([...path, "idempotency", "key"], "idempotency_key_not_in_request", "the key location must be a caller-supplied input in the request");
    }
}

// ---------------------------------------------------------------------------
// Flows and actions
// ---------------------------------------------------------------------------

interface DocIndex {
    nodeIds: ReadonlySet<string>;
    routeIds: ReadonlySet<string>;
    actions: ReadonlyMap<string, Action>;
}

function postconditionRefs(path: IssuePath, pc: Action["postconditions"][number], idx: DocIndex, add: Add, inputs: readonly Input[]) {
    if (pc.type === "response_field_equals_input" && !inputs.some((i) => i.name === pc.input)) add([...path, "input"], "undeclared_input", `input ${pc.input} is not declared`);
    if ((pc.type === "node_present" || pc.type === "node_text_changed") && !idx.nodeIds.has(pc.node)) add([...path, "node"], "dangling_reference", `unknown node ${pc.node}`);
    if (pc.type === "navigates_to" && !idx.routeIds.has(pc.route)) add([...path, "route"], "dangling_reference", `unknown route ${pc.route}`);
}

function checkFlow(flow: Flow, fi: number, idx: DocIndex, add: Add) {
    const p: IssuePath = ["flows", fi];
    checkInputs([...p, "inputs"], flow.inputs, add);
    duplicates(flow.steps, (s) => s.id).forEach((i) => add([...p, "steps", i, "id"], "duplicate_id", "duplicate step id"));

    const stepIndex = new Map<string, number>();
    flow.steps.forEach((s, i) => {
        if (!stepIndex.has(s.id)) stepIndex.set(s.id, i);
    });
    const requestsOf = new Map<string, FlowStep["requests"]>();
    flow.steps.forEach((s) => { if (!requestsOf.has(s.id)) requestsOf.set(s.id, s.requests); });
    const allRequestIds = flow.steps.flatMap((s, si) => s.requests.map((r, ri) => ({ id: r.id, path: [...p, "steps", si, "requests", ri, "id"] as IssuePath })));
    duplicates(allRequestIds, (r) => r.id).forEach((i) => add(allRequestIds[i]!.path, "duplicate_id", "duplicate request id within flow"));

    const edgeRule = (si: number, ri: number | null) => (edge: DependencyEdge, ep: IssuePath) => {
        const src = edge.source;
        if (src.step === undefined) {
            add([...ep, "source"], "dependency_missing_step", "in a flow every dependency names the step it was observed at");
            return;
        }
        const srcStep = stepIndex.get(src.step);
        if (srcStep === undefined) {
            add([...ep, "source", "step"], "dangling_reference", `dependency refers to unknown step ${src.step}`);
            return;
        }
        if (srcStep > si) add([...ep, "source", "step"], "dependency_not_earlier", "a value can only be sourced from the same or an earlier step");
        if (src.kind === "response") {
            const reqs = requestsOf.get(src.step) ?? [];
            const target = reqs.findIndex((r) => r.id === src.request);
            if (target < 0) add([...ep, "source", "request"], "dangling_reference", `dependency refers to unknown request ${src.request} in step ${src.step}`);
            else if (srcStep === si && (ri === null || target >= ri)) add([...ep, "source", "request"], "dependency_not_earlier", "a request or precondition can only depend on an earlier request");
        }
        if (src.kind === "dom" && !idx.nodeIds.has(src.node)) add([...ep, "source", "node"], "dangling_reference", `dependency refers to unknown node ${src.node}`);
    };

    let top = 0;
    flow.steps.forEach((step, si) => {
        const sp = [...p, "steps", si];
        top = Math.max(top, effectRank(step.effect));
        if (step.action !== undefined) {
            const a = idx.actions.get(step.action);
            if (!a) add([...sp, "action"], "dangling_reference", `step refers to unknown action ${step.action}`);
            else if (step.effect !== a.effect) add([...sp, "effect"], "step_effect_mismatch", `step effect differs from its action's effect (${a.effect})`);
        }
        if ((step.effect === "financial") !== (step.payment !== undefined)) {
            add([...sp, step.payment !== undefined ? "payment" : "effect"], "payment_effect_mismatch", "exactly the financial step declares a payment method");
        }
        idempotencyRules(sp, step.effect, step.idempotency, step.requests, add);
        if (step.effect !== "financial" && step.human_gates.some((g) => g === "3ds" || g === "sca")) {
            add([...sp, "human_gates"], "human_gate_not_financial", "a 3-D Secure or SCA challenge belongs to a financial step");
        }
        step.preconditions.forEach((pc, i) => {
            if (pc.type === "prior_action" && !idx.actions.has(pc.action)) add([...sp, "preconditions", i, "action"], "dangling_reference", `unknown action ${pc.action}`);
            if (pc.type === "token") edgeRule(si, null)(pc.source, [...sp, "preconditions", i, "source"]);
        });
        step.postconditions.forEach((pc, i) => postconditionRefs([...sp, "postconditions", i], pc, idx, add, flow.inputs));
        step.requests.forEach((req, ri) => checkRequest([...sp, "requests", ri], req, flow.inputs, step.effect, add, edgeRule(si, ri)));
    });
    if (flow.effect !== EFFECT_ORDER[top]) add([...p, "effect"], "flow_effect_mismatch", "flow effect must equal the most consequential step effect");
    evidenceRules([...p, "evidence"], flow.evidence, add);
    if (flow.status === "blocked") {
        flow.unclassified.forEach((u, i) => {
            const reqs = requestsOf.get(u.step);
            const req = reqs?.find((r) => r.id === u.request);
            if (!reqs) add([...p, "unclassified", i, "step"], "dangling_reference", `unknown step ${u.step}`);
            else if (!req) add([...p, "unclassified", i, "request"], "dangling_reference", `unknown request ${u.request}`);
            else if (req.values.some((rv) => rv.at.in === u.at.in && rv.at.name === u.at.name)) {
                add([...p, "unclassified", i, "at"], "unclassified_has_value", "a value listed as unclassified is also classed in its request");
            }
        });
    }
}

function checkAction(action: Action, ai: number, idx: DocIndex, add: Add) {
    const p: IssuePath = ["actions", ai];
    if (action.node !== undefined && !idx.nodeIds.has(action.node)) add([...p, "node"], "dangling_reference", `unknown node ${action.node}`);
    if (action.kind === "navigate") checkUrl([...p, "target", "value"], action.target.value, add);
    if (action.kind === "navigate" && action.route !== undefined && !idx.routeIds.has(action.route)) add([...p, "route"], "dangling_reference", `unknown route ${action.route}`);
    if (action.kind === "state") action.affects.forEach((n, i) => { if (!idx.nodeIds.has(n)) add([...p, "affects", i], "dangling_reference", `unknown node ${n}`); });
    checkInputs([...p, "inputs"], action.inputs, add);

    // A standalone action has no steps: only dom, cookie and storage sources, with no step.
    const edgeRule = (edge: DependencyEdge, ep: IssuePath) => {
        const src = edge.source;
        if (src.kind === "response") add([...ep, "source", "kind"], "edge_needs_flow", "a response source needs a flow step; model it in a flow");
        else if (src.step !== undefined) add([...ep, "source", "step"], "dangling_reference", "a standalone action has no steps");
        if (src.kind === "dom" && !idx.nodeIds.has(src.node)) add([...ep, "source", "node"], "dangling_reference", `dependency refers to unknown node ${src.node}`);
    };
    action.preconditions.forEach((pc, i) => {
        if (pc.type === "prior_action" && !idx.actions.has(pc.action)) add([...p, "preconditions", i, "action"], "dangling_reference", `unknown action ${pc.action}`);
        if (pc.type === "prior_action" && pc.action === action.id) add([...p, "preconditions", i, "action"], "dependency_not_earlier", "an action cannot be its own prerequisite");
        if (pc.type === "token") edgeRule(pc.source, [...p, "preconditions", i, "source"]);
    });
    action.postconditions.forEach((pc, i) => postconditionRefs([...p, "postconditions", i], pc, idx, add, action.inputs));
    if ("request" in action) {
        checkRequest([...p, "request"], action.request, action.inputs, action.effect, add, edgeRule);
        if ((action.effect === "financial") !== (action.payment !== undefined)) {
            add([...p, action.payment !== undefined ? "payment" : "effect"], "payment_effect_mismatch", "exactly a financial action declares a payment method");
        }
        idempotencyRules(p, action.effect, action.idempotency, [action.request], add);
    } else if (action.idempotency.key !== undefined) {
        add([...p, "idempotency", "key"], "idempotency_key_not_in_request", "an action without a request has no key location");
    }
    evidenceRules([...p, "evidence"], action.evidence, add);
}

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

const TIER_ORDER: readonly Tier[] = ["T0", "T1", "T2", "T3", "T4"];

/** The coverage a document implies: tiers with any content, in tier order; reasons sorted by name. */
export function deriveCoverage(doc: Pick<AgentSurface, "routes" | "actions" | "unsupported">): Coverage {
    const count = new Map<Tier, { routes: number; actions: number; unsupported: number }>();
    const bump = (t: Tier, f: "routes" | "actions" | "unsupported") => {
        const e = count.get(t) ?? { routes: 0, actions: 0, unsupported: 0 };
        e[f]++;
        count.set(t, e);
    };
    doc.routes.forEach((r) => new Set(r.tiers).forEach((t) => bump(t, "routes")));
    doc.actions.forEach((a) => bump(a.tier, "actions"));
    doc.unsupported.forEach((u) => bump(u.tier, "unsupported"));
    const reasons = new Map<string, number>();
    doc.unsupported.forEach((u) => reasons.set(u.reason, (reasons.get(u.reason) ?? 0) + 1));
    return {
        tiers: TIER_ORDER.filter((t) => count.has(t)).map((t) => ({ tier: t, ...count.get(t)! })),
        unsupported_reasons: [...reasons.entries()]
            .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
            .map(([reason, n]) => ({ reason: reason as Coverage["unsupported_reasons"][number]["reason"], count: n })),
    };
}

const OPAQUE_REASONS: ReadonlySet<string> = new Set(["opaque-canvas", "opaque-webgl", "drag-only", "video"]);

/** Hash of a document, or undefined when it cannot be canonicalised (the scan reports why). */
function safeHash(doc: { hash: string }): string | undefined {
    try {
        return computeHash(doc);
    } catch {
        return undefined;
    }
}

/** Checks a structurally valid surface. Returns [] when every invariant holds. */
export function checkSurface(doc: AgentSurface): CheckIssue[] {
    const { list, add } = issues();

    const hash = safeHash(doc);
    if (hash !== undefined && hash !== doc.hash) add(["hash"], "hash_mismatch", "hash does not match the canonical content");
    if (!checkSchemaVersion(doc.schema_version).ok) add(["schema_version"], "schema_version_newer", `this reader supports schema ${SCHEMA_MAJOR}.0 to ${SCHEMA_MAJOR}.${SCHEMA_MINOR}`);
    scan(doc, add);

    duplicates(doc.routes, (r) => r.id).forEach((i) => add(["routes", i, "id"], "duplicate_id", "duplicate route id"));
    const allNodes = doc.routes.flatMap((r, ri) => r.nodes.map((n, ni) => ({ n, path: ["routes", ri, "nodes", ni] as IssuePath })));
    duplicates(allNodes, (x) => x.n.id).forEach((i) => add([...allNodes[i]!.path, "id"], "duplicate_id", "duplicate node id"));
    const ids = [
        ...doc.actions.map((a, i) => ({ id: a.id, path: ["actions", i, "id"] as IssuePath })),
        ...doc.unsupported.map((a, i) => ({ id: a.id, path: ["unsupported", i, "id"] as IssuePath })),
    ];
    duplicates(ids, (x) => x.id).forEach((i) => add(ids[i]!.path, "duplicate_id", "duplicate action id"));
    duplicates(doc.flows, (f) => f.id).forEach((i) => add(["flows", i, "id"], "duplicate_id", "duplicate flow id"));

    const idx: DocIndex = {
        nodeIds: new Set(allNodes.map((x) => x.n.id)),
        routeIds: new Set(doc.routes.map((r) => r.id)),
        actions: new Map(doc.actions.map((a) => [a.id, a])),
    };

    doc.routes.forEach((route, ri) => {
        const parentOf = new Map(route.nodes.map((n) => [n.id, n.parent]));
        // Resolve every node once: a node is "bad" if its parent chain loops. O(n) overall.
        const state = new Map<string, "ok" | "loop">();
        for (const start of route.nodes) {
            if (state.has(start.id)) continue;
            const chain: string[] = [];
            const inChain = new Set<string>();
            let cur: string | undefined = start.id;
            let verdict: "ok" | "loop" = "ok";
            while (cur !== undefined) {
                const known = state.get(cur);
                if (known) {
                    verdict = known;
                    break;
                }
                if (inChain.has(cur)) {
                    verdict = "loop";
                    break;
                }
                inChain.add(cur);
                chain.push(cur);
                cur = parentOf.get(cur);
            }
            for (const id of chain) state.set(id, verdict);
        }
        route.nodes.forEach((n, ni) => {
            const np = ["routes", ri, "nodes", ni];
            if (n.parent !== undefined && !parentOf.has(n.parent)) add([...np, "parent"], "dangling_reference", "parent must be a node of the same route");
            else if (n.parent !== undefined && state.get(n.id) === "loop") add([...np, "parent"], "parent_cycle", "a node cannot be its own ancestor");
            if (n.href) checkUrl([...np, "href", "value"], n.href.value, add);
            if (n.visibility !== "visible" && !doc.options.include_hidden) add([...np, "visibility"], "hidden_not_opted_in", "hidden or inert nodes require options.include_hidden");
        });
        checkUrl(["routes", ri, "url", "value"], route.url.value, add);
        if (route.canonical) checkUrl(["routes", ri, "canonical", "value"], route.canonical.value, add);
        if (new Set(route.tiers).size !== route.tiers.length) add(["routes", ri, "tiers"], "tier_duplicated", "tiers must be unique");
    });

    doc.actions.forEach((a, i) => checkAction(a, i, idx, add));
    doc.unsupported.forEach((u, i) => {
        if (u.node !== undefined && !idx.nodeIds.has(u.node)) add(["unsupported", i, "node"], "dangling_reference", `unknown node ${u.node}`);
        if (OPAQUE_REASONS.has(u.reason) && u.tier !== "T4") add(["unsupported", i, "tier"], "unsupported_tier_mismatch", `${u.reason} is opaque: tier T4`);
    });
    doc.flows.forEach((f, i) => checkFlow(f, i, idx, add));

    // D2: a read-only surface carries T0/T1 only, needs no auth, and is not built where robots or ToS forbid it.
    if (doc.mode === "read-only") {
        const allowed: ReadonlySet<Tier> = new Set<Tier>(["T0", "T1"]);
        doc.actions.forEach((a, i) => {
            if (!allowed.has(a.tier)) add(["actions", i, "tier"], "mode_forbids_tier", "read-only surfaces carry T0/T1 only");
            else if (a.auth.kind !== "none") add(["actions", i, "auth"], "read_only_requires_auth", "read-only surfaces carry no authenticated actions");
            a.preconditions.forEach((pc, pi) => {
                const authLike = pc.type === "authenticated" || (pc.type === "token" && (pc.source.source.kind === "cookie" || pc.source.source.kind === "storage"));
                if (authLike) add(["actions", i, "preconditions", pi], "read_only_requires_auth", "read-only surfaces carry no authenticated actions");
            });
        });
        doc.routes.forEach((r, ri) => r.tiers.forEach((t, ti) => { if (!allowed.has(t)) add(["routes", ri, "tiers", ti], "mode_forbids_tier", "read-only surfaces carry T0/T1 only"); }));
        if (doc.flows.length > 0) add(["flows"], "mode_forbids_flows", "read-only surfaces carry no flows");
        if (doc.gates.robots_txt === "not-checked") add(["gates", "robots_txt"], "gates_unchecked", "a read-only surface requires robots.txt to have been checked");
        if (doc.gates.tos === "not-checked") add(["gates", "tos"], "gates_unchecked", "a read-only surface requires the terms of service to have been checked");
        if (doc.gates.robots_txt === "disallowed") add(["gates", "robots_txt"], "gates_forbid_read_only", "robots.txt disallows access");
        if (doc.gates.tos === "restricted") add(["gates", "tos"], "gates_forbid_read_only", "terms of service restrict automated access");
    }

    // Coverage must equal what the document contains, in exactly one encoding.
    const expected = deriveCoverage(doc);
    const before = list.length;
    const expectedTiers = new Map(expected.tiers.map((t) => [t.tier, t]));
    const seenTiers = new Set<Tier>();
    doc.coverage.tiers.forEach((c, i) => {
        const e = expectedTiers.get(c.tier) ?? { tier: c.tier, routes: 0, actions: 0, unsupported: 0 };
        if (seenTiers.has(c.tier)) add(["coverage", "tiers", i, "tier"], "tier_duplicated", "tier listed twice");
        seenTiers.add(c.tier);
        (["routes", "actions", "unsupported"] as const).forEach((f) => {
            if (c[f] !== e[f]) add(["coverage", "tiers", i, f], "coverage_mismatch", `coverage.${f} is ${c[f]} but the document has ${e[f]}`);
        });
    });
    expectedTiers.forEach((_, t) => {
        if (!seenTiers.has(t)) add(["coverage", "tiers"], "coverage_mismatch", `coverage is missing tier ${t}`);
    });
    const reasons = new Map(expected.unsupported_reasons.map((r) => [r.reason, r.count]));
    const listed = new Set<string>();
    doc.coverage.unsupported_reasons.forEach((r, i) => {
        if (listed.has(r.reason)) add(["coverage", "unsupported_reasons", i, "reason"], "duplicate_id", "reason listed twice");
        listed.add(r.reason);
        if ((reasons.get(r.reason) ?? 0) !== r.count) add(["coverage", "unsupported_reasons", i, "count"], "coverage_mismatch", `count for ${r.reason} is ${r.count} but the document has ${reasons.get(r.reason) ?? 0}`);
    });
    reasons.forEach((_, r) => {
        if (!listed.has(r)) add(["coverage", "unsupported_reasons"], "coverage_mismatch", `coverage is missing unsupported reason ${r}`);
    });
    if (list.length === before && canonicalize(doc.coverage) !== canonicalize(expected)) {
        add(["coverage"], "coverage_not_canonical", "coverage must be exactly deriveCoverage(document): tier order, no empty rows");
    }

    return list;
}

// ---------------------------------------------------------------------------
// Authority documents
// ---------------------------------------------------------------------------

/** Cross-field rules for a mandate. */
export function checkMandate(m: Mandate): CheckIssue[] {
    const { list, add } = issues();
    scan(m, add);
    const hash = safeHash(m);
    if (hash !== undefined && hash !== m.hash) add(["hash"], "hash_mismatch", "hash does not match the canonical content");
    if (m.expires_at <= m.issued_at) add(["expires_at"], "time_order", "expires_at must be after issued_at");
    if (m.per_call_cap_minor !== undefined && m.per_call_cap_minor > m.spend_cap_minor) add(["per_call_cap_minor"], "cap_order", "per-call cap exceeds the spend cap");
    duplicates(m.scope, (s) => `${s.kind}:${s.id}`).forEach((i) => add(["scope", i], "duplicate_id", "duplicate scope entry"));
    duplicates(m.merchants, (h) => h).forEach((i) => add(["merchants", i], "duplicate_id", "duplicate merchant"));
    return list;
}

/** Cross-field rules for a confirmation request. */
export function checkConfirmationRequest(c: ConfirmationRequest): CheckIssue[] {
    const { list, add } = issues();
    scan(c, add);
    if (c.expires_at <= c.issued_at) add(["expires_at"], "time_order", "expires_at must be after issued_at");
    if (c.effect === "financial") {
        let sum = 0;
        c.items.forEach((it, i) => {
            if (it.amount.currency !== c.total.currency) add(["items", i, "amount", "currency"], "currency_mismatch", "item currency differs from total currency");
            sum += it.amount.amount_minor * it.quantity;
        });
        if (sum !== c.total.amount_minor) add(["total", "amount_minor"], "total_mismatch", `items sum to ${sum} but the total is ${c.total.amount_minor}`);
    }
    return list;
}

/** Cross-field rules for a confirmation token (timestamps and string well-formedness). */
export function checkConfirmationToken(t: ConfirmationToken): CheckIssue[] {
    const { list, add } = issues();
    scan(t, add);
    return list;
}

/** Cross-field rules for an operation result (string well-formedness, page text in surface fields). */
export function checkOperationResult(r: OperationResult): CheckIssue[] {
    const { list, add } = issues();
    scan(r, add);
    return list;
}

/** Results that mean the request was sent to the site (it may have failed, or the site raised a challenge). */
const REACHED_SITE: ReadonlySet<string> = new Set(["ok", "error", "requires_human"]);
/** Results that are refused before anything is sent: they can never have a response. */
const NEVER_SENT: ReadonlySet<string> = new Set(["requires_confirmation", "unsupported", "stale"]);

/** Cross-field rules for a receipt. */
export function checkReceipt(r: Receipt): CheckIssue[] {
    const { list, add } = issues();
    scan(r, add);
    // `blocked` (a challenge, a rate limit, an off-host redirect) and `requires_auth` (a 401) can follow a sent request:
    // they count as reached exactly when a response class is recorded.
    const reached = REACHED_SITE.has(r.result) || (!NEVER_SENT.has(r.result) && r.response_class !== undefined);
    // A refusal (blocked, requires_confirmation...) records the attempt, so the mode rule applies to calls that ran.
    if (r.mode === "read-only" && reached) {
        if (r.effect !== "read") add(["effect"], "mode_forbids_effect", "read-only mode records reads only");
        if (r.tier !== "T0" && r.tier !== "T1") add(["tier"], "mode_forbids_tier", "read-only mode records T0/T1 only");
    }
    if (r.effect !== "read" && r.tier !== "T2") add(["tier"], "receipt_tier_effect", "a write is a T2 network call");
    if (r.amount !== undefined && (r.effect !== "financial" || r.result !== "ok")) add(["amount"], "amount_not_financial", "only a completed financial call records an amount");
    if (r.response_class !== undefined && NEVER_SENT.has(r.result)) add(["response_class"], "receipt_result_mismatch", `a ${r.result} result is refused before sending, so it has no response class`);
    if (r.result === "ok" && (r.response_class === "4xx" || r.response_class === "5xx")) add(["response_class"], "receipt_result_mismatch", "an ok result cannot have an error response class");
    // A call that was sent must be reconcilable: a timed-out payment is exactly the case that needs its key, mandate and confirmation.
    if (reached) {
        const need: ("amount" | "confirmation" | "mandate" | "idempotency_key")[] = [];
        if (r.effect === "financial") need.push("confirmation", "mandate", "idempotency_key", ...(r.result === "ok" ? (["amount"] as const) : []));
        else if (requiresConfirmation(r.effect)) need.push("confirmation");
        for (const f of need) if (r[f] === undefined) add([f], "receipt_incomplete", `a ${r.result} ${r.effect} call records ${f}`);
    }
    return list;
}

// ---------------------------------------------------------------------------
// Validation entry point
// ---------------------------------------------------------------------------

export type ValidationResult<T> = { ok: true; value: T } | { ok: false; phase: "schema" | "check"; issues: CheckIssue[] };

export const fromZodIssues = (error: z.ZodError): CheckIssue[] => {
    const seen = new Set<string>();
    const out: CheckIssue[] = [];
    for (const i of error.issues) {
        const path = i.path.map((s) => (typeof s === "symbol" ? String(s) : s));
        const key = JSON.stringify([path, i.code, i.message]);
        if (seen.has(key)) continue; // an intersection reports each element error once per branch
        seen.add(key);
        out.push({ path, code: i.code, message: i.message });
    }
    return out;
};

/** Parse, then check. Consumers (executor, MCP server, verifier) refuse a surface that does not pass. */
export function validateSurface(input: unknown): ValidationResult<AgentSurface> {
    const parsed = AgentSurface.safeParse(input);
    if (!parsed.success) return { ok: false, phase: "schema", issues: fromZodIssues(parsed.error) };
    const found = checkSurface(parsed.data);
    return found.length === 0 ? { ok: true, value: parsed.data } : { ok: false, phase: "check", issues: found };
}
