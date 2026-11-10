//! Offline, bounded source emission. Generated projects are unverified until built
//! and exercised; original application scripts and backend actions are not replayed.
use crate::{
    cache::fingerprint,
    evidence::{Capture, Content},
    project::{self, Project, SourceRef, Target},
    util,
};
use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
    sync::LazyLock,
};
use url::Url;

mod head;
pub(crate) mod structure;
mod surface;
const MODEL_BYTES: u64 = 128 * 1024 * 1024;
const TEXT_BYTES: usize = 128 * 1024 * 1024;
const ASSET_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const TEMPLATE: &[(&str, &str)] = &[
    (
        "src/styles/tailwind.css",
        include_str!("export_template/src/styles/tailwind.css"),
    ),
    ("package.json", include_str!("export_template/package.json")),
    ("bun.lock", include_str!("export_template/bun.lock")),
    (
        "tsconfig.json",
        include_str!("export_template/tsconfig.json"),
    ),
    (
        "vite.config.ts",
        include_str!("export_template/vite.config.ts"),
    ),
    (".gitignore", include_str!("export_template/.gitignore")),
    (".bun-version", include_str!("export_template/.bun-version")),
    (
        "src/router.tsx",
        include_str!("export_template/src/router.tsx"),
    ),
    (
        "src/routes/__root.tsx",
        include_str!("export_template/src/routes/__root.tsx"),
    ),
    (
        "src/lib/runtime.tsx",
        include_str!("export_template/src/lib/runtime.tsx"),
    ),
    (
        "src/lib/document.ts",
        include_str!("export_template/src/lib/document.ts"),
    ),
    (
        "src/lib/policy.ts",
        include_str!("export_template/src/lib/policy.ts"),
    ),
    (
        "scripts/generate-route-tree.ts",
        include_str!("export_template/scripts/generate-route-tree.ts"),
    ),
];
static CSS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)url\(\s*(?:"([^"\\]*)"|'([^'\\]*)'|([^\s()'"\\]*))\s*\)"#)
        .expect("CSS URL expression")
});
/// Inline images the shared runtime keeps (policy.ts: raster and SVG data URIs).
static SVG_DATA_IMAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^data:image/svg\+xml[;,]").expect("svg data image"));
static ICON_DATA_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^data:image/(png|jpeg|gif|webp|avif|svg\+xml)[;,]").expect("data icon")
});
static CSS_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)url\s*\(").expect("CSS URL start"));
static ROUTE_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/(?:[A-Za-z0-9_.-]+/?)*$").expect("literal route path"));

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub count: usize,
    pub examples: Vec<SourceRef>,
    /// Bounded textual examples for diagnostics without a DOM node (URLs, files).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}
/// One distinct URL that the generated source reaches outside the local export.
#[derive(Debug, Serialize)]
pub struct Reference {
    pub url: String,
    /// Every way the generated source reaches this URL, sorted.
    pub kinds: Vec<String>,
    pub occurrences: usize,
    pub examples: Vec<SourceRef>,
}
/// How one captured literal route is exported.
#[derive(Debug, Serialize)]
pub struct RouteMode {
    /// Captured literal path with its query, as in `captured_paths`.
    pub path: String,
    /// `recovered_client_program` (runs formatted original build output, not
    /// semantic source) or `static_structural_replica`.
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Why a route is static, including a refused recovery's error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
/// What replaced one captured canvas, frame, shadow host or runtime-only surface.
#[derive(Debug, Serialize)]
pub struct SurfaceRecord {
    pub route: String,
    pub viewport: [u32; 2],
    pub source: SourceRef,
    pub tag: String,
    pub kind: String,
    pub replacement: String,
    pub bounds: [f64; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
}
/// A request the browser made while the site was captured. Informational: the
/// generated source does not use these URLs.
#[derive(Debug, Serialize)]
pub struct CaptureRequest {
    pub url: String,
    pub status: String,
    pub usages: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub framework: String,
    pub input_sha256: String,
    /// `static_structural_replica` (no route recovered), `recovered_client_programs`
    /// (every route) or `recovered_client_programs_on_some_routes_static_replica_on_others`.
    pub runtime: String,
    /// `static_replica_everywhere`, `explicit_routes_strict` or
    /// `default_every_route_with_per_route_fallback`.
    pub client_recovery_mode: String,
    /// Per captured literal route: recovered client program or static replica, and why.
    pub route_modes: Vec<RouteMode>,
    /// SHA-256 of every recovered source file as written (src/compat/recovered/, the
    /// Client* entry components and, for native custom elements,
    /// src/recovered-elements.d.ts); inputs are in `client_recovery.sources`.
    pub recovered_files: BTreeMap<String, String>,
    pub routes: usize,
    pub captured_paths: Vec<String>,
    pub component_sources: BTreeMap<String, SourceRef>,
    pub components: usize,
    pub layouts: usize,
    /// Distinct files written below `public/` (hashed assets, surface images, aliases).
    pub local_assets: usize,
    /// Captured asset URLs that map onto those files; several URLs can share one file.
    pub local_asset_urls: usize,
    /// Mirrored HTML responses and icon files that shaped the document head, with
    /// SHA-256 digests. They are not covered by `input_sha256` (the saved model
    /// does not include them); export only verifies they stay unchanged while it runs.
    pub head_inputs: BTreeMap<String, String>,
    pub diagnostics: BTreeMap<String, Diagnostic>,
    /// Policy for URLs the generated source cannot serve from its own files.
    pub link_policy: String,
    /// Every distinct URL the generated source reaches outside the export, with counts.
    pub external_references: Vec<Reference>,
    /// Third-party and runtime requests seen during capture (analytics, API calls,
    /// ads). Not referenced by the generated source; listed so none is hidden.
    pub capture_time_requests: Vec<CaptureRequest>,
    pub surfaces: Vec<SurfaceRecord>,
    pub limitations: Vec<String>,
    pub verification: String,
    pub source_quality: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_recovery: Option<crate::recover::Recovery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<crate::interpret::Annotations>,
    #[serde(skip)]
    ledger: BTreeMap<String, Reference>,
}
impl Report {
    fn note(&mut self, code: &str, source: Option<SourceRef>) {
        self.record(code, source, None);
    }
    fn note_detail(&mut self, code: &str, detail: impl Into<String>) {
        self.record(code, None, Some(detail.into()));
    }
    fn record(&mut self, code: &str, source: Option<SourceRef>, detail: Option<String>) {
        let entry = self.diagnostics.entry(code.into()).or_insert(Diagnostic {
            count: 0,
            examples: Vec::new(),
            details: Vec::new(),
        });
        entry.count += 1;
        if entry.examples.len() < 8
            && let Some(source) = source
        {
            entry.examples.push(source);
        }
        if entry.details.len() < 8
            && let Some(detail) = detail
            && !entry.details.contains(&detail)
        {
            entry.details.push(detail);
        }
    }
    /// Record one occurrence of a URL the export cannot serve locally.
    fn reference(&mut self, kind: &str, url: &str, source: Option<&SourceRef>) {
        let entry = self
            .ledger
            .entry(url.to_owned())
            .or_insert_with(|| Reference {
                url: url.to_owned(),
                kinds: Vec::new(),
                occurrences: 0,
                examples: Vec::new(),
            });
        if !entry.kinds.iter().any(|k| k == kind) {
            entry.kinds.push(kind.to_owned());
            entry.kinds.sort();
        }
        entry.occurrences += 1;
        if entry.examples.len() < 3
            && let Some(source) = source
            && !entry.examples.contains(source)
        {
            entry.examples.push(source.clone());
        }
    }
}
struct Files {
    text: BTreeMap<String, String>,
    portable_paths: BTreeSet<String>,
    bytes: usize,
}
impl Files {
    fn add(&mut self, path: impl Into<String>, text: String) -> Result<()> {
        let path = path.into();
        ensure!(
            self.portable_paths.insert(path.to_ascii_lowercase()),
            "generated file collision on case-insensitive filesystems"
        );
        self.bytes = self
            .bytes
            .checked_add(text.len())
            .context("export size overflow")?;
        ensure!(
            self.bytes <= TEXT_BYTES,
            "export source byte budget exceeded"
        );
        ensure!(
            self.text.insert(path, text).is_none(),
            "generated file collision"
        );
        Ok(())
    }
}
/// Files written below the export's `public/`, counted once per distinct path.
struct PublicFiles {
    root: std::path::PathBuf,
    written: BTreeSet<String>,
    bytes: usize,
}
impl PublicFiles {
    fn put(&mut self, relative: &str, bytes: &[u8]) -> Result<()> {
        if !self.written.insert(relative.to_owned()) {
            return Ok(());
        }
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .context("export asset size overflow")?;
        ensure!(
            self.bytes <= ASSET_BYTES,
            "export asset byte budget exceeded"
        );
        util::write(&self.root, &self.root.join(relative), bytes)
    }
}
fn js<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn helpers(buttons: bool) -> String {
    let mut imports = String::from(
        "import {Anchor,elementProps,SourceContext,useSource,selectPage,pageHead,queryKey,type PageData,type Control,tokensCss,capturedCss,CapturedSurface,ShadowHost,ResponsiveCapture} from '@/lib/runtime'\n",
    );
    if buttons {
        imports.push_str("import {Button} from '@/components/ui/button'\n");
    }
    imports
}
fn read(root: &Path, relative: &Path, limit: u64) -> Result<Vec<u8>> {
    let path = util::checked_path(root, &root.join(relative))?;
    let file = std::fs::File::open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit,
        "export input byte limit exceeded or not a regular file"
    );
    let mut out = Vec::new();
    file.take(limit + 1).read_to_end(&mut out)?;
    ensure!(
        out.len() as u64 <= limit,
        "export input byte limit exceeded"
    );
    Ok(out)
}
fn children(capture: &Capture) -> Vec<Vec<usize>> {
    let mut tree = vec![Vec::new(); capture.nodes.len()];
    for (index, node) in capture.nodes.iter().enumerate() {
        if let Some(parent) = node.parent {
            tree[parent].push(index);
        }
    }
    tree
}
fn descendants(tree: &[Vec<usize>], root: usize) -> Vec<usize> {
    let mut stack = vec![root];
    let mut out = Vec::new();
    while let Some(n) = stack.pop() {
        out.push(n);
        stack.extend(tree[n].iter().rev());
    }
    out
}
fn observed_style(capture: &Capture, node: usize) -> String {
    let mut state = capture
        .animation_states
        .get(&node)
        .cloned()
        .unwrap_or_default();
    if let Content::Element { style, .. } = &capture.nodes[node].content {
        let computed = &capture.styles[*style];
        if computed
            .get("animation-name")
            .is_some_and(|value| !value.is_empty() && value != "none")
        {
            for key in [
                "animation-name",
                "animation-duration",
                "animation-timing-function",
                "animation-delay",
                "animation-iteration-count",
                "animation-direction",
                "animation-fill-mode",
                "animation-play-state",
            ] {
                if let Some(value) = computed.get(key) {
                    state.insert(key.to_owned(), value.clone());
                }
            }
        }
    }
    state
        .into_iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(";")
}
fn attrs(capture: &Capture, node: usize) -> BTreeMap<String, String> {
    let mut result = match &capture.nodes[node].content {
        Content::Element { attributes, .. } => attributes
            .iter()
            .map(|a| (a.name.clone(), a.value.clone()))
            .collect(),
        _ => BTreeMap::new(),
    };
    let observed = observed_style(capture, node);
    if !observed.is_empty() {
        let style = result.entry("style".into()).or_default();
        style.push(';');
        style.push_str(&observed);
    }
    result
}
fn document_nodes(capture: &Capture) -> Result<(usize, usize)> {
    let html=capture.nodes.iter().position(|n|matches!(&n.content,Content::Element{tag,namespace,..} if tag=="html" && namespace=="http://www.w3.org/1999/xhtml")).context("export requires an HTML document")?;
    let body = capture
        .nodes
        .iter()
        .position(|n| {
            n.parent == Some(html) && matches!(&n.content,Content::Element{tag,..} if tag=="body")
        })
        .context("export requires a body element")?;
    Ok((html, body))
}
fn dom_signature(capture: &Capture) -> Result<String> {
    let mut nodes = Vec::new();
    for n in &capture.nodes {
        match &n.content {
            Content::Text { value } => nodes.push(json!([n.parent, "text", value])),
            Content::Element {
                tag,
                namespace,
                attributes,
                control,
                ..
            } => nodes.push(json!([n.parent, tag, namespace, attributes, control])),
        }
    }
    Ok(fingerprint(&serde_json::to_vec(&(
        nodes,
        &capture.stylesheets,
        &capture.animation_states,
    ))?))
}
fn css(
    text: &str,
    base: &Url,
    assets: &BTreeMap<String, String>,
    report: &mut Report,
) -> Result<String> {
    ensure!(
        !text.to_ascii_lowercase().contains("@import"),
        "CSS @import requires an import-context adapter; export refused"
    );
    let matches: Vec<_> = CSS_URL.captures_iter(text).collect();
    ensure!(
        matches.len() == CSS_START.find_iter(text).count(),
        "unsupported escaped or nested CSS URL; export refused"
    );
    let mut result = String::new();
    let mut offset = 0;
    for matched in matches {
        let full = matched.get(0).context("missing CSS URL match")?;
        let raw = matched
            .get(1)
            .or(matched.get(2))
            .or(matched.get(3))
            .context("missing CSS URL")?
            .as_str();
        result.push_str(&text[offset..full.start()]);
        if raw.starts_with('#') {
            result.push_str(full.as_str());
        } else {
            let mut url = base.join(raw)?;
            let fragment = url.fragment().map(|f| format!("#{f}")).unwrap_or_default();
            url.set_fragment(None);
            let value = if let Some(local) = assets.get(url.as_str()) {
                format!("{local}{fragment}")
            } else if url.scheme() == "data" && raw.starts_with("data:image/") {
                raw.into()
            } else if url.scheme() == "data"
                && [
                    "data:font/woff;base64,",
                    "data:font/woff2;base64,",
                    "data:font/ttf;base64,",
                    "data:font/otf;base64,",
                ]
                .iter()
                .any(|prefix| raw.starts_with(prefix))
            {
                use base64::Engine;
                let payload = raw.split_once(',').context("missing font data")?.1;
                base64::engine::general_purpose::STANDARD
                    .decode(payload)
                    .context("invalid embedded font encoding")?;
                report.note_detail(
                    "embedded_font_data_preserved",
                    format!(
                        "{} ({} base64 bytes)",
                        raw.split_once(',').map_or("data:", |(head, _)| head),
                        payload.len()
                    ),
                );
                raw.into()
            } else if matches!(url.scheme(), "http" | "https") {
                report.note_detail("external_or_unmapped_css_url", url.as_str());
                report.reference("css_url_unmapped", url.as_str(), None);
                format!("{url}{fragment}")
            } else {
                anyhow::bail!("unsupported CSS URL scheme");
            };
            result.push_str(&format!("url({})", js(&value)?));
        }
        offset = full.end();
    }
    result.push_str(&text[offset..]);
    Ok(result)
}

struct Renderer<'a> {
    model: &'a Project,
    instances: BTreeMap<SourceRef, (&'a str, &'a crate::project::Instance)>,
    trees: BTreeMap<String, Vec<Vec<usize>>>,
    used: BTreeSet<String>,
    report: Report,
    surface_images: BTreeMap<(String, usize), String>,
    asset_urls: BTreeMap<String, String>,
    /// Literal captured route URLs; the runtime rewrites links to these to local paths.
    captured_routes: BTreeSet<String>,
    source_origin: String,
}
/// Function name placeholder while identical components are being detected.
const NAME_TOKEN: &str = "\u{0}NAME\u{0}";
/// Identifiers the generated sources already bind; component names must avoid them.
const RESERVED_NAMES: [&str; 20] = [
    "Anchor",
    "Button",
    "Control",
    "PageData",
    "DocumentPage",
    "ReactNode",
    "ResponsiveCapture",
    "CapturedSurface",
    "ShadowHost",
    "SourceContext",
    "Route",
    "RouteComponent",
    "Document",
    "Fragment",
    "Image",
    "Object",
    "String",
    "Array",
    "Map",
    "Set",
];
/// Document state and head data captured at one viewport of one route.
struct DocumentCapture {
    key: String,
    viewport: (u32, u32),
    /// The captured `<html>` and `<body>` nodes, for reference tracking.
    nodes: (SourceRef, SourceRef),
    css_id: String,
    html: BTreeMap<String, String>,
    body: BTreeMap<String, String>,
    head: Value,
}
/// Why an SVG must not be served from the export's own origin, if it must not.
/// An SVG opened directly is a document: scripts, handlers, `foreignObject`,
/// script-targeting SMIL, `javascript:` and non-image `data:` references, or
/// anything fetched from another origin would run or load as part of the app.
/// Constructs that hide those from a text check (namespace prefixes, character
/// references, DTD entities, XSLT, CSS escapes, non-UTF-8 encodings) are refused
/// too. Such files are refused (and reported) instead of rewritten.
fn svg_hazard(bytes: &[u8]) -> Option<&'static str> {
    static HAZARDS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        [
            // A namespace prefix (`<svg:script>`, `<h:script>`) names the same element.
            (r"<\s*(?:[^\s<>/:]+:)?script\b", "script element"),
            (r"[\s/:\x22\x27]on[a-z]+\s*=", "event handler attribute"),
            (r"javascript\s*:", "javascript: URL"),
            (r"<\s*(?:[^\s<>/:]+:)?foreignobject\b", "foreignObject"),
            // XHTML elements (script, iframe, img src) run or load even outside foreignObject.
            (r"http://www\.w3\.org/1999/xhtml", "XHTML content"),
            // Internal DTD subsets declare entities and default attributes that
            // expand into markup this text check never sees.
            (
                r"<!\s*(?:entity|attlist|element)\b|<!doctype[^>]*\[",
                "DTD internal subset",
            ),
            // XSLT (`type="text/xsl"`) can generate script from markup without `<script`.
            (r"<\?\s*xml-stylesheet", "xml-stylesheet processing instruction"),
            (
                r#"attributename\s*=\s*["']\s*(?:[^"':]+:)?(?:href|on|src|style)"#,
                "script-targeting animation",
            ),
            (
                r#"(?:href|src)\s*=\s*["']\s*(?:https?:|[/\\]{2})|url\(\s*["']?\s*(?:https?:|[/\\]{2})|@import|image-set\(|xml:base"#,
                "external reference",
            ),
            // CSS escapes spell `url(`, `@import` and scheme names without matching them.
            (r"\\", "CSS escape"),
            (
                r#"href\s*=\s*["']\s*data:\s*(?:text|application|image/svg|[;,])"#,
                "non-raster data: reference",
            ),
        ]
        .into_iter()
        .map(|(pattern, reason)| {
            (
                Regex::new(&format!("(?i){pattern}")).expect("svg hazard"),
                reason,
            )
        })
        .collect()
    });
    static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"&#(?:[xX]([0-9a-fA-F]{1,8})|([0-9]{1,10}));?|&(amp|quot|apos);")
            .expect("character reference")
    });
    // NUL (UTF-16) and ESC (ISO-2022-JP shift sequences decode to nothing, so
    // `scr<ESC>(Bipt` reads as `script`) let a browser see markup these bytes hide.
    if bytes.contains(&0) || bytes.contains(&0x1b) {
        return Some("non-UTF-8 text");
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Some("non-UTF-8 text");
    };
    // XML decodes character references in attribute values and text
    // (`&#x6a;avascript:`), and URL parsing drops tab and newline (`java&#9;script:`).
    let decoded = REFERENCE.replace_all(text, |found: &regex::Captures| {
        let code = match (found.get(1), found.get(2), found.get(3)) {
            (Some(hex), _, _) => u32::from_str_radix(hex.as_str(), 16).ok(),
            (_, Some(decimal), _) => decimal.as_str().parse().ok(),
            (_, _, Some(name)) => Some(match name.as_str() {
                "amp" => u32::from('&'),
                "quot" => u32::from('"'),
                _ => u32::from('\''),
            }),
            _ => None,
        };
        code.and_then(char::from_u32)
            .unwrap_or(char::REPLACEMENT_CHARACTER)
            .to_string()
    });
    let compact: String = decoded
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    HAZARDS
        .iter()
        .find(|(pattern, _)| {
            [text, &decoded, &compact]
                .iter()
                .any(|t| pattern.is_match(t))
        })
        .map(|(_, reason)| *reason)
}
/// PascalCase from words split on non-alphanumerics; `None` unless it is a short identifier.
fn pascal_case(raw: &str) -> Option<String> {
    let mut name = String::new();
    for word in raw
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let mut chars = word.chars();
        name.extend(chars.next()?.to_uppercase());
        name.push_str(chars.as_str());
    }
    (name.starts_with(|c: char| c.is_ascii_alphabetic()) && name.len() <= 48).then_some(name)
}
/// A component name taken from the prototype's own markup, with its provenance:
/// the authored `data-component` attribute, else a BEM-style class token
/// (`block__element` or `block--modifier`), which names the authored block.
/// Values bound to props differ between instances, so they never name anything.
fn authored_component_name(model: &Project, id: &str) -> Option<(String, &'static str)> {
    let component = &model.components[id];
    let bound = |attribute: &str| {
        component.props.values().any(|p| {
            p.relative_node == 0
                && matches!(&p.target, Target::Attribute { name, .. } if name == attribute)
        })
    };
    let prototype = &component.prototype;
    let capture = &model.observations[&prototype.observation].capture;
    let Content::Element { attributes, .. } = &capture.nodes[prototype.node].content else {
        return None;
    };
    let value = |attribute: &str| {
        attributes
            .iter()
            .find(|a| a.name == attribute)
            .map(|a| a.value.as_str())
    };
    if !bound("data-component")
        && let Some(name) = value("data-component").and_then(pascal_case)
    {
        return Some((name, "data-component"));
    }
    if bound("class") {
        return None;
    }
    // Tokens with digits are generated (CSS modules, hashes), not authored names;
    // brackets, colons and other punctuation mark utility syntax such as
    // Tailwind's `[--gap:var(--x)]` or `md:`, not a BEM block.
    let plain = |token: &&str| {
        token
            .bytes()
            .all(|b| b.is_ascii_alphabetic() || b == b'-' || b == b'_')
    };
    let classes: Vec<&str> = value("class")?.split_whitespace().filter(plain).collect();
    classes
        .iter()
        .find(|t| t.contains("__"))
        .or_else(|| classes.iter().find(|t| t.contains("--")))
        .and_then(|t| pascal_case(t))
        .map(|name| (name, "class"))
}
/// Icon links and structured data for a route, read from the mirrored HTML
/// response (the DOM snapshot keeps only `<title>` of `<head>`). Same-origin
/// icons are copied under `public/`; others stay absolute and are reported.
fn route_head(
    root: &Path,
    page: &Url,
    base: &Url,
    asset_urls: &BTreeMap<String, String>,
    public: &mut PublicFiles,
    report: &mut Report,
) -> Result<(Vec<Value>, Vec<Value>)> {
    let Some(input) = head::read(root, page)? else {
        report.note_detail("head_source_missing", page.as_str());
        return Ok((Vec::new(), Vec::new()));
    };
    report
        .head_inputs
        .insert(input.path.clone(), input.sha256.clone());
    let parsed = head::parse(&input.text);
    for skipped in &parsed.skipped {
        report.note_detail("head_entry_not_exported", format!("{page}: {skipped}"));
    }
    for _ in 0..parsed.duplicates {
        report.note_detail("head_duplicate_icon_collapsed", page.as_str());
    }
    let mut links = Vec::new();
    for mut icon in parsed.icons {
        let href = icon["href"].clone();
        let Ok(mut absolute) = base.join(&href) else {
            report.note_detail(
                "head_entry_not_exported",
                format!("{page}: bad icon href {href}"),
            );
            continue;
        };
        absolute.set_fragment(None);
        if absolute.scheme() == "data" && !ICON_DATA_IMAGE.is_match(absolute.as_str()) {
            // `data:,` is the no-favicon idiom and loads nothing; any other
            // non-image payload is not an icon the runtime would keep.
            let shown: String = absolute.as_str().chars().take(80).collect();
            let blank = absolute.as_str() == "data:,";
            report.note_detail(
                if blank {
                    "head_icon_inline_data"
                } else {
                    "head_entry_not_exported"
                },
                format!("{page}: {shown}"),
            );
            if blank {
                links.push(json!(icon));
            }
            continue;
        }
        if absolute.scheme() == "data" {
            // Inline icons (including the `data:,` no-favicon idiom) load nothing
            // outside the export; browsers fetch link icons in image mode.
            let shown: String = absolute.as_str().chars().take(80).collect();
            report.note_detail("head_icon_inline_data", format!("{page}: {shown}"));
            links.push(json!(icon));
            continue;
        }
        if !matches!(absolute.scheme(), "http" | "https") {
            report.note_detail(
                "head_entry_not_exported",
                format!("{page}: icon href with scheme {}", absolute.scheme()),
            );
            continue;
        }
        let mut local = asset_urls.get(absolute.as_str()).cloned();
        if local.is_none()
            && matches!(absolute.scheme(), "http" | "https")
            && absolute.origin() == base.origin()
        {
            let mut stripped = absolute.clone();
            stripped.set_query(None);
            let extension = Path::new(stripped.path())
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(
                extension.as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "ico" | "svg"
            ) && let Ok(path) = crate::mirror::output_path(&root.join("site"), &stripped, false)
                && let Ok(bytes) = read(root, path.strip_prefix(root)?, 8 * 1024 * 1024)
            {
                let digest = fingerprint(&bytes);
                // Hashed either way: a refused file still decided what was emitted.
                report.head_inputs.insert(
                    path.strip_prefix(root)?.to_string_lossy().into_owned(),
                    digest.clone(),
                );
                if extension == "svg"
                    && let Some(reason) = svg_hazard(&bytes)
                {
                    report.note_detail("asset_svg_not_copied", format!("{absolute}: {reason}"));
                    icon.insert("href".into(), absolute.to_string());
                    links.push(json!(icon));
                    continue;
                }
                let target = format!("/_mochi/assets/{digest}.{extension}");
                public.put(&format!("public{target}"), &bytes)?;
                if absolute.query().is_some() {
                    report.note_detail(
                        "head_icon_matched_mirror_path_ignoring_query",
                        absolute.as_str(),
                    );
                }
                local = Some(target);
            }
        }
        if let Some(target) = local {
            report.note_detail("head_icon_localized", format!("{absolute} -> {target}"));
            icon.insert("href".into(), target);
        } else {
            // The external reference is recorded per emitted route entry.
            report.note_detail("head_icon_not_local", absolute.as_str());
            icon.insert("href".into(), absolute.to_string());
        }
        links.push(json!(icon));
    }
    let scripts = parsed
        .json_ld
        .iter()
        .map(|text| {
            report.note_detail("head_json_ld_emitted", page.as_str());
            // `<` only occurs inside JSON strings, where \u003c is equivalent.
            json!({"type":"application/ld+json","children":text.replace('<', "\\u003c")})
        })
        .collect();
    Ok((links, scripts))
}
/// Custom elements that exist only to run the original framework's client router.
const FRAMEWORK_ELEMENTS: [&str; 2] = ["next-route-announcer", "nextjs-portal"];
/// `<meta>` entries that carry per-request framework state, not authored content.
fn framework_meta(m: &crate::evidence::Meta) -> bool {
    matches!(m.property.as_deref(), Some("csp-nonce"))
        || matches!(
            m.name.as_deref(),
            Some("next-size-adjust" | "next-head-count" | "csp-nonce")
        )
}
/// Placeholder for a component name that is final only after naming and
/// de-duplication; it contains NUL, which never appears in generated source.
fn component_token(id: &str) -> String {
    format!("\u{0}{id}\u{0}")
}
/// Candidate URLs of a `srcset` that the runtime re-emits: the WHATWG
/// tokenization of `parseSrcset` in policy.ts, keeping only candidates whose
/// descriptors `srcsetDescriptorValid` accepts. Descriptors are never URLs.
fn srcset_urls(value: &str) -> Vec<&str> {
    static W: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+w$").expect("w"));
    static H: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+h$").expect("h"));
    static X: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(\d+(\.\d+)?|\.\d+)([eE][+-]?\d+)?x$").expect("x"));
    let space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\x0c' | '\r');
    let valid = |descriptor: &str| {
        let tokens: Vec<&str> = descriptor.split(space).filter(|t| !t.is_empty()).collect();
        let w = tokens.iter().filter(|t| W.is_match(t)).count();
        let h = tokens.iter().filter(|t| H.is_match(t)).count();
        let x = tokens.iter().filter(|t| X.is_match(t)).count();
        w + h + x == tokens.len()
            && w <= 1
            && h <= 1
            && x <= 1
            && !(w > 0 && x > 0)
            && !(h > 0 && w == 0)
            && !tokens
                .iter()
                .any(|t| W.is_match(t) && t.trim_end_matches('w').bytes().all(|b| b == b'0'))
    };
    let mut out = Vec::new();
    let mut rest = value;
    loop {
        rest = rest.trim_start_matches(|c: char| space(c) || c == ',');
        if rest.is_empty() {
            break;
        }
        let end = rest.find(space).unwrap_or(rest.len());
        let (url, after) = rest.split_at(end);
        rest = after;
        let mut descriptor = "";
        let url = if url.ends_with(',') {
            url.trim_end_matches(',')
        } else {
            let mut parens = false;
            let mut stop = rest.len();
            for (i, c) in rest.char_indices() {
                if parens {
                    parens = c != ')';
                } else if c == '(' {
                    parens = true;
                } else if c == ',' {
                    stop = i;
                    break;
                }
            }
            descriptor = &rest[..stop];
            rest = rest.get(stop + 1..).unwrap_or("");
            url
        };
        if !url.is_empty() && valid(descriptor) {
            out.push(url);
        }
    }
    out
}
impl Renderer<'_> {
    /// Record where a URL attribute will point once the shared runtime resolves it.
    fn track_reference(&mut self, source: &SourceRef, tag: &str, name: &str, value: &str) {
        if name == "srcset" {
            // The runtime keeps srcset on img/source only, and only candidates
            // with valid descriptors (policy.ts parseSrcset/serializeSrcset).
            if matches!(tag, "img" | "source") {
                for url in srcset_urls(value) {
                    self.track_reference(source, tag, "src", url);
                }
            }
            return;
        }
        if !matches!(
            name,
            "href" | "src" | "poster" | "cite" | "background" | "xlink:href"
        ) {
            return;
        }
        let capture = &self.model.observations[&source.observation].capture;
        let Ok(base) = Url::parse(&capture.metadata.base_url) else {
            return;
        };
        let value = value.trim();
        if value.is_empty() || value.starts_with('#') {
            return;
        }
        let Ok(mut url) = base.join(value) else {
            return;
        };
        if !matches!(url.scheme(), "http" | "https") {
            return;
        }
        url.set_fragment(None);
        let anchor = matches!(tag, "a" | "area") && name == "href";
        self.reference_url(source, anchor, anchor, url);
    }
    /// Record an absolute http(s) URL unless it is served locally: an asset, or
    /// (when the emitter rewrites route links) a captured route.
    pub(super) fn reference_url(
        &mut self,
        source: &SourceRef,
        anchor: bool,
        routes_localized: bool,
        mut url: Url,
    ) {
        if !matches!(url.scheme(), "http" | "https") {
            return;
        }
        url.set_fragment(None);
        if self.asset_urls.contains_key(url.as_str())
            || (routes_localized && self.captured_routes.contains(url.as_str()))
        {
            return;
        }
        let same_origin = url.origin().ascii_serialization() == self.source_origin;
        let kind = match (anchor, same_origin) {
            (true, true) => "uncaptured_internal_link",
            (true, false) => "external_link",
            (false, true) => "live_origin_resource",
            (false, false) => "external_resource",
        };
        self.report.reference(kind, url.as_str(), Some(source));
    }
    fn node(
        &mut self,
        source: &SourceRef,
        bindings: &BTreeMap<usize, Vec<(&str, &Target)>>,
        skip: Option<&SourceRef>,
    ) -> Result<String> {
        if let Content::Element { tag, .. } =
            &self.model.observations[&source.observation].capture.nodes[source.node].content
            && FRAMEWORK_ELEMENTS.contains(&tag.as_str())
        {
            self.report.record(
                "framework_runtime_element_omitted",
                Some(source.clone()),
                Some(tag.clone()),
            );
            return Ok("{null}".into());
        }
        if let Some(jsx) = self.surface(source)? {
            return Ok(jsx);
        }
        if skip != Some(source)
            && let Some(&(component, instance)) = self.instances.get(source)
        {
            self.used.insert(component.to_owned());
            let mut out = format!("<{}", component_token(component));
            // Prop values are the instance's own attribute values; track the URLs among them.
            let subtree = if instance.props.is_empty() {
                Vec::new()
            } else {
                descendants(&self.trees[&source.observation], source.node)
            };
            for (name, value) in &instance.props {
                if let (Some(prop), Some(url)) = (
                    self.model.components[component].props.get(name),
                    value.as_str(),
                ) && let Target::Attribute {
                    name: attribute, ..
                } = &prop.target
                    && let Some(&node) = subtree.get(prop.relative_node)
                    && let Content::Element { tag, .. } =
                        &self.model.observations[&source.observation].capture.nodes[node].content
                {
                    let tag = tag.clone();
                    let attribute = attribute.clone();
                    let url = url.to_owned();
                    self.track_reference(source, &tag, &attribute, &url);
                }
            }
            for (name, value) in &instance.props {
                out.push_str(&format!(" {name}={{{}}}", js(value)?));
            }
            out.push_str(" />");
            return Ok(out);
        }
        let capture = &self.model.observations[&source.observation].capture;
        match &capture.nodes[source.node].content {
            Content::Text { value } => {
                if let Some(name) = bindings
                    .get(&source.node)
                    .and_then(|b| b.iter().find(|(_, t)| matches!(t, Target::Text)))
                    .map(|(n, _)| n)
                {
                    Ok(format!("{{props.{name}}}"))
                } else {
                    Ok(format!("{{{}}}", js(value)?))
                }
            }
            Content::Element {
                tag,
                namespace,
                attributes,
                control,
                ..
            } => {
                if matches!(
                    tag.as_str(),
                    "script"
                        | "style"
                        | "template"
                        | "link"
                        | "meta"
                        | "base"
                        | "iframe"
                        | "frame"
                        | "object"
                        | "embed"
                        | "applet"
                ) {
                    self.report.record(
                        "omitted_executable_or_embedded_element",
                        Some(source.clone()),
                        Some(tag.clone()),
                    );
                    return Ok("{null}".into());
                }
                let html = "a abbr address area article aside audio b bdi bdo blockquote br button caption cite code col colgroup data datalist dd del details dfn dialog div dl dt em fieldset figcaption figure footer form h1 h2 h3 h4 h5 h6 header hgroup hr i img input ins kbd label legend li main map mark menu meter nav ol optgroup option output p picture pre progress q rp rt ruby s samp search section select slot small source span strong sub summary sup table tbody td textarea tfoot th thead time tr track u ul var wbr";
                let svg = "animate animateMotion animateTransform circle clipPath defs desc ellipse feBlend feColorMatrix feComponentTransfer feComposite feConvolveMatrix feDiffuseLighting feDisplacementMap feDistantLight feDropShadow feFlood feFuncA feFuncB feFuncG feFuncR feGaussianBlur feImage feMerge feMergeNode feMorphology feOffset fePointLight feSpecularLighting feSpotLight feTile feTurbulence filter foreignObject g image line linearGradient marker mask metadata mpath path pattern polygon polyline radialGradient rect set stop svg switch symbol text textPath title tspan use view";
                ensure!(
                    (namespace == "http://www.w3.org/1999/xhtml"
                        && html.split_whitespace().any(|t| t == tag))
                        || (namespace == "http://www.w3.org/2000/svg"
                            && svg.split_whitespace().any(|t| t == tag)),
                    "unsupported source tag {tag}; export refused"
                );
                let animation = observed_style(capture, source.node);
                if !animation.is_empty() {
                    self.report
                        .note("observed_animation_state_preserved", Some(source.clone()));
                }
                let mut pairs = Vec::new();
                let mut names = BTreeSet::new();
                for attr in attributes {
                    ensure!(
                        names.insert(&attr.name),
                        "duplicate qualified attribute names; export refused"
                    );
                    if attr.name.to_ascii_lowercase().starts_with("on")
                        || matches!(
                            attr.name.as_str(),
                            "action" | "formaction" | "srcdoc" | "is" | "autoplay" | "ping"
                        )
                    {
                        self.report.note(
                            "omitted_action_or_executable_attribute",
                            Some(source.clone()),
                        );
                        continue;
                    }
                    if attr.name == "imagesrcset" {
                        // Only valid on <link>, which is never emitted.
                        self.report
                            .note("imagesrcset_omitted", Some(source.clone()));
                        continue;
                    }
                    if attr.name == "style" {
                        self.report
                            .note("inline_style_requires_review", Some(source.clone()));
                    }
                    let binding=bindings.get(&source.node).and_then(|b|b.iter().find(|(_,t)|matches!(t,Target::Attribute{name,namespace} if name==&attr.name && namespace==&attr.namespace)));
                    // A bound value is not in this source; each instance's prop records it.
                    if binding.is_none() {
                        self.track_reference(source, tag, &attr.name, &attr.value);
                    }
                    let value = if let Some((name, _)) = binding {
                        format!("props.{name}")
                    } else {
                        js(&attr.value)?
                    };
                    let value = if attr.name == "style" && !animation.is_empty() {
                        format!("({value}+';'+{})", js(&animation)?)
                    } else {
                        value
                    };
                    pairs.push(format!("{}:{value}", js(&attr.name)?));
                }
                if !animation.is_empty() && !names.contains(&"style".to_owned()) {
                    pairs.push(format!("style:{}", js(&animation)?));
                }
                let raw = format!("{{{}}}", pairs.join(","));
                let control_value = if let Some((name, _)) = bindings
                    .get(&source.node)
                    .and_then(|b| b.iter().find(|(_, t)| matches!(t, Target::Control)))
                {
                    format!("props.{name}")
                } else {
                    js(control)?
                };
                let output_tag = match tag.as_str() {
                    "a" => "Anchor",
                    "button" => "Button",
                    other => other,
                };
                if tag == "button" {
                    self.report
                        .note("button_action_requires_integration", Some(source.clone()));
                }
                if tag == "select" && attributes.iter().any(|a| a.name == "multiple") {
                    anyhow::bail!(
                        "multi-select state requires a selection adapter; export refused"
                    );
                }
                let mut out = if tag == "a" {
                    format!("<{output_tag} raw={{{raw}}}")
                } else if tag == "button" {
                    // Captured HTML buttons default to submit. The public UI
                    // primitive defaults to button for ordinary new app code.
                    format!(
                        "<Button type=\"submit\" {{...elementProps('button', {raw}, {control_value}, base)}}"
                    )
                } else {
                    format!(
                        "<{output_tag} {{...elementProps({}, {raw}, {control_value}, base)}}",
                        js(tag)?
                    )
                };
                if matches!(
                    tag.as_str(),
                    "area" | "br" | "col" | "hr" | "img" | "input" | "source" | "track" | "wbr"
                ) {
                    out.push_str(" />");
                    return Ok(out);
                }
                out.push('>');
                let children = self.trees[&source.observation][source.node].clone();
                if tag != "textarea" {
                    for node in children {
                        out.push_str(&self.node(
                            &SourceRef {
                                observation: source.observation.clone(),
                                node,
                            },
                            bindings,
                            skip,
                        )?);
                    }
                }
                out.push_str(&format!("</{output_tag}>"));
                Ok(out)
            }
        }
    }
    /// Component source with `name` as its exported function name.
    fn component(&mut self, id: &str, buttons: bool, name: &str) -> Result<String> {
        let component = &self.model.components[id];
        let nodes = descendants(
            &self.trees[&component.prototype.observation],
            component.prototype.node,
        );
        let mut bindings: BTreeMap<usize, Vec<(&str, &Target)>> = BTreeMap::new();
        let mut props = Vec::new();
        for (name, prop) in &component.props {
            bindings
                .entry(nodes[prop.relative_node])
                .or_default()
                .push((name, &prop.target));
            props.push(format!(
                "{name}:{}",
                if matches!(prop.target, Target::Control) {
                    "Control"
                } else {
                    "string"
                }
            ));
        }
        let source = self.node(&component.prototype, &bindings, Some(&component.prototype))?;
        let helpers = helpers(buttons);
        Ok(format!(
            "{helpers}export function {name}(props:{{{}}}){{const base=useSource();return ({source})}}\n",
            props.join(";")
        ))
    }
}

/// Generate a new TanStack Start source project from a saved, unchanged model.
/// This never installs packages, calls a model, executes captured scripts, or
/// overwrites an existing project. Runtime verification is a separate operation.
/// Every route is a static structural replica; no client program is recovered.
pub fn tanstack_start(root: &Path, out: &Path) -> Result<Report> {
    generate(root, out, false, ClientMode::Static)
}
/// Attach an explicitly selected saved interpretation as review-required source
/// comments/report metadata. This never calls a provider or auto-applies labels.
pub fn tanstack_start_with_interpretation(root: &Path, out: &Path) -> Result<Report> {
    generate(root, out, true, ClientMode::Static)
}
/// Recover selected public client roots as editable modules. This is explicit
/// and strict: a selected route that cannot be recovered fails the export. The
/// generated app executes recovered client code, unlike initial-state export.
pub fn tanstack_start_with_clients(
    root: &Path,
    out: &Path,
    interpretation: bool,
    routes: &BTreeSet<String>,
) -> Result<Report> {
    generate(
        root,
        out,
        interpretation,
        if routes.is_empty() {
            ClientMode::Static
        } else {
            ClientMode::Selected(routes)
        },
    )
}
/// The compatibility default: recover the client program of every captured
/// literal route that can be recovered. A route whose modules are unsupported
/// or refused falls back to the static structural replica for that route only;
/// the route, the reason and the fallback are recorded in the report.
pub fn tanstack_start_recovering(root: &Path, out: &Path, interpretation: bool) -> Result<Report> {
    generate(root, out, interpretation, ClientMode::Available)
}
#[derive(Clone, Copy)]
enum ClientMode<'a> {
    /// `--no-recover-client`: static replicas everywhere.
    Static,
    /// `--recover-client-route`: strict; any refusal aborts the export.
    Selected(&'a BTreeSet<String>),
    /// Default: recover what can be recovered, per route.
    Available,
}
/// Final text of a generated source file: unused locals pruned (app source only), then formatted.
fn finish_source(path: &str, text: &str) -> Result<String> {
    let pruned = if ["src/components/", "src/layouts/", "src/features/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
        && path.ends_with(".tsx")
        || path.ends_with("/data.ts")
    {
        structure::prune_unused(text)
    } else {
        text.to_owned()
    };
    structure::format_source(path, &pruned)
}
fn generate(
    root: &Path,
    out: &Path,
    attach_interpretation: bool,
    client_mode: ClientMode<'_>,
) -> Result<Report> {
    let root = root.canonicalize()?;
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let name = out
        .file_name()
        .context("output needs a project directory name")?;
    let destination = util::checked_path(&parent, &parent.join(name))?;
    ensure!(
        destination
            .symlink_metadata()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "export output already exists"
    );
    ensure!(
        !destination.starts_with(&root) && !root.starts_with(&destination),
        "source and export directories must not contain each other"
    );
    let bytes = read(&root, Path::new("reconstruction/project.json"), MODEL_BYTES)
        .context("run model-project before export")?;
    let model: Project = serde_json::from_slice(&bytes)?;
    model.validate()?;
    ensure!(
        project::derive(&root)?.input_sha256 == model.input_sha256,
        "saved model is stale; rerun model-project"
    );
    let interpretation = if attach_interpretation {
        Some(crate::interpret::annotations(&root, &model)?)
    } else {
        None
    };
    let mut primary = BTreeMap::new();
    let mut origins = BTreeSet::new();
    for (id, route) in &model.routes {
        let url = Url::parse(&route.url)?;
        origins.insert(url.origin().ascii_serialization());
        ensure!(
            ROUTE_PATH.is_match(url.path())
                && !url.path().contains("//")
                && url.query() != Some(""),
            "unsupported literal route spelling; export refused"
        );
        let observation = route
            .observations
            .iter()
            .max_by_key(|id| {
                let c = &model.observations[*id].capture;
                (c.viewport.width, c.viewport.height, *id)
            })
            .context("route has no observation")?;
        primary.insert(id.clone(), observation.clone());
    }
    ensure!(
        origins.len() == 1,
        "multiple source origins require an explicit origin mapping; export refused"
    );
    let buttons = primary.values().any(|id| {
        model.observations[id]
            .capture
            .nodes
            .iter()
            .any(|n| matches!(&n.content,Content::Element{tag,..} if tag=="button"))
    });
    let helpers = helpers(buttons);
    let (recovery, fallbacks) = match client_mode {
        ClientMode::Static => (None, Vec::new()),
        ClientMode::Selected(routes) => (
            Some(crate::recover::plan(&root, &model, routes)?),
            Vec::new(),
        ),
        ClientMode::Available => crate::recover::plan_available(&root, &model)?,
    };
    let selected = match client_mode {
        ClientMode::Selected(routes) => Some(routes),
        _ => None,
    };
    // One record per captured route: routes differing only by query share a
    // pathname (and its recovery decision) but are listed separately.
    let mut route_modes = BTreeMap::new();
    for route in model.routes.values() {
        let shown = match &route.query {
            Some(query) => format!("{}?{query}", route.pathname),
            None => route.pathname.clone(),
        };
        let mode = match recovery
            .as_ref()
            .and_then(|r| r.routes.get(&route.pathname))
        {
            Some(recovered) => RouteMode {
                path: shown.clone(),
                mode: "recovered_client_program".into(),
                component: Some(recovered.component.clone()),
                module: Some(recovered.module),
                method: Some(recovered.method.clone()),
                reason: None,
            },
            None => RouteMode {
                path: shown.clone(),
                mode: "static_structural_replica".into(),
                component: None,
                module: None,
                method: None,
                reason: Some(
                    if let Some(fallback) = fallbacks.iter().find(|f| f.path == route.pathname) {
                        format!(
                            "fallback: client recovery refused for this route ({})",
                            fallback.reason
                        )
                    } else if matches!(client_mode, ClientMode::Static) {
                        "client recovery not requested (--no-recover-client or library default)"
                            .into()
                    } else if selected.is_some() {
                        "not selected with --recover-client-route".into()
                    } else {
                        "fallback: no client recovery result for this route".into()
                    },
                ),
            },
        };
        route_modes.insert(shown, mode);
    }
    let route_modes: Vec<RouteMode> = route_modes.into_values().collect();
    let conditions = structure::Conditions::observe(&model, &route_modes);
    let report = Report {
        schema_version: 1,
        framework: "tanstack-start".into(),
        input_sha256: model.input_sha256.clone(),
        runtime: match route_modes
            .iter()
            .filter(|m| m.mode == "recovered_client_program")
            .count()
        {
            0 => "static_structural_replica",
            n if n == route_modes.len() => "recovered_client_programs",
            _ => "recovered_client_programs_on_some_routes_static_replica_on_others",
        }
        .into(),
        client_recovery_mode: match client_mode {
            ClientMode::Static => "static_replica_everywhere",
            ClientMode::Selected(_) => "explicit_routes_strict",
            ClientMode::Available => "default_every_route_with_per_route_fallback",
        }
        .into(),
        route_modes,
        recovered_files: BTreeMap::new(),
        routes: model.routes.len(),
        captured_paths: Vec::new(),
        component_sources: BTreeMap::new(),
        components: 0,
        layouts: 0,
        local_assets: 0,
        local_asset_urls: 0,
        head_inputs: BTreeMap::new(),
        diagnostics: BTreeMap::new(),
        link_policy: structure::LINK_POLICY.into(),
        external_references: Vec::new(),
        capture_time_requests: Vec::new(),
        surfaces: Vec::new(),
        interpretation,
        client_recovery: recovery,
        limitations: structure::limitations(&conditions),
        verification: "not_run".into(),
        source_quality: "compatibility_reconstruction_semantic_refactor_required".into(),
        ledger: BTreeMap::new(),
    };
    let instances = model
        .components
        .iter()
        .flat_map(|(id, c)| {
            c.instances
                .iter()
                .map(move |instance| (instance.source.clone(), (id.as_str(), instance)))
        })
        .collect();
    let trees = model
        .observations
        .iter()
        .map(|(id, o)| (id.clone(), children(&o.capture)))
        .collect();
    let mut renderer = Renderer {
        model: &model,
        instances,
        trees,
        used: BTreeSet::new(),
        report,
        surface_images: BTreeMap::new(),
        asset_urls: BTreeMap::new(),
        captured_routes: BTreeSet::new(),
        source_origin: Url::parse(&model.routes.values().next().context("missing route")?.url)?
            .origin()
            .ascii_serialization(),
    };
    let mut files = Files {
        text: BTreeMap::new(),
        portable_paths: BTreeSet::new(),
        bytes: 0,
    };
    for &(path, text) in TEMPLATE {
        let text = if renderer
            .report
            .client_recovery
            .as_ref()
            .and_then(|r| r.react_version.as_deref())
            == Some("19.2.7")
        {
            match path {
                "package.json" => include_str!("export_template/package-react-19.2.7.json"),
                "bun.lock" => {
                    include_str!("export_template/bun-react-19.2.7.lock")
                }
                _ => text,
            }
        } else {
            text
        };
        files.add(path, text.into())?;
    }
    if buttons {
        files.add(
            "src/components/ui/button.tsx",
            include_str!("export_template/src/components/ui/button.tsx").into(),
        )?;
    }
    let staging = tempfile::Builder::new()
        .prefix(".mochi-export-")
        .tempdir_in(&parent)?;
    let mut asset_urls = BTreeMap::new();
    let mut public = PublicFiles {
        root: staging.path().to_path_buf(),
        written: BTreeSet::new(),
        bytes: 0,
    };
    let source_origin =
        Url::parse(&model.routes.values().next().context("missing route")?.url)?.origin();
    for (url, asset) in &model.assets {
        if asset.status != "local_candidate" {
            let shown: String = url.chars().take(160).collect();
            // The runtime renders SVG data images as <img src> (policy.ts svgDataImage).
            let code = if asset.status == "inline_or_unsupported" && SVG_DATA_IMAGE.is_match(url) {
                "asset_inline_svg_data_image".to_owned()
            } else {
                format!("asset_{}", asset.status)
            };
            renderer.report.note_detail(&code, shown);
            if matches!(asset.status.as_str(), "external" | "runtime_observation") {
                renderer.report.capture_time_requests.push(CaptureRequest {
                    url: url.clone(),
                    status: asset.status.clone(),
                    usages: asset.usages.iter().cloned().collect(),
                });
            }
            continue;
        }
        let parsed = Url::parse(url)?;
        let extension = Path::new(parsed.path())
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(
            extension.as_str(),
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "avif"
                | "ico"
                | "svg"
                | "woff"
                | "woff2"
                | "ttf"
                | "otf"
        ) {
            renderer
                .report
                .note_detail("asset_not_passive_image_or_font", url.as_str());
            continue;
        }
        let source = asset.local_path.as_ref().context("asset path missing")?;
        let bytes = read(&root, Path::new(source), 64 * 1024 * 1024)?;
        let digest = fingerprint(&bytes);
        ensure!(
            Some(&digest) == asset.local_sha256.as_ref() && asset.bytes == Some(bytes.len() as u64),
            "local candidate asset changed"
        );
        if extension == "svg"
            && let Some(reason) = svg_hazard(&bytes)
        {
            renderer
                .report
                .note_detail("asset_svg_not_copied", format!("{url}: {reason}"));
            continue;
        }
        let local = format!("/_mochi/assets/{digest}.{extension}");
        public.put(&format!("public{local}"), &bytes)?;
        if parsed.path() == "/favicon.ico" {
            public.put("public/favicon.ico", &bytes)?;
        }
        // Recovered code can assemble URLs at runtime (for example, an icon
        // slug within a template literal). Preserve authoritative passive paths
        // alongside hashed assets; never copy original scripts or API payloads.
        if renderer.report.client_recovery.is_some()
            && parsed.query().is_none()
            && parsed.origin() == source_origin
        {
            let alias = crate::mirror::output_path(&staging.path().join("public"), &parsed, false)?;
            if !alias.exists() {
                let relative = alias
                    .strip_prefix(staging.path())
                    .context("alias outside export")?
                    .to_string_lossy()
                    .into_owned();
                public.put(&relative, &bytes)?;
                renderer
                    .report
                    .note_detail("recovered_passive_asset_path_preserved", relative);
            }
        }
        asset_urls.insert(url.clone(), local);
        renderer.report.local_asset_urls += 1;
    }
    if let Some(recovery) = &mut renderer.report.client_recovery {
        let base = Url::parse(&model.routes.values().next().context("missing route")?.url)?;
        crate::recover::localize(recovery, &base, &asset_urls)?;
    }
    for (id, observation) in &model.observations {
        if renderer.report.client_recovery.as_ref().is_some_and(|r| {
            r.routes
                .contains_key(&model.routes[&observation.route].pathname)
        }) {
            // The recovered program owns these elements: no snapshot is made. Whether
            // it draws them is not verified here.
            for (&node, surface) in &observation.capture.surfaces {
                let tag = match &observation.capture.nodes[node].content {
                    Content::Element { tag, .. } => tag.clone(),
                    Content::Text { .. } => String::new(),
                };
                renderer.report.surfaces.push(SurfaceRecord {
                    route: observation.capture.url.clone(),
                    viewport: [
                        observation.capture.viewport.width,
                        observation.capture.viewport.height,
                    ],
                    source: SourceRef {
                        observation: id.clone(),
                        node,
                    },
                    tag,
                    kind: surface::kind_name(&surface.kind).into(),
                    replacement: "left_to_recovered_client_program".into(),
                    bounds: [surface.bounds.width, surface.bounds.height],
                    asset: None,
                });
            }
            continue;
        }
        for (&node, surface) in &observation.capture.surfaces {
            if let Some(image) = &surface.image {
                let bytes = read(&root, Path::new(&image.path), 64 * 1024 * 1024)?;
                ensure!(fingerprint(&bytes) == image.sha256, "surface image changed");
                let local = format!("/_mochi/assets/{}.png", image.sha256);
                public.put(&format!("public{local}"), &bytes)?;
                renderer.surface_images.insert((id.clone(), node), local);
            }
        }
    }
    renderer.asset_urls = asset_urls.clone();
    let route_urls: BTreeMap<_, _> = model
        .routes
        .values()
        .map(|r| {
            let url = Url::parse(&r.url)?;
            let mut path = url.path().to_string();
            if let Some(query) = url.query() {
                path.push('?');
                path.push_str(query);
            }
            Ok((r.url.clone(), path))
        })
        .collect::<Result<_>>()?;
    renderer.captured_routes = route_urls.keys().cloned().collect();
    files.add("src/lib/references.ts",format!("export const assets:Record<string,string>={}\nexport const capturedRoutes:Record<string,string>={}\n",js(&asset_urls)?,js(&route_urls)?))?;
    let mut layout_for = BTreeMap::new();
    let mut layout_sources = Vec::new();
    for layout in &model.layouts {
        let selected: Vec<_> = layout
            .occurrences
            .iter()
            .filter(|s| {
                primary.values().any(|o| o == &s.observation)
                    && !renderer.report.client_recovery.as_ref().is_some_and(|r| {
                        r.routes.contains_key(
                            &model.routes[&model.observations[&s.observation].route].pathname,
                        )
                    })
            })
            .collect();
        if selected.len() < 2 {
            continue;
        }
        let name = format!("Layout{}", layout_sources.len() + 1);
        let source = selected[0];
        let mut before = String::new();
        let mut after = String::new();
        for &node in &source.before {
            before.push_str(&renderer.node(
                &SourceRef {
                    observation: source.observation.clone(),
                    node,
                },
                &BTreeMap::new(),
                None,
            )?);
        }
        for &node in &source.after {
            after.push_str(&renderer.node(
                &SourceRef {
                    observation: source.observation.clone(),
                    node,
                },
                &BTreeMap::new(),
                None,
            )?);
        }
        for shell in selected {
            layout_for.insert(shell.observation.clone(), (name.clone(), shell.main));
        }
        layout_sources.push((name, before, after));
    }
    let mut groups: BTreeMap<String, Vec<(&String, &String)>> = BTreeMap::new();
    for (route, observation) in &primary {
        let path = model.routes[route].pathname.trim_end_matches('/');
        let group = groups
            .entry(if path.is_empty() {
                "/".into()
            } else {
                path.into()
            })
            .or_default();
        let signature = dom_signature(&model.observations[observation].capture)?;
        let same = model.routes[route].observations.iter().all(|other| {
            dom_signature(&model.observations[other].capture).is_ok_and(|s| s == signature)
        });
        if same
            && model.observations[observation].capture.surfaces.is_empty()
            && model.observations[observation]
                .capture
                .animation_states
                .is_empty()
        {
            group.push((route, observation));
        } else {
            for other in &model.routes[route].observations {
                group.push((route, other));
            }
        }
    }

    let mut page_sources = Vec::new();
    let mut stylesheet_paths = BTreeMap::new();
    let mut head_cache: BTreeMap<String, (Vec<Value>, Vec<Value>)> = BTreeMap::new();
    for (path, routes) in groups {
        let name = structure::page_name(&path);
        let feature = format!("features/pages{}", path.trim_end_matches('/'));
        let route_file = structure::route_file(&path);
        let route_id = format!("{}/", path.trim_end_matches('/'));
        let mut variants = Vec::new();
        let mut responsive: BTreeMap<String, Vec<(u32, u32, String)>> = BTreeMap::new();
        // Per route key: every captured viewport's document state and head.
        let mut documents: Vec<DocumentCapture> = Vec::new();
        // Stylesheet identifier per compat CSS file, and who imports it.
        let mut css_idents: BTreeMap<String, String> = BTreeMap::new();
        let mut data_css = BTreeSet::new();
        let mut page_css = BTreeSet::new();
        for (route_id, observation) in routes.iter() {
            let capture = &model.observations[*observation].capture;
            let base = Url::parse(&capture.metadata.base_url)?;
            let (html, body) = document_nodes(capture)?;
            let recovering = renderer
                .report
                .client_recovery
                .as_ref()
                .is_some_and(|r| r.routes.contains_key(&model.routes[*route_id].pathname));
            let mut stylesheet = String::new();
            for sheet in &capture.stylesheets {
                if sheet.disabled {
                    continue;
                }
                let base = Url::parse(&sheet.base_url)?;
                let mut contents = String::new();
                let original = if recovering {
                    sheet
                        .href
                        .as_ref()
                        .and_then(|url| model.assets.get(url))
                        .filter(|asset| asset.status == "local_candidate")
                } else {
                    None
                };
                if let Some(asset) = original {
                    let bytes = read(
                        &root,
                        Path::new(
                            asset
                                .local_path
                                .as_ref()
                                .context("stylesheet path missing")?,
                        ),
                        16 * 1024 * 1024,
                    )?;
                    ensure!(
                        Some(&fingerprint(&bytes)) == asset.local_sha256.as_ref(),
                        "source stylesheet changed"
                    );
                    let text = String::from_utf8(bytes)?;
                    contents.push_str(&css(&text, &base, &asset_urls, &mut renderer.report)?);
                } else {
                    for rule in &sheet.rules {
                        contents.push_str(&css(rule, &base, &asset_urls, &mut renderer.report)?);
                        contents.push('\n');
                    }
                }
                if sheet.media.is_empty() {
                    stylesheet.push_str(&contents);
                } else {
                    stylesheet.push_str(&format!("@media {}{{{contents}}}\n", sheet.media));
                }
            }
            // Check inline URL grammar up front; the shared runtime resolves it against each page's base.
            for node in &capture.nodes {
                if let Content::Element { attributes, .. } = &node.content {
                    for attr in attributes.iter().filter(|a| a.name == "style") {
                        css(&attr.value, &base, &asset_urls, &mut renderer.report)?;
                    }
                }
            }
            let responsive_route = routes.iter().filter(|(r, _)| r == route_id).count() > 1;
            let css_id = if responsive_route {
                format!(
                    "{route_id}-{}x{}",
                    capture.viewport.width, capture.viewport.height
                )
            } else {
                (*route_id).clone()
            };
            let css_id = stylesheet_paths
                .entry(stylesheet.clone())
                .or_insert_with(|| css_id.clone())
                .clone();
            let css_file = format!("src/styles/compat/{css_id}.css");
            if !files.text.contains_key(&css_file) {
                files.add(css_file, stylesheet)?;
            }
            let next_ident = css_idents.len();
            let ident = css_idents
                .entry(css_id.clone())
                .or_insert_with(|| format!("css{next_ident}"))
                .clone();
            let recovered = renderer
                .report
                .client_recovery
                .as_ref()
                .and_then(|r| r.routes.get(&model.routes[*route_id].pathname));
            let mut content = String::new();
            if let Some(recovered) = recovered {
                content.push_str(&format!("<{} />", recovered.component));
            } else if let Some((layout, main)) = layout_for.get(*observation) {
                content.push_str(&format!("<{layout}>"));
                content.push_str(&renderer.node(
                    &SourceRef {
                        observation: (*observation).clone(),
                        node: *main,
                    },
                    &BTreeMap::new(),
                    None,
                )?);
                content.push_str(&format!("</{layout}>"));
            } else {
                for &node in &renderer.trees[*observation][body].clone() {
                    content.push_str(&renderer.node(
                        &SourceRef {
                            observation: (*observation).clone(),
                            node,
                        },
                        &BTreeMap::new(),
                        None,
                    )?);
                }
            }
            let key = route_urls[&model.routes[*route_id].url].clone();
            if responsive_route {
                page_css.insert(css_id.clone());
                responsive.entry(key.clone()).or_default().push((capture.viewport.width,capture.viewport.height,format!("<SourceContext value={{page.base}}><style>{{capturedCss({ident}+tokensCss)}}</style>{content}</SourceContext>")));
            } else {
                variants.push(format!(
                    "case {}:return <SourceContext value={{page.base}}>{content}</SourceContext>",
                    js(&key)?
                ));
                data_css.insert(css_id.clone());
            }
            let mut meta = Vec::new();
            for m in
                capture.metadata.meta.iter().filter(|m| {
                    m.http_equiv.is_none() && (m.name.is_some() || m.property.is_some())
                })
            {
                if framework_meta(m) {
                    renderer.report.note_detail(
                        "framework_meta_omitted",
                        m.name.as_deref().or(m.property.as_deref()).unwrap_or(""),
                    );
                    continue;
                }
                let mut v = serde_json::Map::new();
                if let Some(name) = &m.name {
                    v.insert("name".into(), json!(name));
                }
                if let Some(property) = &m.property {
                    v.insert("property".into(), json!(property));
                }
                v.insert("content".into(), json!(&m.content));
                meta.push(Value::Object(v));
            }
            let page_url = Url::parse(&capture.url)?;
            let mut routed = page_url.clone();
            routed.set_query(None);
            if page_url.query().is_some() {
                // The mirror stores one HTML response per path, whatever its query.
                renderer
                    .report
                    .note_detail("head_source_ignores_query", page_url.as_str());
            }
            let (icons, scripts) = match head_cache.get(routed.as_str()) {
                Some(cached) => cached.clone(),
                None => {
                    let resolved = route_head(
                        &root,
                        &page_url,
                        &base,
                        &asset_urls,
                        &mut public,
                        &mut renderer.report,
                    )?;
                    head_cache.insert(routed.as_str().to_owned(), resolved.clone());
                    resolved
                }
            };
            let mut links = Vec::new();
            // The capture keeps the attribute as written; a relative canonical
            // named the original page's origin, so it is emitted resolved.
            if let Some(canonical) = &capture.metadata.canonical {
                match base.join(canonical.trim()) {
                    Ok(url) if matches!(url.scheme(), "http" | "https") => {
                        links.push(json!({"rel":"canonical","href":url.as_str()}));
                    }
                    _ => renderer.report.note_detail(
                        "head_entry_not_exported",
                        format!("{page_url}: canonical {canonical}"),
                    ),
                }
            }
            links.extend(icons);
            let mut html_attrs = attrs(capture, html);
            html_attrs
                .entry("lang".into())
                .or_insert_with(|| capture.metadata.lang.clone());
            let body_attrs = attrs(capture, body);
            documents.push(DocumentCapture {
                key,
                viewport: (capture.viewport.width, capture.viewport.height),
                nodes: (
                    SourceRef {
                        observation: (*observation).clone(),
                        node: html,
                    },
                    SourceRef {
                        observation: (*observation).clone(),
                        node: body,
                    },
                ),
                css_id,
                html: html_attrs,
                body: body_attrs,
                head: json!({"title":capture.metadata.title,"meta":meta,"base":capture.metadata.base_url,"links":links,"scripts":scripts}),
            });
        }
        // One entry per route key. Head data comes from the largest capture; the
        // document attributes of every captured viewport are kept when they differ.
        let mut entries = Vec::new();
        let mut keys = Vec::new();
        for document in &documents {
            if !keys.contains(&document.key) {
                keys.push(document.key.clone());
            }
        }
        for key in keys {
            let mut group: Vec<&DocumentCapture> =
                documents.iter().filter(|d| d.key == key).collect();
            group.sort_by_key(|d| d.viewport);
            let primary = group[group.len() - 1];
            if group.iter().any(|d| d.head != primary.head) {
                renderer
                    .report
                    .note_detail("head_differs_between_viewports", key.clone());
            }
            // Counted per emitted route entry, not per capture or per cached page.
            for link in primary.head["links"].as_array().into_iter().flatten() {
                let Some(href) = link["href"].as_str() else {
                    continue;
                };
                if link["rel"] == "canonical" {
                    renderer.report.note_detail("head_canonical_emitted", href);
                    renderer.report.reference("head_meta", href, None);
                } else if href.starts_with("http://") || href.starts_with("https://") {
                    renderer.report.reference("head_icon", href, None);
                }
            }
            // Absolute URLs in emitted meta (og:image, og:video, twitter:image, og:url...).
            for entry in primary.head["meta"].as_array().into_iter().flatten() {
                if let Some(content) = entry["content"].as_str()
                    && (content.starts_with("http://") || content.starts_with("https://"))
                    && let Ok(url) = Url::parse(content)
                {
                    renderer.report.reference("head_meta", url.as_str(), None);
                }
            }
            let mut value = primary.head.clone();
            let object = value.as_object_mut().context("head is an object")?;
            object.insert("key".into(), json!(key));
            object.insert(
                "document".into(),
                json!({"html":primary.html,"body":primary.body}),
            );
            let differs = group
                .iter()
                .any(|d| d.html != primary.html || d.body != primary.body);
            // <html>/<body> attributes go through the same URL resolution as any
            // element (`<body background>`), so their URLs are references too.
            let emitted = if differs {
                &group[..]
            } else {
                &group[group.len() - 1..]
            };
            for document in emitted {
                for (tag, source, attributes) in [
                    ("html", &document.nodes.0, &document.html),
                    ("body", &document.nodes.1, &document.body),
                ] {
                    for (name, value) in attributes {
                        renderer.track_reference(source, tag, name, value);
                    }
                }
            }
            if differs {
                object.insert(
                    "documents".into(),
                    Value::Array(group.iter().map(|d| json!({"width":d.viewport.0,"height":d.viewport.1,"html":d.html,"body":d.body})).collect()),
                );
                renderer.report.note_detail(
                    "document_attributes_differ_by_viewport",
                    format!(
                        "{key}: {}",
                        group
                            .iter()
                            .map(|d| format!("{}x{}", d.viewport.0, d.viewport.1))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }
            let css = if responsive.contains_key(&key) {
                "''".to_owned()
            } else {
                css_idents[&primary.css_id].clone()
            };
            entries.push(format!("{}:{{...{},css:{css}}}", js(&key)?, js(&value)?));
        }
        let entries = entries.join(",");
        let css_import = |ids: &BTreeSet<String>| {
            ids.iter()
                .map(|id| {
                    format!(
                        "import {} from '@/styles/compat/{id}.css?inline'\n",
                        css_idents[id]
                    )
                })
                .collect::<String>()
        };
        let mut imports = css_import(&page_css);
        let recovered_components = routes
            .iter()
            .filter_map(|(route_id, _)| {
                renderer
                    .report
                    .client_recovery
                    .as_ref()?
                    .routes
                    .get(&model.routes[*route_id].pathname)
            })
            .map(|r| r.component.as_str())
            .collect::<BTreeSet<_>>();
        for component in recovered_components {
            imports.push_str(&format!(
                "import {{{0}}} from '@/components/{0}'\n",
                component
            ));
        }
        for (key, mut captures) in responsive {
            captures.sort_by_key(|(width, height, _)| (*width, *height));
            let widths = captures.iter().map(|(w, _, _)| *w).collect::<Vec<_>>();
            let heights = captures.iter().map(|(_, h, _)| *h).collect::<Vec<_>>();
            ensure!(
                captures
                    .windows(2)
                    .all(|w| (w[0].0, w[0].1) != (w[1].0, w[1].1)),
                "duplicate responsive viewport observations"
            );
            // Viewports whose generated output is byte-identical need no switching:
            // one element serves them all (behaviour is unchanged, the source is not doubled).
            if captures
                .iter()
                .all(|(_, _, element)| *element == captures[0].2)
            {
                renderer.report.note_detail(
                    "identical_viewport_variants_merged",
                    format!(
                        "{key}: {}",
                        captures
                            .iter()
                            .map(|(w, h, _)| format!("{w}x{h}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
                variants.push(format!("case {}:return {}", js(&key)?, captures[0].2));
                continue;
            }
            let elements = captures
                .iter()
                .map(|(_, _, element)| element.as_str())
                .collect::<Vec<_>>()
                .join(",");
            variants.push(format!(
                "case {}:return <ResponsiveCapture widths={{{}}} heights={{{}}} children={{[{elements}]}} />",
                js(&key)?,
                js(&widths)?,js(&heights)?
            ));
        }
        files.add(format!("src/{feature}/data.ts"), format!("import type {{DocumentPage}} from '@/lib/document'\n{}export const pages:Record<string,DocumentPage>={{{entries}}}\n", css_import(&data_css)))?;
        // Physical index routes preserve literal URL paths without inventing
        // parent layouts. Feature pages own markup; routes only wire the router.
        files.add(format!("src/routes/{route_file}"), format!("import {{createFileRoute}} from '@tanstack/react-router'\nimport {{selectPage,queryKey}} from '@/lib/runtime'\nimport {{documentHead}} from '@/lib/document'\nimport {{pages}} from '@/{feature}/data'\nimport {{{name}}} from '@/{feature}/page'\nexport const Route=createFileRoute({})({{loader:({{location}})=>selectPage(pages,location.pathname,location.searchStr),loaderDeps:({{search}})=>queryKey(search),shouldReload:true,head:({{loaderData}})=>documentHead(loaderData),component:RouteComponent}})\nfunction RouteComponent(){{return <{name} page={{Route.useLoaderData()}} />}}\n",js(&route_id)?))?;
        page_sources.push((format!("src/{feature}/page.tsx"),format!("{helpers}{imports}export function {name}({{page}}:{{page:PageData}}){{const base=page.base;switch(page.key){{{}}}return null}}\n",variants.join("\n"))));
    }
    // Candidate source is emitted as JSX, not a runtime interpreter of DOM JSON.
    // Components are named and de-duplicated now that all uses are known.
    let mut bodies: Vec<(String, String)> = Vec::new();
    let mut canonical: BTreeMap<String, String> = BTreeMap::new();
    let mut by_body: BTreeMap<String, String> = BTreeMap::new();
    let mut done = BTreeSet::new();
    loop {
        let mut pending: Vec<String> = renderer.used.difference(&done).cloned().collect();
        if pending.is_empty() {
            break;
        }
        pending.sort_by_key(|id| {
            let provisional = &model.components[id].name;
            (provisional.len(), provisional.clone())
        });
        for id in pending {
            let body = renderer.component(&id, buttons, NAME_TOKEN)?;
            if let Some(first) = by_body.get(&body) {
                renderer.report.note_detail(
                    "component_deduplicated",
                    format!(
                        "{} is identical to {}",
                        model.components[&id].name, model.components[first].name
                    ),
                );
                canonical.insert(id.clone(), first.clone());
            } else {
                by_body.insert(body.clone(), id.clone());
                canonical.insert(id.clone(), id.clone());
                bodies.push((id.clone(), body));
            }
            done.insert(id);
        }
    }
    let mut taken: BTreeSet<String> = RESERVED_NAMES.iter().map(|n| n.to_lowercase()).collect();
    for index in 1..=layout_sources.len() {
        taken.insert(format!("layout{index}"));
    }
    taken.extend(
        page_sources
            .iter()
            .filter_map(|(_, source)| source.split("export function ").nth(1))
            .filter_map(|rest| rest.split('(').next())
            .map(|n| n.to_lowercase()),
    );
    // Recovered client entry components share src/components/ and the page imports.
    if let Some(recovery) = &renderer.report.client_recovery {
        taken.extend(recovery.routes.values().map(|r| r.component.to_lowercase()));
    }
    let mut final_names: BTreeMap<String, String> = BTreeMap::new();
    for (id, _) in &bodies {
        let provisional = model.components[id].name.clone();
        let authored = authored_component_name(&model, id);
        let want = authored
            .as_ref()
            .map_or_else(|| provisional.clone(), |(name, _)| name.clone());
        let mut candidate = want.clone();
        let mut suffix = 2;
        while !taken.insert(candidate.to_lowercase()) {
            candidate = format!("{want}{suffix}");
            suffix += 1;
        }
        if candidate != provisional {
            let (code, origin) = match &authored {
                Some((_, origin)) => ("component_named_from_markup", format!(" ({origin})")),
                None => ("component_renamed_to_avoid_collision", String::new()),
            };
            renderer
                .report
                .note_detail(code, format!("{provisional} -> {candidate}{origin}"));
        }
        final_names.insert(id.clone(), candidate);
    }
    let tokens: BTreeMap<String, String> = canonical
        .iter()
        .map(|(id, first)| (component_token(id), final_names[first].clone()))
        .collect();
    // One pass per source: `\0<id>\0` placeholders are looked up, not searched for per component.
    static PLACEHOLDER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new("\u{0}[^\u{0}]*\u{0}").expect("placeholder"));
    let fix = |source: &str| -> Result<String> {
        let mut missing = false;
        let out = PLACEHOLDER.replace_all(source, |found: &regex::Captures| {
            tokens.get(&found[0]).cloned().unwrap_or_else(|| {
                missing = true;
                String::new()
            })
        });
        // JSON escaping keeps NUL out of captured text, so a raw NUL is a placeholder left unresolved.
        ensure!(
            !missing && !out.contains('\0'),
            "unresolved component placeholder"
        );
        Ok(out.into_owned())
    };
    let mut used: Vec<String> = final_names.values().cloned().collect();
    used.sort();
    for (id, body) in &bodies {
        let name = &final_names[id];
        let mut source = fix(&body.replace(NAME_TOKEN, name))?;
        if let Some(annotation) = renderer.report.interpretation.as_ref()
            && let Some(decisions) = annotation.components.get(&model.components[id].name)
        {
            source = format!(
                "// Mochi semantic proposals: review required, calibration unqualified, not applied. Origins: {}\n// {}\n{source}",
                js(&annotation.provider_origins)?,
                js(decisions)?
            );
        }
        files.add(format!("src/components/{name}.tsx"), source)?;
        renderer
            .report
            .component_sources
            .insert(name.clone(), model.components[id].prototype.clone());
    }
    let component_imports = used
        .iter()
        .map(|n| format!("import {{{n}}} from '@/components/{n}'\n"))
        .collect::<String>();
    for (name, before, after) in &layout_sources {
        files.add(format!("src/layouts/{name}.tsx"),format!("import type {{ReactNode}} from 'react'\n{helpers}{component_imports}export function {name}({{children}}:{{children:ReactNode}}){{const base=useSource();return <>{}{{children}}{}</>}}\n",fix(before)?,fix(after)?))?;
    }
    let layout_imports = layout_sources
        .iter()
        .map(|(n, _, _)| format!("import {{{n}}} from '@/layouts/{n}'\n"))
        .collect::<String>();
    for (path, source) in page_sources {
        files.add(
            path,
            format!("{component_imports}{layout_imports}{}", fix(&source)?),
        )?;
    }
    renderer.report.captured_paths = route_urls.values().cloned().collect();
    renderer.report.components = used.len();
    renderer.report.layouts = layout_sources.len();
    let mut finished = BTreeSet::new();
    if let Some(recovery) = &renderer.report.client_recovery {
        for (path, source) in &recovery.files {
            // Hash the bytes that will be written, not the pre-format text.
            let text = finish_source(path, source)?;
            renderer
                .report
                .recovered_files
                .insert(path.clone(), fingerprint(text.as_bytes()));
            files.add(path.clone(), text)?;
            finished.insert(path.clone());
        }
    }
    renderer.report.local_assets = public.written.len();
    renderer.report.external_references = std::mem::take(&mut renderer.report.ledger)
        .into_values()
        .collect();
    files.add(
        ".mochi/export.json",
        serde_json::to_string_pretty(&renderer.report)?,
    )?;
    files.add("README.md", structure::readme(&renderer.report)?)?;
    files.add(
        "src/styles/tokens.css",
        include_str!("export_template/src/styles/tokens.css").into(),
    )?;
    files.add(".mochi/style-audit.json", structure::style_audit(&model)?)?;
    files.add("EDITING.md", structure::editing_guide(&renderer.report)?)?;
    let mut formatted_bytes = 0usize;
    for (path, text) in files.text {
        let text = if finished.contains(&path) {
            text
        } else {
            finish_source(&path, &text)?
        };
        formatted_bytes = formatted_bytes
            .checked_add(text.len())
            .context("formatted source size overflow")?;
        ensure!(
            formatted_bytes <= TEXT_BYTES,
            "formatted source byte budget exceeded"
        );
        util::write(staging.path(), &staging.path().join(path), text.as_bytes())?;
    }
    ensure!(
        read(&root, Path::new("reconstruction/project.json"), MODEL_BYTES)? == bytes,
        "saved model changed during export"
    );
    ensure!(
        project::derive(&root)?.input_sha256 == model.input_sha256,
        "source or local assets changed during export"
    );
    for (path, digest) in &renderer.report.head_inputs {
        ensure!(
            &fingerprint(&read(&root, Path::new(path), 64 * 1024 * 1024)?) == digest,
            "mirrored head input changed during export: {path}"
        );
    }
    if let Some(annotation) = &renderer.report.interpretation {
        ensure!(
            crate::interpret::annotations(&root, &model)?.report_sha256 == annotation.report_sha256,
            "saved interpretation changed during export"
        );
    }
    if let Some(recovery) = &renderer.report.client_recovery {
        crate::recover::verify(&root, recovery)?;
    }
    publish(staging.path(), &destination)?;
    Ok(renderer.report)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn publish(staging: &Path, destination: &Path) -> Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        staging,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .context("publish export without overwriting an existing path")?;
    Ok(())
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn publish(_staging: &Path, _destination: &Path) -> Result<()> {
    anyhow::bail!("atomic source export is currently implemented only on macOS and Linux")
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    #[test]
    fn svg_files_that_would_run_or_load_from_the_app_origin_are_refused() {
        for (svg, reason) in [
            ("<svg><script>alert(1)</script></svg>", "script element"),
            ("<svg onload=\"x()\"></svg>", "event handler attribute"),
            ("<svg><a href=\"javascript:x()\"/></svg>", "javascript: URL"),
            ("<svg><foreignObject/></svg>", "foreignObject"),
            (
                "<svg><set attributeName=\"href\" to=\"x\"/></svg>",
                "script-targeting animation",
            ),
            (
                "<svg><image href=\"https://x.test/a.png\"/></svg>",
                "external reference",
            ),
            (
                "<svg><style>@import url(a.css)</style></svg>",
                "external reference",
            ),
            (
                "<svg><image href=\"data:text/html,x\"/></svg>",
                "non-raster data: reference",
            ),
        ] {
            assert_eq!(super::svg_hazard(svg.as_bytes()), Some(reason), "{svg}");
        }
        // Spellings a browser decodes into the same hazards.
        for (svg, reason) in [
            (
                "<svg xmlns:s=\"http://www.w3.org/2000/svg\"><s:script>alert(1)</s:script></svg>",
                "script element",
            ),
            (
                "<svg><h:script xmlns:h=\"http://www.w3.org/1999/xhtml\">x()</h:script></svg>",
                "script element",
            ),
            (
                "<svg><h:iframe xmlns:h=\"http://www.w3.org/1999/xhtml\" src=\"/x\"/></svg>",
                "XHTML content",
            ),
            (
                "<svg><a href=\"&#x6a;avascript:alert(1)\"><rect/></a></svg>",
                "javascript: URL",
            ),
            (
                "<svg><a href=\"java&#9;script:alert(1)\"><rect/></a></svg>",
                "javascript: URL",
            ),
            (
                "<svg><a href=\"&#106;&#97;vascript&#58;x()\"><rect/></a></svg>",
                "javascript: URL",
            ),
            (
                "<!DOCTYPE svg [<!ENTITY x \"&#60;script&#62;x()&#60;/script&#62;\">]><svg>&x;</svg>",
                "script element",
            ),
            (
                "<!DOCTYPE svg [<!ATTLIST svg onload CDATA \"x()\">]><svg/>",
                "DTD internal subset",
            ),
            (
                "<?xml version=\"1.0\" encoding=\"ISO-2022-JP\"?><svg><scr\x1b(Bipt>x()</scr\x1b(Bipt></svg>",
                "non-UTF-8 text",
            ),
            (
                "<?xml-stylesheet type=\"text/xsl\" href=\"/_mochi/assets/a.svg\"?><svg/>",
                "xml-stylesheet processing instruction",
            ),
            (
                "<svg><a><set attributeName=\"l:href\" to=\"x\"/></a></svg>",
                "script-targeting animation",
            ),
            (
                "<svg><set attributeName=\"&#104;ref\" to=\"x\"/></svg>",
                "script-targeting animation",
            ),
            (
                "<svg><image href=\"&#104;ttps://x.test/a.png\"/></svg>",
                "external reference",
            ),
            (
                "<svg><image href=\"\\\\x.test/a.png\"/></svg>",
                "external reference",
            ),
            (
                "<svg><image href=\"http:x.test/a.png\"/></svg>",
                "external reference",
            ),
            (
                "<svg><image xml:base=\"https://x.test/\" href=\"a.png\"/></svg>",
                "external reference",
            ),
            (
                "<svg><style>@\\69mport \"https://x.test/a.css\";</style></svg>",
                "CSS escape",
            ),
            (
                "<svg><rect fill=\"\\75rl(https://x.test/a)\"/></svg>",
                "CSS escape",
            ),
            (
                "<svg><rect style=\"fill:\\u\\r\\l(x.test)\"/></svg>",
                "CSS escape",
            ),
            (
                "<svg><style>rect{fill:image-set(\"https://x.test/a.png\" 1x)}</style></svg>",
                "external reference",
            ),
            (
                "<svg><image href=\"data:,%3Cscript%3E\"/></svg>",
                "non-raster data: reference",
            ),
        ] {
            assert_eq!(super::svg_hazard(svg.as_bytes()), Some(reason), "{svg}");
        }
        assert_eq!(
            super::svg_hazard(b"<svg>\xff\xfe</svg>"),
            Some("non-UTF-8 text")
        );
        for plain in [
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1 1\"><path d=\"M0 0\" fill=\"url(#g)\" offset=\"1\"/><animate attributeName=\"opacity\"/></svg>",
            "<svg><image href=\"data:image/png;base64,AA\"/></svg>",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\"><svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><defs><linearGradient id=\"g\"/></defs><use xlink:href=\"#a\" style=\"fill:url(#g)\"/><text>R&amp;D &#169;</text></svg>",
        ] {
            assert_eq!(super::svg_hazard(plain.as_bytes()), None, "{plain}");
        }
    }
    #[test]
    fn srcset_candidates_follow_the_runtime_tokenizer() {
        assert_eq!(
            super::srcset_urls("/s.png 100w 50h, /t.png .5x,/u.png, /bad.png 1x 2x, /h.png 50h"),
            ["/s.png", "/t.png", "/u.png"]
        );
        assert_eq!(
            super::srcset_urls("a.png 1x((,), http://x/b.png, data:image/png;base64,AA 2x"),
            ["http://x/b.png", "data:image/png;base64,AA"]
        );
        assert!(super::srcset_urls(" , ,").is_empty());
    }
    #[test]
    fn atomic_publish_refuses_even_an_empty_existing_directory() {
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join("staging");
        let destination = root.path().join("owned");
        std::fs::create_dir(&staging).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(staging.join("generated"), b"new").unwrap();
        assert!(super::publish(&staging, &destination).is_err());
        assert!(std::fs::read_dir(&destination).unwrap().next().is_none());
        assert_eq!(std::fs::read(staging.join("generated")).unwrap(), b"new");
    }
}
