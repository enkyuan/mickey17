//! Styles of the assembled project: the computed author styles of every route translated into
//! Tailwind utilities and shared recipes (the `tokens` workstream's translation), one central
//! tokens file, the carried `@keyframes`/`@font-face`/`@property` rules and the recipes of all
//! routes in one stylesheet. The translation is per captured page; this module is what makes
//! several pages share one stylesheet without one page's recipe changing another's.
use super::routes::RoutePlan;
use crate::semantic::tokens::css;
use crate::semantic::tokens::styles::run::{StyleOptions, StylePlan, translate};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Everything the translation of all routes contributes to the project.
pub struct Translation {
    /// One plan per route, in route order (class strings are keyed by capture node index).
    pub plans: Vec<StylePlan>,
    pub tokens_css: String,
    pub at_rules_css: String,
    pub recipes_css: String,
    /// Statuses of the per-route static equality checks.
    pub statuses: Vec<(String, String)>,
    pub gaps: Vec<String>,
}

/// Translate every route (`plans[i].evidence` names the page snapshot). Recipe names are unique
/// across routes; the first route's tokens are the project's, and a later route whose tokens differ
/// is a gap, not a silent second set.
pub fn translate_routes(
    capture_dir: &Path,
    plans: &[RoutePlan],
    tailwind_dir: Option<PathBuf>,
) -> Result<Translation> {
    let mut reserved: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<StylePlan> = Vec::new();
    let mut gaps = Vec::new();
    let mut statuses = Vec::new();
    let mut tokens_css: Option<String> = None;
    let mut at_rules: Vec<String> = Vec::new();
    let mut at_seen: BTreeSet<String> = BTreeSet::new();
    let mut recipes_css = String::new();
    for plan in plans {
        let page = Path::new(&plan.evidence);
        let styles = translate(
            capture_dir,
            page,
            &StyleOptions {
                tailwind_dir: tailwind_dir.clone(),
                reserved_names: reserved.clone(),
                ..StyleOptions::standard()
            },
        )
        .with_context(|| format!("translating the styles of {}", plan.path))?;
        reserved.extend(styles.recipe_names.iter().cloned());
        let file = |name: &str| styles.files.get(name).cloned().unwrap_or_default();
        match &tokens_css {
            None => tokens_css = Some(file("src/styles/tokens.css")),
            Some(first) if *first != file("src/styles/tokens.css") => gaps.push(format!(
                "styles: the central tokens computed for {} differ from the first route's; only the first route's tokens file is emitted, so {} may reference a token that is not defined",
                plan.path, plan.path
            )),
            Some(_) => {}
        }
        // At-rules are carried verbatim from the sheets every page shares: one copy of each.
        for node in css::parse_sheet(&file("src/styles/at-rules.css")).nodes {
            let mut text = String::new();
            css::write_node(&node, 0, &mut text);
            if at_seen.insert(text.clone()) {
                at_rules.push(text);
            }
        }
        let recipes = file("src/styles/recipes.css");
        if !recipes.trim().is_empty() {
            recipes_css.push_str(&format!("/* {} */\n{recipes}\n", plan.path));
        }
        statuses.push((plan.path.clone(), styles.report.status.to_owned()));
        out.push(styles);
    }
    Ok(Translation {
        plans: out,
        tokens_css: tokens_css.unwrap_or_default(),
        at_rules_css: at_rules.join("\n"),
        recipes_css,
        statuses,
        gaps,
    })
}

/// Per-route files kept as evidence next to the provenance of the assets.
pub fn evidence_files(ns: &str, plan: &StylePlan) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for (name, text) in &plan.files {
        if name.starts_with("styles.") {
            files.insert(
                format!(".mochi/provenance/styles/{ns}.{name}"),
                text.clone(),
            );
        }
    }
    files
}
