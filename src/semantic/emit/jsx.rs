//! Render a captured subtree as TSX. Pattern parts never come through here:
//! callers pass `overrides` that replace a node with component JSX. What this
//! drops or rewrites is counted in [`Stats`] and surfaced in the report.
use crate::semantic::ir::{Doc, NodeId};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

/// Maps a captured element to the `className` string of the emitted element.
/// The default keeps the original class names; the styles workstream replaces
/// this with Tailwind utilities backed by the central tokens.
pub trait StyleBridge {
    fn class_name(&self, doc: &Doc, node: NodeId, original: &[String]) -> String;
    /// Does the class string already carry what the element's inline `style` declares? (A
    /// translation of computed styles does; captured class names do not.)
    fn translates_inline_styles(&self) -> bool {
        false
    }
}

/// Maps an asset URL as written in the capture to the path the project serves it from.
pub type UrlMap<'a> = &'a dyn Fn(&str) -> Option<String>;

pub struct OriginalClasses;
impl StyleBridge for OriginalClasses {
    fn class_name(&self, _doc: &Doc, _node: NodeId, original: &[String]) -> String {
        original.join(" ")
    }
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub inline_style_attributes_dropped: usize,
    pub state_classes_dropped: BTreeSet<String>,
    pub svg_subtrees: usize,
    pub nodes_emitted: usize,
    /// Components rendered through an override since the last drain (for imports).
    pub used_components: BTreeSet<String>,
    /// Nodes replaced by an override since the last drain.
    pub used_nodes: BTreeSet<NodeId>,
    /// Elements deliberately not emitted, by reason, since the last drain.
    pub elements_dropped: BTreeMap<String, usize>,
    /// Custom element tags (`core-tabs`) needing a JSX type declaration.
    pub custom_elements: BTreeSet<String>,
    /// Non-standard attributes carried through an object spread.
    pub custom_attributes: usize,
}

pub struct Jsx<'a> {
    pub doc: &'a Doc,
    pub style: &'a dyn StyleBridge,
    /// Rewrites asset URLs in `src`, `srcset` and `poster`.
    pub urls: Option<UrlMap<'a>>,
    /// Replace the whole node with this JSX.
    pub overrides: HashMap<NodeId, String>,
    /// Component name behind an override that is a component call.
    pub override_component: HashMap<NodeId, String>,
    /// Extra JSX attributes for a node (`onClick={...}`).
    pub extra_attrs: HashMap<NodeId, Vec<String>>,
    /// Text nodes replaced by an expression (`{title}`) in a component body.
    pub text_slots: HashMap<NodeId, String>,
    /// Attributes replaced by an expression: `(node, attribute name) -> full JSX attribute`.
    pub attr_slots: HashMap<(NodeId, String), String>,
    /// Class tokens that encode a state the primitive now owns (dropped).
    pub state_classes: BTreeSet<String>,
    pub stats: Rc<RefCell<Stats>>,
}

/// Deeper subtrees are cut with an explicit comment rather than recursing without bound.
const MAX_DEPTH: usize = 200;

/// Custom elements that exist only to run the original framework's client router.
const FRAMEWORK_ELEMENTS: &[&str] = &["next-route-announcer", "nextjs-portal"];

const INLINE_TAGS: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "button", "cite", "code", "data", "dfn", "em", "i", "img",
    "input", "kbd", "label", "mark", "q", "s", "samp", "select", "small", "span", "strong", "sub",
    "sup", "svg", "textarea", "time", "u", "var", "wbr",
];

/// Does the `<p>` hold content the HTML parser cannot keep inside it? (Not counting what sits in a
/// `button` or an svg: those are scope boundaries or foreign content.)
pub fn paragraph_holds_block(doc: &Doc, id: NodeId) -> bool {
    doc.descendants(id).into_iter().any(|d| {
        let c = doc.node(d);
        c.is_element()
            && P_CLOSERS.contains(&c.tag.as_str())
            && !doc
                .ancestors(d)
                .take_while(|a| *a != id)
                .any(|a| matches!(doc.node(a).tag.as_str(), "button" | "svg"))
    })
}

/// Start tags that close an open `<p>` in the HTML parser.
pub const P_CLOSERS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "dialog",
    "dd",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "search",
    "section",
    "table",
    "ul",
];

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// State words that make a BEM modifier a state class when no script evidence exists.
const STATE_SUFFIXES: &[&str] = &[
    "selected",
    "active",
    "open",
    "opened",
    "expanded",
    "current",
    "pressed",
    "checked",
    "collapsed",
];

/// Inline-style properties that describe where an animation currently is, not where the element
/// sits: nothing emitted runs the script that would move them on, so they are not carried.
fn is_transient_style(name: &str) -> bool {
    let name = name
        .strip_prefix("-webkit-")
        .or_else(|| name.strip_prefix("-moz-"))
        .or_else(|| name.strip_prefix("-ms-"))
        .unwrap_or(name);
    matches!(
        name,
        "transform"
            | "transform-origin"
            | "translate"
            | "scale"
            | "rotate"
            | "opacity"
            | "visibility"
            | "clip-path"
            | "filter"
            | "backdrop-filter"
            | "will-change"
            | "pointer-events"
            | "stroke-dasharray"
            | "stroke-dashoffset"
            | "stroke-opacity"
            | "fill-opacity"
    ) || name.starts_with("transition")
        || name.starts_with("animation")
}

/// Split an inline `style` attribute into the declarations that are emitted as a typed style
/// object (property name as written, value) and the names of the transient ones left out.
/// Declarations the browser would ignore (empty values) and `!important` flags are not carried.
pub fn inline_style_props(value: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for d in crate::semantic::tokens::css::parse_declarations(value) {
        let text = d.raw.clone().unwrap_or_else(|| d.value_css());
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if is_transient_style(&d.name) {
            dropped.push(d.name.clone());
        } else {
            // A repeated property (`display: -webkit-box; display: flex`) keeps the last value:
            // an object literal cannot hold the key twice.
            kept.retain(|(k, _): &(String, String)| *k != d.name);
            kept.push((d.name.clone(), text.to_owned()));
        }
    }
    (kept, dropped)
}

/// JSX for a text node whose whitespace was collapsed into `collapsed`: a boundary space is kept
/// as `{' '}` (it separates inline neighbours and is part of an accessible name).
fn text_with_boundaries(raw: &str, collapsed: &str) -> String {
    let mut s = jsx_text(collapsed);
    if raw.starts_with(ascii_ws) {
        s = format!("{{' '}}{s}");
    }
    if raw.ends_with(ascii_ws) {
        s.push_str("{' '}");
    }
    s
}

/// Classes a script selects by (`js-menu-toggle`); they carry no style.
pub fn is_script_hook(class: &str) -> bool {
    class.starts_with("js-") || class.starts_with("js_")
}

/// What the bridge says for `classes`, plus the script hook classes (`js-*`) the translation
/// would otherwise take away: they style nothing, but they are how a script finds the element.
pub fn translate_classes(
    style: &dyn StyleBridge,
    doc: &Doc,
    id: NodeId,
    classes: &[String],
) -> String {
    let cn = style.class_name(doc, id, classes);
    if cn == classes.join(" ") {
        return cn;
    }
    let have: BTreeSet<&str> = cn.split_ascii_whitespace().collect();
    let hooks: Vec<&str> = classes
        .iter()
        .map(String::as_str)
        .filter(|c| is_script_hook(c) && !have.contains(c))
        .collect();
    match (cn.is_empty(), hooks.is_empty()) {
        (_, true) => cn,
        (true, false) => hooks.join(" "),
        (false, false) => format!("{cn} {}", hooks.join(" ")),
    }
}

/// A class that encodes a state a primitive now owns: `block--active`, `is-open`. Utility
/// variants (`is-active:opacity-100`, `has-[..]:`) and framework classes (`is-layout-flex`)
/// are styling and are kept.
pub fn is_state_modifier(class: &str) -> bool {
    if class.contains([':', '[', ']']) {
        return false;
    }
    let state = |s: &str| STATE_SUFFIXES.contains(&s);
    class.strip_prefix("is-").is_some_and(state)
        || class.strip_prefix("has-").is_some_and(state)
        || class.rsplit_once("--").is_some_and(|(_, m)| state(m))
}

pub fn ts_string(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// The WHATWG `srcset` candidates: a URL is a run of non-whitespace characters (so a data URI
/// keeps its comma), trailing commas end a candidate without a descriptor, and a descriptor runs
/// to the next comma outside parentheses.
pub fn parse_srcset(value: &str) -> Vec<(&str, &str)> {
    let b = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] == b',' || ascii_ws(b[i] as char)) {
            i += 1;
        }
        let start = i;
        while i < b.len() && !ascii_ws(b[i] as char) {
            i += 1;
        }
        if start == i {
            break;
        }
        let mut url = &value[start..i];
        let mut descriptor = "";
        if url.ends_with(',') {
            url = url.trim_end_matches(',');
        } else {
            let d0 = i;
            let mut depth = 0usize;
            while i < b.len() {
                match b[i] {
                    b'(' => depth += 1,
                    b')' => depth = depth.saturating_sub(1),
                    b',' if depth == 0 => break,
                    _ => {}
                }
                i += 1;
            }
            descriptor = value[d0..i].trim_matches(|c: char| ascii_ws(c));
        }
        if !url.is_empty() {
            out.push((url, descriptor));
        }
    }
    out
}

fn ascii_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// HTML whitespace collapsing: only ASCII whitespace separates words (a no-break space is text).
fn collapse_ws(s: &str) -> String {
    s.split(ascii_ws)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `&` followed by what would parse as a character reference (`&amp;`, `&#38;`).
fn entity_at(rest: &str) -> bool {
    let body = rest.strip_prefix('&').unwrap_or("");
    let end = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '#'))
        .unwrap_or(body.len());
    end > 0 && body[end..].starts_with(';')
}

fn jsx_text(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.char_indices() {
        match c {
            '{' => out.push_str("{'{'}"),
            '}' => out.push_str("{'}'}"),
            '<' => out.push_str("{'<'}"),
            '>' => out.push_str("{'>'}"),
            // A literal "&amp;" in the text must not be decoded again by JSX.
            '&' if entity_at(&s[i..]) => out.push_str("{'&'}"),
            c => out.push(c),
        }
    }
    out
}

pub fn attr_name(name: &str) -> Option<String> {
    Some(match name {
        "class" => "className".into(),
        "for" => "htmlFor".into(),
        "tabindex" => "tabIndex".into(),
        "readonly" => "readOnly".into(),
        "maxlength" => "maxLength".into(),
        "autocomplete" => "autoComplete".into(),
        "crossorigin" => "crossOrigin".into(),
        "srcset" => "srcSet".into(),
        "fetchpriority" => "fetchPriority".into(),
        "colspan" => "colSpan".into(),
        "rowspan" => "rowSpan".into(),
        "viewbox" => "viewBox".into(),
        "novalidate" => "noValidate".into(),
        "formnovalidate" => "formNoValidate".into(),
        "autofocus" => "autoFocus".into(),
        "autoplay" => "autoPlay".into(),
        "playsinline" => "playsInline".into(),
        "contenteditable" => "contentEditable".into(),
        "spellcheck" => "spellCheck".into(),
        "datetime" => "dateTime".into(),
        "enterkeyhint" => "enterKeyHint".into(),
        "inputmode" => "inputMode".into(),
        "hreflang" => "hrefLang".into(),
        "srclang" => "srcLang".into(),
        "usemap" => "useMap".into(),
        "accesskey" => "accessKey".into(),
        "itemprop" => "itemProp".into(),
        "itemscope" => "itemScope".into(),
        "itemtype" => "itemType".into(),
        "itemid" => "itemID".into(),
        "referrerpolicy" => "referrerPolicy".into(),
        "minlength" => "minLength".into(),
        "srcdoc" => "srcDoc".into(),
        "allowfullscreen" => "allowFullScreen".into(),
        "formaction" => "formAction".into(),
        "formenctype" => "formEncType".into(),
        "formmethod" => "formMethod".into(),
        "formtarget" => "formTarget".into(),
        "frameborder" => "frameBorder".into(),
        n if n.starts_with("aria-") || n.starts_with("data-") => n.to_owned(),
        n if n.starts_with("xmlns") => return None,
        n if n.starts_with("xlink:") => format!("xlink{}", pascal_tail(&n[6..])),
        n => n.to_owned(),
    })
}

fn pascal_tail(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

fn camel_hyphen(s: &str) -> String {
    let mut parts = s.split('-');
    let mut out = parts.next().unwrap_or("").to_owned();
    for p in parts {
        out.push_str(&pascal_tail(p));
    }
    out
}

pub fn attr_value(v: &str) -> String {
    if v.contains('"') || v.contains('\n') || v.contains('{') {
        format!("{{{}}}", ts_string(v))
    } else {
        format!("\"{v}\"")
    }
}

const NUMERIC_ATTRS: &[&str] = &[
    "tabIndex",
    "colSpan",
    "rowSpan",
    "maxLength",
    "minLength",
    "size",
    "rows",
    "cols",
    "span",
    "start",
];

/// Whether TSX types `name` on the intrinsic element `tag`: the global attributes plus the ones
/// the element defines. Anything else would not type-check on an intrinsic element, so the caller
/// carries it through an object spread instead.
pub fn is_standard_html_attr(tag: &str, name: &str) -> bool {
    if is_global_attr(name) || GLOBAL_EXTRA.contains(&name) {
        return true;
    }
    let own: &[&str] = match tag {
        "a" | "area" => &[
            "href",
            "target",
            "rel",
            "download",
            "hreflang",
            "type",
            "referrerpolicy",
            "ping",
            "media",
            "alt",
            "coords",
            "shape",
        ],
        "img" => &[
            "src",
            "srcset",
            "sizes",
            "alt",
            "width",
            "height",
            "loading",
            "decoding",
            "crossorigin",
            "referrerpolicy",
            "usemap",
            "fetchpriority",
        ],
        "source" => &["src", "srcset", "sizes", "media", "type", "width", "height"],
        "video" => &[
            "src",
            "controls",
            "loop",
            "muted",
            "autoplay",
            "playsinline",
            "preload",
            "poster",
            "width",
            "height",
            "crossorigin",
        ],
        "audio" => &[
            "src",
            "controls",
            "loop",
            "muted",
            "autoplay",
            "preload",
            "crossorigin",
        ],
        "track" => &["kind", "src", "srclang", "label", "default"],
        "input" => &[
            "type",
            "name",
            "value",
            "placeholder",
            "required",
            "disabled",
            "checked",
            "readonly",
            "maxlength",
            "minlength",
            "min",
            "max",
            "step",
            "pattern",
            "autocomplete",
            "autofocus",
            "multiple",
            "accept",
            "list",
            "size",
            "form",
            "width",
            "height",
            "src",
            "alt",
            "formaction",
            "formenctype",
            "formmethod",
            "formnovalidate",
            "formtarget",
        ],
        "button" => &[
            "type",
            "name",
            "value",
            "disabled",
            "form",
            "autofocus",
            "formaction",
            "formenctype",
            "formmethod",
            "formnovalidate",
            "formtarget",
        ],
        "textarea" => &[
            "name",
            "placeholder",
            "required",
            "disabled",
            "readonly",
            "rows",
            "cols",
            "maxlength",
            "minlength",
            "autocomplete",
            "autofocus",
            "wrap",
            "form",
        ],
        "select" => &[
            "name",
            "required",
            "disabled",
            "multiple",
            "size",
            "autocomplete",
            "autofocus",
            "form",
        ],
        "option" => &["value", "selected", "disabled", "label"],
        "optgroup" => &["label", "disabled"],
        "label" => &["for", "form"],
        "form" => &[
            "action",
            "method",
            "enctype",
            "novalidate",
            "target",
            "autocomplete",
            "name",
        ],
        "fieldset" => &["disabled", "form", "name"],
        "output" => &["for", "form", "name"],
        "iframe" => &[
            "src",
            "srcdoc",
            "width",
            "height",
            "allow",
            "allowfullscreen",
            "loading",
            "name",
            "referrerpolicy",
            "sandbox",
        ],
        "embed" | "object" => &["src", "type", "width", "height", "data", "name", "form"],
        "td" | "th" => &["colspan", "rowspan", "headers", "scope", "abbr"],
        "col" | "colgroup" => &["span"],
        "ol" => &["start", "reversed", "type"],
        "li" => &["value"],
        "time" => &["datetime"],
        "blockquote" | "q" | "del" | "ins" => &["cite"],
        "details" | "dialog" => &["open"],
        "progress" | "meter" => &["value", "max", "min", "low", "high", "optimum"],
        "canvas" => &["width", "height"],
        "html" => &["xmlns"],
        "meta" => &["name", "content", "charset", "property"],
        _ => &[],
    };
    own.contains(&name)
}

/// Global attributes beyond the ones every element accepts in `is_global_attr`.
const GLOBAL_EXTRA: &[&str] = &[
    "accesskey",
    "autocapitalize",
    "contenteditable",
    "draggable",
    "enterkeyhint",
    "inputmode",
    "spellcheck",
    "translate",
    "itemprop",
    "itemscope",
    "itemtype",
    "itemid",
    "nonce",
    "is",
];

/// SVG props React's typings accept (camelCase JSX names). Others go through the spread.
const SVG_TYPED: &[&str] = &[
    "accumulate",
    "additive",
    "alignmentBaseline",
    "amplitude",
    "attributeName",
    "azimuth",
    "baseFrequency",
    "baselineShift",
    "begin",
    "bias",
    "calcMode",
    "clipPath",
    "clipPathUnits",
    "clipRule",
    "color",
    "colorInterpolation",
    "colorInterpolationFilters",
    "crossOrigin",
    "cursor",
    "cx",
    "cy",
    "d",
    "diffuseConstant",
    "direction",
    "display",
    "divisor",
    "dominantBaseline",
    "dur",
    "dx",
    "dy",
    "edgeMode",
    "elevation",
    "end",
    "exponent",
    "fill",
    "fillOpacity",
    "fillRule",
    "filter",
    "filterUnits",
    "floodColor",
    "floodOpacity",
    "focusable",
    "fontFamily",
    "fontSize",
    "fontSizeAdjust",
    "fontStretch",
    "fontStyle",
    "fontVariant",
    "fontWeight",
    "fr",
    "from",
    "fx",
    "fy",
    "gradientTransform",
    "gradientUnits",
    "height",
    "href",
    "imageRendering",
    "in",
    "in2",
    "intercept",
    "k",
    "k1",
    "k2",
    "k3",
    "k4",
    "kernelMatrix",
    "kernelUnitLength",
    "keyPoints",
    "keySplines",
    "keyTimes",
    "lengthAdjust",
    "letterSpacing",
    "lightingColor",
    "limitingConeAngle",
    "markerEnd",
    "markerHeight",
    "markerMid",
    "markerStart",
    "markerUnits",
    "markerWidth",
    "mask",
    "maskContentUnits",
    "maskUnits",
    "max",
    "media",
    "method",
    "min",
    "mode",
    "numOctaves",
    "offset",
    "opacity",
    "operator",
    "order",
    "orient",
    "orientation",
    "overflow",
    "paintOrder",
    "path",
    "pathLength",
    "patternContentUnits",
    "patternTransform",
    "patternUnits",
    "pointerEvents",
    "points",
    "pointsAtX",
    "pointsAtY",
    "pointsAtZ",
    "preserveAlpha",
    "preserveAspectRatio",
    "primitiveUnits",
    "r",
    "radius",
    "refX",
    "refY",
    "repeatCount",
    "repeatDur",
    "requiredExtensions",
    "requiredFeatures",
    "restart",
    "result",
    "rotate",
    "rx",
    "ry",
    "scale",
    "seed",
    "shapeRendering",
    "slope",
    "spacing",
    "specularConstant",
    "specularExponent",
    "speed",
    "spreadMethod",
    "startOffset",
    "stdDeviation",
    "stemh",
    "stemv",
    "stitchTiles",
    "stopColor",
    "stopOpacity",
    "stroke",
    "strokeDasharray",
    "strokeDashoffset",
    "strokeLinecap",
    "strokeLinejoin",
    "strokeMiterlimit",
    "strokeOpacity",
    "strokeWidth",
    "surfaceScale",
    "systemLanguage",
    "tableValues",
    "targetX",
    "targetY",
    "textAnchor",
    "textDecoration",
    "textLength",
    "textRendering",
    "to",
    "transform",
    "transformOrigin",
    "type",
    "values",
    "vectorEffect",
    "version",
    "viewBox",
    "visibility",
    "width",
    "wordSpacing",
    "writingMode",
    "x",
    "x1",
    "x2",
    "xChannelSelector",
    "xlinkHref",
    "xmlSpace",
    "xmlLang",
    "y",
    "y1",
    "y2",
    "yChannelSelector",
    "z",
    "zoomAndPan",
    "id",
    "className",
    "role",
    "tabIndex",
    "lang",
];

fn is_typed_svg_prop(js_name: &str) -> bool {
    SVG_TYPED.contains(&js_name) || js_name.starts_with("aria-") || js_name.starts_with("data-")
}

/// Attributes every HTML element (custom elements included) accepts in TSX.
fn is_global_attr(name: &str) -> bool {
    matches!(
        name,
        "class"
            | "id"
            | "title"
            | "lang"
            | "dir"
            | "role"
            | "slot"
            | "hidden"
            | "inert"
            | "tabindex"
            | "style"
    ) || name.starts_with("aria-")
        || name.starts_with("data-")
}

/// The key React's `style` object uses for a CSS property (`clip-path` -> `clipPath`,
/// `-webkit-x` -> `WebkitX`, `--custom` stays quoted as written).
fn style_key(name: &str) -> String {
    if name.starts_with("--") {
        return ts_string(name);
    }
    let name = name
        .strip_prefix("-ms-")
        .map_or_else(|| name.to_owned(), |rest| format!("ms-{rest}"));
    let mut out = String::new();
    let mut upper = false;
    for c in name.chars() {
        if c == '-' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    if out.chars().all(|c| c.is_alphanumeric()) {
        out
    } else {
        ts_string(&out)
    }
}

fn prop_key(name: &str) -> String {
    if name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        name.to_owned()
    } else {
        ts_string(name)
    }
}

/// SVG attributes: hyphenated presentation attributes become camelCase, `class` -> `className`.
fn svg_attr_name(name: &str) -> Option<String> {
    match name {
        "class" => Some("className".into()),
        n if n.starts_with("aria-") || n.starts_with("data-") => Some(n.to_owned()),
        n if n.starts_with("xmlns") => None,
        n if n.starts_with("xlink:") => Some(format!("xlink{}", pascal_tail(&n[6..]))),
        "viewbox" => Some("viewBox".into()),
        "xml:space" => Some("xmlSpace".into()),
        "xml:lang" => Some("xmlLang".into()),
        n if n.contains('-') => Some(camel_hyphen(n)),
        n => Some(n.to_owned()),
    }
}

const BOOLEAN_ATTRS: &[&str] = &[
    "disabled",
    "checked",
    "required",
    "readonly",
    "multiple",
    "selected",
    "autofocus",
    "async",
    "defer",
    "controls",
    "loop",
    "muted",
    "playsinline",
    "autoplay",
    "inert",
    "hidden",
    "itemscope",
    "novalidate",
    "allowfullscreen",
    "open",
    "reversed",
    "default",
    "formnovalidate",
];

impl<'a> Jsx<'a> {
    pub fn new(doc: &'a Doc, style: &'a dyn StyleBridge) -> Self {
        Jsx {
            doc,
            style,
            urls: None,
            overrides: HashMap::new(),
            override_component: HashMap::new(),
            extra_attrs: HashMap::new(),
            text_slots: HashMap::new(),
            attr_slots: HashMap::new(),
            state_classes: BTreeSet::new(),
            stats: Rc::new(RefCell::new(Stats::default())),
        }
    }

    /// The `className` value an element would get (state classes dropped), or empty. The bridge
    /// is asked for every element, class-less ones included: a translation can style an element
    /// by tag or ancestor selectors that the original markup never named.
    pub fn class_value(&self, id: NodeId) -> String {
        let n = self.doc.node(id);
        let mut classes: Vec<String> = Vec::new();
        for c in &n.classes {
            if self.state_classes.contains(c) || is_state_modifier(c) {
                self.stats
                    .borrow_mut()
                    .state_classes_dropped
                    .insert(c.clone());
            } else {
                classes.push(c.clone());
            }
        }
        self.translated_class(id, &classes)
    }

    fn translated_class(&self, id: NodeId, classes: &[String]) -> String {
        translate_classes(self.style, self.doc, id, classes)
    }

    /// The value of an attribute that points at an asset, mapped to where the project serves it.
    /// Map an asset URL attribute to where the project serves it (also used for the values a
    /// repeated-structure component receives as props).
    pub fn rewrite_url(&self, tag: &str, name: &str, value: &str) -> String {
        let Some(map) = self.urls else {
            return value.to_owned();
        };
        let is_url = matches!(name, "src" | "poster")
            || (matches!(tag, "image" | "use") && matches!(name, "href" | "xlink:href"));
        if is_url {
            // A sprite reference keeps its fragment: `/sprite.svg#icon`.
            let (path, fragment) = match value.split_once('#') {
                Some((p, f)) if tag == "use" => (p, Some(f)),
                _ => (value, None),
            };
            return match (map(path), fragment) {
                (Some(local), Some(f)) => format!("{local}#{f}"),
                (Some(local), None) => local,
                (None, _) => value.to_owned(),
            };
        }
        if name == "srcset" {
            let candidates = parse_srcset(value);
            let rewritten: Vec<Option<String>> =
                candidates.iter().map(|(url, _)| map(url)).collect();
            if rewritten.iter().all(Option::is_none) {
                return value.to_owned();
            }
            return candidates
                .iter()
                .zip(rewritten)
                .map(|((url, descriptor), new)| {
                    let url = new.unwrap_or_else(|| (*url).to_owned());
                    if descriptor.is_empty() {
                        url
                    } else {
                        format!("{url} {descriptor}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
        }
        value.to_owned()
    }

    /// Attributes of a plain markup node as TSX attribute strings: every captured class is kept.
    pub fn attributes(&self, id: NodeId, skip: &[&str]) -> Vec<String> {
        self.attributes_impl(id, skip, false)
    }

    /// Attributes of a part of a Base UI composition: classes that encode a state the primitive
    /// now owns (`--active`, `is-open`, script-driven modifiers) are dropped and listed.
    pub fn part_attributes(&self, id: NodeId, skip: &[&str]) -> Vec<String> {
        self.attributes_impl(id, skip, true)
    }

    fn attributes_impl(&self, id: NodeId, skip: &[&str], drop_state: bool) -> Vec<String> {
        let n = self.doc.node(id);
        let mut out = Vec::new();
        let mut custom: Vec<String> = Vec::new();
        let mut classes: Vec<String> = Vec::new();
        for c in &n.classes {
            if drop_state && (self.state_classes.contains(c) || is_state_modifier(c)) {
                self.stats
                    .borrow_mut()
                    .state_classes_dropped
                    .insert(c.clone());
            } else {
                classes.push(c.clone());
            }
        }
        let cn = self.translated_class(id, &classes);
        if !cn.is_empty() {
            out.push(format!("className={}", attr_value(&cn)));
        }
        for (name, value) in &n.attrs {
            if matches!(name.as_str(), "class" | "data-component") || skip.contains(&name.as_str())
            {
                continue;
            }
            if name == "style" {
                if self.style.translates_inline_styles() {
                    continue;
                }
                let (kept, _) = inline_style_props(value);
                if kept.is_empty() {
                    self.stats.borrow_mut().inline_style_attributes_dropped += 1;
                } else {
                    let body = kept
                        .iter()
                        .map(|(k, v)| format!("{}: {}", style_key(k), ts_string(v)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push(format!("style={{{{ {body} }} as React.CSSProperties}}"));
                }
                continue;
            }
            if let Some(slot) = self.attr_slots.get(&(id, name.clone())) {
                out.push(slot.clone());
                continue;
            }
            let in_svg = n.tag == "svg"
                || self
                    .doc
                    .ancestors(id)
                    .any(|a| self.doc.node(a).tag == "svg");
            let Some(js_name) = (if in_svg {
                svg_attr_name(name)
            } else {
                attr_name(name)
            }) else {
                continue;
            };
            // A name that is neither standard HTML nor hyphenated cannot be typed on an
            // intrinsic element: carry it through an object spread instead of dropping it.
            let custom_element = n.tag.contains('-') && !in_svg;
            let untyped_here = custom_element && !is_global_attr(name);
            // Names TSX cannot write as an attribute (`:class`, `@click`, `xml:lang` on HTML) and
            // numeric props whose captured value is not an integer cannot be typed either.
            let numeric_misfit = NUMERIC_ATTRS.contains(&js_name.as_str())
                && !in_svg
                && value.trim().parse::<i64>().is_err();
            if untyped_here
                || numeric_misfit
                || (in_svg && !is_typed_svg_prop(&js_name))
                || (!in_svg
                    && (name.starts_with([':', '@'])
                        || (name.contains(':') && !name.starts_with("xlink:"))
                        || (!name.contains('-')
                            && !name.contains(':')
                            && !is_standard_html_attr(n.tag.as_str(), name))))
            {
                custom.push(format!("{}: {}", prop_key(name), ts_string(value)));
                continue;
            }
            let value = &self.rewrite_url(n.tag.as_str(), name, value);
            // A captured form value is the initial value, not a controlled one.
            let js_name = match (n.tag.as_str(), name.as_str()) {
                ("input", "value") => "defaultValue".to_owned(),
                ("input", "checked") => "defaultChecked".to_owned(),
                _ => js_name,
            };
            if BOOLEAN_ATTRS.contains(&name.as_str()) {
                out.push(js_name);
            } else if NUMERIC_ATTRS.contains(&js_name.as_str())
                && value.trim().parse::<i64>().is_ok()
            {
                out.push(format!("{js_name}={{{}}}", value.trim()));
            } else {
                out.push(format!("{js_name}={}", attr_value(value)));
            }
        }
        if !custom.is_empty() {
            self.stats.borrow_mut().custom_attributes += custom.len();
            // Typed as a string record so a prop React knows with another type cannot clash.
            out.push(format!(
                "{{...({{ {} }} as Record<string, string>)}}",
                custom.join(", ")
            ));
        }
        if let Some(extra) = self.extra_attrs.get(&id) {
            out.extend(extra.iter().cloned());
        }
        out
    }

    /// Why an element is not emitted at all, when it is not: scripts and styles (nothing here runs
    /// or embeds them), framework runtime elements, nested document elements, and any element
    /// whose `src`/`href`/`action` is a `javascript:` URL (injected detection code).
    fn dropped(&self, id: NodeId) -> Option<String> {
        let n = self.doc.node(id);
        let tag = n.tag.as_str();
        if matches!(tag, "script" | "style" | "template" | "noscript") {
            return Some(format!("<{tag}> is never emitted"));
        }
        if FRAMEWORK_ELEMENTS.contains(&tag) {
            return Some(format!(
                "<{tag}> exists only to run the original framework's router"
            ));
        }
        if matches!(tag, "html" | "head" | "body") {
            return Some(format!("<{tag}> below the document body"));
        }
        for attr in ["src", "href", "action", "formaction", "data"] {
            if let Some(v) = n.attr(attr)
                && v.trim_start()
                    .to_ascii_lowercase()
                    .starts_with("javascript:")
            {
                return Some(format!("<{tag}> with a javascript: {attr}"));
            }
        }
        None
    }

    /// Whitespace is significant inside `pre` and `textarea`.
    fn verbatim(&self, id: NodeId) -> bool {
        self.doc
            .ancestors(id)
            .any(|a| matches!(self.doc.node(a).tag.as_str(), "pre" | "textarea"))
    }

    pub fn render(&self, id: NodeId, indent: usize) -> String {
        let mut out = String::new();
        self.node(id, indent, &mut out);
        out
    }

    fn pad(indent: usize) -> String {
        "  ".repeat(indent)
    }

    /// Children of `id` rendered one per line at `indent`.
    pub fn children(&self, id: NodeId, indent: usize) -> String {
        let mut out = String::new();
        for &c in &self.doc.node(id).children {
            self.node(c, indent, &mut out);
        }
        out
    }

    /// Is the sibling before (or after) `id` rendered inline (text, or an element that is not a
    /// block)? Uses the captured `display` when there is one, the tag otherwise.
    fn inline_neighbour(&self, id: NodeId, after: bool) -> bool {
        let mut at_node = id;
        loop {
            let Some(parent) = self.doc.node(at_node).parent else {
                return false;
            };
            let p = self.doc.node(parent);
            // Whitespace between table structure, in a flex or grid container or in an svg is
            // not rendered.
            let table_structure = |d: &str| {
                matches!(
                    d,
                    "table"
                        | "table-row"
                        | "table-row-group"
                        | "table-header-group"
                        | "table-footer-group"
                        | "table-column"
                        | "table-column-group"
                )
            };
            if p.tag == "svg"
                || self
                    .doc
                    .ancestors(at_node)
                    .any(|a| self.doc.node(a).tag == "svg")
                || matches!(
                    p.tag.as_str(),
                    "table"
                        | "tbody"
                        | "thead"
                        | "tfoot"
                        | "tr"
                        | "colgroup"
                        | "select"
                        | "optgroup"
                        | "ul"
                        | "ol"
                )
                || p.style("display").is_some_and(|d| {
                    d.contains("flex") || d.contains("grid") || table_structure(d) || d == "none"
                })
            {
                return false;
            }
            let kids = &p.children;
            let Some(at) = kids.iter().position(|k| *k == at_node) else {
                return false;
            };
            let mut sibs: Box<dyn Iterator<Item = &NodeId>> = if after {
                Box::new(kids[at + 1..].iter())
            } else {
                Box::new(kids[..at].iter().rev())
            };
            let found = sibs.find_map(|k| {
                let c = self.doc.node(*k);
                if let Some(t) = &c.text {
                    return (!t.trim().is_empty()).then_some(true);
                }
                if self.dropped(*k).is_some() {
                    return None;
                }
                Some(match c.style("display") {
                    Some(d) => d.starts_with("inline"),
                    None => INLINE_TAGS.contains(&c.tag.as_str()),
                })
            });
            match found {
                Some(v) => return v,
                // Nothing on this side inside an inline parent: the neighbour is the parent's.
                None => {
                    let inline_parent = match p.style("display") {
                        Some(d) => d.starts_with("inline"),
                        None => INLINE_TAGS.contains(&p.tag.as_str()),
                    };
                    if !inline_parent {
                        return false;
                    }
                    at_node = parent;
                }
            }
        }
    }

    /// Does the element hold block-level content that HTML does not allow inside a `<p>`?
    fn holds_block(&self, id: NodeId) -> bool {
        paragraph_holds_block(self.doc, id)
    }

    fn node(&self, id: NodeId, indent: usize, out: &mut String) {
        let n = self.doc.node(id);
        if indent > MAX_DEPTH {
            out.push_str(&format!(
                "{}{{/* subtree deeper than {MAX_DEPTH} levels not emitted */}}\n",
                Self::pad(indent)
            ));
            return;
        }
        let pad = Self::pad(indent);
        if let Some(text) = &n.text {
            if let Some(slot) = self.text_slots.get(&id) {
                out.push_str(&format!("{pad}{slot}\n"));
                return;
            }
            if self.verbatim(id) {
                if !text.is_empty() {
                    out.push_str(&format!("{pad}{{{}}}\n", ts_string(text)));
                }
                return;
            }
            let t = collapse_ws(text);
            if t.is_empty() {
                // Whitespace between two inline neighbours is a space in the rendered text (and in
                // accessible names); between blocks it is nothing.
                if !text.is_empty()
                    && self.inline_neighbour(id, false)
                    && self.inline_neighbour(id, true)
                {
                    out.push_str(&format!("{pad}{{' '}}\n"));
                }
                return;
            }
            out.push_str(&format!("{pad}{}\n", text_with_boundaries(text, &t)));
            return;
        }
        if let Some(o) = self.overrides.get(&id) {
            self.stats.borrow_mut().used_nodes.insert(id);
            if let Some(c) = self.override_component.get(&id) {
                self.stats.borrow_mut().used_components.insert(c.clone());
            }
            for line in o.lines() {
                out.push_str(&format!("{pad}{line}\n"));
            }
            return;
        }
        if let Some(reason) = self.dropped(id) {
            *self
                .stats
                .borrow_mut()
                .elements_dropped
                .entry(reason)
                .or_default() += 1;
            return;
        }
        self.stats.borrow_mut().nodes_emitted += 1;
        if n.tag == "svg" {
            self.stats.borrow_mut().svg_subtrees += 1;
        }
        // A `<p>` that holds block content (built by script, which allows it) cannot be written
        // as HTML: the parser closes it at the first block child and hydration fails. It is a
        // `div` with the paragraph role instead.
        let misnested = n.tag == "p" && self.holds_block(id);
        let tag = if misnested { &"div".to_owned() } else { &n.tag };
        if tag.contains('-')
            && !self
                .doc
                .ancestors(id)
                .any(|a| self.doc.node(a).tag == "svg")
        {
            self.stats.borrow_mut().custom_elements.insert(tag.clone());
        }
        let mut attrs = self.attributes(id, &[]);
        if misnested && !n.has_attr("role") {
            attrs.push("role=\"paragraph\"".to_owned());
        }
        let open = if attrs.is_empty() {
            format!("<{tag}")
        } else if attrs.iter().map(String::len).sum::<usize>() < 70 {
            format!("<{tag} {}", attrs.join(" "))
        } else {
            format!(
                "<{tag}\n{}",
                attrs
                    .iter()
                    .map(|a| format!("{pad}  {a}\n"))
                    .collect::<String>()
                    + &pad
            )
        };
        // An iframe's content document is not part of this document: never descend into it.
        let kids: Vec<NodeId> = if tag == "iframe" {
            Vec::new()
        } else {
            n.children.clone()
        };
        let meaningful = kids.iter().any(|k| {
            let c = self.doc.node(*k);
            c.text.as_deref().is_some_and(|t| !t.trim().is_empty())
                || (c.is_element() && self.dropped(*k).is_none())
        });
        if (VOID.contains(&tag.as_str()) || !meaningful)
            && (VOID.contains(&tag.as_str())
                || tag == "path"
                || tag == "circle"
                || tag == "rect"
                || !meaningful)
        {
            out.push_str(&format!("{pad}{open} />\n"));
            return;
        }
        // Single short text child stays on one line.
        if kids.len() == 1 {
            if let Some(slot) = self.text_slots.get(&kids[0])
                && !open.contains('\n')
            {
                out.push_str(&format!("{pad}{open}>{slot}</{tag}>\n"));
                return;
            }
            if let Some(t) = &self.doc.node(kids[0]).text
                && !self.verbatim(kids[0])
            {
                let collapsed = collapse_ws(t);
                if collapsed.len() < 60 && !open.contains('\n') && !collapsed.is_empty() {
                    out.push_str(&format!(
                        "{pad}{open}>{}</{tag}>\n",
                        text_with_boundaries(t, &collapsed)
                    ));
                    return;
                }
            }
        }
        out.push_str(&format!("{pad}{open}>\n"));
        for k in kids {
            self.node(k, indent + 1, out);
        }
        out.push_str(&format!("{pad}</{tag}>\n"));
    }
}

/// Props for a document element (`html`, `body`) as JSX property names and string values.
/// `style` is dropped and non-standard unhyphenated attributes are returned in the second list,
/// so the caller can report what it could not carry.
pub fn element_props(
    doc: &Doc,
    id: NodeId,
    style: &dyn StyleBridge,
) -> (Vec<(String, String)>, Vec<String>) {
    let n = doc.node(id);
    let mut props = Vec::new();
    let mut dropped = Vec::new();
    let class = translate_classes(style, doc, id, &n.classes);
    if !class.is_empty() {
        props.push(("className".to_owned(), class));
    }
    for (name, value) in &n.attrs {
        if matches!(name.as_str(), "class" | "data-component") {
            continue;
        }
        if name == "style" {
            dropped.push("style".to_owned());
            continue;
        }
        let Some(js) = attr_name(name) else { continue };
        if !name.contains('-') && !name.contains(':') && !is_standard_html_attr(&n.tag, name) {
            dropped.push(name.clone());
            continue;
        }
        props.push((js, value.clone()));
    }
    (props, dropped)
}
