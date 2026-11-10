//! Orchestration: tokenize a capture, translate one page's computed author styles into
//! utilities and recipes, prove equality, and describe what could not be translated.

use super::super::capture::{Corpus, Snapshot};
use super::super::cascade::{Cascade, SheetInput};
use super::super::css::{self, Item, Node, Sheet};
use super::super::inventory::semantic_class;
use super::super::pipeline::{self, Output};
use super::super::plan::{Options, TokenSet};
use super::super::selector::Simple;
use super::super::verify::{environment, theme_as_root_sheet, theme_definitions};
use super::classes::Theme;
use super::emit::{apply_css, pseudo_css, recipe_css, render_classes};
use super::layers::{
    ElementLayers, ForcedDom, Layer, State, layer_for, layer_of, place, width_breakpoints,
};
use super::tailwind;
use super::verify::{StyleCheck, check, unreachable_state_rules};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct StyleOptions {
    pub tokens: Options,
    /// A Tailwind package directory; `None` locates one (see [`tailwind::locate`]).
    pub tailwind_dir: Option<PathBuf>,
    /// A group of elements with identical utilities becomes a named recipe from this size on.
    pub min_share: usize,
}

impl StyleOptions {
    pub fn standard() -> StyleOptions {
        StyleOptions {
            tokens: Options::default(),
            tailwind_dir: None,
            min_share: 2,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StyleReport {
    pub schema_version: u32,
    pub page: String,
    pub status: &'static str,
    /// What produced the CSS the equality check ran against.
    pub oracle: String,
    pub tokens_status: String,
    pub viewport: [u32; 2],
    pub breakpoints: Vec<u32>,
    pub elements_translated: usize,
    pub utilities: UtilityStats,
    pub recipes: RecipeStats,
    pub check: StyleCheck,
    pub untranslated: Untranslated,
    pub gaps: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct UtilityStats {
    pub distinct_classes: usize,
    pub named_utilities: usize,
    pub token_backed: usize,
    pub arbitrary_properties: usize,
    pub arbitrary_by_property: BTreeMap<String, usize>,
    pub variant_classes: usize,
    pub average_per_element: f64,
}

#[derive(Debug, Default, Serialize)]
pub struct RecipeStats {
    pub shared: usize,
    pub fallback_hooks: usize,
    pub pseudo_element_hooks: usize,
    pub elements_with_recipe: usize,
    /// Recipes named `styled-<tag>` because no component class identifies them.
    pub nameless_recipes: usize,
    /// Recipes whose name needed a numeric suffix to stay unique (last resort).
    pub numbered_names: usize,
    pub overlap_examples: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Untranslated {
    /// Property -> elements where an undecidable condition (container query, unknown pseudo-class) hides it.
    pub undecidable_properties: BTreeMap<String, usize>,
    pub important_flattened_elements: usize,
    /// Property -> elements where the capture holds an empty value (a `var()` shorthand artifact).
    pub empty_value_declarations: BTreeMap<String, usize>,
    /// Property -> elements where an empty value (a `var()` shorthand artifact) was replaced by the
    /// captured computed value, frozen at that value.
    pub recovered_from_computed: BTreeMap<String, usize>,
    /// Properties among those that were also recovered in forced-state layers, where the resting
    /// computed value is only an approximation of the state's.
    pub recovered_in_states: BTreeMap<String, usize>,
    /// Conditions under which rules never apply at any sampled width/environment (`@media print`,
    /// `prefers-reduced-motion: reduce`): their declarations are not in the output.
    pub never_applicable_conditions: BTreeMap<String, usize>,
    /// Conditions on features that are fixed, not varied, in the analysis (height, orientation,
    /// resolution, `prefers-*`, hover/pointer) and were evaluated once.
    pub environment_conditions: BTreeMap<String, usize>,
    /// At-rules neither translated nor carried (`@page`, ...).
    pub at_rules_dropped: BTreeMap<String, usize>,
    /// Custom properties used by translated declarations but defined nowhere in the output.
    pub unresolved_variables: Vec<String>,
    /// Custom properties read by the page's own styles but declared nowhere in them either (a bug
    /// of the site): the declaration is invalid at computed-value time before and after, so they
    /// are carried unchanged and do not make the check incomplete.
    pub undefined_in_original: Vec<String>,
    pub state_rules_on_ancestors_or_pseudo_elements: usize,
    pub state_rule_examples: Vec<String>,
    pub rules_for_other_pseudo_elements: usize,
    pub other_pseudo_element_examples: Vec<String>,
    pub animation_properties: BTreeMap<String, usize>,
    pub mask_and_timeline_properties: BTreeMap<String, usize>,
    pub canvas_elements: usize,
    pub at_rules_carried_verbatim: BTreeMap<String, usize>,
}

pub struct StylePlan {
    /// Class string per element (capture node index).
    pub classes: HashMap<usize, String>,
    /// Relative path -> content: the token files plus `src/styles/recipes.css`,
    /// `src/styles/at-rules.css`, the class map and the report.
    pub files: BTreeMap<String, String>,
    pub report: StyleReport,
    pub tokens: TokenSet,
}

const SKIPPED_TAGS: &[&str] = &[
    "head", "title", "meta", "link", "style", "script", "base", "noscript", "template",
];

const RESERVED: &[&str] = &[
    "container",
    "prose",
    "sr-only",
    "not-sr-only",
    "truncate",
    "underline",
    "line-through",
    "overline",
    "antialiased",
    "ring",
    "border",
    "shadow",
    "rounded",
    "blur",
    "transition",
    "transform",
    "filter",
    "outline",
    "grid",
    "flex",
    "block",
    "inline",
    "hidden",
    "table",
    "static",
    "fixed",
    "absolute",
    "relative",
    "sticky",
    "invisible",
    "visible",
    "isolate",
    "contents",
    "italic",
    "uppercase",
    "lowercase",
    "capitalize",
    "grow",
    "shrink",
];

fn uses_states(cascade: &Cascade) -> BTreeSet<State> {
    fn visit(simples: &[Simple], out: &mut BTreeSet<State>) {
        for s in simples {
            match s {
                Simple::Pseudo(n) => match n.as_str() {
                    "hover" => {
                        out.insert(State::Hover);
                    }
                    "active" => {
                        out.insert(State::Active);
                    }
                    "focus-within" => {
                        out.insert(State::FocusWithin);
                    }
                    "focus" => {
                        out.insert(State::Focus);
                        out.insert(State::FocusWithin);
                    }
                    "focus-visible" => {
                        out.insert(State::FocusVisible);
                        out.insert(State::Focus);
                        out.insert(State::FocusWithin);
                    }
                    _ => {}
                },
                Simple::Is(l) | Simple::Where(l) | Simple::Not(l) => {
                    for c in &l.0 {
                        for k in &c.compounds {
                            visit(&k.simples, out);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    for rule in &cascade.rules {
        if let Some(last) = rule.complex.compounds.last() {
            visit(&last.simples, &mut out);
        }
    }
    out
}

/// `@keyframes`, `@font-face`, `@property` and friends, outside any style rule, kept as written.
fn at_rules(sheets: &[&Sheet]) -> (String, BTreeMap<String, usize>) {
    fn collect(nodes: &[Node], out: &mut Vec<Node>) {
        for node in nodes {
            if let Node::At(at) = node {
                match at.name.as_str() {
                    "keyframes"
                    | "-webkit-keyframes"
                    | "font-face"
                    | "property"
                    | "counter-style"
                    | "font-feature-values"
                    | "font-palette-values"
                    | "position-try"
                    | "view-transition" => out.push(node.clone()),
                    "layer" | "media" | "supports" => {
                        if let Some(items) = &at.items {
                            let mut inner = Vec::new();
                            let nodes: Vec<Node> = items
                                .iter()
                                .filter_map(|i| match i {
                                    Item::Rule(n) => Some(n.clone()),
                                    Item::Decl(_) => None,
                                })
                                .collect();
                            collect(&nodes, &mut inner);
                            if at.name == "layer" {
                                out.extend(inner);
                            } else if !inner.is_empty() {
                                let mut wrapped = at.clone();
                                wrapped.items = Some(inner.into_iter().map(Item::Rule).collect());
                                out.push(Node::At(wrapped));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let mut nodes = Vec::new();
    for sheet in sheets {
        collect(&sheet.nodes, &mut nodes);
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for n in &nodes {
        if let Node::At(a) = n {
            *counts.entry(a.name.clone()).or_default() += 1;
        }
    }
    (Sheet { nodes }.to_css(), counts)
}

/// Layers per width interval and per forced state, for plain-CSS recipes.
type Layers = (Vec<Layer>, BTreeMap<State, Vec<Layer>>);

struct Parts {
    node: usize,
    utilities: Vec<String>,
    /// Layers for a plain-CSS recipe when the declarations cannot be ordered as utilities.
    fallback: Option<Layers>,
    pseudo: Vec<(String, Vec<Layer>)>,
    semantic: Option<String>,
}

impl Parts {
    fn key(&self) -> String {
        format!("{:?}|{:?}|{:?}", self.utilities, self.fallback, self.pseudo)
    }
}

fn valid_recipe_name(name: &str) -> bool {
    !name.is_empty()
        && !RESERVED.contains(&name)
        && !name.starts_with(|c: char| c.is_ascii_digit() || c == '-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn translate(capture_dir: &Path, page: &Path, opts: &StyleOptions) -> Result<StylePlan> {
    let out: Output = pipeline::run(capture_dir, &opts.tokens)?;
    let corpus = Corpus::load(capture_dir)?;
    let file_name = page.file_name().context("page path has no file name")?;
    let snap: &Snapshot = corpus
        .snapshots
        .iter()
        .find(|s| s.file.file_name() == Some(file_name))
        .with_context(|| format!("no snapshot {} in the capture", page.display()))?;

    let central_text = out
        .files
        .get("src/styles/tokens.css")
        .context("tokens.css missing")?;
    let central_sheet = css::parse_sheet(central_text);
    let tokens_plain = theme_as_root_sheet(&central_sheet);
    let definitions = theme_definitions(&central_sheet);
    let mut rewritten: Vec<Sheet> = vec![Sheet::default(); corpus.sheets.len()];
    for (index, sheet) in &out.styles_input {
        rewritten[*index] = sheet.clone();
    }
    let mut inputs: Vec<SheetInput<'_>> = vec![SheetInput {
        sheet: &tokens_plain,
        index: usize::MAX,
        media: "",
    }];
    inputs.extend(
        snap.sheets
            .iter()
            .filter(|s| !s.disabled)
            .map(|s| SheetInput {
                sheet: &rewritten[s.sheet],
                index: s.sheet,
                media: "",
            }),
    );
    let sheet_refs: Vec<&Sheet> = inputs.iter().map(|i| i.sheet).collect();

    let theme = Theme::from_definitions(
        definitions.keys(),
        out.tokens.spacing.as_ref().map(|s| s.base_px),
    );

    // Width intervals between every media threshold in the sheets.
    let points = width_breakpoints(&sheet_refs);
    if points.len() > 80 {
        bail!(
            "{} distinct media-query width thresholds; refusing to translate",
            points.len()
        );
    }
    let mut bounds: Vec<u32> = vec![0];
    bounds.extend(points.iter().copied());
    let reps: Vec<u32> = bounds
        .iter()
        .enumerate()
        .map(|(i, &b)| match (i, bounds.get(1)) {
            (0, Some(&next)) => next.saturating_sub(1).max(1),
            (0, None) => snap.width,
            _ => b,
        })
        .collect();
    let cascades: Vec<Cascade> = reps
        .iter()
        .map(|&w| {
            let mut env = environment(snap);
            env.width = f64::from(w);
            Cascade::build(&inputs, &env)
        })
        .collect();
    let states_in_play: BTreeSet<State> = cascades.iter().flat_map(uses_states).collect();

    // Per-element layers.
    let plain = ForcedDom {
        inner: snap,
        forced: None,
        classes: None,
    };
    let nodes: Vec<usize> = snap
        .elements()
        .filter(|&n| !SKIPPED_TAGS.contains(&snap.element(n).map_or("", |e| e.tag.as_str())))
        .collect();
    let mut parts: Vec<Parts> = Vec::new();
    let mut state_sensitive: BTreeSet<usize> = BTreeSet::new();
    let mut untranslated = Untranslated::default();
    let mut recipes = RecipeStats::default();
    let mut arbitrary_by_property: BTreeMap<String, usize> = BTreeMap::new();
    for &node in &nodes {
        let element = snap.element(node).expect("element");
        if element.tag == "canvas" {
            untranslated.canvas_elements += 1;
        }
        let recover = snap.computed(node);
        let rest: Vec<Layer> = cascades
            .iter()
            .map(|c| layer_of(c, &plain, node, recover))
            .collect();
        let mut states: BTreeMap<State, Vec<Layer>> = BTreeMap::new();
        for &state in &states_in_play {
            let dom = ForcedDom {
                inner: snap,
                forced: Some((node, state)),
                classes: None,
            };
            states.insert(
                state,
                cascades
                    .iter()
                    .map(|c| layer_of(c, &dom, node, recover))
                    .collect(),
            );
        }
        if states
            .values()
            .any(|layers| layers.iter().zip(&rest).any(|(s, r)| s.decls != r.decls))
        {
            state_sensitive.insert(node);
        }
        let placed: ElementLayers = place(&bounds, &rest, &states);
        for p in &rest
            .iter()
            .flat_map(|l| l.uncertain.iter())
            .cloned()
            .collect::<BTreeSet<_>>()
        {
            *untranslated
                .undecidable_properties
                .entry(p.clone())
                .or_default() += 1;
        }
        if placed.important > 0 {
            untranslated.important_flattened_elements += 1;
        }
        for p in rest
            .iter()
            .flat_map(|l| l.recovered.iter())
            .collect::<BTreeSet<_>>()
        {
            *untranslated
                .recovered_from_computed
                .entry(p.clone())
                .or_default() += 1;
        }
        for p in states
            .values()
            .flatten()
            .flat_map(|l| l.recovered.iter())
            .collect::<BTreeSet<_>>()
        {
            *untranslated
                .recovered_in_states
                .entry(p.clone())
                .or_default() += 1;
        }
        for p in rest
            .iter()
            .flat_map(|l| l.empty.iter())
            .collect::<BTreeSet<_>>()
        {
            *untranslated
                .empty_value_declarations
                .entry(p.clone())
                .or_default() += 1;
        }
        let classes_attr = element
            .attributes
            .iter()
            .find(|(k, _)| k == "class")
            .map_or("", |(_, v)| v.as_str());
        let tokens: Vec<&str> = classes_attr.split_ascii_whitespace().collect();
        let semantic = semantic_class(&tokens);

        let uncertain = &placed.uncertain;
        let usable: Vec<_> = placed
            .placed
            .iter()
            .filter(|p| !uncertain.contains(&p.prop))
            .cloned()
            .collect();
        let rendered = if placed.overlapping.is_empty() {
            render_classes(&usable, &theme)
        } else {
            None
        };
        let (utilities, fallback) = match rendered {
            Some(u) => (u, None),
            None => {
                if recipes.overlap_examples.len() < 8 {
                    let why = placed.overlapping.first().map_or(
                        "a value that cannot be written inside a class token".to_string(),
                        |(a, b)| format!("{a} and {b} in one variant"),
                    );
                    recipes
                        .overlap_examples
                        .push(format!("node {node} ({}): {why}", element.tag));
                }
                (Vec::new(), Some((rest.clone(), states.clone())))
            }
        };
        for pl in &usable {
            if pl.prop.starts_with("animation") || pl.prop.starts_with("transition") {
                *untranslated
                    .animation_properties
                    .entry(pl.prop.clone())
                    .or_default() += 1;
            }
            if pl.prop.starts_with("mask")
                || pl.prop.starts_with("-webkit-mask")
                || pl.prop.contains("timeline")
            {
                *untranslated
                    .mask_and_timeline_properties
                    .entry(pl.prop.clone())
                    .or_default() += 1;
            }
        }
        let mut pseudo = Vec::new();
        for (key, style) in &element.pseudo {
            let pseudo_style = Some(&snap.styles[*style]);
            let layers: Vec<Layer> = cascades
                .iter()
                .map(|c| layer_for(c, &plain, node, Some(key), pseudo_style))
                .collect();
            if layers.iter().any(|l| !l.decls.is_empty()) {
                pseudo.push((key.clone(), layers));
            }
        }
        parts.push(Parts {
            node,
            utilities,
            fallback,
            pseudo,
            semantic,
        });
    }

    // Names: shared recipes, plain-CSS recipes and pseudo-element hooks.
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, p) in parts.iter().enumerate() {
        groups.entry(p.key()).or_default().push(i);
    }
    // Phase 1: which groups become recipes, and the names each could take.
    struct Pending {
        key: String,
        candidates: Vec<String>,
        base: String,
    }
    let tailwind_dir = opts.tailwind_dir.clone().or_else(tailwind::locate);
    let mut pending: Vec<Pending> = Vec::new();
    for (key, members) in &groups {
        let first = &parts[members[0]];
        let needs_hook = first.fallback.is_some() || !first.pseudo.is_empty();
        let shared = members.len() >= opts.min_share.max(2) && first.utilities.len() >= 3;
        if !needs_hook && !shared {
            continue;
        }
        let mut freq: BTreeMap<&str, usize> = BTreeMap::new();
        for &m in members {
            if let Some(s) = &parts[m].semantic {
                *freq.entry(s.as_str()).or_default() += 1;
            }
        }
        let best = freq
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
            .map(|(n, c)| ((*n).to_string(), *c));
        let base = match best {
            Some((name, count)) if count * 2 >= members.len() && valid_recipe_name(&name) => name,
            _ if needs_hook => {
                let tag: String = snap
                    .element(first.node)
                    .map_or("element", |e| e.tag.as_str())
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                recipes.nameless_recipes += 1;
                format!("styled-{}", if tag.is_empty() { "element" } else { &tag })
            }
            _ => continue, // shared but nameless: keep the utilities inline
        };
        // Separate clashing names by what the element is and where it sits before resorting to
        // a number: `<tag>-<name>`, `<parent>-<name>`, `<grandparent>-<parent>-<name>`.
        let semantic_of = |n: usize| {
            parts
                .iter()
                .find(|x| x.node == n)
                .and_then(|x| x.semantic.clone())
        };
        let element = snap.element(first.node);
        let tag = element.map_or("element", |e| e.tag.as_str());
        let parent = element.and_then(|e| e.parent);
        let parent_name = parent.and_then(semantic_of);
        let grand_name = parent
            .and_then(|p| snap.element(p))
            .and_then(|e| e.parent)
            .and_then(semantic_of);
        let mut candidates = vec![base.clone()];
        if tag != base {
            candidates.push(format!("{tag}-{base}"));
        }
        if let Some(p) = &parent_name {
            candidates.push(format!("{p}-{base}"));
            if let Some(g) = &grand_name {
                candidates.push(format!("{g}-{p}-{base}"));
            }
        }
        candidates.retain(|c| valid_recipe_name(c));
        pending.push(Pending {
            key: key.clone(),
            candidates,
            base,
        });
    }

    // Phase 2: names Tailwind itself generates CSS for (`collapse`, `list-item`, ...) would be
    // styled twice by the project; refuse them. Asked of the real Tailwind when present.
    // Numbered fallbacks (`order-2`) are names too: at most one per pending recipe plus the clash
    // candidates can be taken before a free one is found.
    let mut per_base: HashMap<&str, usize> = HashMap::new();
    for p in &pending {
        *per_base.entry(p.base.as_str()).or_default() += 1;
    }
    let universe: Vec<String> = pending
        .iter()
        .flat_map(|p| {
            let numbers = (2..=per_base[p.base.as_str()] + 4).map(|n| format!("{}-{n}", p.base));
            p.candidates
                .iter()
                .cloned()
                .chain(std::iter::once(p.base.clone()))
                .chain(numbers)
                .collect::<Vec<_>>()
        })
        .filter(|n| valid_recipe_name(n))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut forbidden: BTreeSet<String> = universe
        .iter()
        .filter(|n| super::classes::expand(n, &theme).is_some())
        .cloned()
        .collect();
    if let Some(dir) = tailwind_dir.as_ref().filter(|_| tailwind::bun_available()) {
        match tailwind::generated_class_names(dir, &universe) {
            Ok(names) => forbidden.extend(names),
            Err(e) => bail!("could not ask Tailwind which recipe names it generates CSS for: {e}"),
        }
    }
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut recipe_of: HashMap<String, String> = HashMap::new();
    for p in pending {
        let mut name = p
            .candidates
            .iter()
            .find(|c| !taken.contains(*c) && !forbidden.contains(*c))
            .cloned()
            .unwrap_or_else(|| p.base.clone());
        if taken.contains(&name) || forbidden.contains(&name) || !valid_recipe_name(&name) {
            recipes.numbered_names += 1;
            let mut n = 2;
            loop {
                name = format!("{}-{n}", p.base);
                if valid_recipe_name(&name) && !taken.contains(&name) && !forbidden.contains(&name)
                {
                    break;
                }
                n += 1;
                if n > 100_000 {
                    bail!("no free recipe name for {}", p.base);
                }
            }
        }
        taken.insert(name.clone());
        recipe_of.insert(p.key, name);
    }

    // CSS of the recipes and the final class strings.
    let mut recipes_css = String::new();
    let mut classes: HashMap<usize, String> = HashMap::new();
    let mut inline: BTreeSet<String> = BTreeSet::new();
    let mut apply_uses: Vec<(String, String)> = Vec::new();
    let mut plain_recipe_css = String::new();
    let mut done: BTreeSet<&String> = BTreeSet::new();
    for (key, members) in &groups {
        let first = &parts[members[0]];
        match recipe_of.get(key) {
            Some(name) => {
                if done.insert(key) {
                    if !first.utilities.is_empty() {
                        recipes_css.push_str(&apply_css(name, &first.utilities));
                        for u in &first.utilities {
                            apply_uses.push((format!(".{name}"), u.clone()));
                        }
                    }
                    let mut plain_part = String::new();
                    if let Some((rest, states)) = &first.fallback {
                        plain_part.push_str(&recipe_css(name, &bounds, rest, states));
                        recipes.fallback_hooks += 1;
                    } else if members.len() >= opts.min_share.max(2) && first.pseudo.is_empty() {
                        recipes.shared += 1;
                    }
                    for (pseudo, layers) in &first.pseudo {
                        plain_part.push_str(&pseudo_css(name, pseudo, &bounds, layers));
                        recipes.pseudo_element_hooks += 1;
                    }
                    recipes_css.push_str(&plain_part);
                    plain_recipe_css.push_str(&plain_part);
                }
                for &m in members {
                    classes.insert(parts[m].node, name.clone());
                    recipes.elements_with_recipe += 1;
                }
            }
            None => {
                for &m in members {
                    classes.insert(parts[m].node, parts[m].utilities.join(" "));
                    inline.extend(parts[m].utilities.iter().cloned());
                }
            }
        }
    }

    // The CSS the generated project would produce.
    let candidates: Vec<String> = inline.iter().cloned().collect();
    let tailwind_dir =
        tailwind_dir.filter(|d| tailwind::bun_available() && d.join("dist/lib.mjs").is_file());
    let (after_sheet, oracle) = match &tailwind_dir {
        Some(dir) => {
            let input = format!("{central_text}\n{recipes_css}\n@tailwind utilities;\n");
            let compiled = tailwind::compile(dir, &input, &candidates)?;
            (
                css::parse_sheet(&compiled),
                format!("tailwindcss {}", tailwind::version(dir)),
            )
        }
        None => {
            let mut uses: Vec<(String, String)> = candidates
                .iter()
                .map(|c| (tailwind::class_selector(c), c.clone()))
                .collect();
            uses.extend(apply_uses);
            let model = tailwind::modelled(&uses, &theme, "")
                .context("a class has no model expansion; install Tailwind (MOCHI_TAILWIND_DIR) to compile it")?;
            let text = format!(
                "{}\n{model}\n{plain_recipe_css}",
                css::parse_sheet(central_text).to_css()
            );
            let sheet = css::parse_sheet(&text);
            (
                Sheet {
                    nodes: theme_as_root_sheet(&sheet).nodes,
                },
                "modelled utilities (no Tailwind found)".to_string(),
            )
        }
    };

    // Equality proof.
    let mut verify_widths: BTreeSet<u32> = reps.iter().copied().collect();
    verify_widths.extend(bounds.iter().skip(1).map(|b| b - 1));
    verify_widths.insert(snap.width);
    let verify_nodes: Vec<usize> = parts.iter().map(|p| p.node).collect();
    let pseudo_nodes: Vec<(usize, Vec<String>)> = parts
        .iter()
        .filter(|p| !p.pseudo.is_empty())
        .map(|p| (p.node, p.pseudo.iter().map(|(k, _)| k.clone()).collect()))
        .collect();
    let verification = check(
        snap,
        &inputs,
        &after_sheet,
        &classes,
        &verify_nodes,
        &verify_widths.into_iter().collect::<Vec<_>>(),
        &states_in_play,
        &|n| state_sensitive.contains(&n),
        &pseudo_nodes,
    );

    // Gaps.
    let (state_rule_count, state_rule_examples) = unreachable_state_rules(&cascades[0]);
    untranslated.state_rules_on_ancestors_or_pseudo_elements = state_rule_count;
    untranslated.state_rule_examples = state_rule_examples;
    let supported = ["before", "after"];
    let mut other_pseudo = BTreeSet::new();
    let mut other_pseudo_count = 0;
    for rule in &cascades[0].rules {
        if let Some(p) = rule.complex.pseudo_element()
            && !supported.contains(&p)
        {
            other_pseudo_count += 1;
            if other_pseudo.len() < 8 {
                other_pseudo.insert(rule.complex.to_css());
            }
        }
    }
    untranslated.rules_for_other_pseudo_elements = other_pseudo_count;
    untranslated.other_pseudo_element_examples = other_pseudo.into_iter().collect();
    let (at_css, at_counts) = at_rules(&sheet_refs);
    untranslated.at_rules_carried_verbatim = at_counts;

    // Rules that never apply in any sampled environment, and conditions evaluated only once.
    {
        let mut never: BTreeMap<String, usize> = BTreeMap::new();
        let mut environment: BTreeMap<String, usize> = BTreeMap::new();
        for (i, rule) in cascades[0].rules.iter().enumerate() {
            if rule.condition.is_empty() {
                continue;
            }
            let all_no = cascades.iter().all(|c| {
                c.rules
                    .get(i)
                    .is_some_and(|r| r.active == super::super::selector::Tri::No)
            });
            if all_no {
                *never.entry(rule.condition.clone()).or_default() += 1;
            } else if [
                "height",
                "orientation",
                "aspect-ratio",
                "resolution",
                "prefers-",
                "hover",
                "pointer",
                "color-scheme",
            ]
            .iter()
            .any(|f| rule.condition.contains(f))
            {
                *environment.entry(rule.condition.clone()).or_default() += 1;
            }
        }
        untranslated.never_applicable_conditions = never;
        untranslated.environment_conditions = environment;
        untranslated.at_rules_dropped = cascades[0].stats.skipped_at_rules.clone();
    }

    // Every custom property a class or recipe reads must be defined somewhere in the output.
    {
        fn names_after(text: &str, marker: &str) -> BTreeSet<String> {
            let mut out = BTreeSet::new();
            let mut rest = text;
            while let Some(i) = rest.find(marker) {
                let tail = &rest[i + marker.len()..];
                let end = tail
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                    .unwrap_or(tail.len());
                // `var(--x, fallback)` is self-sufficient: no definition is required.
                if !tail[end..].starts_with(',') {
                    out.insert(format!("--{}", &tail[..end]));
                }
                rest = &tail[end..];
            }
            out
        }
        let all_text: String =
            classes.values().cloned().collect::<Vec<_>>().join(" ") + "\n" + &recipes_css;
        let mut used = names_after(&all_text, "var(--");
        let mut defined: BTreeSet<String> = definitions.keys().cloned().collect();
        // A registered property (`@property --x`, carried in at-rules.css) has its initial value
        // (or the guaranteed-invalid value for syntax `*`) in the browser: nothing to define.
        defined.extend(names_after(&at_css, "@property --"));
        // A conditionally defined custom property (behind a condition a capture cannot decide) is
        // dropped from the output, so reading it, even with a fallback, is a real difference.
        for name in untranslated.undecidable_properties.keys() {
            if name.starts_with("--") && all_text.contains(&format!("var({name}")) {
                used.insert(name.clone());
            }
        }
        defined.extend(names_after(&all_text, "[--"));
        // `--x:` inside plain-CSS recipes and `@apply`ed arbitrary properties.
        for line in recipes_css.lines() {
            let t = line.trim_start();
            if t.starts_with("--")
                && let Some((n, _)) = t.split_once(':')
            {
                defined.insert(n.to_string());
            }
        }
        let (undefined, unresolved): (Vec<String>, Vec<String>) = used
            .difference(&defined)
            .cloned()
            .partition(|n| !cascades.iter().any(|c| c.declared_custom.contains(n)));
        untranslated.unresolved_variables = unresolved;
        untranslated.undefined_in_original = undefined;
    }

    // Utility statistics.
    let all_classes: BTreeSet<&String> = parts.iter().flat_map(|p| p.utilities.iter()).collect();
    let mut stats = UtilityStats {
        distinct_classes: all_classes.len(),
        ..Default::default()
    };
    for c in &all_classes {
        let body = tailwind::parse_class(c).map_or(c.as_str(), |p| p.body);
        if tailwind::parse_class(c).is_some_and(|p| p.min.is_some() || p.state.is_some()) {
            stats.variant_classes += 1;
        }
        if body.starts_with('[') {
            stats.arbitrary_properties += 1;
            let prop = body
                .trim_start_matches('[')
                .split(':')
                .next()
                .unwrap_or("?")
                .to_string();
            *arbitrary_by_property.entry(prop).or_default() += 1;
        } else {
            stats.named_utilities += 1;
        }
        if c.contains("var(--")
            || body
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic())
                && theme_backed(body, &theme)
        {
            stats.token_backed += 1;
        }
    }
    let mut top: Vec<(String, usize)> = arbitrary_by_property.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    stats.arbitrary_by_property = top.into_iter().take(25).collect();
    let total_inline: usize = classes
        .values()
        .map(|c| c.split_ascii_whitespace().count())
        .sum();
    stats.average_per_element = if classes.is_empty() {
        0.0
    } else {
        total_inline as f64 / classes.len() as f64
    };

    let mut gaps = vec![
        "Pseudo-elements other than ::before/::after, and state selectors that sit on an ancestor (`.card:hover .title`) or on a pseudo-element, are not translated; see untranslated.".to_string(),
        "Only hover, focus, focus-visible and active are forced, one at a time on the element itself; combinations (hover + focus) and states set by script (open, selected, aria-driven) are not captured, so their styles exist only as far as the captured state shows.".to_string(),
        "Widths are sampled at a representative of every interval between the media-query thresholds, plus one pixel below each; viewport height, orientation, resolution and container queries are not varied.".to_string(),
        "`!important` is flattened: each property keeps its winning value.".to_string(),
        "Declarations are the author cascade only: UA defaults and Tailwind preflight are not emitted, so the project must not import a preflight/reset that disagrees with the captured one (the captured site's own reset is part of the author styles).".to_string(),
        "@keyframes, @font-face and @property rules are carried verbatim in at-rules.css; animation-name, transition and animation-timeline declarations are translated as exact arbitrary properties but their behavior is not verified.".to_string(),
        "SVG presentation attributes and canvas/WebGL drawing are not CSS and are not covered.".to_string(),
        "The equality check is static (cascade engine over the compiled output); it does not replace the browser harness.".to_string(),
    ];
    if tailwind_dir.is_none() {
        gaps.push("No Tailwind package was found: the equality check ran against this crate's own model of the utilities, not a real Tailwind build.".to_string());
    }
    if out.report.status != "static_checks_only_not_browser_verified" {
        gaps.push(format!(
            "The token rewrite reported status {}.",
            out.report.status
        ));
    }
    if !untranslated.recovered_from_computed.is_empty() {
        gaps.push(format!(
            "Empty-valued longhands recovered from the captured computed style ({:?}) are frozen at that resting value at every width and state ({:?} also recovered inside forced states); the static check does not count them ({} comparisons) because the output carries the same text. var()-driven variation of these values is lost.",
            untranslated.recovered_from_computed, untranslated.recovered_in_states, verification.recovered_unverified
        ));
    }
    if !untranslated.undefined_in_original.is_empty() {
        gaps.push(format!(
            "Custom properties read but never defined by the page's own styles ({:?}): carried unchanged; the declarations using them are invalid at computed-value time before and after, and the static check skips them ({} comparisons).",
            untranslated.undefined_in_original, verification.skipped_uncertain
        ));
    }
    for (prop, count) in &untranslated.undecidable_properties {
        gaps.push(format!(
            "{prop}: undecidable condition on {count} element(s); not translated."
        ));
        if gaps.len() > 40 {
            break;
        }
    }

    let status = if verification.different > 0 {
        "static_differences_found"
    } else if out.report.status == "static_differences_found" {
        // The comparison's "before" side is the tokenized sheets: if those are not proven equal to
        // the original, neither is this translation.
        "tokens_check_failed"
    } else if !untranslated.unresolved_variables.is_empty()
        || untranslated
            .undecidable_properties
            .keys()
            .any(|p| !p.starts_with("--"))
    {
        // Properties whose conditions a capture cannot decide are left out of the output.
        "static_check_incomplete"
    } else if verification.compared == 0 {
        "static_check_vacuous"
    } else {
        "static_checks_only_not_browser_verified"
    };
    let report = StyleReport {
        schema_version: 1,
        page: page.display().to_string(),
        status,
        oracle,
        tokens_status: out.report.status.to_string(),
        viewport: [snap.width, snap.height],
        breakpoints: bounds.clone(),
        elements_translated: classes.len(),
        utilities: stats,
        recipes,
        check: verification,
        untranslated,
        gaps,
    };

    let mut files = out.files.clone();
    files.insert("src/styles/recipes.css".into(), recipes_css);
    files.insert("src/styles/at-rules.css".into(), at_css);
    let mut map: BTreeMap<usize, &String> = BTreeMap::new();
    for (k, v) in &classes {
        map.insert(*k, v);
    }
    files.insert(
        "styles.classes.json".into(),
        serde_json::to_string_pretty(&map)?,
    );
    files.insert(
        "styles.report.json".into(),
        serde_json::to_string_pretty(&report)?,
    );
    files.insert("styles.report.md".into(), markdown(&report));
    Ok(StylePlan {
        classes,
        files,
        report,
        tokens: out.tokens,
    })
}

fn theme_backed(body: &str, theme: &Theme) -> bool {
    super::classes::expand(body, theme).is_some_and(|d| d.iter().any(|(_, v)| v.contains("var(--")))
}

fn markdown(r: &StyleReport) -> String {
    let u = &r.utilities;
    let mut out = format!(
        "# Style translation report\n\nPage `{}`, status `{}`, checked against {}.\n\n{} elements translated into {} distinct classes ({} named utilities, {} arbitrary properties, {} with width/state variants, {} backed by tokens), {:.1} classes per element on average. {} shared recipes, {} plain-CSS recipes, {} pseudo-element hooks.\n\n",
        r.page,
        r.status,
        r.oracle,
        r.elements_translated,
        u.distinct_classes,
        u.named_utilities,
        u.arbitrary_properties,
        u.variant_classes,
        u.token_backed,
        u.average_per_element,
        r.recipes.shared,
        r.recipes.fallback_hooks,
        r.recipes.pseudo_element_hooks
    );
    let c = &r.check;
    out.push_str(&format!(
        "## Equality\n\nWidths {:?}, states {:?}: {} element-states, {} property comparisons, {} equal, {} different, {} skipped as undecidable.\n\n",
        c.widths, c.states, c.element_states, c.compared, c.equal, c.different, c.skipped_uncertain
    ));
    for d in c.samples.iter().take(10) {
        out.push_str(&format!(
            "- w{} {:?} {} {}: `{}` -> `{}`\n",
            d.width, d.state, d.element, d.property, d.before, d.after
        ));
    }
    out.push_str("\n## Most used arbitrary properties (no utility fits)\n\n");
    for (p, n) in &u.arbitrary_by_property {
        out.push_str(&format!("- `{p}`: {n}\n"));
    }
    let x = &r.untranslated;
    out.push_str(&format!(
        "\n## Not translated\n\n- undecidable properties: {:?}\n- elements with flattened `!important`: {}\n- empty-valued declarations in the capture: {:?}\n- state rules on ancestors or pseudo-elements: {} {:?}\n- rules for other pseudo-elements: {} {:?}\n- animation/transition properties: {:?}\n- mask/timeline properties: {:?}\n- canvas elements: {}\n- at-rules carried verbatim: {:?}\n",
        x.undecidable_properties, x.important_flattened_elements, x.empty_value_declarations, x.state_rules_on_ancestors_or_pseudo_elements,
        x.state_rule_examples, x.rules_for_other_pseudo_elements, x.other_pseudo_element_examples, x.animation_properties,
        x.mask_and_timeline_properties, x.canvas_elements, x.at_rules_carried_verbatim
    ));
    out.push_str("\n## Explicit gaps\n\n");
    for g in &r.gaps {
        out.push_str(&format!("- {g}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// html > body > [div.card x2, div.tile x2, button.btn, section.hero] with the given stylesheet.
    fn capture(dir: &Path, rules: &[&str], viewport: (u32, u32)) {
        let pages = dir.join("reconstruction").join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        let el = |parent: Option<usize>, tag: &str, class: &str| {
            json!({"parent": parent, "kind": "element", "tag": tag,
                   "attributes": [{"name": "class", "value": class}], "style": 0, "pseudo": {}})
        };
        let mut nodes = vec![el(None, "html", ""), el(Some(0), "body", "")];
        nodes.push(el(Some(1), "div", "card is-open"));
        nodes.push(el(Some(1), "div", "card"));
        nodes.push(el(Some(1), "div", "tile"));
        nodes.push(el(Some(1), "div", "tile"));
        nodes.push(el(Some(1), "button", "btn"));
        nodes.push(el(Some(1), "section", "hero"));
        nodes[2]["pseudo"] = json!({"::before": 0});
        nodes[3]["pseudo"] = json!({"::before": 0});
        let page = json!({
            "url": "https://t.test/", "viewport": {"width": viewport.0, "height": viewport.1}, "color_scheme": "dark",
            "nodes": nodes,
            "styles": {"strings": ["font-size", "16px", "--brand", "#0a84ff"], "maps": [[[0, 1], [2, 3]]]},
            "stylesheets": [{"href": null, "rules": rules}],
        });
        std::fs::write(
            pages.join(format!("page-00000-{}x{}.json", viewport.0, viewport.1)),
            page.to_string(),
        )
        .unwrap();
    }

    const RULES: &[&str] = &[
        ":root { --brand: #0a84ff }",
        ".card { display: flex; align-items: center; padding: 16px; color: var(--brand); position: relative; margin-top: 8px }",
        "@media (min-width: 600px) { .card { padding: 32px; flex-direction: row } }",
        ".card::before { content: \"x\"; display: block }",
        ".tile { display: grid; gap: 8px; padding: 4px; color: #111 }",
        ".btn { color: #111; background: #fff; border-radius: 4px; cursor: pointer }",
        ".btn:hover { color: #fff; opacity: 0.5 }",
        ".hero { background: red; background-color: blue; z-index: 3 }",
        "html { font-size: 16px }",
    ];

    fn plan(rules: &[&str], tailwind: Option<PathBuf>) -> StylePlan {
        let dir = tempfile::tempdir().unwrap();
        capture(dir.path(), rules, (1000, 800));
        let mut opts = StyleOptions::standard();
        opts.tailwind_dir =
            tailwind.or_else(|| std::env::var("MOCHI_TAILWIND_DIR").ok().map(PathBuf::from));
        translate(
            dir.path(),
            Path::new("reconstruction/pages/page-00000-1000x800.json"),
            &opts,
        )
        .unwrap()
    }

    #[test]
    fn translates_equal_with_variants_recipes_and_fallbacks() {
        let p = plan(RULES, None);
        let r = &p.report;
        assert_eq!(r.check.different, 0, "{:?}", r.check.samples);
        assert!(
            r.check.compared > 0 && r.status == "static_checks_only_not_browser_verified",
            "{}",
            r.status
        );
        // The two cards share one utility set: one named recipe with @apply, from the component class.
        let recipes = &p.files["src/styles/recipes.css"];
        assert!(recipes.contains(".tile {\n  @apply"), "{recipes}");
        assert_eq!(
            (&p.classes[&4], &p.classes[&5]),
            (&"tile".to_string(), &"tile".to_string())
        );
        assert_eq!(p.classes[&2], "card");
        assert_eq!(
            p.classes[&3], "card",
            "the transient is-open class is not part of the identity"
        );
        assert!(recipes.contains("min-[600px]:"), "{recipes}");
        // Pseudo-elements become plain CSS on a hook class.
        assert!(recipes.contains(".card::before"), "{recipes}");
        // A shorthand and its longhand cannot be ordered as utilities: exact plain CSS recipe.
        assert!(r.recipes.fallback_hooks >= 1, "{:?}", r.recipes);
        assert!(
            recipes.contains("background: red;") && recipes.contains("background-color: blue;"),
            "{recipes}"
        );
        // Hover becomes a variant class on the button.
        let btn = &p.classes[&6];
        assert!(btn.contains("hover:"), "{btn}");
        assert!(btn.contains("cursor-pointer"), "{btn}");
        // The bridge hands the classes to the emitter and counts what it did not cover.
        let bridge = p.bridge();
        let doc = crate::semantic::ir::Doc {
            url: String::new(),
            title: String::new(),
            viewport: None,
            nodes: vec![],
            by_id: Default::default(),
        };
        use crate::semantic::emit::StyleBridge;
        assert_eq!(bridge.class_name(&doc, 6, &["btn".into()]), *btn);
        assert_eq!(
            bridge.class_name(&doc, 99, &["x".into(), "y".into()]),
            "x y"
        );
        assert_eq!(bridge.missed.get(), 1);
    }

    #[test]
    fn a_wrong_translation_is_caught() {
        // Corrupt the output by hand: change a class and re-run the check against the same CSS.
        let p = plan(RULES, None);
        let mut classes = p.classes.clone();
        classes.insert(6, "text-left".into());
        let dir = tempfile::tempdir().unwrap();
        capture(dir.path(), RULES, (1000, 800));
        let corpus = Corpus::load(dir.path()).unwrap();
        let snap = &corpus.snapshots[0];
        let after = css::parse_sheet(".text-left { text-align: left }");
        let before_sheet = css::parse_sheet(RULES.join("\n").as_str());
        let inputs = [SheetInput {
            sheet: &before_sheet,
            index: 0,
            media: "",
        }];
        let check = super::super::verify::check(
            snap,
            &inputs,
            &after,
            &classes,
            &[6],
            &[1000],
            &BTreeSet::new(),
            &|_| true,
            &[],
        );
        assert!(check.different > 0, "{check:?}");
    }
}
