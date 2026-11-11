/**
 * Hidden-text exclusion rules (D8). Operates on a plain node description (the capture
 * supplies computed style), so it needs no DOM and is deterministic. Text a human was
 * never shown is the usual carrier of hidden instructions and is excluded from T0 by
 * default.
 */

export interface Rgba {
    readonly r: number;
    readonly g: number;
    readonly b: number;
    readonly a: number;
}

export interface NodeStyle {
    readonly display?: string;
    readonly visibility?: string;
    readonly opacity?: number;
    readonly fontSizePx?: number;
    readonly color?: Rgba;
    readonly backgroundColor?: Rgba;
    readonly position?: string;
    readonly leftPx?: number;
    readonly topPx?: number;
    readonly widthPx?: number;
    readonly heightPx?: number;
    readonly overflow?: string;
    readonly textIndentPx?: number;
    readonly clip?: string;
    readonly clipPath?: string;
    readonly scale?: number;
    readonly contentVisibility?: string;
    /** True when a background image or gradient sits behind the text: the backdrop colour is then unknown. */
    readonly backgroundImage?: boolean;
}

export interface Rect {
    readonly x: number;
    readonly y: number;
    readonly width: number;
    readonly height: number;
}

export interface PageNode {
    readonly tag: string;
    readonly attrs?: Readonly<Record<string, string>>;
    readonly style?: NodeStyle;
    /** Border box in document coordinates, when the capture has it. */
    readonly rect?: Rect;
    /** Text directly inside this node (before its children). */
    readonly text?: string;
    readonly children?: readonly PageNode[];
}

export type HiddenReason =
    | "excluded-tag"
    | "display-none"
    | "visibility-hidden"
    | "hidden-attribute"
    | "aria-hidden"
    | "inert"
    | "zero-opacity"
    | "tiny-font"
    | "low-contrast"
    | "offscreen"
    | "clipped"
    | "zero-size"
    | "scaled-to-zero"
    | "content-visibility-hidden"
    | "closed-details";

const EXCLUDED_TAGS = new Set(["script", "style", "template", "noscript", "head", "meta", "link", "title", "base", "iframe", "object", "embed", "datalist", "svg", "math", "canvas", "audio", "video"]);

function luminance(c: Rgba): number {
    const f = (v: number) => {
        const s = v / 255;
        return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b);
}

export function contrastRatio(a: Rgba, b: Rgba): number {
    const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
    return (hi + 0.05) / (lo + 0.05);
}

const WHITE: Rgba = { r: 255, g: 255, b: 255, a: 1 };

/** Composite `top` over an opaque `bottom`. */
function over(top: Rgba, bottom: Rgba): Rgba {
    const a = Math.min(1, Math.max(0, top.a));
    return { r: top.r * a + bottom.r * (1 - a), g: top.g * a + bottom.g * (1 - a), b: top.b * a + bottom.b * (1 - a), a: 1 };
}

/**
 * Reasons this node's *own* box or text is not shown (ancestors are handled by the walk).
 * `backdrop` is the effective opaque background behind the node (the nearest ancestors'
 * backgrounds composited; white page if none), because a node's own computed background
 * is usually transparent even when the text on it is unreadable.
 */
export function ownHiddenReasons(n: PageNode, backdrop: Rgba | null = WHITE, doc?: { readonly width: number; readonly height: number }): HiddenReason[] {
    const out: HiddenReason[] = [];
    const a = n.attrs ?? {};
    const s = n.style ?? {};
    const tag = n.tag.toLowerCase();
    if (EXCLUDED_TAGS.has(tag)) out.push("excluded-tag");
    if (s.display?.toLowerCase() === "none") out.push("display-none");
    if (s.contentVisibility?.toLowerCase() === "hidden") out.push("content-visibility-hidden");
    if ("hidden" in a && a.hidden !== "until-found") out.push("hidden-attribute");
    if ((a["aria-hidden"] ?? "").toLowerCase() === "true") out.push("aria-hidden");
    if ("inert" in a) out.push("inert");
    if (s.opacity !== undefined && s.opacity <= 0.01) out.push("zero-opacity");
    if (s.fontSizePx !== undefined && s.fontSizePx < 2) out.push("tiny-font");
    if (s.color) {
        if (s.color.a < 0.1) out.push("low-contrast"); // (near-)transparent text colour
        else if (backdrop !== null && !s.backgroundImage) {
            // backdrop === null: an ancestor has a background image; the colour behind the text is unknown
            const bg = s.backgroundColor && s.backgroundColor.a > 0 ? over(s.backgroundColor, backdrop) : backdrop;
            if (contrastRatio(over(s.color, bg), bg) < 1.1) out.push("low-contrast");
        }
    }
    // Without a bounding box only *negative* offsets are judged: they leave the page on the
    // left/top edge, where no scrolling can reach. A large positive `top` is just content
    // further down the page. With the box and the document size, any box wholly outside
    // the document is off-screen.
    const moved = s.position === "absolute" || s.position === "fixed" || s.position === "relative" || s.position === "sticky";
    const behind = (v: number | undefined) => v !== undefined && v <= -900;
    const r = n.rect;
    const outside = r !== undefined && (r.x + r.width <= 0 || r.y + r.height <= 0 || (doc !== undefined && (r.x >= doc.width || r.y >= doc.height)));
    if ((moved && (behind(s.leftPx) || behind(s.topPx))) || (s.textIndentPx !== undefined && s.textIndentPx <= -900) || outside) out.push("offscreen");
    const clip = (s.clip ?? "").replace(/[\s,]+/g, "").toLowerCase();
    const cp = (s.clipPath ?? "").replace(/\s+/g, "").toLowerCase();
    if (/^rect\((0(px)?){4}\)$/.test(clip) || insetHidesAll((s.clipPath ?? "").toLowerCase().trim()) || cp === "circle(0)" || cp === "circle(0px)") out.push("clipped");
    const tiny = (v: number | undefined) => v !== undefined && v <= 1;
    if ((tiny(s.widthPx) || tiny(s.heightPx)) && (s.overflow ?? "").toLowerCase() === "hidden") out.push("zero-size");
    if (s.scale !== undefined && s.scale <= 0.01) out.push("scaled-to-zero");
    return out;
}

/** clip-path: inset(...) that leaves nothing: one side insets 100% or more, or opposite sides sum to 100%+. */
function insetHidesAll(cp: string): boolean {
    const m = /^inset\(([^)]*)\)/.exec(cp);
    if (!m) return false;
    const vals = m[1]!.split("round")[0]!.trim().split(/\s+/).filter(Boolean);
    const pct = vals.map((v) => (v.endsWith("%") ? parseFloat(v) : 0));
    if (pct.some((p) => p >= 100)) return true;
    const [t = 0, r = t, b = t, l = r] = pct;
    return t + b >= 100 || l + r >= 100;
}

export interface TextSegment {
    readonly text: string;
    /** Tag path, for audit. Surface-generated from tag names only. */
    readonly path: string;
}
export interface ExcludedSegment {
    readonly reasons: readonly HiddenReason[];
    readonly path: string;
    /** Length only: the hidden text itself is never returned. */
    readonly chars: number;
}

export interface CollectOptions {
    /** Document size, to judge boxes that lie beyond the right/bottom edge. */
    readonly document?: { readonly width: number; readonly height: number };
    /** Page background behind the root (default white, the CSS canvas). */
    readonly pageBackground?: Rgba;
    /** Explicit opt-in: hidden text is returned (with reasons) instead of excluded. */
    readonly includeHidden?: boolean;
}

export interface Collected {
    readonly visible: readonly TextSegment[];
    readonly excluded: readonly ExcludedSegment[];
    /** alt / aria-label of nodes that are shown (same rules, including hidden ancestors). */
    readonly attributes: readonly TextSegment[];
    /** Only populated with includeHidden: segments tagged with why they are hidden. */
    readonly hiddenOptIn: readonly (TextSegment & { readonly reasons: readonly HiddenReason[] })[];
}

const TAG_OK = /^[a-z][a-z0-9-]*$/;

export function collectText(root: PageNode, opts: CollectOptions = {}): Collected {
    const visible: TextSegment[] = [];
    const excluded: ExcludedSegment[] = [];
    const hiddenOptIn: (TextSegment & { reasons: readonly HiddenReason[] })[] = [];
    const attributes: TextSegment[] = [];

    const walk = (n: PageNode, path: string, inherited: readonly HiddenReason[], visibilityHidden: boolean, backdrop: Rgba | null, opacity: number): void => {
        const tag = TAG_OK.test(n.tag.toLowerCase()) ? n.tag.toLowerCase() : "node";
        const here = path === "" ? tag : `${path}>${tag}`;
        const own = ownHiddenReasons(n, backdrop, opts.document);
        const bg = n.style?.backgroundColor;
        // an image or gradient makes the backdrop unknown for the whole subtree (until an opaque colour covers it)
        const childBackdrop: Rgba | null = n.style?.backgroundImage ? null : bg && bg.a >= 1 ? bg : bg && bg.a > 0 && backdrop ? over(bg, backdrop) : backdrop;
        const effOpacity = opacity * (n.style?.opacity ?? 1);
        if (effOpacity <= 0.01 && !own.includes("zero-opacity")) own.push("zero-opacity"); // opacity compounds down the tree
        const vis = (n.style?.visibility ?? "").toLowerCase();
        const hiddenByVisibility = vis === "hidden" || vis === "collapse" ? true : vis === "visible" ? false : visibilityHidden;
        const reasons: HiddenReason[] = [...inherited, ...own];
        if (hiddenByVisibility) reasons.push("visibility-hidden");
        if (reasons.length === 0) {
            const at = n.attrs ?? {};
            for (const v of [at.alt, at["aria-label"]]) if (typeof v === "string" && v.trim() !== "") attributes.push({ text: v, path: here });
        }
        const text = n.text ?? "";
        if (text.trim() !== "") {
            if (reasons.length === 0) visible.push({ text, path: here });
            else {
                excluded.push({ reasons: [...new Set(reasons)], path: here, chars: text.length });
                if (opts.includeHidden) hiddenOptIn.push({ text, path: here, reasons: [...new Set(reasons)] });
            }
        }
        // A hidden box hides its subtree. Computed font size, colour and visibility are
        // recorded per node by the capture, so they are judged on each node, not inherited.
        const pass: HiddenReason[] = [...inherited, ...own.filter((r) => r !== "tiny-font" && r !== "low-contrast" && r !== "visibility-hidden")];
        const isDetails = n.tag.toLowerCase() === "details";
        const detailsOpen = isDetails && "open" in (n.attrs ?? {});
        (n.children ?? []).forEach((c) => {
            let extra: HiddenReason[] = [];
            if (isDetails && !detailsOpen && c.tag.toLowerCase() !== "summary") extra = ["closed-details"];
            walk(c, here, [...pass, ...extra], hiddenByVisibility, childBackdrop, effOpacity);
        });
    };
    walk(root, "", [], false, opts.pageBackground ?? WHITE, 1);
    return { visible, excluded, attributes, hiddenOptIn };
}
