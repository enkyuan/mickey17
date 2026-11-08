//! Bounded source evidence. Completeness describes the recorded initial light-DOM state,
//! not arbitrary interactions, closed shadow roots, or a reconstructed application.
use crate::{analysis, cache::fingerprint, util};
use anyhow::{Context, Result, ensure};
use headless_chrome::{
    Tab,
    protocol::cdp::{CSS, DOM, Emulation, Page, types::Event},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::Path,
    str::FromStr,
    sync::{Arc, Mutex},
};

mod compact;
mod raster;
pub const CAPTURE_SCRIPT: &str = include_str!("source_capture.js");
pub const MAX_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_NODES: usize = 20_000;
const MAX_VALUE: usize = 1024 * 1024;
const MAX_RULES: usize = 20_000;
const PROPERTIES: [&str; 12] = [
    "display",
    "position",
    "flex-direction",
    "grid-template-columns",
    "gap",
    "font-family",
    "font-size",
    "font-weight",
    "line-height",
    "color",
    "background-color",
    "border-radius",
];
const ATTRIBUTES: [&str; 21] = [
    "id",
    "class",
    "role",
    "type",
    "name",
    "href",
    "src",
    "alt",
    "title",
    "aria-label",
    "aria-labelledby",
    "aria-describedby",
    "aria-expanded",
    "aria-pressed",
    "aria-checked",
    "aria-haspopup",
    "disabled",
    "required",
    "multiple",
    "placeholder",
    "for",
];

/// An explicitly observed viewport, in CSS pixels.
///
/// ```
/// let viewport: mochi::evidence::Viewport = "375x812".parse()?;
/// assert_eq!(viewport.width, 375);
/// assert!("0x812".parse::<mochi::evidence::Viewport>().is_err());
/// # Ok::<(), anyhow::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}
impl Viewport {
    fn validate(&self) -> Result<()> {
        ensure!(
            (200..=4096).contains(&self.width) && (200..=4096).contains(&self.height),
            "source viewport dimensions must be between 200 and 4096"
        );
        Ok(())
    }
}
impl FromStr for Viewport {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self> {
        let (width, height) = value.split_once('x').context("expected WIDTHxHEIGHT")?;
        let viewport = Self {
            width: width.parse()?,
            height: height.parse()?,
        };
        viewport.validate()?;
        Ok(viewport)
    }
}

pub struct CaptureOptions {
    pub viewports: Vec<Viewport>,
}
impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            viewports: [(375, 812), (768, 1024), (1440, 900)]
                .into_iter()
                .map(|(width, height)| Viewport { width, height })
                .collect(),
        }
    }
}
impl CaptureOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=8).contains(&self.viewports.len()),
            "source capture needs 1 to 8 viewports"
        );
        let mut unique = BTreeSet::new();
        for viewport in &self.viewports {
            viewport.validate()?;
            ensure!(unique.insert(viewport), "duplicate source viewport");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Attribute {
    pub name: String,
    pub namespace: Option<String>,
    pub value: String,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Control {
    pub value: Option<String>,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub open: Option<bool>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Content {
    Element {
        tag: String,
        namespace: String,
        attributes: Vec<Attribute>,
        style: usize,
        pseudo: BTreeMap<String, usize>,
        control: Option<Control>,
        bounds: Bounds,
    },
    Text {
        value: String,
    },
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Node {
    pub parent: Option<usize>,
    #[serde(flatten)]
    pub content: Content,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Meta {
    pub name: Option<String>,
    pub property: Option<String>,
    pub http_equiv: Option<String>,
    pub content: String,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Metadata {
    pub title: String,
    pub base_url: String,
    pub lang: String,
    pub doctype: Option<String>,
    pub canonical: Option<String>,
    pub meta: Vec<Meta>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Stylesheet {
    pub href: Option<String>,
    pub base_url: String,
    pub disabled: bool,
    pub media: String,
    pub rules: Vec<String>,
    pub readable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_source: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Font {
    pub stylesheet: usize,
    pub descriptors: BTreeMap<String, String>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Resource {
    pub url: String,
    pub initiator: String,
    pub response_status: Option<u16>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Issue {
    pub code: String,
    pub detail: String,
    pub node: Option<usize>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Screenshot {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    Shadow,
    Frame,
    Raster,
    Nonvisual,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    pub kind: SurfaceKind,
    pub mode: String,
    pub css: Vec<String>,
    pub base_url: String,
    pub children: Vec<usize>,
    pub image: Option<Screenshot>,
    pub bounds: Bounds,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Capture {
    pub schema_version: u32,
    pub url: String,
    pub metadata: Metadata,
    pub viewport: Viewport,
    pub color_scheme: String,
    pub state: String,
    pub scope: String,
    pub nodes: Vec<Node>,
    #[serde(with = "compact")]
    pub styles: Vec<BTreeMap<String, String>>,
    pub stylesheets: Vec<Stylesheet>,
    pub fonts: Vec<Font>,
    pub resources: Vec<Resource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub surfaces: BTreeMap<usize, Surface>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub animation_states: BTreeMap<usize, BTreeMap<String, String>>,
    pub screenshot: Option<Screenshot>,
    pub complete: bool,
    pub issues: Vec<Issue>,
}

fn fields(map: &BTreeMap<String, String>) -> Result<()> {
    ensure!(map.len() <= 2048, "too many style fields");
    for (name, value) in map {
        ensure!(
            !name.is_empty() && name.len() <= 1024 && value.len() <= MAX_VALUE,
            "invalid style field"
        );
    }
    Ok(())
}
fn http_url(value: &str) -> Result<url::Url> {
    ensure!(value.len() <= 8192, "source URL exceeds 8 KiB");
    let url = url::Url::parse(value)?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "source URL must be HTTP(S)"
    );
    Ok(url)
}

impl Capture {
    /// Validate before persisting or consuming browser/model-facing evidence.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported source evidence version"
        );
        http_url(&self.url)?;
        http_url(&self.metadata.base_url)?;
        self.viewport.validate()?;
        ensure!(
            self.state == "initial" && self.scope == "light-dom-cssom-before-after",
            "unsupported source capture scope"
        );
        ensure!(
            matches!(self.color_scheme.as_str(), "light" | "dark"),
            "invalid color scheme"
        );
        ensure!(
            self.complete == self.issues.is_empty(),
            "source completeness disagrees with issues"
        );
        ensure!(
            !self.complete || !self.nodes.is_empty(),
            "complete source capture has no nodes"
        );
        ensure!(
            self.issues.len() <= 32 && self.nodes.len() <= MAX_NODES && self.styles.len() <= 4096,
            "source capture limit exceeded"
        );
        for issue in &self.issues {
            ensure!(
                !issue.code.is_empty()
                    && issue.code.len() <= 64
                    && issue.detail.len() <= 2048
                    && issue.node.is_none_or(|id| id < self.nodes.len()),
                "invalid source issue"
            );
        }
        for (&node, state) in &self.animation_states {
            ensure!(
                node < self.nodes.len()
                    && matches!(self.nodes[node].content, Content::Element { .. }),
                "invalid animated node"
            );
            fields(state)?;
        }
        ensure!(self.surfaces.len() <= 512, "surface count limit exceeded");
        for (&node, surface) in &self.surfaces {
            ensure!(
                node < self.nodes.len()
                    && matches!(self.nodes[node].content, Content::Element { .. }),
                "invalid surface node"
            );
            ensure!(
                matches!(surface.mode.as_str(), "open" | "closed"),
                "invalid shadow mode"
            );
            http_url(&surface.base_url)?;
            ensure!(
                surface.css.len() <= MAX_RULES && surface.css.iter().all(|r| r.len() <= MAX_VALUE),
                "surface stylesheet limit"
            );
            let mut unique = BTreeSet::new();
            ensure!(
                surface
                    .children
                    .iter()
                    .all(|&child| child < self.nodes.len()
                        && self.nodes[child].parent == Some(node)
                        && unique.insert(child)),
                "invalid surface child"
            );
            ensure!(
                [
                    surface.bounds.x,
                    surface.bounds.y,
                    surface.bounds.width,
                    surface.bounds.height
                ]
                .iter()
                .all(|n| n.is_finite())
                    && surface.bounds.width >= 0.0
                    && surface.bounds.height >= 0.0,
                "invalid surface bounds"
            );
            if let Some(image) = &surface.image {
                ensure!(
                    image.path.starts_with("reconstruction/surfaces/")
                        && Path::new(&image.path)
                            .components()
                            .all(|c| matches!(c, std::path::Component::Normal(_)))
                        && image.sha256.len() == 64
                        && image.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                    "invalid surface image"
                );
            }
        }
        ensure!(self.metadata.meta.len() <= 256, "metadata limit exceeded");
        for value in [&self.metadata.title, &self.metadata.lang]
            .into_iter()
            .chain(self.metadata.doctype.iter())
            .chain(self.metadata.canonical.iter())
        {
            ensure!(value.len() <= MAX_VALUE, "metadata value limit exceeded");
        }
        for record in &self.metadata.meta {
            for value in [&record.content]
                .into_iter()
                .chain(record.name.iter())
                .chain(record.property.iter())
                .chain(record.http_equiv.iter())
            {
                ensure!(value.len() <= MAX_VALUE, "metadata value limit exceeded");
            }
        }
        for style in &self.styles {
            fields(style)?;
        }
        let mut depths = vec![0; self.nodes.len()];
        for (index, node) in self.nodes.iter().enumerate() {
            match node.parent {
                None => ensure!(
                    index == 0 && matches!(node.content, Content::Element { .. }),
                    "source capture must have one element root"
                ),
                Some(parent) => {
                    ensure!(
                        parent < index
                            && matches!(self.nodes[parent].content, Content::Element { .. }),
                        "invalid source parent"
                    );
                    depths[index] = depths[parent] + 1;
                    ensure!(depths[index] <= 256, "source depth limit exceeded");
                }
            }
            match &node.content {
                Content::Text { value } => {
                    ensure!(value.len() <= MAX_VALUE, "source text limit exceeded")
                }
                Content::Element {
                    tag,
                    namespace,
                    attributes,
                    style,
                    pseudo,
                    control,
                    bounds,
                } => {
                    ensure!(
                        !tag.is_empty() && tag.len() <= 128 && namespace.len() <= 1024,
                        "invalid source element"
                    );
                    ensure!(
                        *style < self.styles.len() && pseudo.len() <= 2,
                        "invalid source style reference"
                    );
                    for (name, id) in pseudo {
                        ensure!(
                            matches!(name.as_str(), "::before" | "::after")
                                && *id < self.styles.len(),
                            "invalid pseudo style reference"
                        );
                    }
                    ensure!(attributes.len() <= 256, "source attribute limit exceeded");
                    let mut unique = BTreeSet::new();
                    for attr in attributes {
                        ensure!(
                            !attr.name.is_empty()
                                && attr.name.len() <= 1024
                                && attr.value.len() <= MAX_VALUE
                                && attr.namespace.as_ref().is_none_or(|ns| ns.len() <= 1024),
                            "invalid source attribute"
                        );
                        ensure!(
                            unique.insert((&attr.namespace, &attr.name)),
                            "duplicate source attribute"
                        );
                    }
                    if let Some(control) = control {
                        ensure!(
                            control.value.as_ref().is_none_or(|s| s.len() <= MAX_VALUE),
                            "source control value limit exceeded"
                        );
                    }
                    ensure!(
                        [bounds.x, bounds.y, bounds.width, bounds.height]
                            .iter()
                            .all(|v| v.is_finite())
                            && bounds.width >= 0.0
                            && bounds.height >= 0.0,
                        "invalid source bounds"
                    );
                }
            }
        }
        ensure!(
            self.stylesheets.len() <= 128
                && self.fonts.len() <= MAX_RULES
                && self.resources.len() <= 10_000,
            "source resource limit exceeded"
        );
        let mut rules = 0;
        for sheet in &self.stylesheets {
            rules += sheet.rules.len();
            ensure!(rules <= MAX_RULES, "stylesheet rule limit exceeded");
            ensure!(
                sheet.readable || !self.complete,
                "inaccessible stylesheet in complete capture"
            );
            for value in [&sheet.base_url, &sheet.media]
                .into_iter()
                .chain(sheet.href.iter())
                .chain(sheet.rules.iter())
            {
                ensure!(value.len() <= MAX_VALUE, "stylesheet value limit exceeded");
            }
        }
        for font in &self.fonts {
            ensure!(
                font.stylesheet < self.stylesheets.len(),
                "invalid font stylesheet reference"
            );
            fields(&font.descriptors)?;
        }
        for resource in &self.resources {
            ensure!(
                resource.url.len() <= MAX_VALUE && resource.initiator.len() <= 128,
                "invalid source resource"
            );
            ensure!(
                resource.response_status.is_none_or(|status| status <= 999),
                "invalid source resource status"
            );
        }
        if let Some(screenshot) = &self.screenshot {
            ensure!(
                screenshot.sha256.len() == 64
                    && screenshot.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid screenshot digest"
            );
            ensure!(
                screenshot.path.starts_with("reconstruction/screens/")
                    && screenshot.path.ends_with(".png")
                    && screenshot.path.len() <= 256
                    && !screenshot.path.contains(['\\', ':']),
                "invalid screenshot path"
            );
            ensure!(
                Path::new(&screenshot.path)
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
                "invalid screenshot path"
            );
        }
        Ok(())
    }

    /// Refuse incomplete observations before inference or source generation.
    pub fn require_complete(&self) -> Result<()> {
        self.validate()?;
        ensure!(
            self.complete,
            "incomplete source evidence: {}",
            self.issues
                .iter()
                .map(|i| i.code.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        ensure!(
            self.surfaces
                .values()
                .all(|s| s.kind != SurfaceKind::Raster || s.image.is_some()),
            "rendered surface has no captured pixels"
        );
        ensure!(
            self.screenshot.is_some(),
            "source evidence has no reference screenshot"
        );
        Ok(())
    }

    /// Project rich ordered content onto the existing analysis schema, without losing
    /// the original evidence. Values are shortened only in this explicitly lossy view.
    pub fn to_analysis_snapshot(&self) -> Result<analysis::Snapshot> {
        self.require_complete()?;
        let root = self
            .nodes
            .iter()
            .position(|node| matches!(&node.content, Content::Element { tag, .. } if tag == "body"))
            .unwrap_or(0);
        let mut mapping = vec![None; self.nodes.len()];
        let mut nodes: Vec<analysis::Node> = Vec::new();
        let mut styles = Vec::new();
        let mut style_ids = BTreeMap::new();
        let mut children = BTreeMap::<usize, usize>::new();
        for (index, node) in self.nodes.iter().enumerate().skip(root) {
            let parent = if index == root {
                None
            } else {
                node.parent.and_then(|p| mapping[p])
            };
            if index != root && parent.is_none() {
                continue;
            }
            match &node.content {
                Content::Element {
                    tag,
                    namespace,
                    attributes,
                    style,
                    ..
                } => {
                    let selected: BTreeMap<_, _> = self.styles[*style]
                        .iter()
                        .filter(|(key, _)| PROPERTIES.contains(&key.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    let id = *style_ids
                        .entry(serde_json::to_string(&selected)?)
                        .or_insert_with(|| {
                            styles.push(selected);
                            styles.len() - 1
                        });
                    let next = nodes.len();
                    mapping[index] = Some(next);
                    if let Some(parent) = parent {
                        *children.entry(parent).or_default() += 1;
                    }
                    nodes.push(analysis::Node {
                        parent,
                        tag: tag.clone(),
                        namespace: namespace.clone(),
                        attributes: attributes
                            .iter()
                            .filter(|a| ATTRIBUTES.contains(&a.name.as_str()))
                            .map(|a| (a.name.clone(), a.value.clone()))
                            .collect(),
                        text: String::new(),
                        text_slots: Some(Vec::new()),
                        style: id,
                    });
                }
                Content::Text { value } => {
                    if let Some(parent) = parent
                        && !value.trim().is_empty()
                    {
                        let slot = *children.get(&parent).unwrap_or(&0);
                        let target = &mut nodes[parent];
                        if let Some(slots) = &mut target.text_slots
                            && slots.last() != Some(&slot)
                        {
                            slots.push(slot);
                        }
                        if !target.text.is_empty() {
                            target.text.push(' ');
                        }
                        target.text.extend(value.chars().take(512));
                        target.text = target
                            .text
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .chars()
                            .take(512)
                            .collect();
                        let mut units = 0;
                        target.text = target
                            .text
                            .chars()
                            .take_while(|c| {
                                units += c.len_utf16();
                                units <= 512
                            })
                            .collect();
                    }
                }
            }
        }
        Ok(analysis::Snapshot {
            schema_version: 1,
            url: self.url.clone(),
            viewport: analysis::Viewport {
                width: self.viewport.width,
                height: self.viewport.height,
            },
            nodes,
            styles,
            truncated: false,
        })
    }
}

fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    ensure!(
        path.symlink_metadata()?.file_type().is_file(),
        "source evidence is not a regular file"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "source evidence byte limit exceeded"
    );
    Ok(bytes)
}

/// Read and validate a capture and its screenshot without contacting the origin.
pub fn load(root: &Path, relative: &Path) -> Result<Capture> {
    Ok(load_with_digest(root, relative)?.0)
}

pub(crate) fn load_with_digest(root: &Path, relative: &Path) -> Result<(Capture, String)> {
    let root = root.canonicalize()?;
    let path = util::checked_path(&root, &root.join(relative))?;
    let bytes = bounded_read(&path, MAX_BYTES)?;
    let digest = fingerprint(&bytes);
    let capture: Capture = serde_json::from_slice(&bytes)?;
    capture.validate()?;
    if let Some(screenshot) = &capture.screenshot {
        let path = util::checked_path(&root, &root.join(&screenshot.path))?;
        let png = bounded_read(&path, 64 * 1024 * 1024)?;
        ensure!(
            fingerprint(&png) == screenshot.sha256,
            "screenshot digest mismatch"
        );
        ensure!(
            png.len() >= 24 && &png[..8] == b"\x89PNG\r\n\x1a\n",
            "invalid source screenshot"
        );
        let width = u32::from_be_bytes(png[16..20].try_into()?);
        let height = u32::from_be_bytes(png[20..24].try_into()?);
        ensure!(
            width == capture.viewport.width && height == capture.viewport.height,
            "screenshot dimensions disagree with viewport"
        );
    }
    for surface in capture.surfaces.values() {
        if let Some(image) = &surface.image {
            let path = util::checked_path(&root, &root.join(&image.path))?;
            let png = bounded_read(&path, 64 * 1024 * 1024)?;
            ensure!(
                fingerprint(&png) == image.sha256
                    && png.len() >= 24
                    && &png[..8] == b"\x89PNG\r\n\x1a\n",
                "surface image integrity mismatch"
            );
        }
    }
    Ok((capture, digest))
}

fn write_new(root: &Path, relative: &Path, bytes: &[u8]) -> Result<()> {
    let path = util::checked_path(root, &root.join(relative))?;
    let parent = path.parent().context("missing evidence parent")?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.persist_noclobber(path)?;
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
struct PageManifest {
    schema_version: u32,
    url: String,
    viewports: Vec<Viewport>,
    snapshots: Vec<String>,
}

/// Capture initial states at explicitly requested sizes. Restore viewport overrides
/// even if capture fails; neither themes nor interactive controls are toggled.
pub fn capture_page(
    tab: &Tab,
    root: &Path,
    page: usize,
    options: &CaptureOptions,
    origin: &url::Url,
) -> Result<Vec<String>> {
    options.validate()?;
    let page_url = tab.get_url();
    http_url(&page_url)?;
    let manifest = format!("reconstruction/manifests/page-{page:05}.json");
    ensure!(
        !util::checked_path(root, &root.join(&manifest))?.exists(),
        "source page already captured"
    );
    let script = format!(
        "({CAPTURE_SCRIPT})({})",
        serde_json::json!({"bytes": MAX_BYTES, "nodes": MAX_NODES, "depth": 256, "styles": 4096, "rules": MAX_RULES, "value_bytes": MAX_VALUE})
    );
    // The DevTools CSS domain reads the actual loaded response even when CSSOM
    // access is denied by the page's origin policy. Do not refetch a potentially
    // changed stylesheet or change its placement in the live cascade.
    let headers = Arc::new(Mutex::new(BTreeMap::new()));
    let received = Arc::clone(&headers);
    let listener = tab.add_event_listener(Arc::new(move |event: &Event| {
        if let Event::CSSStyleSheetAdded(event) = event
            && let Ok(mut headers) = received.lock()
            && headers.len() < 128
        {
            let header = &event.params.header;
            headers.insert(header.source_url.clone(), header.style_sheet_id.clone());
        }
    }))?;
    tab.call_method(DOM::Enable {
        include_whitespace: None,
    })?;
    tab.call_method(CSS::Enable(None))?;
    let result = (|| -> Result<Vec<String>> {
        let mut paths = Vec::new();
        for viewport in &options.viewports {
            headers
                .lock()
                .map_err(|_| anyhow::anyhow!("stylesheet header lock"))?
                .clear();
            let metrics: Emulation::SetDeviceMetricsOverride = serde_json::from_value(
                serde_json::json!({"width": viewport.width, "height": viewport.height, "deviceScaleFactor": 1, "mobile": false}),
            )?;
            crate::crawl::navigate_dom(tab, &url::Url::parse("about:blank")?)
                .context("reset document before viewport change")?;
            tab.call_method(metrics).context("set capture viewport")?;
            crate::crawl::navigate_dom(tab, &url::Url::parse(&page_url)?)
                .context("load document at capture viewport")?;
            tab.evaluate("new Promise(resolve=>setTimeout(resolve,800))", true)
                .context("settle new viewport document")?;
            tab.evaluate("new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(resolve,400))))", true).context("settle capture viewport")?;
            let headers = headers
                .lock()
                .map_err(|_| anyhow::anyhow!("stylesheet header lock"))?
                .clone();
            let mut texts = BTreeMap::new();
            let mut bytes = 0;
            for (url, id) in headers {
                if let Ok(url) = url::Url::parse(&url)
                    && matches!(url.scheme(), "http" | "https")
                    && url.origin() != url::Url::parse(&page_url)?.origin()
                    && let Ok(text) = tab.call_method(CSS::GetStyleSheetText { style_sheet_id: id })
                    && text.text.len() <= MAX_VALUE
                {
                    bytes += text.text.len();
                    ensure!(
                        bytes <= MAX_BYTES as usize,
                        "stylesheet response budget exceeded"
                    );
                    texts.insert(url.to_string(), text.text);
                }
            }
            tab.evaluate(
                &format!(
                    "window.__mochiStylesheetResponses={}",
                    serde_json::to_string(&texts)?
                ),
                false,
            )?;
            let raw = tab
                .evaluate(&script, true)
                .context("evaluate capture script")?
                .value
                .context("missing source evidence")?;
            let raw = raw.as_str().context("invalid source evidence")?;
            ensure!(
                raw.len() as u64 <= MAX_BYTES,
                "source evidence byte limit exceeded"
            );
            let mut capture: Capture = serde_json::from_str(raw)?;
            capture.validate()?;
            for (&node, surface) in &mut capture.surfaces {
                if surface.kind != SurfaceKind::Raster {
                    continue;
                }
                let b = &surface.bounds;
                if b.width < 0.5 || b.height < 0.5 {
                    surface.kind = SurfaceKind::Nonvisual;
                    continue;
                }
                ensure!(
                    b.width * b.height <= 16_777_216.0,
                    "surface pixel budget exceeded"
                );
                let image = format!(
                    "reconstruction/surfaces/page-{page:05}-{}x{}-node-{node}.png",
                    viewport.width, viewport.height
                );
                let pixels=tab.evaluate(&format!("(()=>{{const element=window.__mochiCapturedNodes?.[{node}];if(element?.localName!=='canvas')return null;try{{return element.toDataURL('image/png')}}catch{{return null}}}})()"),false).context("read isolated canvas pixels")?.value.and_then(|value|value.as_str().map(str::to_owned));
                let png = if let Some(data) = pixels {
                    use base64::Engine;
                    ensure!(data.len() <= 90 * 1024 * 1024, "canvas encoding byte limit");
                    let encoded = data
                        .strip_prefix("data:image/png;base64,")
                        .context("invalid canvas PNG encoding")?;
                    base64::engine::general_purpose::STANDARD.decode(encoded)?
                } else {
                    raster::isolated(tab, node, b)
                        .with_context(|| format!("isolate rendered surface {node}"))?
                };
                ensure!(png.len() <= 64 * 1024 * 1024, "surface image byte limit");
                surface.image = Some(Screenshot {
                    path: image.clone(),
                    sha256: fingerprint(&png),
                });
                write_new(root, Path::new(&image), &png)?;
            }
            capture.validate()?;
            ensure!(
                capture.url == page_url,
                "source page changed URL while capturing viewports"
            );
            ensure!(
                crate::mirror::same_site(&http_url(&capture.url)?, origin),
                "source capture redirected outside site"
            );
            ensure!(
                capture.viewport == *viewport,
                "source viewport override did not take effect"
            );
            tab.evaluate("window.__mochiPaint ? window.__mochiPaint() : new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))",true).context("present captured surface frame")?;
            let name = format!("page-{page:05}-{}x{}", viewport.width, viewport.height);
            let image = format!("reconstruction/screens/{name}.png");
            let png = tab.capture_screenshot(
                Page::CaptureScreenshotFormatOption::Png,
                None,
                Some(Page::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: viewport.width.into(),
                    height: viewport.height.into(),
                    scale: 1.0,
                }),
                true,
            )?;
            ensure!(
                png.len() <= 64 * 1024 * 1024,
                "source screenshot exceeds 64 MiB"
            );
            capture.screenshot = Some(Screenshot {
                path: image.clone(),
                sha256: fingerprint(&png),
            });
            capture.validate()?;
            let bytes = serde_json::to_vec(&capture)?;
            ensure!(
                bytes.len() as u64 <= MAX_BYTES,
                "source evidence byte limit exceeded"
            );
            write_new(root, Path::new(&image), &png)?;
            let path = format!("reconstruction/pages/{name}.json");
            write_new(root, Path::new(&path), &bytes)?;
            paths.push(path);
        }
        let record = PageManifest {
            schema_version: 1,
            url: page_url.clone(),
            viewports: options.viewports.clone(),
            snapshots: paths.clone(),
        };
        write_new(root, Path::new(&manifest), &serde_json::to_vec(&record)?)?;
        Ok(paths)
    })();
    tab.remove_event_listener(&listener)?;
    tab.call_method(CSS::Disable(None))?;
    let restored = tab.call_method(Emulation::ClearDeviceMetricsOverride(None));
    match result {
        Err(error) => Err(error),
        Ok(paths) => {
            restored?;
            Ok(paths)
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PageSummary {
    pub snapshot: String,
    pub url: String,
    pub viewport: Viewport,
    pub nodes: usize,
    pub complete: bool,
    pub issues: Vec<String>,
    pub sha256: String,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub captures: Vec<PageSummary>,
    pub complete: usize,
}

/// Validate all source captures and atomically refresh the offline summary.
/// Incomplete captures remain inspectable and are counted separately.
pub fn inspect(root: &Path) -> Result<Report> {
    let report = inventory(root)?;
    let root = root.canonicalize()?;
    let path = util::checked_path(&root, &root.join("reconstruction/report.json"))?;
    let bytes = serde_json::to_vec_pretty(&report)?;
    if path.exists() {
        util::atomic_replace(&path, &bytes)?;
    } else {
        write_new(&root, Path::new("reconstruction/report.json"), &bytes)?;
    }
    Ok(report)
}

/// Read and verify the inventory without modifying the clone.
pub(crate) fn inventory(root: &Path) -> Result<Report> {
    let root = root.canonicalize()?;
    let directory = util::checked_path(&root, &root.join("reconstruction/manifests"))?;
    ensure!(
        directory.is_dir(),
        "no source evidence manifests; clone with --source-evidence first"
    );
    let mut manifests = Vec::new();
    for entry in directory.read_dir()? {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s == "json") {
            manifests.push(path);
            ensure!(manifests.len() <= 1000, "source manifest limit exceeded");
        }
    }
    manifests.sort();
    ensure!(!manifests.is_empty(), "no source manifests found");
    let mut paths = BTreeMap::new();
    for path in manifests {
        let path = util::checked_path(&root, &path)?;
        let manifest: PageManifest = serde_json::from_slice(&bounded_read(&path, 64 * 1024)?)?;
        ensure!(
            manifest.schema_version == 1,
            "unsupported source manifest version"
        );
        http_url(&manifest.url)?;
        CaptureOptions {
            viewports: manifest.viewports.clone(),
        }
        .validate()?;
        ensure!(
            manifest.viewports.len() == manifest.snapshots.len(),
            "source manifest viewport count mismatch"
        );
        for (path, viewport) in manifest.snapshots.into_iter().zip(manifest.viewports) {
            ensure!(
                path.starts_with("reconstruction/pages/")
                    && path.ends_with(".json")
                    && path.len() <= 256
                    && !path.contains(['\\', ':']),
                "invalid source snapshot path"
            );
            let relative = std::path::PathBuf::from(path);
            util::checked_path(&root, &root.join(&relative))?;
            ensure!(
                paths
                    .insert(relative, (manifest.url.clone(), viewport))
                    .is_none(),
                "duplicate source manifest snapshot"
            );
            ensure!(paths.len() <= 1000, "source snapshot limit exceeded");
        }
    }
    let directory = util::checked_path(&root, &root.join("reconstruction/pages"))?;
    for entry in directory.read_dir()? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            ensure!(
                paths.contains_key(path.strip_prefix(&root)?),
                "uncommitted source capture has no manifest"
            );
        }
    }
    // The clone inventory also anchors whole pages, not only individual viewports.
    let clone_manifest = util::checked_path(&root, &root.join("manifest.json"))?;
    if clone_manifest.exists() {
        #[derive(Deserialize)]
        struct CloneManifest {
            pages: Vec<PageInventory>,
        }
        #[derive(Deserialize)]
        struct PageInventory {
            source_evidence: Vec<String>,
        }
        let inventory: CloneManifest =
            serde_json::from_slice(&bounded_read(&clone_manifest, MAX_BYTES)?)?;
        let expected: BTreeSet<_> = inventory
            .pages
            .into_iter()
            .flat_map(|p| p.source_evidence)
            .map(std::path::PathBuf::from)
            .collect();
        ensure!(
            expected == paths.keys().cloned().collect(),
            "source page inventory disagrees with capture manifests"
        );
    }
    let mut report = Report {
        schema_version: 1,
        captures: Vec::new(),
        complete: 0,
    };
    let mut nodes = 0;
    for (path, (url, viewport)) in paths {
        let (capture, digest) = load_with_digest(&root, &path)?;
        ensure!(
            capture.url == url && capture.viewport == viewport,
            "source capture disagrees with viewport manifest"
        );
        nodes += capture.nodes.len();
        ensure!(nodes <= 250_000, "source total node budget exceeded");
        let complete = capture.complete && capture.screenshot.is_some();
        report.complete += usize::from(complete);
        let mut issues: Vec<_> = capture.issues.into_iter().map(|i| i.code).collect();
        if capture.screenshot.is_none() {
            issues.push("screenshot_missing".into());
        }
        report.captures.push(PageSummary {
            sha256: digest,
            snapshot: path.to_string_lossy().replace('\\', "/"),
            url: capture.url,
            viewport: capture.viewport,
            nodes: capture.nodes.len(),
            complete,
            issues,
        });
    }
    Ok(report)
}
