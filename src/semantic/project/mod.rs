//! Whole-project assembly: every captured route becomes a TanStack Start file route whose page
//! component renders the captured body with Base UI compositions and repeated-structure
//! components in place; assets and fonts are copied with provenance; captured stylesheets
//! come through the tokens pipeline as provisional styles. Missing pieces (final styles, behavior,
//! motion, canvas/WebGL, scripts) stay explicit placeholders and gap entries in the report.
//! Nothing here runs or embeds a recovered bundle.
mod assets;
mod head;
mod inventory;
mod refs;
mod routes;
pub mod structure;
mod styles;
mod template;

use super::emit::{self, EmitOptions, OriginalClasses};
use super::ir::Doc;
use super::{analyze_capture, report};
use crate::evidence;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use url::Url;

pub use assets::{AssetEntry, AssetSkip};
pub use inventory::RouteInventory;
pub use routes::{RoutePlan, RouteSkip};

/// Where the project's styles come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StyleSource {
    /// Each element carries the Tailwind utilities and shared recipes its computed author styles
    /// translate into (one central tokens file).
    #[default]
    Translated,
    /// Elements keep their captured class names and the captured stylesheets (tokenized) are
    /// imported as they are.
    CapturedSheets,
}

pub struct Options<'a> {
    /// Evidence viewport (`1440x900`); default: the widest captured per page.
    pub viewport: Option<&'a str>,
    /// Assemble at most this many routes.
    pub routes: Option<usize>,
    /// Run the tokens pipeline and include its central styles and rewritten stylesheets.
    pub tokens: bool,
    pub styles: StyleSource,
}

#[derive(Debug, Serialize)]
pub struct RouteReport {
    pub plan: RoutePlan,
    pub semantic: report::Summary,
    pub components: Vec<String>,
    pub repeated_components: Vec<String>,
    pub placed_in_page: Vec<String>,
    pub head_skipped: Vec<String>,
    pub html_body_attributes_dropped: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AssetsReport {
    pub copied: usize,
    pub bytes: u64,
    pub entries: Vec<AssetEntry>,
    pub skipped: Vec<AssetSkip>,
}

#[derive(Debug, Serialize)]
pub struct StylesReport {
    pub source: &'static str,
    pub tokens: usize,
    pub sheets: Vec<String>,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ProjectReport {
    pub schema_version: u32,
    pub capture: String,
    pub viewport_policy: String,
    pub routes: Vec<RouteReport>,
    pub skipped_routes: Vec<RouteSkip>,
    pub inventory: Vec<RouteInventory>,
    pub assets: AssetsReport,
    pub styles: StylesReport,
    pub ui_primitives: Vec<String>,
    pub files: usize,
    /// Commands that check the project, in order.
    pub verify: Vec<&'static str>,
    pub gaps: Vec<String>,
}

pub struct Project {
    /// Project-relative path -> bytes.
    pub files: BTreeMap<String, Vec<u8>>,
    pub report: ProjectReport,
}

fn put(files: &mut BTreeMap<String, Vec<u8>>, path: &str, text: impl Into<Vec<u8>>) {
    files.insert(path.to_owned(), text.into());
}

/// The part of a capture's `<head>`/`<html>`/`<body>` that the route file carries.
struct Document {
    html: Vec<(String, String)>,
    body: Vec<(String, String)>,
    dropped: Vec<String>,
}

fn document_attrs(doc: &Doc, style: &dyn emit::StyleBridge) -> Document {
    let find = |tag: &str| doc.nodes.iter().find(|n| n.tag == tag).map(|n| n.id);
    let mut dropped = Vec::new();
    let mut props = |tag: &str| {
        find(tag)
            .map(|id| {
                let (p, d) = emit::element_props(doc, id, style);
                dropped.extend(d.into_iter().map(|a| format!("{tag}@{a}")));
                p
            })
            .unwrap_or_default()
    };
    let html = props("html");
    let body = props("body");
    Document {
        html,
        body,
        dropped,
    }
}

pub fn assemble(capture_dir: &Path, opts: &Options<'_>) -> Result<Project> {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(capture_dir.join("manifest.json"))
            .context("reading capture manifest.json")?,
    )?;
    let (plans, skipped_routes) = routes::plan(&manifest, opts.viewport, opts.routes);
    ensure!(!plans.is_empty(), "no capturable route: {skipped_routes:?}");
    let origin = Url::parse(&plans[0].url)?;
    let route_paths: BTreeSet<String> = plans.iter().map(|p| p.path.clone()).collect();
    let assets = assets::Assets::new(capture_dir, &origin);
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut gaps: Vec<String> = Vec::new();
    let mut route_reports = Vec::new();
    let mut inventories = Vec::new();
    let mut ui: BTreeSet<String> = BTreeSet::new();
    let mut custom: BTreeSet<String> = BTreeSet::new();

    // Styles first: the translation decides which class names the markup carries.
    let mut gaps_styles: Vec<String> = Vec::new();
    let translation = if opts.tokens && opts.styles == StyleSource::Translated {
        match styles::translate_routes(capture_dir, &plans, None) {
            Ok(t) => Some(t),
            Err(e) => {
                gaps_styles.push(format!(
                    "styles: the translation into utilities failed ({e:#}); the captured stylesheets are imported instead and elements keep their captured class names"
                ));
                None
            }
        }
    } else {
        None
    };
    gaps.append(&mut gaps_styles);

    for (route_ix, plan) in plans.iter().enumerate() {
        let bridge = translation.as_ref().map(|t| t.plans[route_ix].bridge());
        let style: &dyn emit::StyleBridge = match &bridge {
            Some(b) => b,
            None => &OriginalClasses,
        };
        let capture = evidence::load(capture_dir, Path::new(&plan.evidence))
            .with_context(|| format!("loading {}", plan.evidence))?;
        let base = Url::parse(&capture.url)?;
        let referrer = format!("route:{}", plan.path);
        let analysis = analyze_capture(capture_dir, &plan.evidence, &capture, true)?;
        let rewrite = |raw: &str| assets.resolve(raw, &base, &referrer);
        let emitted = emit::emit(
            &analysis,
            &EmitOptions {
                style,
                namespace: &plan.ns,
                page: true,
                urls: Some(&rewrite),
            },
        );
        // Head: the mirrored response carries icons and JSON-LD that the snapshot does not.
        let head_info = head::read(capture_dir, &base)
            .map(|h| head::parse(&h))
            .unwrap_or_default();
        let mut links: Vec<Vec<(String, String)>> = Vec::new();
        for icon in &head_info.icons {
            let mut link = Vec::new();
            for (k, v) in icon {
                let value = if k == "href" {
                    assets
                        .resolve(v, &base, &referrer)
                        .unwrap_or_else(|| v.clone())
                } else {
                    v.clone()
                };
                link.push((k.clone(), value));
            }
            links.push(link);
        }
        if let Some(canonical) = capture
            .metadata
            .canonical
            .as_deref()
            .filter(|c| !c.is_empty())
        {
            links.push(vec![
                ("rel".into(), "canonical".into()),
                ("href".into(), canonical.to_owned()),
            ]);
        }
        let mut meta = Vec::new();
        for m in &capture.metadata.meta {
            // Per-response framework state and http-equiv directives (a CSP or refresh meta would
            // change how the project behaves) are not authored content.
            if m.http_equiv.is_some()
                || m.property.as_deref() == Some("csp-nonce")
                || matches!(
                    m.name.as_deref(),
                    Some("csp-nonce" | "next-size-adjust" | "next-head-count")
                )
            {
                continue;
            }
            if let Some(n) = &m.name {
                meta.push(("name".to_owned(), n.clone(), m.content.clone()));
            } else if let Some(p) = &m.property {
                meta.push(("property".to_owned(), p.clone(), m.content.clone()));
            } else if let Some(h) = &m.http_equiv {
                meta.push(("httpEquiv".to_owned(), h.clone(), m.content.clone()));
            }
        }
        let document = document_attrs(&analysis.doc, style);
        let route_head = routes::RouteHead {
            title: &capture.metadata.title,
            meta,
            links,
            json_ld: head_info.json_ld.clone(),
            lang: Some(capture.metadata.lang.as_str()),
            html: document.html.clone(),
            body: document.body.clone(),
        };
        let page = emitted.page.clone().unwrap_or_default();
        put(
            &mut files,
            &format!("src/routes/{}.tsx", plan.file),
            routes::route_file(plan, &route_head),
        );
        put(
            &mut files,
            &format!("src/pages/{}.tsx", plan.ns),
            routes::page_file(plan, &page.jsx, &page.hooks, &page.imports),
        );
        for (path, text) in &emitted.files {
            if path == "src/types/custom-elements.d.ts" {
                continue;
            }
            put(&mut files, path, text.clone());
        }
        custom.extend(emitted.custom_elements.iter().cloned());
        ui.extend(emitted.ui_primitives.iter().map(|p| format!("{p:?}")));
        let semantic = report::build(&analysis, emitted.clone());
        put(
            &mut files,
            &format!(".mochi/semantic/{}.report.json", plan.ns),
            serde_json::to_vec_pretty(&semantic)?,
        );
        put(
            &mut files,
            &format!(".mochi/semantic/{}.report.md", plan.ns),
            report::markdown(&analysis, &semantic),
        );
        for u in &page.unplaced {
            gaps.push(format!(
                "{}: pattern {} not placed in the page: {}",
                plan.path, u.0, u.1
            ));
        }
        for d in &document.dropped {
            gaps.push(format!(
                "{}: captured attribute dropped from <html>/<body>: {d}",
                plan.path
            ));
        }
        if page.scripts_omitted > 0 {
            gaps.push(format!(
                "{}: {} script/style/template element(s) in <body> were not emitted; the page runs none of the original scripts",
                plan.path, page.scripts_omitted
            ));
        }
        for (reason, n) in &page.elements_dropped {
            if !reason.contains("is never emitted") {
                gaps.push(format!(
                    "{}: {n} element(s) not emitted: {reason}",
                    plan.path
                ));
            }
        }
        if let Some(b) = &bridge {
            let missed = b.missed.borrow();
            if !missed.is_empty() {
                gaps.push(format!(
                    "{}: {} emitted element(s) are not covered by the style translation and keep their captured class names, if any (nothing styles them)",
                    plan.path,
                    missed.len()
                ));
            }
        }
        let misnested = invalid_paragraphs(&analysis.doc);
        if misnested > 0 {
            gaps.push(format!(
                "{}: {misnested} <p> element(s) contain block-level elements in the capture (built by script, which allows it); HTML cannot hold that (the parser closes the <p>, hydration fails), so each one rendered as plain markup is emitted as a `div` with role=\"paragraph\" (author selectors on `p` no longer match it under --captured-css)",
                plan.path
            ));
        }
        let mut inv = inventory::route(&plan.path, &analysis, &emitted);
        if inv.inline_style_attributes_in_capture > 0 {
            gaps.push(format!(
                "{}: {} element(s) carry an inline `style` attribute in the capture (set by script as the captured state of the page): {} emit it as a typed style object (positions, sizes, custom properties); animation-state declarations are left out because nothing runs the script that moves them on: {:?}",
                plan.path,
                inv.inline_style_attributes_in_capture,
                inv.inline_style_carried,
                inv.inline_style_transient_dropped
            ));
        }
        if inv.patterns_without_composition > 0 {
            gaps.push(format!(
                "{}: {} recognized pattern(s) have no composition emitter and render as plain markup (see the per-route report)",
                plan.path, inv.patterns_without_composition
            ));
        }
        inv.links_to_uncaptured_routes = uncaptured_links(&analysis.doc, &base, &route_paths);
        if !inv.links_to_uncaptured_routes.is_empty() {
            gaps.push(format!(
                "{}: {} link target(s) are same-origin paths that were not captured and answer 404 in this project: {}",
                plan.path,
                inv.links_to_uncaptured_routes.len(),
                inv.links_to_uncaptured_routes.iter().take(6).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        inventories.push(inv);
        route_reports.push(RouteReport {
            plan: plan.clone(),
            semantic: semantic.summary,
            components: emitted.components.iter().map(|c| c.file.clone()).collect(),
            repeated_components: emitted
                .group_components
                .iter()
                .map(|c| c.file.clone())
                .collect(),
            placed_in_page: page.placed.clone(),
            head_skipped: head_info.skipped.clone(),
            html_body_attributes_dropped: document.dropped.clone(),
        });
    }

    // Styles: tokens and the rewritten captured stylesheets, provisional until the class bridge lands.
    let mut style_imports: Vec<String> = Vec::new();
    let mut styles = StylesReport {
        source: "none",
        tokens: 0,
        sheets: Vec::new(),
        status: "no stylesheet is included: the pages render unstyled".into(),
    };
    let mut app_css = template::APP_CSS.to_owned();
    if let Some(t) = &translation {
        // One Tailwind compilation sees tokens, carried at-rules and every route's recipes.
        for (name, text) in [
            ("tokens.css", &t.tokens_css),
            ("at-rules.css", &t.at_rules_css),
            ("recipes.css", &t.recipes_css),
        ] {
            assets.scan_css(text, &origin, &format!("css:{name}"));
            put(&mut files, &format!("src/styles/{name}"), text.clone());
            app_css.push_str(&format!("@import \"./{name}\";\n"));
        }
        for (plan, tp) in plans.iter().zip(&t.plans) {
            // `url(..)` inside an arbitrary-value class names a file the project must serve.
            let joined: String = tp.classes.values().cloned().collect::<Vec<_>>().join(" ");
            let base = Url::parse(&plan.url)?;
            for u in assets::css_urls(&joined) {
                assets.resolve(&u, &base, &format!("class:{}", plan.path));
            }
            for (path, text) in styles::evidence_files(&plan.ns, tp) {
                put(&mut files, &path, text);
            }
        }
        styles.source = "mochi tokens: computed author styles translated into Tailwind utilities and shared recipes";
        styles.tokens = t.plans.first().map_or(0, |p| p.tokens.tokens.len());
        styles.sheets = vec![
            "src/styles/tokens.css".into(),
            "src/styles/at-rules.css".into(),
            "src/styles/recipes.css".into(),
        ];
        let statuses: Vec<String> = t
            .statuses
            .iter()
            .map(|(route, status)| format!("{route}: {status}"))
            .collect();
        styles.status = format!(
            "static equality checks only, not browser-verified ({})",
            statuses.join("; ")
        );
        gaps.extend(t.gaps.iter().cloned());
        for (plan, tp) in plans.iter().zip(&t.plans) {
            let r = &tp.report;
            gaps.push(format!(
                "styles: {}: {} element(s) translated into {} distinct classes ({} arbitrary properties, {} shared recipes, {} plain-CSS recipes); static check `{}` over {} comparisons; details in .mochi/provenance/styles/{}.styles.report.md",
                plan.path,
                r.elements_translated,
                r.utilities.distinct_classes,
                r.utilities.arbitrary_properties,
                r.recipes.shared,
                r.recipes.fallback_hooks,
                r.status,
                r.check.compared,
                plan.ns
            ));
        }
        gaps.push("styles: styles of interactive states are baked as the capture showed them (the selected tab's look, an open menu); no state variant is derived from a capture of one state".into());
    } else if opts.tokens {
        match super::tokens::pipeline::run(capture_dir, &Default::default()) {
            Ok(out) => {
                let mut sheets: Vec<(usize, String)> = out
                    .report
                    .sheets
                    .iter()
                    .map(|s| (s.index, s.output.clone()))
                    .collect();
                sheets.sort();
                sheets.dedup_by(|a, b| a.1 == b.1);
                for (path, text) in &out.files {
                    if let Some(name) = path.strip_prefix("src/styles/rewritten/") {
                        assets.scan_css(text, &origin, &format!("css:{name}"));
                        // A declaration with no value is dropped by the browser together with
                        // whatever the original shorthand set: say so instead of passing it on.
                        let empty = text.matches(": ;").count();
                        if empty > 0 {
                            gaps.push(format!(
                                "styles: {name} contains {empty} declaration(s) with an empty value (`prop: ;`) from the tokens rewrite; the browser ignores them, so the property they came from is lost"
                            ));
                        }
                    }
                    let target = if path.starts_with("src/") {
                        path.clone()
                    } else {
                        format!(".mochi/provenance/tokens/{path}")
                    };
                    put(&mut files, &target, text.clone());
                }
                if out.files.contains_key("src/styles/tokens.css") {
                    app_css.push_str("@import \"./tokens.css\";\n");
                }
                for (_, output) in &sheets {
                    let rel = output.strip_prefix("src/").unwrap_or(output);
                    style_imports.push(format!("@/{rel}"));
                    styles.sheets.push(output.clone());
                }
                styles.source =
                    "mochi tokens (captured stylesheets with literals replaced by token variables)";
                styles.tokens = out.report.summary.tokens;
                styles.status = format!(
                    "provisional: the captured class names are styled by the rewritten captured stylesheets ({}), not by Tailwind utilities; equality status `{}`",
                    out.report.status, out.report.status
                );
                gaps.push("styles: the captured stylesheets of all routes and viewports are imported together by the root route (no per-route split, no dead-rule pruning); a Tailwind class translation replaces them when the tokens bridge lands".into());
            }
            Err(e) => {
                gaps.push(format!(
                    "styles: the tokens pipeline failed ({e:#}); the project has no stylesheet"
                ));
            }
        }
    } else {
        gaps.push("styles: --no-tokens given; the project has no stylesheet".into());
    }

    // Assets and fonts, with provenance.
    let (asset_files, entries, mut skipped_assets) = assets.finish();
    let bytes: u64 = entries.iter().map(|e| e.bytes).sum();
    for (path, data) in asset_files {
        files.insert(path, data);
    }
    // What the emitted files actually point at, independent of what the resolver was asked.
    let dangling = refs::dangling(&files);
    for d in &dangling {
        skipped_assets.push(assets::AssetSkip {
            url: d.url.clone(),
            reason: "referenced by an emitted file but no file is present under public/".into(),
            referenced_by: d.file.clone(),
        });
    }
    if !dangling.is_empty() {
        gaps.push(format!(
            "assets: {} root-relative asset reference(s) in the emitted files have no file under public/ (they answer 404): {}",
            dangling.len(),
            dangling.iter().take(6).map(|d| d.url.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    put(
        &mut files,
        ".mochi/provenance/assets.json",
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "note": "Each file under public/ was copied byte for byte from the capture's mirrored site/ directory.",
            "assets": entries,
            "skipped": skipped_assets,
        }))?,
    );
    if !skipped_assets.is_empty() {
        gaps.push(format!(
            "assets: {} referenced asset(s) were not copied (other hosts, missing from the mirror); see .mochi/provenance/assets.json",
            skipped_assets.len()
        ));
    }

    if !custom.is_empty() {
        let decls: String = custom
            .iter()
            .map(|t| format!("      '{t}': React.DetailedHTMLProps<React.HTMLAttributes<HTMLElement>, HTMLElement>\n"))
            .collect();
        put(
            &mut files,
            "src/types/custom-elements.d.ts",
            format!(
                "// Custom elements rendered by the captured pages, declared so TSX accepts them.\nimport type React from 'react'\n\ndeclare module 'react' {{\n  namespace JSX {{\n    interface IntrinsicElements {{\n{decls}    }}\n  }}\n}}\n"
            ),
        );
    }

    // Project scaffolding.
    put(&mut files, "package.json", template::PACKAGE_JSON);
    put(&mut files, "bun.lock", template::BUN_LOCK);
    put(&mut files, "tsconfig.json", template::TSCONFIG);
    put(&mut files, "vite.config.ts", template::VITE_CONFIG);
    put(&mut files, ".bun-version", template::BUN_VERSION);
    put(&mut files, ".gitignore", template::GITIGNORE);
    put(
        &mut files,
        "scripts/generate-route-tree.ts",
        template::ROUTE_TREE_SCRIPT,
    );
    put(&mut files, "src/router.tsx", template::ROUTER);
    put(&mut files, "src/lib/document.ts", template::DOCUMENT_TS);
    put(&mut files, "src/styles/app.css", app_css);
    put(
        &mut files,
        "src/routes/__root.tsx",
        template::root_route(&style_imports),
    );

    gaps.extend([
        "behavior: no original script runs. Controls that were recognized are Base UI compositions with the state the captured bundle showed; every other interactive behavior (animation, scroll effects, forms, analytics) is absent".into(),
        "motion: entrance, scroll and state animations are not translated (milestone: src/semantic/motion)".into(),
        "graphics: canvas, WebGL and video elements are markup only, with no drawing code or shaders (milestone: src/semantic/graphics, `mochi lift`)".into(),
        "responsive: each route renders the DOM of one evidence viewport; DOM that differs between viewports is not merged".into(),
        "layout: header, footer and other markup shared by routes is duplicated per route; no layout route is extracted".into(),
        "text: copy is inline in the page and component sources; no content or translation layer".into(),
    ]);
    let report = ProjectReport {
        schema_version: 1,
        capture: capture_dir.display().to_string(),
        viewport_policy: opts
            .viewport
            .map(|v| format!("evidence viewport {v}"))
            .unwrap_or_else(|| "widest captured viewport per route".into()),
        routes: route_reports,
        skipped_routes,
        inventory: inventories,
        assets: AssetsReport {
            copied: entries.len(),
            bytes,
            entries,
            skipped: skipped_assets,
        },
        styles,
        ui_primitives: ui.into_iter().collect(),
        files: files.len(),
        verify: vec![
            "bun install --frozen-lockfile",
            "bun run check",
            "bun run build",
        ],
        gaps,
    };
    put(
        &mut files,
        ".mochi/semantic/project.report.json",
        serde_json::to_vec_pretty(&report)?,
    );
    put(
        &mut files,
        ".mochi/semantic/project.report.md",
        markdown(&report),
    );
    put(&mut files, "README.md", readme(&report));
    Ok(Project { files, report })
}

/// `<p>` elements that hold block-level content: valid in a DOM built by script, not in HTML.
fn invalid_paragraphs(doc: &Doc) -> usize {
    doc.nodes
        .iter()
        .filter(|n| n.is_element() && n.tag == "p")
        .filter(|n| emit::paragraph_holds_block(doc, n.id))
        .count()
}

/// Compare every route of a capture with the HTML the assembled project server-rendered for it
/// (`<ssr_dir>/<route ns>.html`, written by `scripts/semantic/ssr-check.mjs --save`).
pub fn compare_structure(
    capture_dir: &Path,
    ssr_dir: &Path,
    viewport: Option<&str>,
    routes: Option<usize>,
) -> Result<Vec<structure::RouteStructure>> {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(capture_dir.join("manifest.json"))
            .context("reading capture manifest.json")?,
    )?;
    let (plans, _) = routes::plan(&manifest, viewport, routes);
    let mut out = Vec::new();
    for plan in &plans {
        let Ok(html) = std::fs::read_to_string(ssr_dir.join(format!("{}.html", plan.ns))) else {
            out.push(structure::RouteStructure {
                route: plan.path.clone(),
                note: Some(format!(
                    "no rendered page {}.html in {}",
                    plan.ns,
                    ssr_dir.display()
                )),
                ..Default::default()
            });
            continue;
        };
        let capture = evidence::load(capture_dir, Path::new(&plan.evidence))
            .with_context(|| format!("loading {}", plan.evidence))?;
        let analysis = analyze_capture(capture_dir, &plan.evidence, &capture, true)?;
        let dialog = analysis.recognition.matches.iter().any(|m| {
            matches!(
                m.primitive,
                super::patterns::Primitive::Dialog | super::patterns::Primitive::AlertDialog
            )
        });
        out.push(structure::compare(
            &plan.path,
            &analysis.doc,
            &Doc::from_html(&html),
            dialog,
        ));
    }
    Ok(out)
}

/// Same-origin link targets (extension-less paths) that are not an assembled route.
fn uncaptured_links(doc: &Doc, base: &Url, routes: &BTreeSet<String>) -> Vec<String> {
    let mut out = BTreeSet::new();
    for n in &doc.nodes {
        if n.tag != "a" {
            continue;
        }
        let Some(href) = n.attr("href") else { continue };
        let Ok(url) = base.join(href) else { continue };
        if url.origin() != base.origin() {
            continue;
        }
        let mut path = url.path().to_owned();
        if path.len() > 1 && path.ends_with('/') {
            path.pop();
        }
        if path
            .rsplit('/')
            .next()
            .is_some_and(|last| last.contains('.'))
            || routes.contains(&path)
        {
            continue;
        }
        out.insert(path);
    }
    out.into_iter().collect()
}

fn readme(r: &ProjectReport) -> String {
    format!(
        "# Reconstructed site (semantic assembly)\n\nGenerated by `mochi semantic --assemble` from `{}`.\n\n\
         This is an editable TanStack Start project: {} route(s), Base UI primitives only under `src/components/ui/`, \
         compositions and data modules per route under `src/components/<route>/` and `src/data/<route>/`, \
         assets under `public/` with provenance in `.mochi/provenance/assets.json`.\n\n\
         It is **not** a finished conversion. {} Behavior, motion, \
         canvas/WebGL and scripts are absent. Read `.mochi/semantic/project.report.md` for every recognized pattern, \
         inventory count and gap before relying on it.\n\n```sh\nbun install --frozen-lockfile\nbun run check\nbun run build\nbun run dev   # http://127.0.0.1:3000\n```\n",
        r.capture,
        r.routes.len(),
        if r.styles.source == "none" {
            "No stylesheet is included, so pages render unstyled."
        } else {
            "Styles are provisional rewritten captured stylesheets."
        }
    )
}

fn markdown(r: &ProjectReport) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let _ = writeln!(
        o,
        "# Semantic project report\n\n- capture: `{}`\n- viewport: {}\n- routes: {} assembled, {} skipped\n- files: {}",
        r.capture,
        r.viewport_policy,
        r.routes.len(),
        r.skipped_routes.len(),
        r.files
    );
    let _ = writeln!(
        o,
        "- assets: {} copied ({} bytes), {} not copied",
        r.assets.copied,
        r.assets.bytes,
        r.assets.skipped.len()
    );
    let _ = writeln!(o, "- styles: {} — {}", r.styles.source, r.styles.status);
    let _ = writeln!(
        o,
        "- ui primitives: {:?}\n- verify with: {}\n",
        r.ui_primitives,
        r.verify.join(" && ")
    );
    let _ = writeln!(o, "## Routes\n");
    for (route, inv) in r.routes.iter().zip(&r.inventory) {
        let _ = writeln!(
            o,
            "### `{}` ({})\n\n- evidence: `{}`\n- body elements {}: {} inside components, {} plain markup\n- controls {}: by primitive {:?}, {} native links, {} unrecognized\n- patterns recognized {:?}; not placed: {:?}\n- repeated structures {} ({} extracted), near-duplicate pairs {}\n- landmarks (total/in components): {}\n- graphics/embeds {:?}; forms {:?}; scripts omitted {}; custom elements {:?}\n- components: {} compositions, {} repeated-structure files\n",
            route.plan.path,
            route.plan.component,
            route.plan.evidence,
            inv.body_elements,
            inv.component_elements,
            inv.plain_elements,
            inv.controls,
            inv.controls_by_primitive,
            inv.native_links,
            inv.unrecognized_controls,
            inv.patterns_recognized,
            inv.patterns_not_placed,
            inv.repeated_groups,
            inv.repeated_groups_extracted,
            inv.near_duplicate_pairs,
            inv.landmarks
                .iter()
                .map(|(k, c)| format!("{k} {}/{}", c.total, c.mapped))
                .collect::<Vec<_>>()
                .join(", "),
            inv.graphics,
            inv.forms,
            inv.scripts_omitted,
            inv.custom_elements,
            route.components.len(),
            route.repeated_components.len()
        );
    }
    if !r.skipped_routes.is_empty() {
        let _ = writeln!(o, "## Skipped routes\n");
        for s in &r.skipped_routes {
            let _ = writeln!(o, "- {}: {}", s.url, s.reason);
        }
    }
    let _ = writeln!(o, "\n## Gaps\n");
    for g in &r.gaps {
        let _ = writeln!(o, "- {g}");
    }
    o
}
