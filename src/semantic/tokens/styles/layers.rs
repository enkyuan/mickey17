//! Per-element style layers: the cascade resolved for each width interval and each forced
//! pointer/focus state, turned into mobile-first deltas (`base`, `min-[Wpx]:`, `hover:` ...).
//!
//! Only declarations the author cascade actually produces are translated, so the output
//! never contains UA defaults. Layers are computed from the cascade engine itself (media
//! conditions evaluated at a representative width of every interval between the media
//! thresholds found in the stylesheets); nothing is read back from computed style.

use super::super::cascade::{Cascade, Winner, winner_cmp};
use super::super::css::{self, Component, Item, Node, Sheet, Tok};
use super::super::eval;
use super::super::selector::{Dom, Tri};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    FocusWithin,
    Hover,
    Focus,
    FocusVisible,
    Active,
    /// Not an element state: `@media (prefers-reduced-motion: reduce)` (Tailwind's `motion-reduce:`).
    ReducedMotion,
}

impl State {
    /// Tailwind's output order for these variants (later wins at equal specificity).
    pub const ALL: [State; 6] = [
        State::FocusWithin,
        State::Hover,
        State::Focus,
        State::FocusVisible,
        State::Active,
        State::ReducedMotion,
    ];

    /// The state this one builds on: `:focus` implies `:focus-within`, `:focus-visible` implies `:focus`.
    pub fn parent(self) -> Option<State> {
        match self {
            State::Focus => Some(State::FocusWithin),
            State::FocusVisible => Some(State::Focus),
            _ => None,
        }
    }

    pub fn variant(self) -> &'static str {
        match self {
            State::Hover => "hover",
            State::FocusWithin => "focus-within",
            State::Focus => "focus",
            State::FocusVisible => "focus-visible",
            State::Active => "active",
            State::ReducedMotion => "motion-reduce",
        }
    }

    /// Pseudo-class names that are true for the subject while this state is forced.
    fn forces(self, name: &str) -> bool {
        match self {
            State::Hover => name == "hover",
            State::FocusWithin => name == "focus-within",
            State::Focus => matches!(name, "focus" | "focus-within"),
            State::FocusVisible => matches!(name, "focus" | "focus-within" | "focus-visible"),
            State::Active => name == "active",
            State::ReducedMotion => false,
        }
    }
}

/// A DOM view in which one state is forced on one element, optionally with other class names.
pub struct ForcedDom<'a, D: Dom + ?Sized> {
    pub inner: &'a D,
    pub forced: Option<(usize, State)>,
    pub classes: Option<&'a HashMap<usize, String>>,
}

impl<D: Dom + ?Sized> Dom for ForcedDom<'_, D> {
    fn tag(&self, node: usize) -> &str {
        self.inner.tag(node)
    }
    fn attr(&self, node: usize, name: &str) -> Option<&str> {
        if let Some(map) = self.classes {
            if name == "class" {
                return map.get(&node).map(String::as_str);
            }
            // The translated output carries no inline styles: their winners became classes.
            if name == "style" {
                return None;
            }
        }
        self.inner.attr(node, name)
    }
    fn parent(&self, node: usize) -> Option<usize> {
        self.inner.parent(node)
    }
    fn element_children(&self, node: usize) -> &[usize] {
        self.inner.element_children(node)
    }
    fn is_empty(&self, node: usize) -> bool {
        self.inner.is_empty(node)
    }
    fn state(&self, node: usize, name: &str) -> Tri {
        if let Some((subject, state)) = self.forced
            && matches!(
                name,
                "hover" | "active" | "focus" | "focus-visible" | "focus-within"
            )
        {
            return (node == subject && state.forces(name)).into();
        }
        self.inner.state(node, name)
    }
}

/// Width (CSS px) at which some media query in the sheets can change its answer.
pub fn width_breakpoints(sheets: &[&Sheet]) -> BTreeSet<u32> {
    fn thresholds(c: &Component, out: &mut BTreeSet<u32>) {
        match c {
            Component::Paren(inner) => {
                if inner.iter().any(|x| {
                    x.ident()
                        .is_some_and(|i| i.to_ascii_lowercase().contains("width"))
                }) {
                    for x in inner {
                        if let Component::Tok(Tok::Dimension { value, unit, .. }) = x {
                            let v = f64::from(*value);
                            let px = match unit.to_ascii_lowercase().as_str() {
                                "px" => v,
                                "em" | "rem" => v * 16.0,
                                "pt" => v * 96.0 / 72.0,
                                "pc" => v * 16.0,
                                "in" => v * 96.0,
                                "cm" => v * 96.0 / 2.54,
                                "mm" => v * 96.0 / 25.4,
                                "q" => v * 96.0 / 101.6,
                                _ => continue,
                            };
                            // `<= t` flips just after t, `>= t` at t: keep both neighbours.
                            for p in [px.floor(), px.floor() + 1.0, px.ceil()] {
                                if (1.0..=100_000.0).contains(&p) {
                                    out.insert(p as u32);
                                }
                            }
                        }
                    }
                }
                for x in inner {
                    thresholds(x, out);
                }
            }
            Component::Func(_, inner) => {
                for x in inner {
                    thresholds(x, out);
                }
            }
            _ => {}
        }
    }
    fn walk_items(items: &[Item], out: &mut BTreeSet<u32>) {
        for item in items {
            if let Item::Rule(node) = item {
                walk(node, out);
            }
        }
    }
    fn walk(node: &Node, out: &mut BTreeSet<u32>) {
        match node {
            Node::Style(r) => walk_items(&r.items, out),
            Node::At(at) => {
                if at.name == "media" {
                    for c in &at.prelude {
                        thresholds(c, out);
                    }
                }
                if let Some(items) = &at.items {
                    walk_items(items, out);
                }
            }
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    for sheet in sheets {
        for node in &sheet.nodes {
            walk(node, &mut out);
        }
    }
    out
}

/// One resolved declaration of a layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decl {
    pub prop: String,
    pub value: String,
}

/// The author declarations of one element in one environment, in a normal form where the box
/// shorthands are expanded (so deltas between layers compare like with like).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layer {
    pub decls: BTreeMap<String, String>,
    /// The same properties in ascending cascade precedence (a later entry beats an earlier one
    /// when they overlap); plain-CSS recipes write declarations in this order.
    pub order: Vec<String>,
    /// Exact winners that write to the same longhand (`padding` and `padding-top`).
    pub conflicts: Vec<(String, String)>,
    /// Properties that depend on a condition the capture cannot decide.
    pub uncertain: BTreeSet<String>,
    /// Count of `!important` winners flattened to plain declarations.
    pub important: usize,
    /// Declarations the capture serialized with an empty value (Chrome does this for some
    /// shorthands that contain `var()`); they cannot be recovered and are skipped.
    pub empty: Vec<String>,
    /// Empty-valued declarations replaced by the captured computed value.
    pub recovered: Vec<String>,
    /// Declarations replaced by the captured end state (see [`settle_plan`]).
    pub settled: Vec<String>,
}

/// An author declaration whose captured computed value says the element ended up elsewhere:
/// the original's script (GSAP, WAAPI, an observer) revealed it after load, so the stylesheet's
/// `opacity: 0` / `translate: 0 12px` is only the pre-reveal state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settle {
    pub prop: String,
    /// The author value (text of the resolved declaration) this replaces.
    pub author: String,
    pub value: String,
}

fn identity(prop: &str, computed: &str) -> bool {
    let c = computed.trim();
    match prop {
        "transform" => {
            c == "none"
                || c == "matrix(1, 0, 0, 1, 0, 0)"
                || c == "matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)"
        }
        "translate" => matches!(c, "none" | "0px" | "0px 0px" | "0px 0px 0px"),
        "scale" => matches!(c, "none" | "1" | "1 1" | "1 1 1"),
        "rotate" => matches!(c, "none" | "0deg"),
        _ => false,
    }
}

/// What the element's captured end state says about its resting-width author declarations:
/// opacity and visibility that differ, and transform-like properties that settled to identity.
pub fn settle_plan(rest: &Layer, computed: &BTreeMap<String, String>) -> Vec<Settle> {
    let mut out = Vec::new();
    for prop in [
        "opacity",
        "visibility",
        "transform",
        "translate",
        "scale",
        "rotate",
    ] {
        let (Some(author), Some(value)) = (rest.decls.get(prop), computed.get(prop)) else {
            continue;
        };
        let (a, c) = (author.trim(), value.trim());
        let differs = match prop {
            "opacity" => match (a.parse::<f64>(), c.parse::<f64>()) {
                (Ok(x), Ok(y)) => (x - y).abs() > 1e-3,
                _ => false,
            },
            "visibility" => {
                matches!(a, "hidden" | "visible" | "collapse")
                    && matches!(c, "hidden" | "visible" | "collapse")
                    && a != c
            }
            _ => a != "none" && identity(prop, c) && offsets_at_rest(a, computed),
        };
        if differs {
            let settled = if prop == "opacity" || prop == "visibility" {
                c.to_string()
            } else {
                "none".to_string()
            };
            out.push(Settle {
                prop: prop.to_string(),
                author: author.clone(),
                value: settled,
            });
        }
    }
    out
}

/// Does the author's transform-like value, with its custom properties read from the captured
/// computed style, move the element? False when it is identity only because a variable is 0 at
/// rest (`translateY(var(--y, 0px))`): that is not a reveal, and hover/media rules that change
/// only the variable must keep working. Unresolvable variables count as "no".
fn offsets_at_rest(author: &str, computed: &BTreeMap<String, String>) -> bool {
    let components = css::parse_components(author);
    let text = if eval::needs_substitution(&components) {
        let lookup = |name: &str| -> Option<Vec<Component>> {
            computed.get(name).map(|v| css::parse_components(v))
        };
        let Some(resolved) = eval::substitute_vars(&components, &lookup, 0) else {
            return false;
        };
        css::to_css(&resolved)
    } else {
        author.to_string()
    };
    // Any non-zero number (or percentage/length) means the value moves the element.
    let mut number = String::new();
    let mut found = false;
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
        } else {
            if number.chars().any(|d| d.is_ascii_digit())
                && number.parse::<f64>().is_ok_and(|n| n != 0.0)
            {
                found = true;
            }
            number.clear();
        }
    }
    found
}

/// What another captured viewport says about the elements of the primary snapshot: for every
/// paired element, its own author layer at that viewport's width, valid for the widths
/// `lo..hi` (the run of width intervals that activate the same rules as the captured width).
#[derive(Debug, Clone)]
pub struct Overlay {
    pub label: String,
    pub width: u32,
    pub lo: u32,
    pub hi: u32,
    /// Primary-snapshot node -> the paired element's layer.
    pub layers: HashMap<usize, Layer>,
    /// The same elements in each forced state and under reduced motion, resolved on the other
    /// capture's own DOM.
    pub states: HashMap<usize, BTreeMap<State, Layer>>,
}

impl Layer {
    /// Replace the declarations a [`Settle`] names, where they still carry the author value.
    pub fn apply_settle(&mut self, plan: &[Settle]) {
        for s in plan {
            if self.decls.get(&s.prop) == Some(&s.author) {
                self.decls.insert(s.prop.clone(), s.value.clone());
                self.settled.push(s.prop.clone());
            }
        }
    }
}

const EXPANDED: &[&str] = &[
    "margin",
    "padding",
    "inset",
    "gap",
    "border-radius",
    "border-width",
    "border-style",
    "border-color",
];

fn four(parts: &[String]) -> Option<[String; 4]> {
    Some(match parts.len() {
        1 => [
            parts[0].clone(),
            parts[0].clone(),
            parts[0].clone(),
            parts[0].clone(),
        ],
        2 => [
            parts[0].clone(),
            parts[1].clone(),
            parts[0].clone(),
            parts[1].clone(),
        ],
        3 => [
            parts[0].clone(),
            parts[1].clone(),
            parts[2].clone(),
            parts[1].clone(),
        ],
        4 => [
            parts[0].clone(),
            parts[1].clone(),
            parts[2].clone(),
            parts[3].clone(),
        ],
        _ => return None,
    })
}

/// Expand a box shorthand into longhands, or `None` to keep it as written. A bare `var()`
/// could hold several values, so it blocks the expansion (`calc(var(--spacing) * 4)` does not).
pub fn expand_box(name: &str, value: &[Component]) -> Option<Vec<(String, String)>> {
    if !EXPANDED.contains(&name) {
        return None;
    }
    let trimmed = css::trim(value);
    if trimmed
        .iter()
        .any(|c| matches!(c, Component::Func(f, _) if matches!(f.as_str(), "var" | "env" | "attr")))
    {
        return None;
    }
    if trimmed
        .iter()
        .any(|c| matches!(c, Component::Tok(Tok::Delim('/'))))
    {
        return None;
    }
    let parts: Vec<String> = trimmed
        .iter()
        .filter(|c| !c.is_whitespace())
        .map(|c| css::to_css(std::slice::from_ref(c)))
        .collect();
    let sides = ["top", "right", "bottom", "left"];
    Some(match name {
        "margin" | "padding" => {
            let v = four(&parts)?;
            sides
                .iter()
                .zip(v)
                .map(|(s, v)| (format!("{name}-{s}"), v))
                .collect()
        }
        "inset" => {
            let v = four(&parts)?;
            sides
                .iter()
                .zip(v)
                .map(|(s, v)| ((*s).to_string(), v))
                .collect()
        }
        "border-width" | "border-style" | "border-color" => {
            let kind = &name["border-".len()..];
            let v = four(&parts)?;
            sides
                .iter()
                .zip(v)
                .map(|(s, v)| (format!("border-{s}-{kind}"), v))
                .collect()
        }
        "border-radius" => {
            let v = four(&parts)?;
            ["top-left", "top-right", "bottom-right", "bottom-left"]
                .iter()
                .zip(v)
                .map(|(c, v)| (format!("border-{c}-radius"), v))
                .collect()
        }
        "gap" => match parts.len() {
            1 => vec![
                ("row-gap".into(), parts[0].clone()),
                ("column-gap".into(), parts[0].clone()),
            ],
            2 => vec![
                ("row-gap".into(), parts[0].clone()),
                ("column-gap".into(), parts[1].clone()),
            ],
            _ => return None,
        },
        _ => return None,
    })
}

fn value_text(w: &Winner) -> String {
    css::to_css(&css::trim(&w.value))
}

/// Resolve one element in one environment into a [`Layer`]. `recover` is the element's
/// captured computed style: Chrome serializes a shorthand containing `var()` as longhands with
/// *empty* values, so the real value is gone from the stylesheet text; where the capture has the
/// computed value of such a longhand it is used instead (frozen at the captured value).
pub fn layer_of<D: Dom + ?Sized>(
    cascade: &Cascade,
    dom: &D,
    node: usize,
    recover: Option<&BTreeMap<String, String>>,
) -> Layer {
    layer_for(cascade, dom, node, None, recover)
}

/// Like [`layer_of`] for a pseudo-element (`before`, `after`) of the element.
pub fn layer_for<D: Dom + ?Sized>(
    cascade: &Cascade,
    dom: &D,
    node: usize,
    pseudo: Option<&str>,
    recover: Option<&BTreeMap<String, String>>,
) -> Layer {
    let resolved = cascade.resolve(dom, node, pseudo);
    let mut layer = Layer {
        uncertain: resolved.uncertain.clone(),
        ..Default::default()
    };
    let mut ordered: Vec<(&String, &Winner)> = resolved.winners.iter().collect();
    ordered.sort_by(|a, b| winner_cmp(a.1, b.1).then(a.0.cmp(b.0)));
    for (i, (a, _)) in ordered.iter().enumerate() {
        for (b, _) in &ordered[i + 1..] {
            if overlaps(a, b) && a != b {
                layer.conflicts.push(((*a).clone(), (*b).clone()));
            }
        }
    }
    for (name, w) in ordered {
        let mut text = value_text(w);
        let mut value = w.value.clone();
        if text.is_empty() && !name.starts_with("--") {
            match recover
                .and_then(|c| c.get(name.as_str()))
                .filter(|v| !v.trim().is_empty())
            {
                Some(v) => {
                    text = v.trim().to_string();
                    value = css::parse_components(&text);
                    layer.recovered.push(name.clone());
                }
                None => {
                    layer.empty.push(name.clone());
                    continue;
                }
            }
        }
        if w.important {
            layer.important += 1;
        }
        match expand_box(name, &value) {
            Some(list) => {
                for (prop, value) in list {
                    layer.order.push(prop.clone());
                    layer.decls.insert(prop, value);
                }
            }
            None => {
                layer.order.push(name.clone());
                layer.decls.insert(name.clone(), text);
            }
        }
    }
    layer
}

const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

fn side_set(suffix: &str) -> Vec<&'static str> {
    // Logical sides are mapped to both physical ones of their axis: writing mode and direction
    // are not known, and over-approximating an overlap only costs a plain-CSS recipe.
    match suffix {
        "" => SIDES.to_vec(),
        "-top" => vec!["top"],
        "-right" => vec!["right"],
        "-bottom" => vec!["bottom"],
        "-left" => vec!["left"],
        s if s.starts_with("-inline") => vec!["left", "right"],
        s if s.starts_with("-block") => vec!["top", "bottom"],
        _ => SIDES.to_vec(),
    }
}

/// Atomic longhands a property writes (shorthands, logical aliases and axis aliases expanded).
/// Two properties conflict when their atom sets intersect.
pub fn atoms(prop: &str) -> BTreeSet<String> {
    let prop = match prop.strip_prefix("-webkit-") {
        Some(rest) if rest.starts_with("mask") => rest,
        _ => prop,
    };
    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut add = |s: String| {
        out.insert(s);
    };
    let kinds = ["width", "style", "color"];
    match prop {
        "margin" | "padding" | "scroll-margin" | "scroll-padding" => {
            SIDES.iter().for_each(|s| add(format!("{prop}-{s}")))
        }
        p if ["margin", "padding", "scroll-margin", "scroll-padding"]
            .iter()
            .any(|b| p.starts_with(&format!("{b}-"))) =>
        {
            let base = ["scroll-margin", "scroll-padding", "margin", "padding"]
                .iter()
                .find(|b| p.starts_with(&format!("{b}-")))
                .copied()
                .unwrap_or("margin");
            for s in side_set(&p[base.len()..]) {
                add(format!("{base}-{s}"));
            }
        }
        "inset" => SIDES.iter().for_each(|s| add((*s).to_string())),
        "top" | "right" | "bottom" | "left" => add(prop.to_string()),
        p if p.starts_with("inset-") => side_set(&p["inset".len()..])
            .into_iter()
            .for_each(|s| add(s.to_string())),
        "border" => {
            for s in SIDES {
                for k in kinds {
                    add(format!("border-{s}-{k}"));
                }
            }
            for k in ["source", "slice", "width", "outset", "repeat"] {
                add(format!("border-image-{k}"));
            }
        }
        "border-image" => {
            for k in ["source", "slice", "width", "outset", "repeat"] {
                add(format!("border-image-{k}"));
            }
        }
        "border-radius" => {
            for c in ["top-left", "top-right", "bottom-right", "bottom-left"] {
                add(format!("border-{c}-radius"));
            }
        }
        "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-right-radius"
        | "border-bottom-left-radius" => add(prop.to_string()),
        // Logical corners (`border-start-end-radius`): any physical corner, writing mode unknown.
        p if p.starts_with("border-") && p.ends_with("-radius") => {
            for c in ["top-left", "top-right", "bottom-right", "bottom-left"] {
                add(format!("border-{c}-radius"));
            }
        }
        p if p.starts_with("border-") => {
            // border-width|style|color, border-<side>, border-<side>-<kind>, border-block|inline[-...]
            let rest = &p["border-".len()..];
            let (sides, kind): (Vec<&str>, Option<&str>) = if let Some(k) =
                kinds.iter().find(|k| rest == **k)
            {
                (SIDES.to_vec(), Some(*k))
            } else if let Some(side) = SIDES.iter().find(|s| rest == **s) {
                (vec![*side], None)
            } else if let Some(side) = SIDES.iter().find(|s| rest.starts_with(&format!("{s}-"))) {
                (
                    vec![*side],
                    kinds
                        .iter()
                        .find(|k| rest.ends_with(&format!("-{k}")))
                        .copied(),
                )
            } else if rest.starts_with("block") || rest.starts_with("inline") {
                let axis = if rest.starts_with("block") {
                    vec!["top", "bottom"]
                } else {
                    vec!["left", "right"]
                };
                (
                    axis,
                    kinds
                        .iter()
                        .find(|k| rest.ends_with(&format!("-{k}")))
                        .copied(),
                )
            } else {
                (Vec::new(), None)
            };
            if sides.is_empty() {
                add(prop.to_string());
            } else {
                for s in sides {
                    for k in kinds {
                        if kind.is_none_or(|x| x == k) {
                            add(format!("border-{s}-{k}"));
                        }
                    }
                }
            }
        }
        "font-variant" => {
            for k in [
                "caps",
                "ligatures",
                "numeric",
                "east-asian",
                "alternates",
                "position",
                "emoji",
            ] {
                add(format!("font-variant-{k}"));
            }
        }
        "text-wrap" => {
            add("text-wrap-mode".into());
            add("text-wrap-style".into());
        }
        "white-space" => {
            add("white-space-collapse".into());
            add("text-wrap-mode".into());
        }
        "place-items" => {
            add("align-items".into());
            add("justify-items".into());
        }
        "place-self" => {
            add("align-self".into());
            add("justify-self".into());
        }
        "place-content" => {
            add("align-content".into());
            add("justify-content".into());
        }
        "gap" | "grid-gap" => {
            add("row-gap".into());
            add("column-gap".into());
        }
        "grid-row-gap" => add("row-gap".into()),
        "grid-column-gap" => add("column-gap".into()),
        "inline-size" | "block-size" => {
            add("width".into());
            add("height".into());
        }
        "min-inline-size" | "min-block-size" => {
            add("min-width".into());
            add("min-height".into());
        }
        "max-inline-size" | "max-block-size" => {
            add("max-width".into());
            add("max-height".into());
        }
        "overscroll-behavior" => {
            add("overscroll-behavior-x".into());
            add("overscroll-behavior-y".into());
        }
        _ => {
            let opaque = eval::opaque_of(prop);
            if opaque.is_empty() {
                add(prop.to_string());
            } else {
                opaque.into_iter().for_each(|x| add(x.to_string()));
            }
        }
    }
    out
}

/// Do two properties write to the same longhand? `all` overlaps everything.
pub fn overlaps(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if a == "all" || b == "all" {
        return true;
    }
    if prefixed_family(a, b) || prefixed_family(b, a) {
        return true;
    }
    let (x, y) = (atoms(a), atoms(b));
    x.intersection(&y).next().is_some()
}

/// `b` is spelled `a-…`: in CSS that nearly always means `a` is a shorthand of (or resets) `b`
/// (`background`/`background-position`, `font`/`font-variant-numeric`, `grid-row`/`grid-row-start`,
/// `container`/`container-type`, `column-rule`/`column-rule-color`). Tailwind orders arbitrary
/// properties by its own table, so such a pair must keep the author's order in plain CSS.
/// Over-approximating (`outline`/`outline-offset`) only costs a plain-CSS recipe.
fn prefixed_family(a: &str, b: &str) -> bool {
    let strip = |n: &str| {
        n.trim_start_matches("-webkit-")
            .trim_start_matches("-moz-")
            .to_string()
    };
    let (a, b) = (strip(a), strip(b));
    !a.starts_with("--")
        && b.len() > a.len()
        && b.starts_with(a.as_str())
        && b.as_bytes()[a.len()] == b'-'
}

/// One class-to-be: where it applies and what it declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// Lower bound of the width interval (`min-[Npx]:`); `None` = every width.
    pub min: Option<u32>,
    pub state: Option<State>,
    pub prop: String,
    /// `None` = the author declaration is absent here (rendered as `revert`).
    pub value: Option<String>,
}

pub const REVERT: &str = "revert";

/// Everything the layer analysis learned about one element.
#[derive(Debug, Default, Clone)]
pub struct ElementLayers {
    pub placed: Vec<Placed>,
    /// Elements whose placed declarations cannot be ordered unambiguously by utilities.
    pub overlapping: Vec<(String, String)>,
    pub uncertain: BTreeSet<String>,
    pub important: usize,
}

/// Compute mobile-first placements from the per-interval, per-state layers.
///
/// `rest[i]` is the layer at interval `i` (lower bound `bounds[i]`, `bounds[0] == 0`);
/// `states[s][i]` the layer with state `s` forced.
pub fn place(
    bounds: &[u32],
    rest: &[Layer],
    states: &BTreeMap<State, Vec<Layer>>,
) -> ElementLayers {
    let mut out = ElementLayers::default();
    let min_of = |i: usize| (i > 0).then(|| bounds[i]);
    for layer in rest {
        out.uncertain.extend(layer.uncertain.iter().cloned());
        out.important = out.important.max(layer.important);
        out.overlapping.extend(layer.conflicts.iter().cloned());
    }
    // Rest chain.
    for (i, layer) in rest.iter().enumerate() {
        let before: BTreeMap<String, String> = if i == 0 {
            BTreeMap::new()
        } else {
            rest[i - 1].decls.clone()
        };
        delta(&before, &layer.decls, min_of(i), None, &mut out.placed);
    }
    // State chains: each state's overrides persist to wider intervals until a delta cancels them.
    // FocusVisible builds on Focus because `:focus-visible` implies `:focus`.
    // `None` = the state reverted the property (a tombstone: the rest value no longer shows).
    let mut carried: BTreeMap<State, BTreeMap<String, Option<String>>> = BTreeMap::new();
    for state in State::ALL {
        let Some(layers) = states.get(&state) else {
            continue;
        };
        if state == State::ReducedMotion {
            // Tailwind sorts `motion-reduce:` before `min-[W]:`, so a wider interval's plain
            // utility beats an earlier `motion-reduce:` one, while `min-[W]:motion-reduce:`
            // beats the plain one again: simulate what applies under reduced motion and write
            // `motion-reduce:` utilities wherever it differs from the layer.
            let mut applied: BTreeMap<String, String> = BTreeMap::new();
            for (i, layer) in layers.iter().enumerate() {
                out.uncertain.extend(layer.uncertain.iter().cloned());
                out.overlapping.extend(layer.conflicts.iter().cloned());
                if i == 0 {
                    applied = rest[0].decls.clone();
                } else {
                    for p in rest[i - 1].decls.keys().chain(rest[i].decls.keys()) {
                        if rest[i - 1].decls.get(p) != rest[i].decls.get(p) {
                            match rest[i].decls.get(p) {
                                Some(v) => applied.insert(p.clone(), v.clone()),
                                None => applied.remove(p),
                            };
                        }
                    }
                }
                delta(
                    &applied,
                    &layer.decls,
                    min_of(i),
                    Some(state),
                    &mut out.placed,
                );
                applied = layer.decls.clone();
            }
            continue;
        }
        for (i, layer) in layers.iter().enumerate() {
            out.uncertain.extend(layer.uncertain.iter().cloned());
            out.overlapping.extend(layer.conflicts.iter().cloned());
            // What is in force when this state applies: the state it builds on (its full layer at
            // this interval) or the plain layer, plus what earlier intervals of this state carried.
            let mut effective = match state.parent().and_then(|p| states.get(&p)) {
                Some(parent) => parent[i].decls.clone(),
                None => rest[i].decls.clone(),
            };
            for (p, v) in carried.get(&state).into_iter().flatten() {
                match v {
                    Some(v) => {
                        effective.insert(p.clone(), v.clone());
                    }
                    None => {
                        effective.remove(p);
                    }
                }
            }
            let mut placed = Vec::new();
            delta(
                &effective,
                &layer.decls,
                min_of(i),
                Some(state),
                &mut placed,
            );
            let entry = carried.entry(state).or_default();
            for pl in &placed {
                entry.insert(pl.prop.clone(), pl.value.clone());
            }
            out.placed.extend(placed);
        }
    }
    // Declarations in one variant may not overlap: utilities in the same media/state have no
    // reliable relative order.
    for (i, a) in out.placed.iter().enumerate() {
        for b in &out.placed[i + 1..] {
            if a.min == b.min && a.state == b.state && overlaps(&a.prop, &b.prop) {
                out.overlapping.push((a.prop.clone(), b.prop.clone()));
            }
        }
    }
    out
}

fn delta(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    min: Option<u32>,
    state: Option<State>,
    out: &mut Vec<Placed>,
) {
    for (prop, value) in after {
        if before.get(prop) != Some(value) {
            out.push(Placed {
                min,
                state,
                prop: prop.clone(),
                value: Some(value.clone()),
            });
        }
    }
    for prop in before.keys() {
        if !after.contains_key(prop) {
            out.push(Placed {
                min,
                state,
                prop: prop.clone(),
                value: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(pairs: &[(&str, &str)]) -> Layer {
        Layer {
            decls: pairs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn mobile_first_deltas_only_record_changes_and_removals() {
        let rest = vec![
            layer(&[("display", "none"), ("padding-top", "4px")]),
            layer(&[("padding-top", "8px")]),
            layer(&[("padding-top", "8px")]),
        ];
        let placed = place(&[0, 600, 900], &rest, &BTreeMap::new()).placed;
        let find = |min: Option<u32>, prop: &str| {
            placed
                .iter()
                .find(|p| p.min == min && p.prop == prop)
                .cloned()
        };
        assert_eq!(
            find(None, "display").unwrap().value.as_deref(),
            Some("none")
        );
        assert_eq!(
            find(Some(600), "padding-top").unwrap().value.as_deref(),
            Some("8px")
        );
        assert_eq!(
            find(Some(600), "display").unwrap().value,
            None,
            "removal becomes revert"
        );
        assert!(
            find(Some(900), "padding-top").is_none(),
            "unchanged intervals add nothing"
        );
    }

    #[test]
    fn state_overrides_persist_until_cancelled() {
        let rest = vec![
            layer(&[("color", "red")]),
            layer(&[("color", "red")]),
            layer(&[("color", "blue")]),
        ];
        let mut states = BTreeMap::new();
        states.insert(
            State::Hover,
            vec![
                layer(&[("color", "green")]),
                layer(&[("color", "green")]),
                layer(&[("color", "blue")]),
            ],
        );
        let placed = place(&[0, 600, 900], &rest, &states).placed;
        let hover: Vec<&Placed> = placed
            .iter()
            .filter(|p| p.state == Some(State::Hover))
            .collect();
        assert_eq!(hover.len(), 2, "{hover:?}");
        assert_eq!(
            (hover[0].min, hover[0].value.as_deref()),
            (None, Some("green"))
        );
        // At >= 900 the hover value equals the rest value, which must cancel the carried green.
        assert_eq!(
            (hover[1].min, hover[1].value.as_deref()),
            (Some(900), Some("blue"))
        );
    }

    #[test]
    fn focus_visible_builds_on_focus_and_overlap_is_reported() {
        let rest = vec![layer(&[("outline", "none")])];
        let mut states = BTreeMap::new();
        states.insert(
            State::Focus,
            vec![layer(&[("outline", "none"), ("color", "red")])],
        );
        states.insert(
            State::FocusVisible,
            vec![layer(&[
                ("outline", "none"),
                ("color", "red"),
                ("opacity", "1"),
            ])],
        );
        let placed = place(&[0], &rest, &states).placed;
        assert!(
            placed
                .iter()
                .any(|p| p.state == Some(State::Focus) && p.prop == "color")
        );
        assert!(
            !placed
                .iter()
                .any(|p| p.state == Some(State::FocusVisible) && p.prop == "color"),
            "already carried from focus"
        );
        assert!(
            placed
                .iter()
                .any(|p| p.state == Some(State::FocusVisible) && p.prop == "opacity")
        );

        let rest = vec![layer(&[
            ("background", "red"),
            ("background-color", "blue"),
        ])];
        let result = place(&[0], &rest, &BTreeMap::new());
        assert_eq!(
            result.overlapping.len(),
            1,
            "shorthand and longhand in one variant are unordered"
        );
    }

    #[test]
    fn breakpoints_cover_both_sides_of_every_threshold() {
        let sheet = css::parse_sheet(
            "@media (min-width: 40rem) { .a { top: 0 } } @media (width <= 599.5px) { .b { top: 0 } } @media print { .c {} }",
        );
        let points = width_breakpoints(&[&sheet]);
        assert!(points.contains(&640) && points.contains(&600) && points.contains(&599));
        assert!(!points.contains(&0));
    }
}
