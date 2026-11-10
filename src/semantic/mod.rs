//! Semantic conversion: capture -> accessibility/structure IR -> Base UI pattern
//! recognition -> typed TSX. Each submodule is owned by one workstream.
//!
//! * `ir`, `js`, `patterns`, `components`, `emit`, `report`: IR and Base UI
//!   pattern recognition (this workstream).
//! * `tokens`: central design tokens (tokens workstream; it adds its own
//!   `pub mod tokens;` line below once its file exists).
pub mod cli;
pub mod components;
pub mod emit;
pub mod ir;
pub mod js;
pub mod patterns;
pub mod report;

use crate::evidence::{self, Capture};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

/// Everything recognized on one captured page.
pub struct PageAnalysis {
    pub page: String,
    pub doc: ir::Doc,
    pub js: Vec<js::JsFacts>,
    pub recognition: patterns::Recognition,
    pub components: components::Components,
}

impl PageAnalysis {
    /// Analyze an already built IR and script facts (fixtures, static sources).
    pub fn from_parts(page: &str, doc: ir::Doc, js: Vec<js::JsFacts>) -> PageAnalysis {
        let recognition = patterns::recognize(&doc, &js);
        let components = components::extract(&doc);
        PageAnalysis { page: page.to_owned(), doc, js, recognition, components }
    }
}

const MAX_JS_BYTES: u64 = 12 * 1024 * 1024;

/// Original bundle files the page loaded: capture resources that are scripts and
/// exist under `site/`. Files that do not parse yield no facts and are listed with
/// `parse_errors` so the report can say so.
pub fn load_js(capture_dir: &Path, capture: &Capture) -> Vec<js::JsFacts> {
    let site = capture_dir.join("site");
    let page = url::Url::parse(&capture.url).ok();
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let skipped = |file: &str, why: &str| js::JsFacts {
        file: file.to_owned(),
        skipped_reason: Some(why.to_owned()),
        ..Default::default()
    };
    for r in &capture.resources {
        let Ok(u) = url::Url::parse(&r.url) else { continue };
        if !u.path().ends_with(".js") && !u.path().ends_with(".mjs") {
            continue;
        }
        if page.as_ref().and_then(|p| p.host_str()) != u.host_str() {
            // Third-party scripts are not the page's own program.
            continue;
        }
        let rel = u.path().trim_start_matches('/');
        if rel.split('/').any(|c| c == ".." || c.is_empty()) || !seen.insert(rel.to_owned()) {
            continue;
        }
        let path: PathBuf = site.join(rel);
        let Ok(meta) = std::fs::metadata(&path) else {
            out.push(skipped(rel, "listed as a resource but not present under site/"));
            continue;
        };
        if meta.len() > MAX_JS_BYTES {
            out.push(skipped(rel, "larger than 12 MiB"));
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            out.push(skipped(rel, "not valid UTF-8"));
            continue;
        };
        out.push(js::extract(rel, &text));
    }
    out
}

/// Load `page_json` (a `reconstruction/pages/page-*.json` path relative to the
/// capture directory) and analyze it.
pub fn analyze_page(capture_dir: &Path, page_json: &Path, with_js: bool) -> Result<PageAnalysis> {
    let capture = evidence::load(capture_dir, page_json)
        .with_context(|| format!("loading {}", page_json.display()))?;
    ensure!(capture.complete, "capture is incomplete: issues are recorded in the evidence");
    let doc = ir::Doc::from_capture(&capture)?;
    let js = if with_js { load_js(capture_dir, &capture) } else { Vec::new() };
    let recognition = patterns::recognize(&doc, &js);
    let components = components::extract(&doc);
    Ok(PageAnalysis {
        page: page_json.display().to_string(),
        doc,
        js,
        recognition,
        components,
    })
}
