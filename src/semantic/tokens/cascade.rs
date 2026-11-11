//! Specificity-aware cascade resolution over the CSS AST for one captured snapshot.
//!
//! Implemented from CSS Cascade 5: importance, `style=""`, `@layer` order (normal and
//! reversed for `!important`), selector specificity and source order, plus CSS Nesting
//! (`&` expands to `:is(parent)`). Conditions that cannot be decided
//! ([`Tri::Unknown`]) never select a rule; instead the rule's property names are
//! reported as *uncertain* on every element it might match so callers can exclude
//! them from equality claims.
//!
//! Not modelled (reported as gaps by callers): animations/transitions in flight,
//! `@scope`, `@container`, shadow DOM, `revert`/`revert-layer` keywords, user-agent
//! and user origins.

use super::css::{self, AtRule, Component, Declaration, Item, Node, Sheet, StyleRule, Tok};
use super::eval::{self, Expansion};
use super::media::{self, Environment};
use super::selector::{
    self, Combinator, Complex, Compound, Dom, SelectorList, Simple, Specificity, Tri,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

/// Layer order: the first appearance of a layer name fixes its place among siblings.
#[derive(Debug, Default)]
struct LayerTree {
    children: Vec<(String, Vec<usize>)>, // (name, child indices) arena; index 0 is the implicit root
    root: Vec<usize>,
    anonymous: usize,
}

impl LayerTree {
    /// Intern a layer path and return its rank path (sibling index per level).
    fn intern(&mut self, path: &[String]) -> Vec<usize> {
        let mut ranks = Vec::new();
        let mut siblings_is_root = true;
        let mut parent = 0usize;
        for name in path {
            let list: Vec<usize> = if siblings_is_root {
                self.root.clone()
            } else {
                self.children[parent].1.clone()
            };
            let found = list.iter().position(|&i| self.children[i].0 == *name);
            let (index, rank) = match found {
                Some(rank) => (list[rank], rank),
                None => {
                    self.children.push((name.clone(), Vec::new()));
                    let index = self.children.len() - 1;
                    if siblings_is_root {
                        self.root.push(index);
                    } else {
                        self.children[parent].1.push(index);
                    }
                    (index, list.len())
                }
            };
            ranks.push(rank);
            parent = index;
            siblings_is_root = false;
        }
        ranks
    }

    fn anonymous_name(&mut self) -> String {
        self.anonymous += 1;
        format!("\u{0}anonymous-{}", self.anonymous)
    }
}

/// `None` = unlayered.
type LayerRank = Option<Vec<usize>>;

fn layer_cmp(a: &LayerRank, b: &LayerRank) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            for (p, q) in x.iter().zip(y.iter()) {
                if p != q {
                    return p.cmp(q);
                }
            }
            // A layer's own declarations beat those of its sub-layers.
            y.len().cmp(&x.len())
        }
    }
}

/// See [`Cascade::opacity_toggles`].
#[derive(Debug, Default, Clone)]
pub struct Toggles {
    /// Some rule that would make the element visible is a hover/focus/open/active style toggle.
    pub other: bool,
    /// Some such rule is a once-only reveal (`.is-revealed`, `[data-revealed]`, `.in-view`).
    pub reveal: bool,
    /// The `opacity`/`translate`/`transform`/`scale`/`rotate` values the reveal rules declare.
    pub end: BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct FlatRule {
    pub complex: Complex,
    pub decls: Rc<Vec<Declaration>>,
    pub layer: LayerRank,
    pub order: u32,
    pub sheet: usize,
    /// AND of the enclosing media/supports/container conditions.
    pub active: Tri,
    /// The enclosing conditions as written, outermost first, joined with ` and `.
    pub condition: String,
    pub specificity: Specificity,
}

#[derive(Debug, Clone)]
pub struct Winner {
    pub value: Vec<Component>,
    pub important: bool,
    pub from_inline: bool,
    /// Longhand set by a shorthand (or unresolved `var()`) whose value is not known here.
    pub opaque: bool,
    pub rule: Option<u32>,
    /// Position of the declaration inside its rule (source order key together with `rule`).
    pub idx: usize,
    pub specificity: Specificity,
    /// Layer the declaration came from (`None` = unlayered).
    pub layer: Option<Vec<usize>>,
}

/// Cascade precedence of two winners of the same element: `Less` = `a` is overridden by `b`.
pub fn winner_cmp(a: &Winner, b: &Winner) -> Ordering {
    if a.important != b.important {
        return a.important.cmp(&b.important);
    }
    if a.from_inline != b.from_inline {
        return a.from_inline.cmp(&b.from_inline);
    }
    let layers = if a.important {
        layer_cmp(&a.layer, &b.layer).reverse()
    } else {
        layer_cmp(&a.layer, &b.layer)
    };
    layers
        .then(a.specificity.cmp(&b.specificity))
        .then((a.rule, a.idx).cmp(&(b.rule, b.idx)))
}

#[derive(Debug, Default)]
pub struct Resolved {
    pub winners: BTreeMap<String, Winner>,
    /// Properties declared by a rule that might match this element but could not be decided.
    pub uncertain: BTreeSet<String>,
}

#[derive(Debug, Default, Clone)]
pub struct CascadeStats {
    pub rules: usize,
    pub inactive_rules: usize,
    pub unknown_condition_rules: usize,
    pub skipped_at_rules: BTreeMap<String, usize>,
}

/// Supplies the value of a custom property by name.
pub type VarLookup<'a> = dyn Fn(&str) -> Option<Vec<Component>> + 'a;

pub struct Cascade {
    pub rules: Vec<FlatRule>,
    /// Custom properties some rule declares (script- or inline-set ones are not here).
    pub declared_custom: std::collections::HashSet<String>,
    /// Properties animated by each `@keyframes` name (animations override the cascade).
    pub keyframes: HashMap<String, BTreeSet<String>>,
    index_id: HashMap<String, Vec<usize>>,
    index_class: HashMap<String, Vec<usize>>,
    index_tag: HashMap<String, Vec<usize>>,
    universal: Vec<usize>,
    pub stats: CascadeStats,
}

struct Builder<'a> {
    env: &'a Environment,
    layers: LayerTree,
    rules: Vec<FlatRule>,
    order: u32,
    sheet: usize,
    stats: CascadeStats,
    keyframes: HashMap<String, BTreeSet<String>>,
}

#[derive(Clone)]
struct Ctx {
    layer: Vec<String>,
    active: Tri,
    /// Enclosing conditional at-rules as written (`@media (min-width: 40rem)`).
    cond: Vec<String>,
}

fn and(a: Tri, b: Tri) -> Tri {
    match (a, b) {
        (Tri::No, _) | (_, Tri::No) => Tri::No,
        (Tri::Yes, Tri::Yes) => Tri::Yes,
        _ => Tri::Unknown,
    }
}

fn layer_names(prelude: &[Component]) -> Vec<Vec<String>> {
    // `a.b, c` -> [[a, b], [c]]
    let mut out: Vec<Vec<String>> = vec![Vec::new()];
    for c in prelude {
        match c {
            Component::Tok(Tok::Ident(name)) => {
                out.last_mut().expect("non-empty").push(name.clone())
            }
            Component::Tok(Tok::Comma) => out.push(Vec::new()),
            _ => {}
        }
    }
    out.into_iter().filter(|p| !p.is_empty()).collect()
}

fn nest_simple(simple: &Simple, parent: &SelectorList) -> Simple {
    match simple {
        Simple::Nesting => Simple::Is(parent.clone()),
        Simple::Is(l) => Simple::Is(nest_list(l, parent)),
        Simple::Where(l) => Simple::Where(nest_list(l, parent)),
        Simple::Not(l) => Simple::Not(nest_list(l, parent)),
        other => other.clone(),
    }
}

fn nest_list(list: &SelectorList, parent: &SelectorList) -> SelectorList {
    SelectorList(list.0.iter().map(|c| nest_complex(c, parent)).collect())
}

fn has_nesting(complex: &Complex) -> bool {
    fn simple_has(s: &Simple) -> bool {
        match s {
            Simple::Nesting => true,
            Simple::Is(l) | Simple::Where(l) | Simple::Not(l) => l.0.iter().any(has_nesting),
            _ => false,
        }
    }
    complex
        .compounds
        .iter()
        .any(|c| c.simples.iter().any(simple_has))
}

/// Expand `&` and implicit relative nesting into an absolute complex selector.
fn nest_complex(child: &Complex, parent: &SelectorList) -> Complex {
    let mut compounds: Vec<Compound> = child
        .compounds
        .iter()
        .map(|c| Compound {
            simples: c.simples.iter().map(|s| nest_simple(s, parent)).collect(),
        })
        .collect();
    let mut combinators = child.combinators.clone();
    if !has_nesting(child) {
        // Implicit `& ` (or `& >` etc. for a leading combinator).
        compounds.insert(
            0,
            Compound {
                simples: vec![Simple::Is(parent.clone())],
            },
        );
        combinators.insert(0, child.leading.unwrap_or(Combinator::Descendant));
    }
    Complex {
        leading: None,
        compounds,
        combinators,
    }
}

impl<'a> Builder<'a> {
    fn push_rule(&mut self, complex: Complex, decls: &Rc<Vec<Declaration>>, ctx: &Ctx) {
        if decls.is_empty() {
            return;
        }
        let layer = if ctx.layer.is_empty() {
            None
        } else {
            Some(self.layers.intern(&ctx.layer))
        };
        let specificity = complex.specificity();
        self.rules.push(FlatRule {
            complex,
            decls: decls.clone(),
            layer,
            order: self.order,
            sheet: self.sheet,
            active: ctx.active,
            condition: ctx.cond.join(" and "),
            specificity,
        });
        self.order += 1;
    }

    fn nodes(&mut self, nodes: &[Node], ctx: &Ctx) {
        for node in nodes {
            match node {
                Node::Style(rule) => self.style(rule, None, ctx),
                Node::At(at) => self.at_rule(at, ctx),
                Node::Frame { .. } | Node::Raw { .. } => {}
            }
        }
    }

    fn style(&mut self, rule: &StyleRule, parent: Option<&SelectorList>, ctx: &Ctx) {
        let selectors = match parent {
            Some(p) => nest_list(&rule.selectors, p),
            None => rule.selectors.clone(),
        };
        self.style_items(&selectors, &rule.items, ctx);
    }

    fn style_items(&mut self, selectors: &SelectorList, items: &[Item], ctx: &Ctx) {
        let mut run: Vec<Declaration> = Vec::new();
        let flush = |this: &mut Self, run: &mut Vec<Declaration>| {
            if run.is_empty() {
                return;
            }
            let decls = Rc::new(std::mem::take(run));
            for complex in &selectors.0 {
                this.push_rule(complex.clone(), &decls, ctx);
            }
        };
        for item in items {
            match item {
                Item::Decl(d) => run.push(d.clone()),
                Item::Rule(node) => {
                    flush(self, &mut run);
                    match node {
                        Node::Style(child) => self.style(child, Some(selectors), ctx),
                        Node::At(at) => self.nested_at(at, selectors, ctx),
                        Node::Frame { .. } | Node::Raw { .. } => {}
                    }
                }
            }
        }
        flush(self, &mut run);
    }

    /// A conditional group rule nested in a style rule applies to the parent selector.
    fn nested_at(&mut self, at: &AtRule, selectors: &SelectorList, ctx: &Ctx) {
        let Some(items) = &at.items else { return };
        let mut inner = ctx.clone();
        if matches!(
            at.name.as_str(),
            "media" | "supports" | "container" | "scope"
        ) {
            inner
                .cond
                .push(format!("@{} {}", at.name, css::to_css(&at.prelude)));
        }
        match at.name.as_str() {
            "media" => inner.active = and(inner.active, media::eval_media(&at.prelude, self.env)),
            "supports" => inner.active = and(inner.active, media::eval_supports(&at.prelude)),
            "container" | "scope" => inner.active = and(inner.active, Tri::Unknown),
            "layer" => {
                let name = layer_names(&at.prelude)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| vec![self.layers.anonymous_name()]);
                inner.layer.extend(name);
            }
            "starting-style" => return,
            other => {
                *self
                    .stats
                    .skipped_at_rules
                    .entry(other.to_string())
                    .or_default() += 1;
                return;
            }
        }
        self.style_items(selectors, items, &inner);
    }

    fn at_rule(&mut self, at: &AtRule, ctx: &Ctx) {
        match at.name.as_str() {
            "media" | "supports" | "container" | "scope" | "layer" | "document" => {
                let mut inner = ctx.clone();
                if matches!(
                    at.name.as_str(),
                    "media" | "supports" | "container" | "scope" | "document"
                ) {
                    inner
                        .cond
                        .push(format!("@{} {}", at.name, css::to_css(&at.prelude)));
                }
                match at.name.as_str() {
                    "media" => {
                        inner.active = and(inner.active, media::eval_media(&at.prelude, self.env))
                    }
                    "supports" => {
                        inner.active = and(inner.active, media::eval_supports(&at.prelude))
                    }
                    "container" | "scope" | "document" => {
                        inner.active = and(inner.active, Tri::Unknown)
                    }
                    _ => {
                        let names = layer_names(&at.prelude);
                        match &at.items {
                            None => {
                                // `@layer a, b;` fixes order on first appearance.
                                for name in names {
                                    let mut full = ctx.layer.clone();
                                    full.extend(name);
                                    self.layers.intern(&full);
                                }
                                return;
                            }
                            Some(_) => {
                                let name = names
                                    .into_iter()
                                    .next()
                                    .unwrap_or_else(|| vec![self.layers.anonymous_name()]);
                                inner.layer.extend(name);
                                self.layers.intern(&inner.layer);
                            }
                        }
                    }
                }
                if let Some(items) = &at.items {
                    let nodes: Vec<Node> = items
                        .iter()
                        .filter_map(|i| match i {
                            Item::Rule(n) => Some(n.clone()),
                            Item::Decl(_) => None,
                        })
                        .collect();
                    self.nodes(&nodes, &inner);
                }
            }
            "keyframes" | "-webkit-keyframes" => {
                let name = css::to_css(&at.prelude)
                    .trim_matches(|c| c == '"' || c == '\'')
                    .to_string();
                let props = self.keyframes.entry(name).or_default();
                for item in at.items.iter().flatten() {
                    if let Item::Rule(Node::Frame { items, .. }) = item {
                        for inner in items {
                            if let Item::Decl(d) = inner {
                                props.insert(d.name.clone());
                            }
                        }
                    }
                }
            }
            "font-face"
            | "property"
            | "page"
            | "counter-style"
            | "font-feature-values"
            | "font-palette-values"
            | "position-try"
            | "view-transition"
            | "import"
            | "charset"
            | "namespace"
            | "starting-style" => {}
            other => {
                *self
                    .stats
                    .skipped_at_rules
                    .entry(other.to_string())
                    .or_default() += 1;
            }
        }
    }
}

/// One stylesheet participating in a snapshot, in cascade order.
pub struct SheetInput<'a> {
    pub sheet: &'a Sheet,
    pub index: usize,
    pub media: &'a str,
}

fn key_of(complex: &Complex) -> (Option<&'static str>, Option<String>) {
    let Some(last) = complex.compounds.last() else {
        return (None, None);
    };
    for simple in &last.simples {
        if let Simple::Id(name) = simple {
            return (Some("id"), Some(name.clone()));
        }
    }
    for simple in &last.simples {
        if let Simple::Class(name) = simple {
            return (Some("class"), Some(name.clone()));
        }
    }
    for simple in &last.simples {
        if let Simple::Type(name) = simple {
            return (Some("tag"), Some(name.clone()));
        }
    }
    (None, None)
}

impl Cascade {
    pub fn build(sheets: &[SheetInput<'_>], env: &Environment) -> Cascade {
        let mut builder = Builder {
            env,
            layers: LayerTree::default(),
            rules: Vec::new(),
            order: 0,
            sheet: 0,
            stats: CascadeStats::default(),
            keyframes: HashMap::new(),
        };
        for input in sheets {
            builder.sheet = input.index;
            let mut ctx = Ctx {
                layer: Vec::new(),
                active: Tri::Yes,
                cond: Vec::new(),
            };
            if !input.media.trim().is_empty() {
                ctx.active = and(
                    ctx.active,
                    media::eval_media(&css::parse_components(input.media), env),
                );
            }
            builder.nodes(&input.sheet.nodes, &ctx);
        }
        let declared_custom = builder
            .rules
            .iter()
            .flat_map(|r| r.decls.iter())
            .filter(|d| d.is_custom())
            .map(|d| d.name.clone())
            .collect();
        let mut cascade = Cascade {
            declared_custom,
            rules: builder.rules,
            index_id: HashMap::new(),
            index_class: HashMap::new(),
            index_tag: HashMap::new(),
            universal: Vec::new(),
            stats: builder.stats,
            keyframes: builder.keyframes,
        };
        cascade.stats.rules = cascade.rules.len();
        for (i, rule) in cascade.rules.iter().enumerate() {
            match rule.active {
                Tri::No => cascade.stats.inactive_rules += 1,
                Tri::Unknown => cascade.stats.unknown_condition_rules += 1,
                Tri::Yes => {}
            }
            match key_of(&rule.complex) {
                (Some("id"), Some(k)) => cascade.index_id.entry(k).or_default().push(i),
                (Some("class"), Some(k)) => cascade.index_class.entry(k).or_default().push(i),
                (Some("tag"), Some(k)) => cascade.index_tag.entry(k).or_default().push(i),
                _ => cascade.universal.push(i),
            }
        }
        cascade
    }

    /// What the environment decided: which rules are active. Two environments with the same
    /// fingerprint resolve every element identically.
    pub fn fingerprint(&self) -> Vec<u8> {
        self.rules
            .iter()
            .map(|r| match r.active {
                Tri::Yes => 1,
                Tri::No => 0,
                Tri::Unknown => 2,
            })
            .collect()
    }

    fn candidates<D: Dom + ?Sized>(&self, dom: &D, node: usize) -> Vec<usize> {
        let mut out: Vec<usize> = self.universal.clone();
        if let Some(id) = dom.attr(node, "id")
            && let Some(v) = self.index_id.get(id)
        {
            out.extend(v);
        }
        if let Some(classes) = dom.attr(node, "class") {
            for class in classes.split_ascii_whitespace() {
                if let Some(v) = self.index_class.get(class) {
                    out.extend(v);
                }
            }
        }
        if let Some(v) = self.index_tag.get(dom.tag(node)) {
            out.extend(v);
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Rules that could make this element visible later than the capture shows it: they declare a
    /// non-zero `opacity`, their last compound can match the element, and they are not already
    /// part of its cascade. Classified by the words in their selector: a hover/focus/open/active
    /// style toggle (`other`), or a once-only reveal class/attribute (`revealed`, `in-view`,
    /// `is-visible`), whose declared end values (`opacity`, `translate` ...) are collected.
    pub fn opacity_toggles<D: Dom + ?Sized>(&self, dom: &D, node: usize) -> Toggles {
        const OTHER: &[&str] = &[
            "hover",
            "focus",
            "active",
            "open",
            "opened",
            "closed",
            "expanded",
            "collapsed",
            "selected",
            "checked",
            "current",
            "show",
            "shown",
            "pressed",
            "disabled",
            "state",
            "on",
            "visited",
            "target",
            "hidden",
        ];
        const REVEAL: &[&str] = &["revealed", "inview", "appeared", "entered"];
        const END_PROPS: &[&str] = &["opacity", "translate", "transform", "scale", "rotate"];
        let words = |t: &str| -> Vec<String> {
            t.to_ascii_lowercase()
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        };
        let is_other = |w: &[String]| w.iter().any(|t| OTHER.contains(&t.as_str()));
        let is_reveal = |w: &[String]| {
            w.iter().any(|t| REVEAL.contains(&t.as_str()))
                || w.windows(2).any(|p| p[0] == "in" && p[1] == "view")
        };
        let toggling = |text: &str| {
            let w = words(text);
            is_other(&w) || is_reveal(&w)
        };
        // A compound without its toggling parts (`.is-revealed`, `[data-state=open]`, `:hover`,
        // `:not(.is-hidden)`): what the rule is about when the toggle is off.
        let strip = |c: &Compound| Compound {
            simples: c
                .simples
                .iter()
                .filter(|s| match s {
                    Simple::Class(n) => !toggling(n),
                    Simple::Attr { name, value, .. } => !toggling(name) && !toggling(value),
                    Simple::Pseudo(_) => false,
                    Simple::Is(l) | Simple::Where(l) | Simple::Not(l) | Simple::Has(l) => {
                        !toggling(&l.to_css())
                    }
                    _ => true,
                })
                .cloned()
                .collect(),
        };
        let mut out = Toggles::default();
        for rule in &self.rules {
            if rule.active == Tri::No {
                continue;
            }
            let Some(last) = rule.complex.compounds.last() else {
                continue;
            };
            if last.pseudo_element().is_some() {
                continue;
            }
            let shows = rule.decls.iter().any(|d| {
                d.name == "opacity" && {
                    let v = css::to_css(&css::trim(&d.value));
                    !v.is_empty() && v != "0" && v != "0%"
                }
            });
            if !shows {
                continue;
            }
            let stripped = Complex {
                leading: rule.complex.leading,
                compounds: rule.complex.compounds.iter().map(strip).collect(),
                combinators: rule.complex.combinators.clone(),
            };
            let last_left = stripped.compounds.last().map_or(0, |c| c.simples.len());
            // A rule about nothing but its toggle (`[data-state=open]`, `:disabled`) says nothing
            // about this element unless the element carries the toggling attribute itself.
            if last_left == 0 {
                let attrs: Vec<&String> = last
                    .simples
                    .iter()
                    .filter_map(|s| match s {
                        Simple::Attr { name, .. } => Some(name),
                        _ => None,
                    })
                    .collect();
                if attrs.is_empty() || !attrs.iter().all(|n| dom.attr(node, n).is_some()) {
                    continue;
                }
            } else if selector::complex_match(&stripped, dom, node) == Tri::No {
                continue;
            }
            if selector::complex_match(&rule.complex, dom, node) == Tri::Yes {
                continue;
            }
            let w = words(&rule.complex.to_css());
            if is_other(&w) {
                out.other = true;
            } else if is_reveal(&w) {
                out.reveal = true;
                for d in rule.decls.iter() {
                    if END_PROPS.contains(&d.name.as_str()) {
                        out.end
                            .insert(d.name.clone(), css::to_css(&css::trim(&d.value)));
                    }
                }
            }
        }
        out
    }

    /// Could forcing an interactive state on `node` (the `dom` already says which) change what
    /// the element resolves to? False when no rule that names `:hover`, `:focus`, ... on its
    /// subject matches it in that state, so the state's layer equals the resting one.
    pub fn state_matters<D: Dom + ?Sized>(&self, dom: &D, node: usize) -> bool {
        fn mentions(simples: &[Simple]) -> bool {
            simples.iter().any(|s| match s {
                Simple::Pseudo(n) => matches!(
                    n.as_str(),
                    "hover" | "active" | "focus" | "focus-visible" | "focus-within"
                ),
                Simple::Is(l) | Simple::Where(l) | Simple::Not(l) => {
                    l.0.iter()
                        .any(|c| c.compounds.iter().any(|k| mentions(&k.simples)))
                }
                _ => false,
            })
        }
        self.candidates(dom, node).into_iter().any(|i| {
            let rule = &self.rules[i];
            rule.active != Tri::No
                && rule
                    .complex
                    .compounds
                    .last()
                    .is_some_and(|last| last.pseudo_element().is_none() && mentions(&last.simples))
                && selector::complex_match(&rule.complex, dom, node) != Tri::No
        })
    }

    /// Resolve the cascade for an element (`pseudo = None`) or one of its pseudo-elements.
    /// Winners are keyed by the property name as written (shorthands stay shorthands).
    pub fn resolve<D: Dom + ?Sized>(&self, dom: &D, node: usize, pseudo: Option<&str>) -> Resolved {
        self.resolve_with(dom, node, pseudo, None)
    }

    /// Like [`Cascade::resolve`] but declarations are expanded to longhands first (see
    /// [`eval::expand`]); `lookup` supplies custom property values for `var()` inside
    /// shorthand values. Longhands set by a shorthand we cannot expand win as *opaque*.
    pub fn resolve_longhands<D: Dom + ?Sized>(
        &self,
        dom: &D,
        node: usize,
        pseudo: Option<&str>,
        lookup: &VarLookup,
    ) -> Resolved {
        self.resolve_with(dom, node, pseudo, Some(lookup))
    }

    fn resolve_with<D: Dom + ?Sized>(
        &self,
        dom: &D,
        node: usize,
        pseudo: Option<&str>,
        expand: Option<&VarLookup>,
    ) -> Resolved {
        struct Cand {
            value: Vec<Component>,
            important: bool,
            opaque: bool,
            rule: Option<u32>,
            idx: usize,
            layer: LayerRank,
            spec: Specificity,
            inline: bool,
        }
        let mut resolved = Resolved::default();
        let mut best: BTreeMap<String, Cand> = BTreeMap::new();
        let none_layer: LayerRank = None;
        let inline_decls: Vec<Declaration> = if pseudo.is_none() {
            dom.attr(node, "style")
                .map(css::parse_declarations)
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let better = |a: &Cand, b: &Cand| -> bool {
            // true when `a` beats `b`
            if a.important != b.important {
                return a.important;
            }
            if a.inline != b.inline {
                return a.inline;
            }
            let layers = if a.important {
                layer_cmp(&a.layer, &b.layer).reverse()
            } else {
                layer_cmp(&a.layer, &b.layer)
            };
            if layers != Ordering::Equal {
                return layers == Ordering::Greater;
            }
            if a.spec != b.spec {
                return a.spec > b.spec;
            }
            (a.rule, a.idx) > (b.rule, b.idx)
        };

        // Declarations of one origin, expanded when requested.
        let offer = |best: &mut BTreeMap<String, Cand>,
                     decl: &Declaration,
                     rule: Option<u32>,
                     idx: usize,
                     layer: &LayerRank,
                     spec: Specificity,
                     inline: bool| {
            let mut parts: Vec<(String, Vec<Component>, bool)> = Vec::new();
            match expand {
                None => parts.push((decl.name.clone(), decl.value.clone(), false)),
                Some(lookup) if !decl.is_custom() => {
                    let substituted = if eval::needs_substitution(&decl.value) {
                        eval::substitute_vars(&decl.value, lookup, 0)
                    } else {
                        Some(decl.value.clone())
                    };
                    match substituted {
                        Some(value) => match eval::expand(&decl.name, &value) {
                            Expansion::Longhands(list) => {
                                parts.extend(list.into_iter().map(|(n, v)| (n, v, false)));
                            }
                            Expansion::Opaque(list) => {
                                parts.extend(
                                    list.into_iter().map(|n| (n.to_string(), Vec::new(), true)),
                                );
                            }
                            Expansion::Not => parts.push((decl.name.clone(), value, false)),
                        },
                        None => {
                            // var() could not be resolved: the property value is unknown.
                            let covered = eval::opaque_of(&decl.name);
                            if covered.is_empty() {
                                parts.push((decl.name.clone(), Vec::new(), true));
                            } else {
                                parts.extend(
                                    covered
                                        .into_iter()
                                        .map(|n| (n.to_string(), Vec::new(), true)),
                                );
                            }
                        }
                    }
                }
                Some(_) => parts.push((decl.name.clone(), decl.value.clone(), false)),
            }
            for (name, value, opaque) in parts {
                let cand = Cand {
                    value,
                    important: decl.important,
                    opaque,
                    rule,
                    idx,
                    layer: layer.clone(),
                    spec,
                    inline,
                };
                match best.get(&name) {
                    Some(current) if !better(&cand, current) => {}
                    _ => {
                        best.insert(name, cand);
                    }
                }
            }
        };

        for rule_index in self.candidates(dom, node) {
            let rule = &self.rules[rule_index];
            if rule.active == Tri::No || rule.complex.pseudo_element() != pseudo {
                continue;
            }
            let matched = selector::complex_match(&rule.complex, dom, node);
            match and(matched, rule.active) {
                Tri::No => continue,
                Tri::Unknown => {
                    for d in rule.decls.iter() {
                        resolved.uncertain.insert(d.name.clone());
                        if expand.is_some() {
                            for covered in eval::opaque_of(&d.name) {
                                resolved.uncertain.insert(covered.to_string());
                            }
                        }
                    }
                    continue;
                }
                Tri::Yes => {}
            }
            for (idx, decl) in rule.decls.iter().enumerate() {
                offer(
                    &mut best,
                    decl,
                    Some(rule.order),
                    idx,
                    &rule.layer,
                    rule.specificity,
                    false,
                );
            }
        }
        for (idx, decl) in inline_decls.iter().enumerate() {
            offer(&mut best, decl, None, idx, &none_layer, (0, 0, 0), true);
        }
        for (name, cand) in best {
            resolved.winners.insert(
                name,
                Winner {
                    value: cand.value,
                    important: cand.important,
                    from_inline: cand.inline,
                    opaque: cand.opaque,
                    rule: cand.rule,
                    idx: cand.idx,
                    specificity: cand.spec,
                    layer: cand.layer,
                },
            );
        }
        resolved
    }
}

#[cfg(test)]
mod tests {
    use super::super::selector::Dom;
    use super::*;

    struct T {
        tags: Vec<&'static str>,
        attrs: Vec<Vec<(&'static str, &'static str)>>,
        parents: Vec<Option<usize>>,
        children: Vec<Vec<usize>>,
    }
    type TestNode = (
        &'static str,
        Vec<(&'static str, &'static str)>,
        Option<usize>,
    );

    impl T {
        fn new(nodes: Vec<TestNode>) -> T {
            let mut children = vec![Vec::new(); nodes.len()];
            for (i, n) in nodes.iter().enumerate() {
                if let Some(p) = n.2 {
                    children[p].push(i);
                }
            }
            T {
                tags: nodes.iter().map(|n| n.0).collect(),
                attrs: nodes.iter().map(|n| n.1.clone()).collect(),
                parents: nodes.iter().map(|n| n.2).collect(),
                children,
            }
        }
    }
    impl Dom for T {
        fn tag(&self, n: usize) -> &str {
            self.tags[n]
        }
        fn attr(&self, n: usize, name: &str) -> Option<&str> {
            self.attrs[n]
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| *v)
        }
        fn parent(&self, n: usize) -> Option<usize> {
            self.parents[n]
        }
        fn element_children(&self, n: usize) -> &[usize] {
            &self.children[n]
        }
        fn is_empty(&self, n: usize) -> bool {
            self.children[n].is_empty()
        }
        fn state(&self, _: usize, name: &str) -> Tri {
            match name {
                "hover" | "active" | "focus" | "focus-visible" | "focus-within" => Tri::No,
                _ => Tri::Unknown,
            }
        }
    }

    fn run(css_text: &str, width: f64, node: usize, tree: &T) -> Resolved {
        let sheet = css::parse_sheet(css_text);
        let env = Environment {
            width,
            height: 900.0,
            color_scheme: "dark".into(),
            reduced_motion: false,
        };
        let cascade = Cascade::build(
            &[SheetInput {
                sheet: &sheet,
                index: 0,
                media: "",
            }],
            &env,
        );
        cascade.resolve(tree, node, None)
    }

    fn val(r: &Resolved, name: &str) -> String {
        css::to_css(&r.winners[name].value)
    }

    fn tree() -> T {
        T::new(vec![
            ("html", vec![], None),
            ("body", vec![], Some(0)),
            ("p", vec![("class", "a b"), ("id", "x")], Some(1)),
        ])
    }

    #[test]
    fn specificity_order_and_important() {
        let t = tree();
        let r = run(
            "p { color: red } .a { color: blue } #x { color: green } p.b { color: pink }",
            1000.0,
            2,
            &t,
        );
        assert_eq!(val(&r, "color"), "green");
        let r = run(
            "#x { color: green } p { color: red !important } .a { color: blue }",
            1000.0,
            2,
            &t,
        );
        assert_eq!(val(&r, "color"), "red");
        let r = run(".a { color: red } .b { color: blue }", 1000.0, 2, &t);
        assert_eq!(
            val(&r, "color"),
            "blue",
            "later rule wins at equal specificity"
        );
    }

    #[test]
    fn inline_style_beats_author_rules_but_not_important() {
        let t = T::new(vec![
            ("html", vec![], None),
            (
                "div",
                vec![("style", "color: red; margin: 1px !important"), ("id", "d")],
                Some(0),
            ),
        ]);
        let r = run("#d { color: blue; margin: 5px !important }", 1000.0, 1, &t);
        assert_eq!(val(&r, "color"), "red");
        assert_eq!(
            val(&r, "margin"),
            "1px",
            "important inline beats important author"
        );
    }

    #[test]
    fn layers_unlayered_wins_and_important_reverses() {
        let t = tree();
        let text = "@layer base, theme; @layer theme { p { color: red } } @layer base { p { color: blue } } p { margin: 1px }";
        let r = run(text, 1000.0, 2, &t);
        assert_eq!(val(&r, "color"), "red", "later layer beats earlier layer");
        let r = run(
            "@layer a, b; @layer a { p { color: red !important } } @layer b { p { color: blue !important } } p { color: green !important }",
            1000.0,
            2,
            &t,
        );
        assert_eq!(
            val(&r, "color"),
            "red",
            "important: earlier layer wins, unlayered loses"
        );
        let r = run(
            "@layer a { p { color: red } } p { color: blue }",
            1000.0,
            2,
            &t,
        );
        assert_eq!(val(&r, "color"), "blue");
        let r = run(
            "@layer a { @layer b { p { color: red } } p { color: blue } }",
            1000.0,
            2,
            &t,
        );
        assert_eq!(val(&r, "color"), "blue", "own declarations beat sub-layers");
    }

    #[test]
    fn media_queries_follow_the_snapshot() {
        let t = tree();
        let text = "p { color: red } @media (min-width: 768px) { p { color: blue } }";
        assert_eq!(val(&run(text, 1440.0, 2, &t), "color"), "blue");
        assert_eq!(val(&run(text, 375.0, 2, &t), "color"), "red");
    }

    #[test]
    fn nesting_expands_parent_selectors() {
        let t = tree();
        let text =
            ".a { color: red; & .never { color: pink } &.b { margin: 2px } p#x { padding: 3px } }";
        // `p#x` nested without `&` means `.a p#x`, which does not match (p is the .a element).
        let r = run(text, 1000.0, 2, &t);
        assert_eq!(val(&r, "color"), "red");
        assert_eq!(val(&r, "margin"), "2px");
        assert!(!r.winners.contains_key("padding"));
        let text = "body { > p { color: green } }";
        assert_eq!(val(&run(text, 1000.0, 2, &t), "color"), "green");
        let text = "p { @media (min-width: 100px) { color: teal } }";
        assert_eq!(val(&run(text, 1000.0, 2, &t), "color"), "teal");
    }

    #[test]
    fn undecidable_rules_mark_properties_uncertain_instead_of_guessing() {
        let t = tree();
        let text = "p { color: red } p:lang(fr) { color: blue; margin: 3px } @container (width > 10px) { p { padding: 1px } }";
        let r = run(text, 1000.0, 2, &t);
        assert_eq!(val(&r, "color"), "red");
        assert!(
            r.uncertain.contains("color")
                && r.uncertain.contains("margin")
                && r.uncertain.contains("padding")
        );
        assert!(!r.winners.contains_key("margin"));
    }

    #[test]
    fn hover_rules_do_not_apply_to_a_resting_snapshot() {
        let t = tree();
        let r = run("p { color: red } p:hover { color: blue }", 1000.0, 2, &t);
        assert_eq!(val(&r, "color"), "red");
        assert!(r.uncertain.is_empty());
    }
}
