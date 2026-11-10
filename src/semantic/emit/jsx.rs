//! Render a captured subtree as TSX. Pattern parts never come through here:
//! callers pass `overrides` that replace a node with component JSX. What this
//! drops or rewrites is counted in [`Stats`] and surfaced in the report.
use crate::semantic::ir::{Doc, NodeId};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

/// Maps a captured element to the `className` string of the emitted element.
/// The default keeps the original class names; the styles workstream replaces
/// this with Tailwind utilities backed by the central tokens.
pub trait StyleBridge {
    fn class_name(&self, doc: &Doc, node: NodeId, original: &[String]) -> String;
}

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
    /// Custom element tags (`core-tabs`) needing a JSX type declaration.
    pub custom_elements: BTreeSet<String>,
    /// Non-standard attributes carried through an object spread.
    pub custom_attributes: usize,
}

pub struct Jsx<'a> {
    pub doc: &'a Doc,
    pub style: &'a dyn StyleBridge,
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

fn jsx_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '{' => out.push_str("{'{'}"),
            '}' => out.push_str("{'}'}"),
            '<' => out.push_str("{'<'}"),
            '>' => out.push_str("{'>'}"),
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

const HTML_ATTRS: &[&str] = &[
    "id",
    "href",
    "src",
    "alt",
    "title",
    "lang",
    "dir",
    "role",
    "type",
    "name",
    "value",
    "placeholder",
    "target",
    "rel",
    "width",
    "height",
    "loading",
    "decoding",
    "srcset",
    "sizes",
    "media",
    "download",
    "autocomplete",
    "for",
    "min",
    "max",
    "step",
    "pattern",
    "accept",
    "action",
    "method",
    "enctype",
    "colspan",
    "rowspan",
    "scope",
    "span",
    "cols",
    "rows",
    "label",
    "kind",
    "controls",
    "poster",
    "preload",
    "crossorigin",
    "integrity",
    "referrerpolicy",
    "tabindex",
    "autofocus",
    "disabled",
    "checked",
    "required",
    "readonly",
    "multiple",
    "selected",
    "maxlength",
    "minlength",
    "size",
    "hreflang",
    "srclang",
    "start",
    "reversed",
    "datetime",
    "cite",
    "open",
    "wrap",
    "autocapitalize",
    "inputmode",
    "enterkeyhint",
    "spellcheck",
    "translate",
    "draggable",
    "contenteditable",
    "accesskey",
    "itemprop",
    "itemscope",
    "itemtype",
    "itemid",
    "slot",
    "is",
    "nonce",
    "fetchpriority",
    "async",
    "defer",
    "loop",
    "muted",
    "playsinline",
    "autoplay",
    "formaction",
    "novalidate",
    "form",
    "list",
    "usemap",
    "ismap",
    "shape",
    "coords",
    "headers",
    "abbr",
    "char",
    "charset",
    "content",
    "http-equiv",
    "property",
    "as",
    "blocking",
    "color",
    "high",
    "low",
    "optimum",
];

pub fn is_standard_html_attr(name: &str) -> bool {
    HTML_ATTRS.contains(&name)
        || name.starts_with("aria-")
        || name.starts_with("data-")
        || name == "class"
        || name == "style"
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
];

impl<'a> Jsx<'a> {
    pub fn new(doc: &'a Doc, style: &'a dyn StyleBridge) -> Self {
        Jsx {
            doc,
            style,
            overrides: HashMap::new(),
            override_component: HashMap::new(),
            extra_attrs: HashMap::new(),
            text_slots: HashMap::new(),
            attr_slots: HashMap::new(),
            state_classes: BTreeSet::new(),
            stats: Rc::new(RefCell::new(Stats::default())),
        }
    }

    /// The `className` value an element would get (state classes dropped), or empty.
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
        if classes.is_empty() {
            String::new()
        } else {
            self.style.class_name(self.doc, id, &classes)
        }
    }

    /// Attributes of a node as TSX attribute strings.
    pub fn attributes(&self, id: NodeId, skip: &[&str]) -> Vec<String> {
        let n = self.doc.node(id);
        let mut out = Vec::new();
        let mut custom: Vec<String> = Vec::new();
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
        if !classes.is_empty() || n.has_attr("class") {
            let cn = self.style.class_name(self.doc, id, &classes);
            if !cn.is_empty() {
                out.push(format!("className={}", attr_value(&cn)));
            }
        }
        for (name, value) in &n.attrs {
            if matches!(name.as_str(), "class" | "data-component") || skip.contains(&name.as_str())
            {
                continue;
            }
            if name == "style" {
                self.stats.borrow_mut().inline_style_attributes_dropped += 1;
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
            if !in_svg
                && !name.contains('-')
                && !name.contains(':')
                && !is_standard_html_attr(name)
                && !BOOLEAN_ATTRS.contains(&name.as_str())
            {
                custom.push(format!("{}: {}", prop_key(name), ts_string(value)));
                continue;
            }
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
            out.push(format!("{{...{{ {} }}}}", custom.join(", ")));
        }
        if let Some(extra) = self.extra_attrs.get(&id) {
            out.extend(extra.iter().cloned());
        }
        out
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
            let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if t.is_empty() {
                return;
            }
            // Preserve a boundary space between inline neighbours.
            let lead = text.starts_with(char::is_whitespace);
            let trail = text.ends_with(char::is_whitespace);
            let mut s = jsx_text(&t);
            if lead {
                s = format!("{{' '}}{s}");
            }
            if trail {
                s.push_str("{' '}");
            }
            out.push_str(&format!("{pad}{s}\n"));
            return;
        }
        if let Some(o) = self.overrides.get(&id) {
            if let Some(c) = self.override_component.get(&id) {
                self.stats.borrow_mut().used_components.insert(c.clone());
            }
            for line in o.lines() {
                out.push_str(&format!("{pad}{line}\n"));
            }
            return;
        }
        if matches!(n.tag.as_str(), "script" | "style" | "template" | "noscript") {
            return;
        }
        self.stats.borrow_mut().nodes_emitted += 1;
        if n.tag == "svg" {
            self.stats.borrow_mut().svg_subtrees += 1;
        }
        let tag = &n.tag;
        if tag.contains('-')
            && !self
                .doc
                .ancestors(id)
                .any(|a| self.doc.node(a).tag == "svg")
        {
            self.stats.borrow_mut().custom_elements.insert(tag.clone());
        }
        let attrs = self.attributes(id, &[]);
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
        let kids: Vec<NodeId> = n.children.clone();
        let meaningful = kids.iter().any(|k| {
            let c = self.doc.node(*k);
            c.text.as_deref().is_some_and(|t| !t.trim().is_empty())
                || (c.is_element()
                    && !matches!(c.tag.as_str(), "script" | "style" | "template" | "noscript"))
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
            if let Some(t) = &self.doc.node(kids[0]).text {
                let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
                if t.len() < 60 && !open.contains('\n') {
                    out.push_str(&format!("{pad}{open}>{}</{tag}>\n", jsx_text(&t)));
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
