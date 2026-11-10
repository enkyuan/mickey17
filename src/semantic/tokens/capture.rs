//! Read-only view of a captured page snapshot (`reconstruction/pages/*.json`).
//!
//! Deliberately independent of `crate::evidence`: the tokens pipeline only needs the
//! element tree, the computed-style maps and the CSSOM rule texts, and it never writes
//! to the capture. No integrity validation is done here; the report records the SHA-256
//! of every snapshot file it read so the input is identifiable.

use super::selector::{Dom, Tri};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct RawViewport {
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize)]
struct RawAttr {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct RawControl {
    checked: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RawNode {
    parent: Option<usize>,
    kind: String,
    tag: Option<String>,
    attributes: Option<Vec<RawAttr>>,
    style: Option<usize>,
    pseudo: Option<BTreeMap<String, usize>>,
    value: Option<String>,
    control: Option<RawControl>,
}

#[derive(Debug, Deserialize)]
struct RawStyles {
    strings: Vec<String>,
    maps: Vec<Vec<[usize; 2]>>,
}

#[derive(Debug, Deserialize)]
struct RawSheet {
    href: Option<String>,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    media: String,
    #[serde(default)]
    rules: Vec<String>,
    #[serde(default)]
    text_source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawSurface {
    kind: String,
    #[serde(default)]
    css: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawSnapshot {
    url: String,
    viewport: RawViewport,
    color_scheme: String,
    nodes: Vec<RawNode>,
    styles: RawStyles,
    stylesheets: Vec<RawSheet>,
    #[serde(default)]
    surfaces: BTreeMap<String, RawSurface>,
    #[serde(default)]
    animation_states: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub parent: Option<usize>,
    pub tag: String,
    pub attributes: Vec<(String, String)>,
    /// Computed style of the element.
    pub style: usize,
    /// Computed style per pseudo-element key without colons (`before`, `after`).
    pub pseudo: BTreeMap<String, usize>,
    pub checked: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct SheetRef {
    /// Index into [`Corpus::sheets`].
    pub sheet: usize,
    pub media: String,
    pub disabled: bool,
}

/// One captured page state: element tree, computed styles and the ordered sheets.
#[derive(Debug)]
pub struct Snapshot {
    pub label: String,
    pub file: PathBuf,
    pub sha256: String,
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub color_scheme: String,
    /// `nodes[i]` is `None` for text nodes (indices stay aligned with the capture).
    pub nodes: Vec<Option<Element>>,
    /// Indices of element nodes with non-whitespace text children, for `:empty`.
    has_text: Vec<bool>,
    children: Vec<Vec<usize>>,
    pub styles: Vec<BTreeMap<String, String>>,
    pub sheets: Vec<SheetRef>,
    /// Shadow/frame surfaces whose subtree is not covered by document stylesheets.
    pub shadow_roots: Vec<usize>,
    /// Elements captured with an animation in flight; their computed values are not cascade values.
    pub animated: std::collections::BTreeSet<usize>,
}

/// A stylesheet deduplicated across snapshots.
#[derive(Debug)]
pub struct SheetSource {
    pub href: Option<String>,
    pub texts: Vec<String>,
    pub from_text_source: bool,
    pub sha256: String,
}

#[derive(Debug, Default)]
pub struct Corpus {
    pub sheets: Vec<SheetSource>,
    pub snapshots: Vec<Snapshot>,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

impl Corpus {
    /// Load every `reconstruction/pages/*.json` snapshot below a capture directory.
    pub fn load(capture_dir: &Path) -> Result<Corpus> {
        let pages = capture_dir.join("reconstruction").join("pages");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&pages)
            .with_context(|| format!("read {}", pages.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        if files.is_empty() {
            bail!("no snapshots in {}", pages.display());
        }
        let mut corpus = Corpus::default();
        let mut by_hash: BTreeMap<String, usize> = BTreeMap::new();
        for file in files {
            let bytes = std::fs::read(&file).with_context(|| format!("read {}", file.display()))?;
            let sha256 = hex(&Sha256::digest(&bytes));
            let raw: RawSnapshot = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse {}", file.display()))?;
            let label = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            corpus.add(raw, label, file, sha256, &mut by_hash)?;
        }
        Ok(corpus)
    }

    /// Add one snapshot from JSON bytes (fixtures and tests; `load` is the normal entry).
    pub fn add_json(&mut self, label: &str, bytes: &[u8]) -> Result<()> {
        let raw: RawSnapshot = serde_json::from_slice(bytes)?;
        let mut by_hash: BTreeMap<String, usize> = BTreeMap::new();
        for (i, s) in self.sheets.iter().enumerate() {
            by_hash.insert(s.sha256.clone(), i);
        }
        let sha = hex(&Sha256::digest(bytes));
        self.add(
            raw,
            label.to_string(),
            PathBuf::from(label),
            sha,
            &mut by_hash,
        )
    }

    fn add(
        &mut self,
        raw: RawSnapshot,
        label: String,
        file: PathBuf,
        sha256: String,
        by_hash: &mut BTreeMap<String, usize>,
    ) -> Result<()> {
        let mut sheets = Vec::new();
        for sheet in &raw.stylesheets {
            // CSSOM rule texts are authoritative; text_source is only the fallback for
            // sheets whose rules could not be read.
            let (texts, from_text_source) = if sheet.rules.is_empty() {
                match &sheet.text_source {
                    Some(text) => (vec![text.clone()], true),
                    None => (Vec::new(), false),
                }
            } else {
                (sheet.rules.clone(), false)
            };
            let mut hasher = Sha256::new();
            hasher.update(sheet.href.as_deref().unwrap_or("").as_bytes());
            for text in &texts {
                hasher.update((text.len() as u64).to_le_bytes());
                hasher.update(text.as_bytes());
            }
            let digest = hex(&hasher.finalize());
            let index = *by_hash.entry(digest.clone()).or_insert_with(|| {
                self.sheets.push(SheetSource {
                    href: sheet.href.clone(),
                    texts,
                    from_text_source,
                    sha256: digest,
                });
                self.sheets.len() - 1
            });
            sheets.push(SheetRef {
                sheet: index,
                media: sheet.media.clone(),
                disabled: sheet.disabled,
            });
        }
        // Compact style maps; every index is validated, the capture is untrusted input.
        let string_count = raw.styles.strings.len();
        let mut styles: Vec<BTreeMap<String, String>> = Vec::with_capacity(raw.styles.maps.len());
        for map in &raw.styles.maps {
            let mut out = BTreeMap::new();
            for [k, v] in map {
                if *k >= string_count || *v >= string_count {
                    bail!("style string index out of range in {}", file.display());
                }
                out.insert(
                    raw.styles.strings[*k].clone(),
                    raw.styles.strings[*v].clone(),
                );
            }
            styles.push(out);
        }
        let mut nodes = Vec::with_capacity(raw.nodes.len());
        let mut children = vec![Vec::new(); raw.nodes.len()];
        let mut has_text = vec![false; raw.nodes.len()];
        for (index, node) in raw.nodes.iter().enumerate() {
            // Nodes are in document order: a parent always precedes its children. Anything
            // else (out-of-range or cyclic parents) is a corrupt capture, not something to walk.
            if let Some(parent) = node.parent
                && parent >= index
            {
                bail!(
                    "node {index} in {} has parent {parent} that does not precede it",
                    file.display()
                );
            }
            match node.kind.as_str() {
                "element" => {
                    let style = node.style.context("element without style")?;
                    if style >= styles.len() {
                        bail!("style index out of range in {}", file.display());
                    }
                    if let Some(parent) = node.parent {
                        children[parent].push(index);
                    }
                    let mut pseudo = BTreeMap::new();
                    for (k, v) in node.pseudo.iter().flatten() {
                        if *v >= styles.len() {
                            bail!(
                                "pseudo-element style index out of range in {}",
                                file.display()
                            );
                        }
                        pseudo.insert(k.trim_start_matches(':').to_string(), *v);
                    }
                    nodes.push(Some(Element {
                        parent: node.parent,
                        tag: node.tag.clone().unwrap_or_default().to_ascii_lowercase(),
                        attributes: node
                            .attributes
                            .iter()
                            .flatten()
                            .map(|a| (a.name.to_ascii_lowercase(), a.value.clone()))
                            .collect(),
                        style,
                        pseudo,
                        checked: node.control.as_ref().and_then(|c| c.checked),
                    }));
                }
                _ => {
                    if let (Some(parent), Some(text)) = (node.parent, &node.value) {
                        // Browsers still treat whitespace-only text as content for :empty.
                        if !text.is_empty() {
                            has_text[parent] = true;
                        }
                    }
                    nodes.push(None);
                }
            }
        }
        // Element children only: keep the list of element ids per parent.
        let children: Vec<Vec<usize>> = children
            .into_iter()
            .map(|list| list.into_iter().filter(|&i| nodes[i].is_some()).collect())
            .collect();
        let shadow_roots = raw
            .surfaces
            .iter()
            .filter(|(_, s)| s.kind == "shadow" || !s.css.is_empty())
            .filter_map(|(k, _)| k.parse().ok())
            .collect();
        self.snapshots.push(Snapshot {
            label,
            file,
            sha256,
            url: raw.url,
            width: raw.viewport.width,
            height: raw.viewport.height,
            color_scheme: raw.color_scheme,
            nodes,
            has_text,
            children,
            styles,
            sheets,
            shadow_roots,
            animated: raw
                .animation_states
                .keys()
                .filter_map(|k| k.parse().ok())
                .collect(),
        });
        Ok(())
    }
}

impl Snapshot {
    pub fn element(&self, node: usize) -> Option<&Element> {
        self.nodes.get(node).and_then(Option::as_ref)
    }

    pub fn elements(&self) -> impl Iterator<Item = usize> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.as_ref().map(|_| i))
    }

    pub fn computed(&self, node: usize) -> Option<&BTreeMap<String, String>> {
        self.element(node).map(|e| &self.styles[e.style])
    }

    pub fn root_font_size_px(&self) -> Option<f64> {
        let root = self
            .elements()
            .find(|&i| self.nodes[i].as_ref().is_some_and(|e| e.parent.is_none()))?;
        let value = self.computed(root)?.get("font-size")?;
        value.strip_suffix("px")?.parse().ok()
    }
}

impl Dom for Snapshot {
    fn tag(&self, node: usize) -> &str {
        self.element(node).map_or("", |e| e.tag.as_str())
    }
    fn attr(&self, node: usize, name: &str) -> Option<&str> {
        self.element(node)?
            .attributes
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn parent(&self, node: usize) -> Option<usize> {
        self.element(node)?.parent
    }
    fn element_children(&self, node: usize) -> &[usize] {
        self.children.get(node).map_or(&[], Vec::as_slice)
    }
    fn is_empty(&self, node: usize) -> bool {
        self.element_children(node).is_empty() && !self.has_text.get(node).copied().unwrap_or(false)
    }
    fn state(&self, node: usize, name: &str) -> Tri {
        let has = |attr: &str| self.attr(node, attr).is_some();
        let tag = self.tag(node);
        let form_control = matches!(tag, "input" | "select" | "textarea" | "button");
        match name {
            "checked" => match (tag, self.element(node).and_then(|e| e.checked)) {
                ("input" | "option", Some(value)) => value.into(),
                ("input" | "option", None) => Tri::Unknown,
                _ => Tri::No,
            },
            "disabled" => (form_control && has("disabled")).into(),
            "enabled" => (form_control && !has("disabled")).into(),
            "required" => (form_control && has("required")).into(),
            "optional" => (form_control && !has("required")).into(),
            "open" => (matches!(tag, "details" | "dialog") && has("open")).into(),
            // A resting snapshot: nothing is hovered, focused or pressed.
            "hover" | "active" | "focus" | "focus-visible" | "focus-within" => Tri::No,
            _ => Tri::Unknown,
        }
    }
}
