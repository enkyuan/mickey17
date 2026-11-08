//! Deterministic structural evidence, not inferred application source.
use crate::{cache::fingerprint, util};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
};

pub const CAPTURE_SCRIPT: &str = include_str!("capture.js");
pub const MAX_SNAPSHOT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_NODES: usize = 250_000;
type Hash = [u8; 32];

#[derive(Debug, Deserialize, Serialize)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Node {
    pub parent: Option<usize>,
    pub tag: String,
    #[serde(default = "html_namespace")]
    pub namespace: String,
    pub attributes: BTreeMap<String, String>,
    pub text: String,
    #[serde(default)]
    pub text_slots: Option<Vec<usize>>,
    pub style: usize,
}

fn html_namespace() -> String {
    "http://www.w3.org/1999/xhtml".into()
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub url: String,
    pub viewport: Viewport,
    pub nodes: Vec<Node>,
    pub styles: Vec<BTreeMap<String, String>>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub url: String,
    pub snapshot: String,
    pub nodes: usize,
    pub status: &'static str,
    pub template: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Template {
    pub fingerprint: String,
    pub representative: String,
    pub pages: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Occurrence {
    pub page: String,
    pub snapshot: String,
    pub node: usize,
}

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub fingerprint: String,
    pub tag: String,
    pub subtree_nodes: usize,
    pub occurrences: usize,
    pub pages: usize,
    pub examples: Vec<Occurrence>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub algorithm: &'static str,
    pub interpretation: &'static str,
    pub nodes_analyzed: usize,
    pub pages: Vec<Page>,
    pub templates: Vec<Template>,
    pub component_candidates: Vec<Candidate>,
}

struct Group {
    tag: String,
    size: usize,
    count: usize,
    pages: BTreeSet<usize>,
    examples: Vec<(usize, usize)>,
}

/// Hash each node once, preserving child order, semantics, and observed style.
/// Content, IDs, and class names remain in the evidence but do not define identity.
fn hashes(snapshot: &Snapshot) -> Result<(Vec<Hash>, Vec<usize>)> {
    ensure!(snapshot.schema_version == 1, "unsupported snapshot version");
    ensure!(
        !snapshot.nodes.is_empty() && snapshot.nodes.len() <= 20_000,
        "invalid snapshot node count"
    );
    ensure!(
        snapshot.styles.len() <= 4096,
        "snapshot style limit exceeded"
    );
    let mut children = vec![Vec::new(); snapshot.nodes.len()];
    let mut depths = vec![0; snapshot.nodes.len()];
    for (index, node) in snapshot.nodes.iter().enumerate() {
        ensure!(
            node.style < snapshot.styles.len(),
            "invalid style reference"
        );
        ensure!(
            node.tag.len() <= 128 && node.attributes.len() <= 32,
            "invalid node fields"
        );
        match node.parent {
            None => ensure!(index == 0, "snapshot must have one root"),
            Some(parent) => {
                ensure!(parent < index, "parent must precede child");
                depths[index] = depths[parent] + 1;
                ensure!(depths[index] <= 256, "snapshot depth limit exceeded");
                children[parent].push(index);
            }
        }
    }
    let mut hashes = vec![[0; 32]; snapshot.nodes.len()];
    let mut sizes = vec![1usize; snapshot.nodes.len()];
    let style_hashes: Vec<Hash> = snapshot
        .styles
        .iter()
        .map(|style| Ok(Sha256::digest(serde_json::to_vec(style)?).into()))
        .collect::<Result<_>>()?;
    for (index, node) in snapshot.nodes.iter().enumerate().rev() {
        let text_slots = node
            .text_slots
            .as_deref()
            .unwrap_or(if node.text.trim().is_empty() {
                &[]
            } else {
                &[0]
            });
        ensure!(
            text_slots.windows(2).all(|pair| pair[0] < pair[1])
                && text_slots
                    .last()
                    .is_none_or(|slot| *slot <= children[index].len()),
            "invalid text positions"
        );
        ensure!(
            node.text_slots.is_some() || children[index].is_empty() || node.text.trim().is_empty(),
            "mixed text requires position evidence"
        );
        let semantics: BTreeMap<_, _> = node
            .attributes
            .iter()
            .filter(|(name, _)| {
                matches!(
                    name.as_str(),
                    "role"
                        | "type"
                        | "name"
                        | "aria-haspopup"
                        | "aria-expanded"
                        | "aria-pressed"
                        | "aria-checked"
                        | "disabled"
                        | "required"
                        | "multiple"
                )
            })
            .collect();
        let atom = serde_json::to_vec(&(
            &node.tag,
            &node.namespace,
            semantics,
            text_slots,
            node.attributes.contains_key("aria-labelledby"),
            node.attributes.contains_key("aria-describedby"),
        ))?;
        let mut hash = Sha256::new();
        hash.update(b"mochi-dom-v1");
        hash.update((atom.len() as u64).to_le_bytes());
        hash.update(atom);
        hash.update(style_hashes[node.style]);
        hash.update((children[index].len() as u64).to_le_bytes());
        for &child in &children[index] {
            hash.update(hashes[child]);
            sizes[index] += sizes[child];
        }
        hashes[index] = hash.finalize().into();
    }
    Ok((hashes, sizes))
}

pub fn analyze(clone: &Path) -> Result<Report> {
    let root = clone.canonicalize()?;
    let directory = util::checked_path(&root, &root.join("analysis/pages"))?;
    ensure!(
        directory.is_dir(),
        "no DOM snapshots; create a clone with --analyze first"
    );
    let mut paths = Vec::new();
    for entry in directory.read_dir()? {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s == "json") {
            ensure!(
                path.symlink_metadata()?.file_type().is_file(),
                "snapshot is not a regular file"
            );
            paths.push(util::checked_path(&root, &path)?);
            ensure!(
                paths.len() <= 1000,
                "analysis snapshot limit exceeded (1000)"
            );
        }
    }
    paths.sort();
    ensure!(!paths.is_empty(), "no DOM snapshots found");
    let mut report = Report {
        schema_version: 1,
        algorithm: "ordered-dom-style-sha256-v1",
        interpretation: "Exact structural candidates at the captured viewport/state; no semantic or behavioral equivalence is inferred.",
        nodes_analyzed: 0,
        pages: Vec::new(),
        templates: Vec::new(),
        component_candidates: Vec::new(),
    };
    let mut templates: BTreeMap<String, Template> = BTreeMap::new();
    let mut groups: BTreeMap<Hash, Group> = BTreeMap::new();
    for path in paths {
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take(MAX_SNAPSHOT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_SNAPSHOT_BYTES,
            "snapshot exceeds 8 MiB"
        );
        let snapshot: Snapshot =
            serde_json::from_slice(&bytes).with_context(|| format!("read {}", path.display()))?;
        ensure!(snapshot.url.len() <= 8192, "snapshot URL exceeds 8 KiB");
        let source = url::Url::parse(&snapshot.url).context("invalid snapshot URL")?;
        ensure!(
            matches!(source.scheme(), "http" | "https") && source.host_str().is_some(),
            "snapshot URL must be HTTP(S)"
        );
        let relative = path
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        let mut page = Page {
            url: snapshot.url.clone(),
            snapshot: relative.clone(),
            nodes: snapshot.nodes.len(),
            status: "analyzed",
            template: None,
        };
        ensure!(snapshot.schema_version == 1, "unsupported snapshot version");
        ensure!(
            snapshot.nodes.len() <= 20_000,
            "snapshot node limit exceeded"
        );
        if snapshot.truncated {
            page.status = "capture_truncated";
        } else if report.nodes_analyzed + snapshot.nodes.len() > MAX_TOTAL_NODES {
            page.status = "node_budget_exceeded";
        } else {
            let (hashes, sizes) = hashes(&snapshot)?;
            report.nodes_analyzed += snapshot.nodes.len();
            let mut identity = hashes[0].to_vec();
            identity.extend_from_slice(&snapshot.viewport.width.to_le_bytes());
            identity.extend_from_slice(&snapshot.viewport.height.to_le_bytes());
            let template = fingerprint(&identity);
            templates
                .entry(template.clone())
                .or_insert_with(|| Template {
                    fingerprint: template.clone(),
                    representative: snapshot.url.clone(),
                    pages: Vec::new(),
                })
                .pages
                .push(snapshot.url.clone());
            page.template = Some(template);
            for (index, node) in snapshot.nodes.iter().enumerate().skip(1) {
                if !(3..=5000).contains(&sizes[index]) {
                    continue;
                }
                let group = groups.entry(hashes[index]).or_insert_with(|| Group {
                    tag: node.tag.clone(),
                    size: sizes[index],
                    count: 0,
                    pages: BTreeSet::new(),
                    examples: Vec::new(),
                });
                group.count += 1;
                group.pages.insert(report.pages.len());
                if group.examples.len() < 8 {
                    group.examples.push((report.pages.len(), index));
                }
            }
        }
        report.pages.push(page);
    }
    report.templates = templates.into_values().collect();
    report.templates.sort_by(|a, b| {
        b.pages
            .len()
            .cmp(&a.pages.len())
            .then(a.fingerprint.cmp(&b.fingerprint))
    });
    let mut ranked: Vec<_> = groups
        .into_iter()
        .filter(|(_, group)| group.count >= 2)
        .collect();
    ranked.sort_by(|(hash_a, a), (hash_b, b)| {
        let score = |group: &Group| (group.size - 1) * (group.count - 1);
        score(b).cmp(&score(a)).then(hash_a.cmp(hash_b))
    });
    ranked.truncate(100);
    report.component_candidates = ranked
        .into_iter()
        .map(|(hash, group)| Candidate {
            fingerprint: hash.iter().map(|byte| format!("{byte:02x}")).collect(),
            tag: group.tag,
            subtree_nodes: group.size,
            occurrences: group.count,
            pages: group.pages.len(),
            examples: group
                .examples
                .into_iter()
                .map(|(page, node)| Occurrence {
                    page: report.pages[page].url.clone(),
                    snapshot: report.pages[page].snapshot.clone(),
                    node,
                })
                .collect(),
        })
        .collect();
    let output = util::checked_path(&root, &root.join("analysis/report.json"))?;
    let bytes = serde_json::to_vec_pretty(&report)?;
    if output.exists() {
        util::atomic_replace(&output, &bytes)?;
    } else {
        util::write(&root, &output, &bytes)?;
    }
    Ok(report)
}
