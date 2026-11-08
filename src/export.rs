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
];
static CSS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)url\(\s*(?:"([^"\\]*)"|'([^'\\]*)'|([^\s()'"\\]*))\s*\)"#)
        .expect("CSS URL expression")
});
static CSS_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)url\s*\(").expect("CSS URL start"));
static ROUTE_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/(?:[A-Za-z0-9_.-]+/?)*$").expect("literal route path"));

#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub count: usize,
    pub examples: Vec<SourceRef>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub framework: String,
    pub input_sha256: String,
    pub routes: usize,
    pub captured_paths: Vec<String>,
    pub component_sources: BTreeMap<String, SourceRef>,
    pub components: usize,
    pub layouts: usize,
    pub local_assets: usize,
    pub diagnostics: BTreeMap<String, Diagnostic>,
    pub limitations: Vec<String>,
    pub verification: String,
    pub source_quality: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_recovery: Option<crate::recover::Recovery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<crate::interpret::Annotations>,
}
impl Report {
    fn note(&mut self, code: &str, source: Option<SourceRef>) {
        let entry = self.diagnostics.entry(code.into()).or_insert(Diagnostic {
            count: 0,
            examples: Vec::new(),
        });
        entry.count += 1;
        if entry.examples.len() < 8
            && let Some(source) = source
        {
            entry.examples.push(source);
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
                report.note("embedded_font_data_preserved", None);
                raw.into()
            } else if matches!(url.scheme(), "http" | "https") {
                report.note("external_or_unmapped_css_url", None);
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
}
impl Renderer<'_> {
    fn node(
        &mut self,
        source: &SourceRef,
        bindings: &BTreeMap<usize, Vec<(&str, &Target)>>,
        skip: Option<&SourceRef>,
    ) -> Result<String> {
        if let Some(jsx) = self.surface(source)? {
            return Ok(jsx);
        }
        if skip != Some(source)
            && let Some(&(component, instance)) = self.instances.get(source)
        {
            let component = &self.model.components[component];
            self.used.insert(component.name.clone());
            let mut out = format!("<{}", component.name);
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
                    self.report.note(
                        "omitted_executable_or_embedded_element",
                        Some(source.clone()),
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
                            "action" | "formaction" | "srcdoc" | "is" | "autoplay"
                        )
                    {
                        self.report.note(
                            "omitted_action_or_executable_attribute",
                            Some(source.clone()),
                        );
                        continue;
                    }
                    if matches!(attr.name.as_str(), "srcset" | "imagesrcset") {
                        self.report
                            .note("srcset_not_reconstructed", Some(source.clone()));
                        continue;
                    }
                    if attr.name == "style" {
                        self.report
                            .note("inline_style_requires_review", Some(source.clone()));
                    }
                    let binding=bindings.get(&source.node).and_then(|b|b.iter().find(|(_,t)|matches!(t,Target::Attribute{name,namespace} if name==&attr.name && namespace==&attr.namespace)));
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
    fn component(&mut self, id: &str, buttons: bool) -> Result<String> {
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
            "{helpers}export function {}(props:{{{}}}){{const base=useSource();return ({source})}}\n",
            component.name,
            props.join(";")
        ))
    }
}

/// Generate a new TanStack Start source project from a saved, unchanged model.
/// This never installs packages, calls a model, executes captured scripts, or
/// overwrites an existing project. Runtime verification is a separate operation.
pub fn tanstack_start(root: &Path, out: &Path) -> Result<Report> {
    generate(root, out, false, &BTreeSet::new())
}
/// Attach an explicitly selected saved interpretation as review-required source
/// comments/report metadata. This never calls a provider or auto-applies labels.
pub fn tanstack_start_with_interpretation(root: &Path, out: &Path) -> Result<Report> {
    generate(root, out, true, &BTreeSet::new())
}
/// Recover selected public client roots as editable modules. This is explicit:
/// the generated app executes recovered client code, unlike initial-state export.
pub fn tanstack_start_with_clients(
    root: &Path,
    out: &Path,
    interpretation: bool,
    routes: &BTreeSet<String>,
) -> Result<Report> {
    generate(root, out, interpretation, routes)
}
fn generate(
    root: &Path,
    out: &Path,
    attach_interpretation: bool,
    client_routes: &BTreeSet<String>,
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
    let report=Report{schema_version:1,framework:"tanstack-start".into(),input_sha256:model.input_sha256.clone(),routes:model.routes.len(),captured_paths:Vec::new(),component_sources:BTreeMap::new(),components:0,layouts:0,local_assets:0,diagnostics:BTreeMap::new(),interpretation,
        client_recovery:if client_routes.is_empty(){None}else{Some(crate::recover::plan(&root,&model,client_routes)?)},
        limitations:vec!["Observed DOM, shadow/frame content and rendered surface states; behavior and visual fidelity require verification of this generated project.".into(),
            "Compatibility CSS preserves global selectors and original cascade per document. Semantic tokens and component-local selector extraction are not reconstructed.".into(),
            "Literal routes only. Query strings are matched exactly; uncaptured query states return not found. No dynamic parameters or route nesting are inferred.".into(),
            "On initial-state routes, viewport-specific DOM/CSSOM selects the nearest capture. Recovered client routes run their own responsive logic against the actual viewport.".into(),
            "On initial-state routes, Base UI wraps native buttons and canvas/WebGL scenes use captured image assets. Explicitly recovered client routes preserve their own controls, drawing and event code. Backend services are not recreated.".into(),
            "CSS imports, escaped/nested CSS URLs, multi-select and unsupported route/tag syntax are refused.".into(),
            "Only passive local image/font candidates are copied. Query-bearing mirror assets remain unresolved; remote resources may still be needed.".into()],verification:"not_run".into(),source_quality:"compatibility_reconstruction_semantic_refactor_required".into()};
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
    let mut asset_bytes = 0;
    let source_origin =
        Url::parse(&model.routes.values().next().context("missing route")?.url)?.origin();
    for (url, asset) in &model.assets {
        if asset.status != "local_candidate" {
            renderer
                .report
                .note(&format!("asset_{}", asset.status), None);
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
                .note("asset_not_passive_image_or_font", None);
            continue;
        }
        let source = asset.local_path.as_ref().context("asset path missing")?;
        let bytes = read(&root, Path::new(source), 64 * 1024 * 1024)?;
        let digest = fingerprint(&bytes);
        ensure!(
            Some(&digest) == asset.local_sha256.as_ref() && asset.bytes == Some(bytes.len() as u64),
            "local candidate asset changed"
        );
        asset_bytes += bytes.len();
        ensure!(
            asset_bytes <= ASSET_BYTES,
            "export asset byte budget exceeded"
        );
        let public = format!("/_mochi/assets/{digest}.{extension}");
        util::write(
            staging.path(),
            &staging.path().join(format!("public{public}")),
            &bytes,
        )?;
        if parsed.path() == "/favicon.ico" {
            asset_bytes += bytes.len();
            ensure!(
                asset_bytes <= ASSET_BYTES,
                "export asset byte budget exceeded"
            );
            util::write(
                staging.path(),
                &staging.path().join("public/favicon.ico"),
                &bytes,
            )?;
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
                asset_bytes += bytes.len();
                ensure!(
                    asset_bytes <= ASSET_BYTES,
                    "recovered asset alias budget exceeded"
                );
                util::write(staging.path(), &alias, &bytes)?;
                renderer
                    .report
                    .note("recovered_passive_asset_path_preserved", None);
            }
        }
        asset_urls.insert(url.clone(), public);
        renderer.report.local_assets += 1;
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
            continue;
        }
        for (&node, surface) in &observation.capture.surfaces {
            if let Some(image) = &surface.image {
                let bytes = read(&root, Path::new(&image.path), 64 * 1024 * 1024)?;
                ensure!(fingerprint(&bytes) == image.sha256, "surface image changed");
                asset_bytes += bytes.len();
                ensure!(asset_bytes <= ASSET_BYTES, "surface asset budget exceeded");
                let public = format!("/_mochi/assets/{}.png", image.sha256);
                util::write(
                    staging.path(),
                    &staging.path().join(format!("public{public}")),
                    &bytes,
                )?;
                renderer.surface_images.insert((id.clone(), node), public);
                renderer.report.local_assets += 1;
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
    for (path, routes) in groups {
        let name = structure::page_name(&path);
        let feature = format!("features/pages{}", path.trim_end_matches('/'));
        let route_file = structure::route_file(&path);
        let route_id = format!("{}/", path.trim_end_matches('/'));
        let mut variants = Vec::new();
        let mut responsive: BTreeMap<String, Vec<(u32, u32, String)>> = BTreeMap::new();
        let mut data = Vec::new();
        for (index, (route_id, observation)) in routes.iter().enumerate() {
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
                responsive.entry(key).or_default().push((capture.viewport.width,capture.viewport.height,format!("<SourceContext value={{page.base}}><style>{{capturedCss(css{index}+tokensCss)}}</style>{content}</SourceContext>")));
            } else {
                variants.push(format!(
                    "case {}:return <SourceContext value={{page.base}}>{content}</SourceContext>",
                    js(&key)?
                ));
            }
            let meta: Vec<_> = capture
                .metadata
                .meta
                .iter()
                .filter(|m| m.http_equiv.is_none() && (m.name.is_some() || m.property.is_some()))
                .map(|m| {
                    let mut v = serde_json::Map::new();
                    if let Some(name) = &m.name {
                        v.insert("name".into(), json!(name));
                    }
                    if let Some(property) = &m.property {
                        v.insert("property".into(), json!(property));
                    }
                    v.insert("content".into(), json!(&m.content));
                    Value::Object(v)
                })
                .collect();
            let mut html_attrs = attrs(capture, html);
            html_attrs
                .entry("lang".into())
                .or_insert_with(|| capture.metadata.lang.clone());
            let document = json!({"key":route_urls[&model.routes[*route_id].url],"document":{"html":html_attrs,"body":attrs(capture,body)},"title":capture.metadata.title,"meta":meta,"base":capture.metadata.base_url});
            data.push((
                format!("import css{index} from '@/styles/compat/{css_id}.css?inline'\n"),
                format!(
                    "{}:{{...{},css:{}}}",
                    js(&route_urls[&model.routes[*route_id].url])?,
                    js(&document)?,
                    if responsive_route {
                        "''".to_owned()
                    } else {
                        format!("css{index}")
                    }
                ),
            ));
        }
        let css_imports = data.iter().map(|d| d.0.as_str()).collect::<String>();
        let mut imports = css_imports.clone();
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
        let mut seen_keys = BTreeSet::new();
        let entries = data
            .iter()
            .zip(&routes)
            .filter_map(|(d, (route, _))| {
                seen_keys.insert((*route).clone()).then_some(d.1.as_str())
            })
            .collect::<Vec<_>>()
            .join(",");
        files.add(format!("src/{feature}/data.ts"), format!("import type {{PageData}} from '@/lib/runtime'\n{css_imports}export const pages:Record<string,PageData>={{{entries}}}\n"))?;
        // Physical index routes preserve literal URL paths without inventing
        // parent layouts. Feature pages own markup; routes only wire the router.
        files.add(format!("src/routes/{route_file}"), format!("import {{createFileRoute}} from '@tanstack/react-router'\nimport {{selectPage,queryKey,pageHead}} from '@/lib/runtime'\nimport {{pages}} from '@/{feature}/data'\nimport {{{name}}} from '@/{feature}/page'\nexport const Route=createFileRoute({})({{loader:({{location}})=>selectPage(pages,location.pathname,location.searchStr),loaderDeps:({{search}})=>queryKey(search),shouldReload:true,head:({{loaderData}})=>pageHead(loaderData),component:RouteComponent}})\nfunction RouteComponent(){{return <{name} page={{Route.useLoaderData()}} />}}\n",js(&route_id)?))?;
        page_sources.push((format!("src/{feature}/page.tsx"),format!("{helpers}{imports}export function {name}({{page}}:{{page:PageData}}){{const base=page.base;switch(page.key){{{}}}return null}}\n",variants.join("\n"))));
    }
    // Candidate source is emitted as JSX, not a runtime interpreter of DOM JSON.
    let used: Vec<_> = renderer.used.iter().cloned().collect();
    for name in &used {
        let id = model
            .components
            .iter()
            .find(|(_, c)| &c.name == name)
            .map(|(id, _)| id)
            .context("component identity missing")?;
        let mut source = renderer.component(id, buttons)?;
        if let Some(annotation) = renderer.report.interpretation.as_ref()
            && let Some(decisions) = annotation.components.get(name)
        {
            source = format!(
                "// Mochi semantic proposals: review required, calibration unqualified, not applied. Origins: {}\n// {}\n{source}",
                js(&annotation.provider_origins)?,
                js(decisions)?
            );
        }
        files.add(format!("src/components/{name}.tsx"), source)?;
    }
    let component_imports = used
        .iter()
        .map(|n| format!("import {{{n}}} from '@/components/{n}'\n"))
        .collect::<String>();
    for (name, before, after) in &layout_sources {
        files.add(format!("src/layouts/{name}.tsx"),format!("import type {{ReactNode}} from 'react'\n{helpers}{component_imports}export function {name}({{children}}:{{children:ReactNode}}){{const base=useSource();return <>{before}{{children}}{after}</>}}\n"))?;
    }
    let layout_imports = layout_sources
        .iter()
        .map(|(n, _, _)| format!("import {{{n}}} from '@/layouts/{n}'\n"))
        .collect::<String>();
    for (path, source) in page_sources {
        files.add(path, format!("{component_imports}{layout_imports}{source}"))?;
    }
    renderer.report.captured_paths = route_urls.values().cloned().collect();
    for name in &used {
        if let Some(component) = model.components.values().find(|c| &c.name == name) {
            renderer
                .report
                .component_sources
                .insert(name.clone(), component.prototype.clone());
        }
    }
    renderer.report.components = used.len();
    renderer.report.layouts = layout_sources.len();
    if let Some(recovery) = &renderer.report.client_recovery {
        for (path, source) in &recovery.files {
            files.add(path.clone(), source.clone())?;
        }
    }
    files.add(
        ".mochi/export.json",
        serde_json::to_string_pretty(&renderer.report)?,
    )?;
    files.add(
        "README.md",
        include_str!("export_template/README.md").into(),
    )?;
    files.add(
        "src/styles/tokens.css",
        include_str!("export_template/src/styles/tokens.css").into(),
    )?;
    files.add(".mochi/style-audit.json", structure::style_audit(&model)?)?;
    files.add("EDITING.md", structure::editing_guide(&renderer.report)?)?;
    let mut formatted_bytes = 0usize;
    for (path, text) in files.text {
        let text = structure::format_source(&path, &text)?;
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
