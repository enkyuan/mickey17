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

#[derive(Debug)]
pub struct FlatRule {
    pub complex: Complex,
    pub decls: Rc<Vec<Declaration>>,
    pub layer: LayerRank,
    pub order: u32,
    pub sheet: usize,
    /// AND of the enclosing media/supports/container conditions.
    pub active: Tri,
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
    pub specificity: Specificity,
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
                    specificity: cand.spec,
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
        fn state(&self, _: usize, _: &str) -> Tri {
            Tri::Unknown
        }
    }

    fn run(css_text: &str, width: f64, node: usize, tree: &T) -> Resolved {
        let sheet = css::parse_sheet(css_text);
        let env = Environment {
            width,
            height: 900.0,
            color_scheme: "dark".into(),
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
