//! Inventory of every tokenizable value, site-defined custom property and breakpoint in
//! a set of stylesheets, with usage evidence (properties, selector subjects, element
//! weights taken from the cascade).

use super::cascade::Cascade;
use super::css::{AtRule, Component, Item, Node, Sheet, Tok, to_css};
use super::recognize::{self, Hit};
use super::selector::{AttrOp, Complex, Dom, SelectorList, Simple};
use super::value::{self, Category, Context, Length, fmt_num};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct Group {
    pub spellings: BTreeMap<String, usize>,
    pub units: BTreeMap<String, usize>,
    pub properties: BTreeMap<String, usize>,
    pub subjects: BTreeMap<String, usize>,
    /// Declarations containing the value.
    pub decls: usize,
    /// Captured element/pseudo-element states whose winning declaration contains it.
    pub weight: u64,
    pub length: Option<Length>,
}

impl Group {
    pub fn px(&self) -> Option<f64> {
        self.length.as_ref().and_then(Length::px)
    }
    /// Most frequent spelling (ties: shortest, then lexicographic).
    pub fn preferred_spelling(&self) -> String {
        self.spellings
            .iter()
            .max_by(|a, b| {
                a.1.cmp(b.1)
                    .then(b.0.len().cmp(&a.0.len()))
                    .then(b.0.cmp(a.0))
            })
            .map(|(s, _)| s.clone())
            .unwrap_or_default()
    }
    pub fn dominant_subject(&self) -> Option<(&str, f64)> {
        let total: usize = self.subjects.values().sum();
        let (name, count) = self
            .subjects
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))?;
        // Declarations without a subject count against dominance.
        let denominator = self.decls.max(total).max(1);
        Some((name.as_str(), *count as f64 / denominator as f64))
    }
}

#[derive(Debug, Clone)]
pub struct VarDef {
    pub value: Vec<Component>,
    /// The value as written in the source.
    pub raw: Option<String>,
    pub important: bool,
    /// Selector is exactly `:root`/`html` (any combination) with no at-rule condition.
    pub root_unconditional: bool,
    /// Text of the enclosing conditional at-rules, outermost first (`media (…)`).
    pub context: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SiteVar {
    pub defs: Vec<VarDef>,
    /// `var(--name)` references found in stylesheet declarations.
    pub references: usize,
    pub registered: bool,
    /// Also set by an inline `style=""` attribute somewhere in a snapshot.
    pub inline_set: bool,
}

#[derive(Debug, Default)]
pub struct Inventory {
    pub groups: BTreeMap<(Category, String), Group>,
    pub vars: BTreeMap<String, SiteVar>,
    /// Width breakpoints (px) found in media queries with how often.
    pub breakpoints: BTreeMap<String, usize>,
    pub foreground: Option<(Category, String)>,
    pub background: Option<(Category, String)>,
    pub declarations: usize,
    pub skipped_at_rules: BTreeMap<String, usize>,
}

const SEMANTIC_TAGS: &[&str] = &[
    "nav",
    "header",
    "footer",
    "main",
    "section",
    "article",
    "aside",
    "button",
    "a",
    "input",
    "textarea",
    "select",
    "dialog",
    "form",
    "table",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "li",
    "ul",
    "blockquote",
    "code",
    "pre",
    "img",
    "label",
    "body",
    "html",
    "figure",
    "figcaption",
    "summary",
    "details",
];

const UTILITY_PREFIXES: &[&str] = &[
    "p",
    "px",
    "py",
    "pt",
    "pb",
    "pl",
    "pr",
    "ps",
    "pe",
    "m",
    "mx",
    "my",
    "mt",
    "mb",
    "ml",
    "mr",
    "ms",
    "me",
    "w",
    "h",
    "min",
    "max",
    "size",
    "text",
    "bg",
    "border",
    "rounded",
    "shadow",
    "ring",
    "outline",
    "flex",
    "grid",
    "gap",
    "space",
    "font",
    "leading",
    "tracking",
    "z",
    "top",
    "left",
    "right",
    "bottom",
    "inset",
    "opacity",
    "duration",
    "delay",
    "ease",
    "transition",
    "items",
    "justify",
    "self",
    "content",
    "place",
    "overflow",
    "order",
    "col",
    "row",
    "aspect",
    "cursor",
    "select",
    "pointer",
    "object",
    "fill",
    "stroke",
    "transform",
    "translate",
    "rotate",
    "scale",
    "skew",
    "origin",
    "animate",
    "blur",
    "backdrop",
    "brightness",
    "filter",
    "list",
    "whitespace",
    "break",
    "line",
    "decoration",
    "underline",
    "divide",
    "sr",
    "columns",
    "float",
    "clear",
    "isolate",
    "mix",
    "will",
    "touch",
    "resize",
    "scroll",
    "snap",
    "accent",
    "caret",
    "appearance",
    "basis",
    "grow",
    "shrink",
    "hyphens",
    "group",
    "peer",
    "not",
    "has",
    "in",
    "data",
    "aria",
    "supports",
    "motion",
    "dark",
    "light",
    "hover",
    "focus",
];

const UTILITY_WORDS: &[&str] = &[
    "flex",
    "grid",
    "block",
    "hidden",
    "relative",
    "absolute",
    "fixed",
    "sticky",
    "static",
    "inline",
    "inline-block",
    "inline-flex",
    "truncate",
    "uppercase",
    "lowercase",
    "capitalize",
    "italic",
    "underline",
    "container",
    "contents",
    "table",
    "visible",
    "invisible",
    "antialiased",
    "transition",
    "transform",
    "group",
    "peer",
    "isolate",
    "grow",
    "shrink",
    "border",
    "rounded",
    "shadow",
    "ring",
    "outline",
    "blur",
    "filter",
];

fn looks_like_utility(class: &str) -> bool {
    if class.contains([':', '[', ']', '(', ')', '/', '!', '@', '*', '%']) || class.starts_with('-')
    {
        return true;
    }
    if UTILITY_WORDS.contains(&class) {
        return true;
    }
    match class.split_once('-') {
        Some((prefix, rest)) => {
            UTILITY_PREFIXES.contains(&prefix)
                && (rest.chars().next().is_some_and(|c| c.is_ascii_digit())
                    || rest.len() <= 4
                    || UTILITY_PREFIXES.contains(&prefix))
        }
        None => false,
    }
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn subject_of_complex(complex: &Complex) -> Option<String> {
    let last = complex.compounds.last()?;
    for simple in &last.simples {
        if let Simple::Attr {
            name,
            op: AttrOp::Equals,
            value,
            ..
        } = simple
            && matches!(
                name.as_str(),
                "data-component" | "data-slot" | "data-ui" | "data-testid"
            )
        {
            let s = slug(value);
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    for simple in &last.simples {
        if let Simple::Class(class) = simple {
            if looks_like_utility(class) {
                continue;
            }
            // BEM: block__element--modifier -> block
            let block = class.split("__").next().unwrap_or(class);
            let block = block.split("--").next().unwrap_or(block);
            let s = slug(block);
            if !s.is_empty() && s.len() <= 32 {
                return Some(s);
            }
        }
    }
    for simple in &last.simples {
        if let Simple::Type(tag) = simple
            && SEMANTIC_TAGS.contains(&tag.as_str())
        {
            return Some(tag.clone());
        }
    }
    None
}

/// The component a rule styles, read from its selectors (never from hashed or utility names).
pub fn subject_of(selectors: &SelectorList) -> Option<String> {
    selectors.0.iter().find_map(subject_of_complex)
}

fn root_selector(selectors: &SelectorList) -> bool {
    // Every selector is `:root`, `html` or `:host`; `:host` matches nothing in a document
    // stylesheet (Tailwind writes `:root, :host`), but at least one must really be the root.
    let single = |c: &Complex, wanted: &dyn Fn(&Simple) -> bool| {
        c.compounds.len() == 1
            && c.compounds[0].simples.len() == 1
            && wanted(&c.compounds[0].simples[0])
    };
    let is_root = |s: &Simple| {
        matches!(s, Simple::Pseudo(name) if name == "root")
            || matches!(s, Simple::Type(tag) if tag == "html")
    };
    let is_host = |s: &Simple| matches!(s, Simple::Pseudo(name) if name == "host");
    !selectors.0.is_empty()
        && selectors
            .0
            .iter()
            .all(|c| single(c, &|s| is_root(s) || is_host(s)))
        && selectors.0.iter().any(|c| single(c, &is_root))
}

fn count_var_refs(value: &[Component], out: &mut BTreeMap<String, usize>) {
    for c in value {
        match c {
            Component::Func(name, inner) => {
                if name.eq_ignore_ascii_case("var")
                    && let Some(Component::Tok(Tok::Ident(n))) =
                        inner.iter().find(|c| !c.is_whitespace())
                {
                    *out.entry(n.clone()).or_default() += 1;
                }
                count_var_refs(inner, out);
            }
            Component::Paren(inner) | Component::Square(inner) | Component::Curly(inner) => {
                count_var_refs(inner, out)
            }
            _ => {}
        }
    }
}

struct Walker<'a> {
    inv: &'a mut Inventory,
    ctx: Context,
    refs: BTreeMap<String, usize>,
}

impl Walker<'_> {
    fn nodes(&mut self, nodes: &[Node], subject: Option<&str>, root: bool, scope: &[String]) {
        for node in nodes {
            match node {
                Node::Style(rule) => {
                    let own = subject_of(&rule.selectors);
                    let subject = own.as_deref().or(subject);
                    let is_root = root && scope.is_empty() && root_selector(&rule.selectors);
                    self.items(&rule.items, subject, is_root, false, scope);
                }
                Node::At(at) => self.at(at, subject, root, scope),
                Node::Frame { items, .. } => self.items(items, subject, false, false, scope),
                Node::Raw { .. } => {}
            }
        }
    }

    fn items(
        &mut self,
        items: &[Item],
        subject: Option<&str>,
        rule_root: bool,
        container_root: bool,
        scope: &[String],
    ) {
        for item in items {
            match item {
                Item::Decl(decl) => {
                    self.inv.declarations += 1;
                    count_var_refs(&decl.value, &mut self.refs);
                    if decl.is_custom() {
                        self.inv
                            .vars
                            .entry(decl.name.clone())
                            .or_default()
                            .defs
                            .push(VarDef {
                                value: decl.value.clone(),
                                raw: decl.raw.clone(),
                                important: decl.important,
                                root_unconditional: rule_root,
                                context: scope.to_vec(),
                            });
                        continue;
                    }
                    let inv = &mut *self.inv;
                    recognize::visit_value(&decl.name, &decl.value, &self.ctx, &mut |hit: &Hit| {
                        let group = inv
                            .groups
                            .entry((hit.category, hit.key.clone()))
                            .or_default();
                        group.decls += 1;
                        *group.spellings.entry(hit.spelling.clone()).or_default() += 1;
                        *group.properties.entry(hit.property.clone()).or_default() += 1;
                        if let Some(unit) = &hit.unit {
                            *group.units.entry(unit.clone()).or_default() += 1;
                        }
                        if let Some(s) = subject {
                            *group.subjects.entry(s.to_string()).or_default() += 1;
                        }
                        if group.length.is_none() {
                            group.length = hit.length.clone();
                        }
                        None
                    });
                }
                Item::Rule(node) => {
                    self.nodes(std::slice::from_ref(node), subject, container_root, scope)
                }
            }
        }
    }

    fn at(&mut self, at: &AtRule, subject: Option<&str>, container_root: bool, scope: &[String]) {
        match at.name.as_str() {
            "media" | "supports" | "container" | "scope" | "layer" | "starting-style" => {
                let mut inner = scope.to_vec();
                if at.name == "media" {
                    breakpoints(&at.prelude, &mut self.inv.breakpoints);
                }
                // @layer alone does not make a definition conditional.
                if at.name != "layer" {
                    inner.push(format!("{} {}", at.name, to_css(&at.prelude)));
                }
                if let Some(items) = &at.items {
                    // Root-ness only survives through @layer.
                    self.items(
                        items,
                        subject,
                        false,
                        container_root && at.name == "layer",
                        &inner,
                    );
                }
            }
            "property" => {
                let name = to_css(&at.prelude);
                self.inv
                    .vars
                    .entry(name.trim().to_string())
                    .or_default()
                    .registered = true;
            }
            "keyframes" | "-webkit-keyframes" => {
                if let Some(items) = &at.items {
                    self.items(items, None, false, false, scope);
                }
            }
            "font-face"
            | "page"
            | "counter-style"
            | "font-feature-values"
            | "font-palette-values"
            | "position-try"
            | "view-transition"
            | "import"
            | "charset"
            | "namespace" => {}
            other => {
                *self
                    .inv
                    .skipped_at_rules
                    .entry(other.to_string())
                    .or_default() += 1;
            }
        }
    }
}

/// Collect width thresholds (CSS px at the 16px query font size) from a media prelude.
fn breakpoints(prelude: &[Component], out: &mut BTreeMap<String, usize>) {
    fn walk(c: &Component, out: &mut BTreeMap<String, usize>) {
        if let Component::Paren(inner) = c {
            let has_width = inner.iter().any(|x| {
                x.ident()
                    .is_some_and(|i| i.to_ascii_lowercase().contains("width"))
            });
            if has_width {
                for x in inner {
                    if let Component::Tok(Tok::Dimension { value, unit, .. }) = x {
                        let v = f64::from(*value);
                        let px = match unit.to_ascii_lowercase().as_str() {
                            "px" => Some(v),
                            "em" | "rem" => Some(v * 16.0),
                            _ => None,
                        };
                        if let Some(px) = px {
                            *out.entry(fmt_num(px)).or_default() += 1;
                        }
                    }
                }
            }
            for x in inner {
                walk(x, out);
            }
        }
    }
    for c in prelude {
        walk(c, out);
    }
}

impl Inventory {
    pub fn collect(sheets: &[&Sheet], ctx: Context) -> Inventory {
        let pairs: Vec<(&Sheet, bool)> = sheets.iter().map(|s| (*s, false)).collect();
        Inventory::collect_with(&pairs, ctx)
    }

    /// Like [`Inventory::collect`]; the flag marks a sheet that only applies under a media
    /// condition (`<link media>`), so its definitions are never unconditional.
    pub fn collect_with(sheets: &[(&Sheet, bool)], ctx: Context) -> Inventory {
        let mut inventory = Inventory::default();
        let refs = {
            let mut walker = Walker {
                inv: &mut inventory,
                ctx,
                refs: BTreeMap::new(),
            };
            for (sheet, conditional) in sheets {
                let scope: Vec<String> = if *conditional {
                    vec!["sheet media".into()]
                } else {
                    Vec::new()
                };
                walker.nodes(&sheet.nodes, None, true, &scope);
            }
            walker.refs
        };
        for (name, count) in refs {
            inventory.vars.entry(name).or_default().references = count;
        }
        inventory
    }

    /// Add element weights and page foreground/background from one snapshot's cascade.
    pub fn add_weights<D: Dom + ?Sized>(
        &mut self,
        dom: &D,
        elements: impl Iterator<Item = (usize, Vec<Option<String>>)>,
        cascade: &Cascade,
        ctx: Context,
    ) {
        let mut body_fg = None;
        let mut body_bg = None;
        for (node, pseudos) in elements {
            for pseudo in pseudos {
                let resolved = cascade.resolve(dom, node, pseudo.as_deref());
                for (name, winner) in &resolved.winners {
                    if name.starts_with("--") {
                        continue;
                    }
                    let groups = &mut self.groups;
                    let mut found: Vec<(Category, String)> = Vec::new();
                    recognize::visit_value(name, &winner.value, &ctx, &mut |hit: &Hit| {
                        found.push((hit.category, hit.key.clone()));
                        None
                    });
                    for key in found {
                        if let Some(g) = groups.get_mut(&key) {
                            g.weight += 1;
                        }
                    }
                    if pseudo.is_none()
                        && matches!(dom.tag(node), "html" | "body")
                        && (name == "color" || name == "background-color")
                    {
                        let mut first = None;
                        recognize::visit_value(name, &winner.value, &ctx, &mut |hit: &Hit| {
                            first.get_or_insert((hit.category, hit.key.clone()));
                            None
                        });
                        let slot = if name == "color" {
                            &mut body_fg
                        } else {
                            &mut body_bg
                        };
                        if let Some(k) = first
                            && (dom.tag(node) == "body" || slot.is_none())
                        {
                            *slot = Some(k);
                        }
                    }
                }
            }
        }
        if self.foreground.is_none() {
            self.foreground = body_fg;
        }
        if self.background.is_none() {
            self.background = body_bg;
        }
    }

    pub fn group_names(&self) -> BTreeSet<Category> {
        self.groups.keys().map(|(c, _)| *c).collect()
    }
}

/// Classify a custom property value as a token of some category (name hints disambiguate
/// plain numbers and lengths). `None` = not a recognizable design value.
pub fn classify_var(
    name: &str,
    value: &[Component],
    ctx: &Context,
) -> Option<(Category, String, Option<Length>)> {
    let trimmed = super::css::trim(value);
    let lname = name.to_ascii_lowercase();
    // Suffix hints first: `--text-body--letter-spacing` is a tracking value, not a size.
    let suffix_category = if lname.contains("letter-spacing") || lname.contains("tracking") {
        Some(Category::LetterSpacing)
    } else if lname.contains("line-height") || lname.contains("leading") {
        Some(Category::LineHeight)
    } else if lname.contains("font-weight") {
        Some(Category::FontWeight)
    } else {
        None
    };
    if let (Some(category), [single]) = (suffix_category, trimmed.as_slice()) {
        if let Some(length) = value::parse_length(single, ctx) {
            return Some((category, length.key(), Some(length)));
        }
        if let Component::Tok(Tok::Number { value: v, .. }) = single {
            return Some((category, fmt_num(f64::from(*v)), None));
        }
    }
    if trimmed.len() == 1 {
        if let Some(color) = value::parse_color(&trimmed[0])
            && !matches!(color, value::Color::Srgb([_, _, _, 0]))
        {
            return Some((Category::Color, color.key(), None));
        }
        if let Some(key) = value::easing_key(&trimmed[0]) {
            return Some((Category::Easing, key, None));
        }
        if let Some(ms) = value::parse_time(&trimmed[0]) {
            return Some((Category::Duration, fmt_num(ms), None));
        }
        if let Some(length) = value::parse_length(&trimmed[0], ctx) {
            let category = if lname.contains("radius") || lname.contains("rounded") {
                Category::Radius
            } else if lname == "--spacing"
                || lname.contains("space")
                || lname.contains("gutter")
                || lname.contains("gap")
            {
                Category::Spacing
            } else if lname.starts_with("--text") || lname.contains("font-size") {
                Category::FontSize
            } else if lname.contains("tracking") || lname.contains("letter-spacing") {
                Category::LetterSpacing
            } else if lname.contains("border-width") {
                Category::BorderWidth
            } else {
                return None;
            };
            return Some((category, length.key(), Some(length)));
        }
        if let Component::Tok(Tok::Number { value: v, .. }) = &trimmed[0] {
            let n = f64::from(*v);
            let category = if lname.contains("leading") || lname.contains("line-height") {
                Category::LineHeight
            } else if lname.contains("font-weight") || lname.contains("weight") {
                Category::FontWeight
            } else if lname.starts_with("--z-") || lname.contains("z-index") {
                Category::ZIndex
            } else {
                return None;
            };
            return Some((category, fmt_num(n), None));
        }
    }
    if (lname.contains("shadow")) && !lname.starts_with("--tw-") {
        let text = lname.contains("text");
        if let Some(layers) = value::parse_shadow(&trimmed, ctx, text)
            && layers
                .iter()
                .all(|l| l.lengths.iter().all(|x| x.px().is_some()))
        {
            return Some((Category::Shadow, value::shadow_key(&layers, text), None));
        }
    }
    if (lname.contains("font") || lname.contains("family"))
        && let Some(key) = value::font_family_key(&trimmed)
    {
        return Some((Category::FontFamily, key, None));
    }
    None
}

/// Could any selector in the list match the root element? Deliberately broad: `html.dark`,
/// `:root:has(.x)`, `html, body`, `*`.
fn may_match_root(selectors: &SelectorList) -> bool {
    fn simple_may(s: &Simple) -> bool {
        match s {
            Simple::Type(tag) => tag == "html",
            Simple::Pseudo(name) => name == "root",
            Simple::Universal | Simple::Nesting => true,
            Simple::Is(l) | Simple::Where(l) => l.0.iter().any(complex_may),
            _ => false,
        }
    }
    fn complex_may(c: &Complex) -> bool {
        // Only the subject (last compound) can be the root element.
        c.compounds
            .last()
            .is_some_and(|last| last.simples.iter().any(simple_may))
    }
    selectors.0.iter().any(complex_may)
}

/// True when the root element's font size can differ between viewports no snapshot visited:
/// it is set under a media/supports/container condition (also nested inside the root rule or
/// inherited from a `<link media>` sheet), or from viewport units. `rem` must then not be
/// merged with px.
pub fn root_font_size_is_conditional(sheets: &[(&Sheet, bool)]) -> bool {
    fn viewport_relative(components: &[Component]) -> bool {
        components.iter().any(|c| match c {
            Component::Tok(Tok::Dimension { unit, .. }) => matches!(
                unit.to_ascii_lowercase().as_str(),
                "vw" | "vh"
                    | "vmin"
                    | "vmax"
                    | "svw"
                    | "svh"
                    | "lvw"
                    | "lvh"
                    | "dvw"
                    | "dvh"
                    | "vi"
                    | "vb"
                    | "svi"
                    | "svb"
                    | "svmin"
                    | "svmax"
                    | "lvi"
                    | "lvb"
                    | "lvmin"
                    | "lvmax"
                    | "dvi"
                    | "dvb"
                    | "dvmin"
                    | "dvmax"
                    | "cqw"
                    | "cqh"
                    | "cqi"
                    | "cqb"
                    | "cqmin"
                    | "cqmax"
            ),
            // var() could hold anything, including a value a media query changes.
            Component::Func(name, _) if name.eq_ignore_ascii_case("var") => true,
            Component::Func(_, inner) | Component::Paren(inner) => viewport_relative(inner),
            _ => false,
        })
    }
    fn items(list: &[Item], root: bool, conditional: bool) -> bool {
        for item in list {
            match item {
                Item::Decl(d) => {
                    if root
                        && matches!(d.name.as_str(), "font-size" | "font")
                        && (conditional || viewport_relative(&d.value))
                    {
                        return true;
                    }
                }
                Item::Rule(Node::At(at)) => {
                    // A conditional group nested in the root rule keeps the rule's root-ness.
                    let cond = conditional
                        || matches!(
                            at.name.as_str(),
                            "media" | "supports" | "container" | "scope"
                        );
                    if let Some(inner) = &at.items
                        && items(inner, root, cond)
                    {
                        return true;
                    }
                }
                Item::Rule(node) => {
                    if node_has(node, conditional) {
                        return true;
                    }
                }
            }
        }
        false
    }
    fn node_has(node: &Node, conditional: bool) -> bool {
        match node {
            Node::Style(rule) => {
                // `html.dark`, `:root:has(.x)`, `html, body`: the rule can reach the root only
                // sometimes, which is as good as a condition no snapshot visited.
                let may = may_match_root(&rule.selectors);
                let qualified = may && !root_selector(&rule.selectors);
                items(&rule.items, may, conditional || qualified)
            }
            Node::At(at) => match &at.items {
                Some(inner) => {
                    let cond = conditional
                        || matches!(
                            at.name.as_str(),
                            "media" | "supports" | "container" | "scope"
                        );
                    items(inner, false, cond)
                }
                None => false,
            },
            _ => false,
        }
    }
    sheets
        .iter()
        .any(|(sheet, conditional)| sheet.nodes.iter().any(|n| node_has(n, *conditional)))
}

/// The component class of an element: the first class that is neither a utility nor a
/// transient state (`is-open`, `js-ready`). Used to name shared style recipes.
pub fn semantic_class(classes: &[&str]) -> Option<String> {
    fn transient(c: &str) -> bool {
        c.starts_with("is-")
            || c.starts_with("has-")
            || c.starts_with("js-")
            || c.starts_with("--")
            || matches!(
                c,
                "active"
                    | "open"
                    | "opened"
                    | "closed"
                    | "visible"
                    | "hidden"
                    | "show"
                    | "shown"
                    | "selected"
                    | "disabled"
                    | "loading"
                    | "loaded"
                    | "ready"
                    | "current"
                    | "expanded"
                    | "collapsed"
            )
    }
    classes
        .iter()
        .find(|c| {
            !looks_like_utility(c)
                && !transient(c)
                && !c.is_empty()
                && c.chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
                && !c.starts_with(|ch: char| ch.is_ascii_digit())
        })
        .map(|c| (*c).to_string())
}

#[cfg(test)]
mod tests {
    use super::super::css::parse_sheet;
    use super::*;

    fn collect(text: &str) -> Inventory {
        let sheet = parse_sheet(text);
        Inventory::collect(&[&sheet], Context { rem_px: Some(16.0) })
    }

    #[test]
    fn counts_values_by_normalized_key_across_spellings() {
        let inv = collect(
            ".a { color: #fff; padding: 1rem } .b { color: white; padding: 16px } .c { color: rgb(255 255 255) }",
        );
        let white = inv
            .groups
            .get(&(Category::Color, "srgb:255,255,255,255".to_string()))
            .unwrap();
        assert_eq!(white.decls, 3);
        assert_eq!(white.spellings.len(), 3);
        let pad = inv
            .groups
            .get(&(Category::Spacing, "px:16".to_string()))
            .unwrap();
        assert_eq!(pad.decls, 2);
        assert_eq!(pad.units.get("rem"), Some(&1));
    }

    #[test]
    fn subjects_come_from_component_names_not_utilities() {
        let inv = collect(
            ".nav__link { color: #111 } .px-4 { padding: 4px } button { color: #111 } .hover\\:bg-red:hover { color: #111 }",
        );
        let g = inv
            .groups
            .get(&(Category::Color, "srgb:17,17,17,255".to_string()))
            .unwrap();
        assert_eq!(g.subjects.get("nav"), Some(&1));
        assert_eq!(g.subjects.get("button"), Some(&1));
        assert_eq!(g.subjects.len(), 2, "utility classes yield no subject");
        assert_eq!(
            subject_of(
                &super::super::selector::parse_list("[data-component=\"Card\"] > span").unwrap()
            ),
            None
        );
        assert_eq!(
            subject_of(&super::super::selector::parse_list("[data-component=\"Card\"]").unwrap())
                .as_deref(),
            Some("card")
        );
    }

    #[test]
    fn site_variables_record_scope_and_references() {
        let inv = collect(
            ":root { --ink: #111; --gap: 8px } .dark { --ink: #eee } .a { color: var(--ink); margin: var(--gap) } @media (min-width: 60rem) { :root { --gap: 16px } }",
        );
        let ink = &inv.vars["--ink"];
        assert_eq!(ink.defs.len(), 2);
        assert!(ink.defs[0].root_unconditional && !ink.defs[1].root_unconditional);
        assert_eq!(ink.references, 1);
        assert!(
            !inv.vars["--gap"].defs[1].root_unconditional,
            "media-scoped definition is conditional"
        );
        assert_eq!(inv.breakpoints.get("960"), Some(&1));
    }

    #[test]
    fn media_sheets_and_starting_style_definitions_are_conditional() {
        let tw = parse_sheet(":root, :host { --t: 1 } :host { --h: 1 }");
        let inv = Inventory::collect(&[&tw], Context { rem_px: Some(16.0) });
        assert!(
            inv.vars["--t"].defs[0].root_unconditional,
            "`:root, :host` is the Tailwind theme selector"
        );
        assert!(
            !inv.vars["--h"].defs[0].root_unconditional,
            "`:host` alone matches nothing in a document"
        );
        let sheet =
            parse_sheet(":root { --a: 1 } @starting-style { :root { --a: 0 } } :root { --b: 1 }");
        let inv = Inventory::collect(&[&sheet], Context { rem_px: Some(16.0) });
        assert!(inv.vars["--a"].defs.iter().any(|d| !d.root_unconditional));
        assert!(inv.vars["--b"].defs[0].root_unconditional);
        let print = parse_sheet(":root { --c: red }");
        let inv = Inventory::collect_with(&[(&print, true)], Context { rem_px: Some(16.0) });
        assert!(!inv.vars["--c"].defs[0].root_unconditional);
    }

    #[test]
    fn detects_conditional_root_font_size() {
        let check = |css: &str, link_media: bool| {
            root_font_size_is_conditional(&[(&parse_sheet(css), link_media)])
        };
        assert!(!check(
            "html { font-size: 62.5% } .a { font-size: 2rem }",
            false
        ));
        assert!(check(
            "@media (min-width: 1600px) { html { font-size: 20px } }",
            false
        ));
        assert!(!check(
            "@media (min-width: 1600px) { .a { font-size: 20px } }",
            false
        ));
        assert!(
            check(
                "html { font-size: 16px; @media (max-width: 600px) { font-size: 14px } }",
                false
            ),
            "nested in the root rule"
        );
        assert!(
            check("html { font-size: 14px }", true),
            "a <link media> sheet is conditional"
        );
        assert!(
            check("html { font-size: calc(12px + .4vw) }", false),
            "fluid root size"
        );
        assert!(check("html { font-size: clamp(14px, 1vw, 18px) }", false));
    }

    #[test]
    fn classifies_variables_by_value_and_name() {
        let ctx = Context { rem_px: Some(16.0) };
        let c = |n: &str, v: &str| {
            classify_var(n, &super::super::css::parse_components(v), &ctx).map(|x| x.0)
        };
        assert_eq!(c("--ink", "#111"), Some(Category::Color));
        assert_eq!(
            c("--ease-standard", "cubic-bezier(.4,0,.2,1)"),
            Some(Category::Easing)
        );
        assert_eq!(c("--radius-card", "12px"), Some(Category::Radius));
        assert_eq!(c("--spacing", ".25rem"), Some(Category::Spacing));
        assert_eq!(c("--max-width", "1728"), None);
        assert_eq!(
            c("--shadow-lift", "0 1px 2px #0003"),
            Some(Category::Shadow)
        );
        assert_eq!(
            c("--uhf-header-height", "54px"),
            None,
            "unhinted lengths are not classified"
        );
    }
}
