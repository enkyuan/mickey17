//! Structural rewrite of a parsed stylesheet: tokenizable literals become `var()` references
//! and hoisted site variables leave their original rules. The tree is rebuilt, never
//! string-patched; rule order, nesting, layers and declaration order are preserved.

use super::css::{Component, Declaration, Item, Node, Sheet, StyleRule};
use super::plan::TokenSet;
use super::recognize::{self, Hit};
use super::value::Context;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone, Serialize)]
pub struct RewriteStats {
    pub declarations_rewritten: usize,
    pub replacements: usize,
    pub by_token: BTreeMap<String, usize>,
    pub spacing_replacements: usize,
    pub hoisted_declarations_removed: usize,
    pub empty_rules_removed: usize,
}

struct Rewriter<'a> {
    tokens: &'a TokenSet,
    ctx: Context,
    stats: RewriteStats,
    replace_values: bool,
}

fn token_name_of(replacement: &[Component]) -> Option<String> {
    match replacement.first() {
        Some(Component::Func(name, inner)) if name == "var" => match inner.first() {
            Some(Component::Tok(super::css::Tok::Ident(n))) => Some(n.clone()),
            _ => None,
        },
        _ => None,
    }
}

impl Rewriter<'_> {
    fn declaration(&mut self, decl: &Declaration) -> Option<Declaration> {
        if decl.is_custom() {
            if self.tokens.hoisted.iter().any(|h| h.name == decl.name) {
                self.stats.hoisted_declarations_removed += 1;
                return None;
            }
            return Some(decl.clone());
        }
        if !self.replace_values {
            return Some(decl.clone());
        }
        let tokens = self.tokens;
        let mut count = 0usize;
        let mut names: Vec<(String, bool)> = Vec::new();
        let value = recognize::visit_value(&decl.name, &decl.value, &self.ctx, &mut |hit: &Hit| {
            let replacement = tokens.replacement(hit)?;
            count += 1;
            names.push((
                token_name_of(&replacement).unwrap_or_else(|| "--spacing".into()),
                hit.category == super::value::Category::Spacing,
            ));
            Some(replacement)
        });
        if count > 0 {
            self.stats.declarations_rewritten += 1;
            self.stats.replacements += count;
            for (name, spacing) in names {
                *self.stats.by_token.entry(name).or_default() += 1;
                if spacing {
                    self.stats.spacing_replacements += 1;
                }
            }
        }
        if count == 0 {
            return Some(decl.clone()); // untouched: keeps the source text verbatim
        }
        Some(Declaration {
            name: decl.name.clone(),
            value,
            important: decl.important,
            raw: None,
        })
    }

    fn items(&mut self, items: &[Item]) -> Vec<Item> {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Item::Decl(d) => {
                    if let Some(d) = self.declaration(d) {
                        out.push(Item::Decl(d));
                    }
                }
                Item::Rule(node) => {
                    if let Some(node) = self.node(node) {
                        out.push(Item::Rule(node));
                    }
                }
            }
        }
        out
    }

    fn node(&mut self, node: &Node) -> Option<Node> {
        match node {
            Node::Style(rule) => {
                let items = self.items(&rule.items);
                if items.is_empty() {
                    self.stats.empty_rules_removed += 1;
                    return None;
                }
                Some(Node::Style(StyleRule {
                    selectors: rule.selectors.clone(),
                    items,
                    source: rule.source.clone(),
                }))
            }
            Node::At(at) => {
                let keep_values = matches!(
                    at.name.as_str(),
                    "media"
                        | "supports"
                        | "layer"
                        | "container"
                        | "scope"
                        | "keyframes"
                        | "-webkit-keyframes"
                        | "starting-style"
                );
                match &at.items {
                    Some(items) if keep_values => {
                        let inner = self.items(items);
                        // A conditional group left empty by hoisting has no effect: drop it.
                        let droppable = matches!(
                            at.name.as_str(),
                            "media" | "supports" | "container" | "scope"
                        );
                        if inner.is_empty() && droppable {
                            self.stats.empty_rules_removed += 1;
                            return None;
                        }
                        let mut at = at.clone();
                        at.items = Some(inner);
                        Some(Node::At(at))
                    }
                    _ => Some(node.clone()),
                }
            }
            Node::Frame { selector, items } => Some(Node::Frame {
                selector: selector.clone(),
                items: self.items(items),
            }),
            Node::Raw { .. } => Some(node.clone()),
        }
    }
}

/// `replace_values = false` only moves hoisted variable definitions and leaves every other
/// declaration as written.
pub fn rewrite_sheet(
    sheet: &Sheet,
    tokens: &TokenSet,
    ctx: Context,
    replace_values: bool,
) -> (Sheet, RewriteStats) {
    let mut rewriter = Rewriter {
        tokens,
        ctx,
        stats: RewriteStats::default(),
        replace_values,
    };
    let nodes = sheet
        .nodes
        .iter()
        .filter_map(|n| rewriter.node(n))
        .collect();
    (Sheet { nodes }, rewriter.stats)
}

#[cfg(test)]
mod tests {
    use super::super::css::parse_sheet;
    use super::super::inventory::Inventory;
    use super::super::plan::{Options, TokenSet};
    use super::*;

    fn run(text: &str) -> (String, RewriteStats, TokenSet) {
        let sheet = parse_sheet(text);
        let ctx = Context { rem_px: Some(16.0) };
        let inv = Inventory::collect(&[&sheet], ctx);
        let set = TokenSet::build(&inv, ctx, &Options::default());
        let (out, stats) = rewrite_sheet(&sheet, &set, ctx, true);
        (out.to_css(), stats, set)
    }

    #[test]
    fn literals_become_token_references_and_site_variables_move() {
        let (css, stats, _) = run(
            ":root { --color-ink: #111; --gap: 8px } .a { color: #111; padding: 16px } .b { color: rgb(17 17 17); padding: 8px 16px } .c { background: var(--color-ink); margin: var(--gap) }",
        );
        assert!(
            !css.contains("--color-ink: #111"),
            "definition moved to the central file:\n{css}"
        );
        assert!(css.contains("color: var(--color-ink)"), "{css}");
        assert!(css.contains("padding: calc(var(--spacing) * 4)"), "{css}");
        assert!(
            css.contains("padding: var(--gap) calc(var(--spacing) * 4)"),
            "site spacing variable wins over the scale:\n{css}"
        );
        assert_eq!(stats.hoisted_declarations_removed, 2);
        assert!(stats.replacements >= 4);
        assert!(!css.contains("--gap: 8px"), "{css}");
    }

    #[test]
    fn rules_conditions_and_order_are_preserved() {
        let src = "@layer base { .a { color: #fff; margin: 16px } } @media (min-width: 40rem) { .a { color: #fff; margin: 16px } } .b { color: #fff; &:hover { margin: 16px } }";
        let (css, _, _) = run(src);
        let before = parse_sheet(src).to_css();
        // Same shape: same line count and same selectors in the same order.
        assert_eq!(css.lines().count(), before.lines().count());
        let selectors = |t: &str| {
            t.lines()
                .filter(|l| l.trim_end().ends_with('{'))
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(selectors(&css), selectors(&before));
    }

    #[test]
    fn important_and_unrecognized_values_are_untouched() {
        let (css, _, _) = run(
            ".a { color: #fff !important; width: calc(100% - 16px); margin: 0 auto } .b { color: #fff; margin: 0 auto }",
        );
        assert!(
            css.contains("color: var(--color-white) !important"),
            "{css}"
        );
        assert!(css.contains("calc(100% - 16px)"), "{css}");
        assert!(css.contains("margin: 0 auto"), "{css}");
    }
}
