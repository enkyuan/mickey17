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
}

export interface PageNode {
    readonly tag: string;
    readonly attrs?: Readonly<Record<string, string>>;
    readonly style?: NodeStyle;
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

/** Reasons this node's *own* box or text is not shown (ancestors are handled by the walk). */
export function ownHiddenReasons(n: PageNode): HiddenReason[] {
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
    if (s.color && s.backgroundColor && s.color.a > 0 && s.backgroundColor.a > 0 && contrastRatio(s.color, s.backgroundColor) < 1.1) out.push("low-contrast");
    if (s.color && s.color.a === 0) out.push("low-contrast");
    const abs = s.position === "absolute" || s.position === "fixed";
    if ((abs && ((s.leftPx ?? 0) <= -900 || (s.topPx ?? 0) <= -900)) || (s.textIndentPx !== undefined && s.textIndentPx <= -900)) out.push("offscreen");
    const clip = (s.clip ?? "").replace(/\s+/g, "").toLowerCase();
    const cp = (s.clipPath ?? "").replace(/\s+/g, "").toLowerCase();
    if (/^rect\(0(px)?,0(px)?,0(px)?,0(px)?\)$/.test(clip) || /^inset\((50|100)%\)/.test(cp) || cp === "circle(0)" || cp === "circle(0px)") out.push("clipped");
    const tiny = (v: number | undefined) => v !== undefined && v <= 1;
    if ((tiny(s.widthPx) || tiny(s.heightPx)) && (s.overflow ?? "").toLowerCase() === "hidden") out.push("zero-size");
    if (s.scale !== undefined && s.scale <= 0.01) out.push("scaled-to-zero");
    return out;
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
    /** Explicit opt-in: hidden text is returned (with reasons) instead of excluded. */
    readonly includeHidden?: boolean;
}

export interface Collected {
    readonly visible: readonly TextSegment[];
    readonly excluded: readonly ExcludedSegment[];
    /** Only populated with includeHidden: segments tagged with why they are hidden. */
    readonly hiddenOptIn: readonly (TextSegment & { readonly reasons: readonly HiddenReason[] })[];
}

const TAG_OK = /^[a-z][a-z0-9-]*$/;

export function collectText(root: PageNode, opts: CollectOptions = {}): Collected {
    const visible: TextSegment[] = [];
    const excluded: ExcludedSegment[] = [];
    const hiddenOptIn: (TextSegment & { reasons: readonly HiddenReason[] })[] = [];

    const walk = (n: PageNode, path: string, inherited: readonly HiddenReason[], visibilityHidden: boolean): void => {
        const tag = TAG_OK.test(n.tag.toLowerCase()) ? n.tag.toLowerCase() : "node";
        const here = path === "" ? tag : `${path}>${tag}`;
        const own = ownHiddenReasons(n);
        const vis = (n.style?.visibility ?? "").toLowerCase();
        const hiddenByVisibility = vis === "hidden" || vis === "collapse" ? true : vis === "visible" ? false : visibilityHidden;
        const reasons: HiddenReason[] = [...inherited, ...own];
        if (hiddenByVisibility) reasons.push("visibility-hidden");
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
            walk(c, here, [...pass, ...extra], hiddenByVisibility);
        });
    };
    walk(root, "", [], false);
    return { visible, excluded, hiddenOptIn };
}

/** alt / aria-label text of a node, only when the node itself is shown. */
export function visibleAttributeText(n: PageNode): string[] {
    if (ownHiddenReasons(n).length > 0) return [];
    const a = n.attrs ?? {};
    return [a.alt, a["aria-label"]].filter((v): v is string => typeof v === "string" && v.trim() !== "");
}
