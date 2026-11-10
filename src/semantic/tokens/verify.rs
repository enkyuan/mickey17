//! Checks against the capture: (1) the cascade resolver reproduces the computed styles the
//! browser reported; (2) a rewritten stylesheet set resolves to the same values as the original.

use super::capture::{Corpus, Snapshot};
use super::cascade::{Cascade, SheetInput};
use super::css::{self, Component, Sheet};
use super::eval::{self, Metrics};
use super::media::Environment;
use serde::Serialize;
use std::collections::BTreeMap;

/// Parse every deduplicated corpus sheet (all CSSOM rule texts concatenated in order).
pub fn parse_corpus(corpus: &Corpus) -> Vec<Sheet> {
    corpus
        .sheets
        .iter()
        .map(|source| {
            let mut sheet = Sheet::default();
            for text in &source.texts {
                sheet.nodes.extend(css::parse_sheet(text).nodes);
            }
            sheet
        })
        .collect()
}

pub fn environment(snapshot: &Snapshot) -> Environment {
    Environment {
        width: f64::from(snapshot.width),
        height: f64::from(snapshot.height),
        color_scheme: snapshot.color_scheme.clone(),
    }
}

pub fn cascade_for(snapshot: &Snapshot, sheets: &[Sheet]) -> Cascade {
    let inputs: Vec<SheetInput<'_>> = snapshot
        .sheets
        .iter()
        .filter(|s| !s.disabled)
        .map(|s| SheetInput {
            sheet: &sheets[s.sheet],
            index: s.sheet,
            media: &s.media,
        })
        .collect();
    Cascade::build(&inputs, &environment(snapshot))
}

#[derive(Debug, Clone, Serialize)]
pub struct Mismatch {
    pub snapshot: String,
    pub node: usize,
    pub pseudo: Option<String>,
    pub element: String,
    pub property: String,
    pub captured: String,
    pub resolved: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ResolverCheck {
    pub snapshot: String,
    pub elements: usize,
    pub pseudo_elements: usize,
    pub compared: usize,
    pub matched: usize,
    pub mismatched: usize,
    pub skipped_uncertain: usize,
    pub skipped_opaque: usize,
    pub skipped_not_comparable: usize,
    pub skipped_animated_elements: usize,
    pub skipped_animated_props: usize,
    pub by_property: BTreeMap<String, [usize; 2]>,
    pub samples: Vec<Mismatch>,
}

fn px(value: Option<&String>) -> Option<f64> {
    value?.trim().strip_suffix("px")?.parse().ok()
}

fn describe(snapshot: &Snapshot, node: usize) -> String {
    let Some(e) = snapshot.element(node) else {
        return String::new();
    };
    let class = e
        .attributes
        .iter()
        .find(|(k, _)| k == "class")
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    let class: String = class.chars().take(60).collect();
    if class.is_empty() {
        e.tag.clone()
    } else {
        format!("{}.{}", e.tag, class.replace(' ', "."))
    }
}

pub const SAMPLE_LIMIT: usize = 60;

/// Compare cascade winners with the computed styles captured from the browser.
pub fn validate_resolver(snapshot: &Snapshot, cascade: &Cascade) -> ResolverCheck {
    let mut check = ResolverCheck {
        snapshot: snapshot.label.clone(),
        ..Default::default()
    };
    let root_fs = snapshot.root_font_size_px().unwrap_or(16.0);
    for node in snapshot.elements() {
        let element = snapshot.element(node).expect("element");
        if snapshot.animated.contains(&node) {
            check.skipped_animated_elements += 1;
            continue;
        }
        check.elements += 1;
        let mut targets: Vec<(Option<&str>, usize)> = vec![(None, element.style)];
        for (key, style) in &element.pseudo {
            targets.push((Some(key.as_str()), *style));
        }
        for (pseudo, style) in targets {
            if pseudo.is_some() {
                check.pseudo_elements += 1;
            }
            let computed = &snapshot.styles[style];
            let lookup = |name: &str| -> Option<Vec<Component>> {
                let v = computed.get(name)?;
                Some(css::trim(&css::parse_components(v)))
            };
            let resolved = cascade.resolve_longhands(snapshot, node, pseudo, &lookup);
            let own_fs = px(computed.get("font-size")).unwrap_or(16.0);
            let parent_fs = element
                .parent
                .and_then(|p| snapshot.computed(p))
                .and_then(|c| px(c.get("font-size")))
                .unwrap_or(16.0);
            let animated_props: std::collections::BTreeSet<&str> = computed
                .get("animation-name")
                .map(|names| {
                    names
                        .split(',')
                        .filter_map(|n| cascade.keyframes.get(n.trim().trim_matches('"')))
                        .flat_map(|set| set.iter().map(String::as_str))
                        .collect()
                })
                .unwrap_or_default();
            for (name, winner) in &resolved.winners {
                if animated_props.contains(name.as_str()) {
                    check.skipped_animated_props += 1;
                    continue;
                }
                // Border widths compute to 0 when the side's style is none/hidden.
                if let Some(side) = name
                    .strip_prefix("border-")
                    .and_then(|n| n.strip_suffix("-width"))
                {
                    let style = computed
                        .get(&format!("border-{side}-style"))
                        .map(String::as_str);
                    if matches!(style, Some("none" | "hidden")) {
                        check.skipped_not_comparable += 1;
                        continue;
                    }
                }
                if !eval::comparable_property(name)
                    || matches!(name.as_str(), "width" | "height" | "flex-basis")
                {
                    continue;
                }
                if resolved.uncertain.contains(name) {
                    check.skipped_uncertain += 1;
                    continue;
                }
                if winner.opaque {
                    check.skipped_opaque += 1;
                    continue;
                }
                let Some(captured) = computed.get(name) else {
                    check.skipped_not_comparable += 1;
                    continue;
                };
                let metrics = Metrics {
                    font_size: if name == "font-size" {
                        parent_fs
                    } else {
                        own_fs
                    },
                    root_font_size: root_fs,
                    vw: f64::from(snapshot.width),
                    vh: f64::from(snapshot.height),
                    layout_rounding: true,
                };
                let value = match eval::needs_substitution(&winner.value) {
                    true => eval::substitute_vars(&winner.value, &lookup, 0),
                    false => Some(winner.value.clone()),
                };
                let Some(value) = value else {
                    check.skipped_not_comparable += 1;
                    continue;
                };
                // `auto` margins/insets resolve to used pixel values in computed style.
                if value.len() == 1 && value[0].ident() == Some("auto") {
                    check.skipped_not_comparable += 1;
                    continue;
                }
                let own = Metrics {
                    font_size: own_fs,
                    ..metrics
                };
                let expected = eval::canonical(name, &css::parse_components(captured), &own);
                let got = eval::canonical(name, &value, &metrics);
                let (Some(expected), Some(got)) = (expected, got) else {
                    check.skipped_not_comparable += 1;
                    continue;
                };
                check.compared += 1;
                let entry = check.by_property.entry(name.clone()).or_default();
                if eval::canonical_eq(&expected, &got) {
                    check.matched += 1;
                    entry[0] += 1;
                } else {
                    check.mismatched += 1;
                    entry[1] += 1;
                    if check.samples.len() < SAMPLE_LIMIT {
                        check.samples.push(Mismatch {
                            snapshot: snapshot.label.clone(),
                            node,
                            pseudo: pseudo.map(str::to_string),
                            element: describe(snapshot, node),
                            property: name.clone(),
                            captured: expected,
                            resolved: format!("{got}   <- {}", css::to_css(&winner.value)),
                        });
                    }
                }
            }
        }
    }
    check
}

// ----- rewrite equality -------------------------------------------------------

use super::css::{Item, Node, StyleRule, Tok};
use super::selector::{Combinator, Complex, Compound, Simple};
use super::value::{self, Context};
use std::collections::BTreeSet;

/// Definitions (`name -> value`) found in the `@theme` blocks of the central file.
pub fn theme_definitions(central: &Sheet) -> BTreeMap<String, Vec<Component>> {
    let mut out = BTreeMap::new();
    for node in &central.nodes {
        if let Node::At(at) = node
            && at.name == "theme"
        {
            for item in at.items.iter().flatten() {
                if let Item::Decl(d) = item {
                    out.insert(d.name.clone(), d.value.clone());
                }
            }
        }
    }
    out
}

/// The central file as plain CSS: each `@theme` block becomes `:root { ... }`, which is how
/// Tailwind emits it (`:root, :host`).
pub fn theme_as_root_sheet(central: &Sheet) -> Sheet {
    let mut nodes = Vec::new();
    for node in &central.nodes {
        match node {
            Node::At(at) if at.name == "theme" => {
                let selectors = super::selector::SelectorList(vec![Complex {
                    leading: None,
                    compounds: vec![Compound {
                        simples: vec![Simple::Pseudo("root".into())],
                    }],
                    combinators: Vec::<Combinator>::new(),
                }]);
                nodes.push(Node::Style(StyleRule {
                    selectors,
                    items: at.items.clone().unwrap_or_default(),
                    source: ":root".into(),
                }));
            }
            other => nodes.push(other.clone()),
        }
    }
    Sheet { nodes }
}

/// Exact decimal text of a number (six decimals, trailing zeros trimmed): equal numbers
/// print equally, different authored numbers do not collapse.
fn exact(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

fn gsig_into(components: &[Component], ctx: &Context, m: &Metrics, out: &mut String) {
    for c in components {
        match c {
            Component::Tok(Tok::Space) => {
                if !out.ends_with(' ') && !out.ends_with(',') {
                    out.push(' ');
                }
            }
            Component::Tok(Tok::Comma) => {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push(',');
            }
            Component::Tok(Tok::Ident(i)) => match value::parse_color(c) {
                Some(color) => out.push_str(&color.key()),
                None => out.push_str(&i.to_ascii_lowercase()),
            },
            Component::Tok(Tok::Number { value, .. }) => out.push_str(&exact(f64::from(*value))),
            Component::Tok(Tok::Dimension { value, unit, .. }) => {
                let v = f64::from(*value);
                match value::parse_length(c, ctx) {
                    Some(value::Length::Px(px)) => out.push_str(&format!("px:{}", exact(px))),
                    _ => out.push_str(&format!("{}{}", exact(v), unit.to_ascii_lowercase())),
                }
            }
            Component::Tok(Tok::Percentage { unit_value, .. }) => {
                out.push_str(&format!("{}%", exact(f64::from(*unit_value) * 100.0)));
            }
            Component::Tok(Tok::Hash(_) | Tok::IdHash(_)) => match value::parse_color(c) {
                Some(color) => out.push_str(&color.key()),
                None => c.write(out),
            },
            Component::Func(name, inner) => {
                if let Some(color) = value::parse_color(c) {
                    out.push_str(&color.key());
                } else if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "calc" | "min" | "max" | "clamp"
                ) {
                    match eval::to_px(std::slice::from_ref(c), m) {
                        Some(px) => out.push_str(&format!("px:{}", exact(px))),
                        None => {
                            out.push_str(&name.to_ascii_lowercase());
                            out.push('(');
                            gsig_into(inner, ctx, m, out);
                            out.push(')');
                        }
                    }
                } else {
                    out.push_str(&name.to_ascii_lowercase());
                    out.push('(');
                    gsig_into(inner, ctx, m, out);
                    out.push(')');
                }
            }
            Component::Paren(inner) => {
                out.push('(');
                gsig_into(inner, ctx, m, out);
                out.push(')');
            }
            other => other.write(out),
        }
    }
}

/// Normalized signature of a (substituted) value: colors by channel, absolute lengths in px,
/// `calc()` evaluated when it reduces to one length. Equal signatures = equal computed value
/// at the snapshot's root font size and viewport. Comparison is exact (no layout rounding).
pub fn signature(property: &str, value: &[Component], m: &Metrics) -> String {
    let trimmed = css::trim(value);
    let exact_metrics = Metrics {
        layout_rounding: false,
        ..*m
    };
    if let Some(c) = eval::canonical(property, &trimmed, &exact_metrics) {
        return format!("c:{c}");
    }
    let ctx = Context {
        rem_px: Some(m.root_font_size),
    };
    let mut out = String::new();
    gsig_into(&trimmed, &ctx, &exact_metrics, &mut out);
    format!("g:{out}")
}

#[derive(Debug, Clone, Serialize)]
pub struct RewriteDifference {
    pub snapshot: String,
    pub node: usize,
    pub pseudo: Option<String>,
    pub element: String,
    pub property: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RewriteCheck {
    pub snapshot: String,
    pub states: usize,
    pub compared: usize,
    pub equal: usize,
    pub different: usize,
    /// Properties only present after the rewrite because the central file defines them.
    pub introduced_by_tokens: usize,
    pub skipped_uncertain: usize,
    /// Differences caused by a `var()` that resolves on one side only.
    pub unresolvable: usize,
    /// Comparisons (equal or not) of a declaration that references a token: the ones that
    /// actually exercise the rewrite. The rest are untouched declarations or token definitions.
    pub rewritten_compared: usize,
    /// Token name -> compared winners that reference or define it.
    pub exercised: BTreeMap<String, usize>,
    pub samples: Vec<RewriteDifference>,
}

/// Custom property values as the rewritten cascade computes them: nearest ancestor (or the
/// element itself) whose winner defines the name, substituted at that element.
struct Inherited<'a> {
    snapshot: &'a Snapshot,
    cascade: &'a Cascade,
    /// Custom properties the original stylesheets declare. A name outside this set is set by
    /// script or inline style; its value is the same before and after, so the captured one is used.
    declared_before: &'a std::collections::HashSet<String>,
    cache:
        std::cell::RefCell<std::collections::HashMap<usize, std::rc::Rc<super::cascade::Resolved>>>,
    /// Set when a lookup met a custom property whose definition is undecidable.
    hit_uncertain: std::cell::Cell<bool>,
}

impl Inherited<'_> {
    fn resolved(&self, node: usize) -> std::rc::Rc<super::cascade::Resolved> {
        if let Some(r) = self.cache.borrow().get(&node) {
            return r.clone();
        }
        let r = std::rc::Rc::new(self.cascade.resolve(self.snapshot, node, None));
        self.cache.borrow_mut().insert(node, r.clone());
        r
    }

    fn value(&self, node: usize, name: &str, depth: usize) -> Option<Vec<Component>> {
        if depth > 16 {
            return None;
        }
        let mut current = Some(node);
        while let Some(n) = current {
            let r = self.resolved(n);
            if r.uncertain.contains(name) {
                self.hit_uncertain.set(true);
                return None;
            }
            if let Some(w) = r.winners.get(name) {
                let lookup = |inner: &str| self.value(n, inner, depth + 1);
                return eval::substitute_vars(&w.value, &lookup, 0).map(|v| css::trim(&v));
            }
            current = self.snapshot.element(n).and_then(|e| e.parent);
        }
        if !self.declared_before.contains(name) {
            return self
                .snapshot
                .computed(node)
                .and_then(|c| c.get(name))
                .map(|v| css::trim(&css::parse_components(v)));
        }
        None
    }
}

fn missing_vars(
    value: &[Component],
    lookup: &dyn Fn(&str) -> Option<Vec<Component>>,
    out: &mut Vec<String>,
) {
    for c in value {
        match c {
            Component::Func(name, inner) => {
                if name.eq_ignore_ascii_case("var")
                    && let Some(Component::Tok(Tok::Ident(n))) =
                        inner.iter().find(|c| !c.is_whitespace())
                    && lookup(n).is_none()
                    && !out.contains(n)
                {
                    out.push(n.clone());
                }
                missing_vars(inner, lookup, out);
            }
            Component::Paren(inner) | Component::Square(inner) | Component::Curly(inner) => {
                missing_vars(inner, lookup, out)
            }
            _ => {}
        }
    }
}

fn token_refs(value: &[Component], introduced: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    for c in value {
        match c {
            Component::Func(name, inner) => {
                if name.eq_ignore_ascii_case("var")
                    && let Some(Component::Tok(Tok::Ident(n))) =
                        inner.iter().find(|c| !c.is_whitespace())
                    && introduced.contains(n.as_str())
                {
                    out.insert(n.clone());
                }
                token_refs(inner, introduced, out);
            }
            Component::Paren(inner) | Component::Square(inner) | Component::Curly(inner) => {
                token_refs(inner, introduced, out)
            }
            _ => {}
        }
    }
}

/// Compare the winning declarations of every element/pseudo-element before and after a
/// rewrite. Before: `var()` is substituted from the captured computed custom properties.
/// After: from the rewritten cascade itself, with inheritance, so an override that shadows a
/// token is seen. Values are compared exactly by normalized signature; a `var()` that
/// resolves on only one side is a difference.
pub fn verify_rewrite(
    snapshot: &Snapshot,
    before: &Cascade,
    after: &Cascade,
    introduced: &BTreeSet<String>,
) -> RewriteCheck {
    let mut check = RewriteCheck {
        snapshot: snapshot.label.clone(),
        ..Default::default()
    };
    let root_fs = snapshot.root_font_size_px().unwrap_or(16.0);
    let inherited = Inherited {
        snapshot,
        cascade: after,
        declared_before: &before.declared_custom,
        cache: Default::default(),
        hit_uncertain: Default::default(),
    };
    for node in snapshot.elements() {
        let element = snapshot.element(node).expect("element");
        let mut targets: Vec<(Option<&str>, usize)> = vec![(None, element.style)];
        for (key, style) in &element.pseudo {
            targets.push((Some(key.as_str()), *style));
        }
        let parent_fs = element
            .parent
            .and_then(|p| snapshot.computed(p))
            .and_then(|c| px(c.get("font-size")))
            .unwrap_or(16.0);
        for (pseudo, style) in targets {
            check.states += 1;
            let computed = &snapshot.styles[style];
            let captured = |name: &str| -> Option<Vec<Component>> {
                computed
                    .get(name)
                    .map(|v| css::trim(&css::parse_components(v)))
            };
            let rb = before.resolve(snapshot, node, pseudo);
            let ra = after.resolve(snapshot, node, pseudo);
            // A pseudo-element may declare custom properties of its own; otherwise they inherit
            // from the originating element.
            let defined = |name: &str| -> Option<Vec<Component>> {
                if pseudo.is_some()
                    && let Some(w) = ra.winners.get(name)
                {
                    let lookup = |inner: &str| inherited.value(node, inner, 0);
                    return eval::substitute_vars(&w.value, &lookup, 0).map(|v| css::trim(&v));
                }
                inherited.value(node, name, 0)
            };
            let own_fs = px(computed.get("font-size")).unwrap_or(16.0);
            let base = Metrics {
                font_size: own_fs,
                root_font_size: root_fs,
                vw: f64::from(snapshot.width),
                vh: f64::from(snapshot.height),
                layout_rounding: false,
            };
            let names: BTreeSet<&String> = rb.winners.keys().chain(ra.winners.keys()).collect();
            for name in names {
                if rb.uncertain.contains(name) || ra.uncertain.contains(name) {
                    check.skipped_uncertain += 1;
                    continue;
                }
                let metrics = Metrics {
                    font_size: if name == "font-size" {
                        parent_fs
                    } else {
                        own_fs
                    },
                    ..base
                };
                match (rb.winners.get(name), ra.winners.get(name)) {
                    (Some(b), Some(a)) => {
                        // An untouched declaration (same components, no token inside) resolves the
                        // same on both sides as long as the variables it reads are compared in
                        // their own right, which they are.
                        let mut refs = BTreeSet::new();
                        token_refs(&a.value, introduced, &mut refs);
                        if refs.is_empty()
                            && !introduced.contains(name.as_str())
                            && b.value == a.value
                            && b.important == a.important
                        {
                            check.compared += 1;
                            check.equal += 1;
                            continue;
                        }
                        inherited.hit_uncertain.set(false);
                        let sb = eval::substitute_vars(&b.value, &captured, 0);
                        let sa = eval::substitute_vars(&a.value, &defined, 0);
                        if inherited.hit_uncertain.get() {
                            // A variable this value needs is defined by an undecidable rule.
                            check.skipped_uncertain += 1;
                            continue;
                        }
                        check.compared += 1;
                        let (x, y) = match (sb, sa) {
                            (Some(sb), Some(sa)) => (
                                signature(name, &sb, &metrics),
                                signature(name, &sa, &metrics),
                            ),
                            // Invalid at computed-value time on both sides: the property is unset
                            // either way.
                            (None, None) => ("invalid".to_string(), "invalid".to_string()),
                            (Some(sb), None) => {
                                check.unresolvable += 1;
                                let mut missing = Vec::new();
                                missing_vars(&a.value, &defined, &mut missing);
                                (
                                    signature(name, &sb, &metrics),
                                    format!(
                                        "(var() unresolved after the rewrite: {} in {})",
                                        missing.join(", "),
                                        css::to_css(&a.value)
                                    ),
                                )
                            }
                            (None, Some(sa)) => {
                                check.unresolvable += 1;
                                (
                                    "(var() unresolved before the rewrite)".to_string(),
                                    signature(name, &sa, &metrics),
                                )
                            }
                        };
                        // Comparisons of a declaration that references a token are the ones that
                        // exercise the rewrite; a token's own definition counts as verifying that
                        // token, but not as a rewritten declaration.
                        if !refs.is_empty() {
                            check.rewritten_compared += 1;
                        }
                        if introduced.contains(name.as_str()) {
                            refs.insert(name.clone());
                        }
                        if x == y && b.important == a.important {
                            check.equal += 1;
                            for r in refs {
                                *check.exercised.entry(r).or_default() += 1;
                            }
                        } else {
                            check.different += 1;
                            record(&mut check, snapshot, node, pseudo, name, x, y);
                        }
                    }
                    (None, Some(_)) if introduced.contains(name.as_str()) => {
                        check.introduced_by_tokens += 1;
                    }
                    (None, Some(a)) => {
                        check.compared += 1;
                        check.different += 1;
                        record(
                            &mut check,
                            snapshot,
                            node,
                            pseudo,
                            name,
                            "(none)".into(),
                            css::to_css(&a.value),
                        );
                    }
                    (Some(b), None) => {
                        check.compared += 1;
                        check.different += 1;
                        record(
                            &mut check,
                            snapshot,
                            node,
                            pseudo,
                            name,
                            css::to_css(&b.value),
                            "(none)".into(),
                        );
                    }
                    (None, None) => {}
                }
            }
        }
    }
    check
}

fn record(
    check: &mut RewriteCheck,
    snapshot: &Snapshot,
    node: usize,
    pseudo: Option<&str>,
    property: &str,
    before: String,
    after: String,
) {
    if check.samples.len() < SAMPLE_LIMIT {
        check.samples.push(RewriteDifference {
            snapshot: snapshot.label.clone(),
            node,
            pseudo: pseudo.map(str::to_string),
            element: describe(snapshot, node),
            property: property.to_string(),
            before,
            after,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// html > body > div.card with the computed styles a browser would report.
    fn snapshot() -> Corpus {
        let json = serde_json::json!({
            "url": "https://t.test/", "viewport": {"width": 1000, "height": 800}, "color_scheme": "dark",
            "nodes": [
                {"parent": null, "kind": "element", "tag": "html", "attributes": [], "style": 0, "pseudo": {}},
                {"parent": 0, "kind": "element", "tag": "body", "attributes": [], "style": 0, "pseudo": {}},
                {"parent": 1, "kind": "element", "tag": "div", "attributes": [{"name": "class", "value": "card"}], "style": 1, "pseudo": {}}
            ],
            "styles": {"strings": ["font-size", "16px", "color", "rgb(17, 17, 17)", "padding-top", "16px", "--ink", "#111"],
                       "maps": [[[0, 1]], [[0, 1], [2, 3], [4, 5], [6, 7]]]},
            "stylesheets": [{"href": null, "rules": [":root { --ink: #111 } .card { color: var(--ink); padding-top: 16px }"], "media": ""}]
        });
        let mut corpus = Corpus::default();
        corpus
            .add_json("fixture", json.to_string().as_bytes())
            .unwrap();
        corpus
    }

    #[test]
    fn identical_cascades_are_equal_and_a_changed_token_is_detected() {
        let corpus = snapshot();
        let s = &corpus.snapshots[0];
        let run = |tokens: &str, rules: &str| {
            let before_sheets = parse_corpus(&corpus);
            let before = cascade_for(s, &before_sheets);
            let central = css::parse_sheet(tokens);
            let defs = theme_definitions(&central);
            let sheets = [theme_as_root_sheet(&central), css::parse_sheet(rules)];
            let inputs: Vec<SheetInput<'_>> = sheets
                .iter()
                .enumerate()
                .map(|(i, sheet)| SheetInput {
                    sheet,
                    index: i,
                    media: "",
                })
                .collect();
            let after = Cascade::build(&inputs, &environment(s));
            let introduced: BTreeSet<String> = defs.keys().cloned().collect();
            verify_rewrite(s, &before, &after, &introduced)
        };
        let good = run(
            "@theme static { --ink: #111111; --spacing: 4px; }",
            ".card { color: var(--ink); padding-top: calc(var(--spacing) * 4) }",
        );
        assert_eq!(good.different, 0, "{:?}", good.samples);
        assert!(good.compared > 0 && good.equal == good.compared);
        // Mutation 1: a token with the wrong color.
        let bad = run(
            "@theme static { --ink: #222222; --spacing: 4px; }",
            ".card { color: var(--ink); padding-top: 16px }",
        );
        assert!(
            bad.different >= 1 && bad.samples.iter().any(|d| d.property == "color"),
            "{:?}",
            bad.samples
        );
        // Mutation 2: spacing scale off by one step.
        let bad = run(
            "@theme static { --ink: #111111; --spacing: 4px; }",
            ".card { color: var(--ink); padding-top: calc(var(--spacing) * 5) }",
        );
        assert!(
            bad.samples.iter().any(|d| d.property == "padding-top"),
            "{:?}",
            bad.samples
        );
        // Mutation 3: a declaration that disappeared.
        let bad = run(
            "@theme static { --ink: #111111; --spacing: 4px; }",
            ".card { color: var(--ink) }",
        );
        assert!(
            bad.samples
                .iter()
                .any(|d| d.property == "padding-top" && d.after == "(none)"),
            "{:?}",
            bad.samples
        );
    }

    #[test]
    fn an_override_that_shadows_a_token_is_detected() {
        // The central file defines --ink, but a rewritten rule redefines it locally: the
        // element's color changes even though the token definition itself is right.
        let corpus = snapshot();
        let s = &corpus.snapshots[0];
        let before_sheets = parse_corpus(&corpus);
        let before = cascade_for(s, &before_sheets);
        let central = css::parse_sheet("@theme static { --ink: #111111; --spacing: 4px; }");
        let defs = theme_definitions(&central);
        let sheets = [
            theme_as_root_sheet(&central),
            css::parse_sheet(".card { --ink: #000000; color: var(--ink); padding-top: 16px }"),
        ];
        let inputs: Vec<SheetInput<'_>> = sheets
            .iter()
            .enumerate()
            .map(|(i, sheet)| SheetInput {
                sheet,
                index: i,
                media: "",
            })
            .collect();
        let after = Cascade::build(&inputs, &environment(s));
        let introduced: BTreeSet<String> = defs.keys().cloned().collect();
        let check = verify_rewrite(s, &before, &after, &introduced);
        assert!(
            check.samples.iter().any(|d| d.property == "color"),
            "{:?}",
            check.samples
        );
    }

    #[test]
    fn a_variable_that_only_resolves_after_the_rewrite_is_a_difference() {
        let mut corpus = snapshot();
        corpus.sheets[0].texts = vec![".card { color: var(--brand); padding-top: 16px }".into()];
        let s = &corpus.snapshots[0];
        let before_sheets = parse_corpus(&corpus);
        let before = cascade_for(s, &before_sheets);
        let central = css::parse_sheet("@theme static { --brand: #ff0000; }");
        let sheets = [theme_as_root_sheet(&central), before_sheets[0].clone()];
        let inputs: Vec<SheetInput<'_>> = sheets
            .iter()
            .enumerate()
            .map(|(i, sheet)| SheetInput {
                sheet,
                index: i,
                media: "",
            })
            .collect();
        let after = Cascade::build(&inputs, &environment(s));
        let introduced: BTreeSet<String> = theme_definitions(&central).keys().cloned().collect();
        let check = verify_rewrite(s, &before, &after, &introduced);
        assert!(
            check.unresolvable >= 1 && check.samples.iter().any(|d| d.property == "color"),
            "{:?}",
            check
        );
    }

    #[test]
    fn exercised_tokens_are_counted_and_unreferenced_ones_are_not() {
        let corpus = snapshot();
        let s = &corpus.snapshots[0];
        let before_sheets = parse_corpus(&corpus);
        let before = cascade_for(s, &before_sheets);
        let central = css::parse_sheet(
            "@theme static { --ink: #111111; --unused: #123456; --spacing: 4px; }",
        );
        let sheets = [
            theme_as_root_sheet(&central),
            css::parse_sheet(".card { color: var(--ink); padding-top: calc(var(--spacing) * 4) }"),
        ];
        let inputs: Vec<SheetInput<'_>> = sheets
            .iter()
            .enumerate()
            .map(|(i, sheet)| SheetInput {
                sheet,
                index: i,
                media: "",
            })
            .collect();
        let after = Cascade::build(&inputs, &environment(s));
        let introduced: BTreeSet<String> = theme_definitions(&central).keys().cloned().collect();
        let check = verify_rewrite(s, &before, &after, &introduced);
        assert_eq!(check.different, 0, "{:?}", check.samples);
        assert!(check.exercised.contains_key("--ink") && check.exercised.contains_key("--spacing"));
        assert!(check.rewritten_compared >= 2);
        assert!(
            !check.exercised.contains_key("--unused") || check.exercised["--unused"] == 1,
            "only its own definition"
        );
    }

    #[test]
    fn resolver_check_matches_the_captured_computed_style() {
        let corpus = snapshot();
        let sheets = parse_corpus(&corpus);
        let s = &corpus.snapshots[0];
        let cascade = cascade_for(s, &sheets);
        let check = validate_resolver(s, &cascade);
        assert!(check.compared >= 2, "{check:?}");
        assert_eq!(check.mismatched, 0, "{:?}", check.samples);
    }

    #[test]
    fn a_var_fallback_that_starts_resolving_is_reported() {
        // The central file defines --accent for the whole site, but this page's own CSS used
        // `var(--accent, #111)` with no definition: the computed color would change.
        let mut corpus = snapshot();
        corpus.sheets[0].texts =
            vec![".card { color: var(--accent, #111111); padding-top: 16px }".into()];
        let s = &corpus.snapshots[0];
        let before_sheets = parse_corpus(&corpus);
        let before = cascade_for(s, &before_sheets);
        let central = css::parse_sheet("@theme static { --accent: #ff0000; }");
        let defs = theme_definitions(&central);
        let sheets = [theme_as_root_sheet(&central), before_sheets[0].clone()];
        let inputs: Vec<SheetInput<'_>> = sheets
            .iter()
            .enumerate()
            .map(|(i, sheet)| SheetInput {
                sheet,
                index: i,
                media: "",
            })
            .collect();
        let after = Cascade::build(&inputs, &environment(s));
        let introduced: BTreeSet<String> = defs.keys().cloned().collect();
        let check = verify_rewrite(s, &before, &after, &introduced);
        assert!(
            check.samples.iter().any(|d| d.property == "color"),
            "{:?}",
            check.samples
        );
    }
}
