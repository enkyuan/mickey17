//! Deterministic, framework-independent source model. Candidates preserve evidence;
//! they do not assert recovered source code, behavior, or router nesting.
use crate::{cache::fingerprint, evidence, mirror, util};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::Path,
};
use url::Url;

const INPUT_BYTES: u64 = 64 * 1024 * 1024;
const OUTPUT_BYTES: u64 = 128 * 1024 * 1024;
const ASSET_BYTES: u64 = 256 * 1024 * 1024;
const MAX_COMPONENTS: usize = 256;

#[derive(Debug, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub algorithm: String,
    pub input_sha256: String,
    pub routes: BTreeMap<String, Route>,
    pub observations: BTreeMap<String, Observation>,
    pub styles: BTreeMap<String, BTreeMap<String, String>>,
    pub components: BTreeMap<String, Component>,
    pub layouts: Vec<Layout>,
    pub navigation: Vec<Navigation>,
    pub assets: BTreeMap<String, Asset>,
    pub notes: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Route {
    pub url: String,
    pub pathname: String,
    pub query: Option<String>,
    pub observations: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Observation {
    pub snapshot: String,
    pub sha256: String,
    pub route: String,
    /// Full ordered evidence, including CSSOM, fonts, pseudo styles and screenshot.
    pub capture: evidence::Capture,
    /// Maps capture-local style indices into the shared content-addressed pool.
    pub style_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceRef {
    pub observation: String,
    pub node: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum Target {
    Text,
    Attribute {
        name: String,
        namespace: Option<String>,
    },
    Control,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Prop {
    /// Offset in depth-first ordered subtree traversal, not a global node index.
    pub relative_node: usize,
    pub target: Target,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Instance {
    pub source: SourceRef,
    /// Route plus occurrence rank within this shape. This is a counting convention,
    /// not proof of cross-viewport identity; repeated viewports cannot create reuse.
    pub logical_occurrence: String,
    pub style_variant: String,
    pub props: BTreeMap<String, Value>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    pub status: String,
    pub prototype: SourceRef,
    pub node_count: usize,
    pub logical_occurrences: usize,
    pub props: BTreeMap<String, Prop>,
    pub instances: Vec<Instance>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Layout {
    pub status: String,
    /// Exact shared siblings around one main element, for at least two routes.
    pub occurrences: Vec<Shell>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Shell {
    pub observation: String,
    pub before: Vec<usize>,
    pub main: usize,
    pub after: Vec<usize>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Navigation {
    pub source: SourceRef,
    pub href: String,
    pub resolved: Option<String>,
    /// Set only for an exact captured URL, with fragment removed.
    pub route: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Asset {
    pub url: String,
    pub usages: BTreeSet<String>,
    pub status: String,
    pub local_path: Option<String>,
    pub local_sha256: Option<String>,
    pub bytes: Option<u64>,
}

fn hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(fingerprint(&serde_json::to_vec(value)?))
}
fn route_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value)?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "invalid route URL"
    );
    url.set_fragment(None);
    Ok(url)
}
fn children(capture: &evidence::Capture) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new(); capture.nodes.len()];
    for (i, node) in capture.nodes.iter().enumerate() {
        if let Some(parent) = node.parent {
            out[parent].push(i);
        }
    }
    out
}
fn subtree(tree: &[Vec<usize>], root: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        out.push(node);
        stack.extend(tree[node].iter().rev());
    }
    out
}
fn semantic(name: &str) -> bool {
    matches!(
        name,
        "role"
            | "type"
            | "name"
            | "disabled"
            | "required"
            | "multiple"
            | "readonly"
            | "aria-haspopup"
            | "aria-expanded"
            | "aria-pressed"
            | "aria-checked"
    )
}
struct Tree {
    children: Vec<Vec<usize>>,
    shape: Vec<String>,
    exact: Vec<String>,
    size: Vec<usize>,
}
fn tree(observation: &Observation) -> Result<Tree> {
    let capture = &observation.capture;
    let children = children(capture);
    let mut shape = vec![String::new(); capture.nodes.len()];
    let mut exact = shape.clone();
    let mut size = vec![1; capture.nodes.len()];
    for i in (0..capture.nodes.len()).rev() {
        let child_shape: Vec<_> = children[i].iter().map(|&n| &shape[n]).collect();
        let child_exact: Vec<_> = children[i].iter().map(|&n| &exact[n]).collect();
        size[i] += children[i].iter().map(|&n| size[n]).sum::<usize>();
        match &capture.nodes[i].content {
            evidence::Content::Text { value } => {
                shape[i] = hash(&("text", value.trim().is_empty()))?;
                exact[i] = hash(&("text", value))?;
            }
            evidence::Content::Element {
                tag,
                namespace,
                attributes,
                style,
                pseudo,
                control,
                ..
            } => {
                let mut attrs: Vec<_> = attributes
                    .iter()
                    .map(|a| {
                        (
                            &a.namespace,
                            &a.name,
                            if semantic(&a.name) {
                                Some(&a.value)
                            } else {
                                None
                            },
                        )
                    })
                    .collect();
                attrs.sort();
                shape[i] = hash(&(
                    tag,
                    namespace,
                    attrs,
                    control.is_some(),
                    child_shape,
                    capture.surfaces.get(&i),
                    capture.animation_states.get(&i),
                ))?;
                let pseudos: BTreeMap<_, _> = pseudo
                    .iter()
                    .map(|(k, &v)| (k, &observation.style_ids[v]))
                    .collect();
                exact[i] = hash(&(
                    tag,
                    namespace,
                    attributes,
                    &observation.style_ids[*style],
                    pseudos,
                    control,
                    child_exact,
                    capture.surfaces.get(&i),
                    capture.animation_states.get(&i),
                ))?;
            }
        }
    }
    Ok(Tree {
        children,
        shape,
        exact,
        size,
    })
}
fn fields(node: &evidence::Node) -> Result<Vec<(Target, Value)>> {
    Ok(match &node.content {
        evidence::Content::Text { value } => vec![(Target::Text, Value::String(value.clone()))],
        evidence::Content::Element {
            attributes,
            control,
            ..
        } => {
            let mut attrs: Vec<_> = attributes.iter().collect();
            attrs.sort_by_key(|a| (&a.namespace, &a.name));
            let mut values: Vec<_> = attrs
                .into_iter()
                .map(|a| {
                    (
                        Target::Attribute {
                            name: a.name.clone(),
                            namespace: a.namespace.clone(),
                        },
                        Value::String(a.value.clone()),
                    )
                })
                .collect();
            values.push((Target::Control, serde_json::to_value(control)?));
            values
        }
    })
}
fn value_at(node: &evidence::Node, target: &Target) -> Result<Value> {
    match (&node.content, target) {
        (evidence::Content::Text { value }, Target::Text) => Ok(Value::String(value.clone())),
        (evidence::Content::Element { attributes, .. }, Target::Attribute { name, namespace }) => {
            attributes
                .iter()
                .find(|a| &a.name == name && &a.namespace == namespace)
                .map(|a| Value::String(a.value.clone()))
                .context("prop attribute absent")
        }
        (evidence::Content::Element { control, .. }, Target::Control) => {
            Ok(serde_json::to_value(control)?)
        }
        _ => anyhow::bail!("prop target absent"),
    }
}
fn style_variant(observation: &Observation, nodes: &[usize]) -> Result<String> {
    let mut styles = Vec::new();
    for &i in nodes {
        if let evidence::Content::Element { style, pseudo, .. } =
            &observation.capture.nodes[i].content
        {
            let pseudos: BTreeMap<_, _> = pseudo
                .iter()
                .map(|(k, &v)| (k, &observation.style_ids[v]))
                .collect();
            styles.push((&observation.style_ids[*style], pseudos));
        }
    }
    hash(&styles)
}

fn occurrence_keys(
    sources: &[SourceRef],
    observations: &BTreeMap<String, Observation>,
) -> Vec<String> {
    let mut ranks = BTreeMap::new();
    sources
        .iter()
        .map(|source| {
            let rank = ranks.entry(&source.observation).or_insert(0usize);
            let key = format!("{}:{}", observations[&source.observation].route, rank);
            *rank += 1;
            key
        })
        .collect()
}

fn components(
    observations: &BTreeMap<String, Observation>,
    trees: &BTreeMap<String, Tree>,
) -> Result<BTreeMap<String, Component>> {
    let mut groups: BTreeMap<&str, Vec<SourceRef>> = BTreeMap::new();
    for (id, observation) in observations {
        let tree = &trees[id];
        for (node, item) in observation.capture.nodes.iter().enumerate() {
            if let evidence::Content::Element { tag, .. } = &item.content
                && !matches!(tag.as_str(), "html" | "head" | "body" | "main")
                && (3..=5000).contains(&tree.size[node])
            {
                groups
                    .entry(&tree.shape[node])
                    .or_default()
                    .push(SourceRef {
                        observation: id.clone(),
                        node,
                    });
            }
        }
    }
    let mut candidates: Vec<_> = groups
        .into_iter()
        .filter_map(|(shape, sources)| {
            let count = occurrence_keys(&sources, observations)
                .into_iter()
                .collect::<BTreeSet<_>>()
                .len();
            let size = trees[&sources[0].observation].size[sources[0].node];
            (count >= 2).then_some((shape, sources, count, size))
        })
        .collect();
    candidates.sort_by(|a, b| b.2.cmp(&a.2).then(b.3.cmp(&a.3)).then(a.0.cmp(b.0)));
    let mut occupied: BTreeSet<(String, usize)> = BTreeSet::new();
    let mut result = BTreeMap::new();
    for (id, sources, _, size) in candidates {
        if result.len() == MAX_COMPONENTS {
            break;
        }
        let sources: Vec<_> = sources
            .into_iter()
            .filter(|s| {
                // Ancestors and descendants are mutually exclusive across selected instances.
                !subtree(&trees[&s.observation].children, s.node)
                    .iter()
                    .any(|&n| occupied.contains(&(s.observation.clone(), n)))
            })
            .collect();
        let logical = occurrence_keys(&sources, observations);
        let count = logical.iter().collect::<BTreeSet<_>>().len();
        if count < 2 {
            continue;
        }
        let aligned: Vec<_> = sources
            .iter()
            .map(|s| subtree(&trees[&s.observation].children, s.node))
            .collect();
        let prototype = sources[0].clone();
        let mut props = BTreeMap::new();
        let mut bindings: Vec<BTreeMap<String, Value>> =
            (0..sources.len()).map(|_| BTreeMap::new()).collect();
        for relative_node in 0..size {
            let node =
                &observations[&prototype.observation].capture.nodes[aligned[0][relative_node]];
            for (target, first) in fields(node)? {
                let values: Vec<_> = sources
                    .iter()
                    .zip(&aligned)
                    .map(|(s, nodes)| {
                        value_at(
                            &observations[&s.observation].capture.nodes[nodes[relative_node]],
                            &target,
                        )
                    })
                    .collect::<Result<_>>()?;
                if values.iter().any(|v| v != &first) {
                    let name = format!("prop_{}", props.len() + 1);
                    props.insert(
                        name.clone(),
                        Prop {
                            relative_node,
                            target,
                        },
                    );
                    for (binding, value) in bindings.iter_mut().zip(values) {
                        binding.insert(name.clone(), value);
                    }
                }
            }
        }
        let mut instances = Vec::new();
        for (((source, nodes), props), logical_occurrence) in
            sources.into_iter().zip(aligned).zip(bindings).zip(logical)
        {
            let style_variant = style_variant(&observations[&source.observation], &nodes)?;
            for &node in &nodes {
                occupied.insert((source.observation.clone(), node));
            }
            instances.push(Instance {
                logical_occurrence,
                source,
                style_variant,
                props,
            });
        }
        result.insert(
            id.into(),
            Component {
                name: format!("Component{}", result.len() + 1),
                status: "structural_candidate".into(),
                prototype,
                node_count: size,
                logical_occurrences: count,
                props,
                instances,
            },
        );
    }
    Ok(result)
}

fn layouts(
    observations: &BTreeMap<String, Observation>,
    trees: &BTreeMap<String, Tree>,
) -> Result<Vec<Layout>> {
    let mut groups: BTreeMap<String, Vec<Shell>> = BTreeMap::new();
    for (id, observation) in observations {
        let tree = &trees[id];
        for (body, node) in observation.capture.nodes.iter().enumerate() {
            if !matches!(&node.content,evidence::Content::Element{tag,..} if tag=="body") {
                continue;
            }
            let siblings = &tree.children[body];
            let mains: Vec<_> = siblings.iter().enumerate().filter(|(_,n)|
                matches!(&observation.capture.nodes[**n].content,evidence::Content::Element{tag,..} if tag=="main")).collect();
            if mains.len() != 1 {
                continue;
            }
            let position = mains[0].0;
            let before = siblings[..position].to_vec();
            let after = siblings[position + 1..].to_vec();
            // Whitespace alone does not qualify as an application shell.
            if !before.iter().chain(&after).any(|&n| {
                matches!(
                    observation.capture.nodes[n].content,
                    evidence::Content::Element { .. }
                )
            }) {
                continue;
            }
            let signature = hash(&(
                before.iter().map(|&n| &tree.exact[n]).collect::<Vec<_>>(),
                after.iter().map(|&n| &tree.exact[n]).collect::<Vec<_>>(),
            ))?;
            groups.entry(signature).or_default().push(Shell {
                observation: id.clone(),
                before,
                main: *mains[0].1,
                after,
            });
        }
    }
    Ok(groups
        .into_values()
        .filter(|shells| {
            shells
                .iter()
                .map(|s| &observations[&s.observation].route)
                .collect::<BTreeSet<_>>()
                .len()
                >= 2
        })
        .map(|occurrences| Layout {
            status: "shared_shell_candidate".into(),
            occurrences,
        })
        .collect())
}

fn navigation(
    observations: &BTreeMap<String, Observation>,
    routes: &BTreeMap<String, Route>,
) -> Result<Vec<Navigation>> {
    let mut out = Vec::new();
    for (id, observation) in observations {
        let base = Url::parse(&observation.capture.metadata.base_url)?;
        for (node, item) in observation.capture.nodes.iter().enumerate() {
            if let evidence::Content::Element {
                tag, attributes, ..
            } = &item.content
                && matches!(tag.as_str(), "a" | "area")
            {
                for attr in attributes
                    .iter()
                    .filter(|a| a.name == "href" && a.namespace.is_none())
                {
                    let resolved = base.join(&attr.value).ok();
                    let route = resolved
                        .as_ref()
                        .filter(|u| matches!(u.scheme(), "http" | "https"))
                        .map(|u| route_url(u.as_str()).map(|u| fingerprint(u.as_str().as_bytes())))
                        .transpose()?
                        .filter(|id| routes.contains_key(id));
                    out.push(Navigation {
                        source: SourceRef {
                            observation: id.clone(),
                            node,
                        },
                        href: attr.value.clone(),
                        resolved: resolved.map(String::from),
                        route,
                    });
                }
            }
        }
    }
    Ok(out)
}

fn collect_assets(
    root: &Path,
    observations: &BTreeMap<String, Observation>,
) -> Result<BTreeMap<String, Asset>> {
    let site = util::checked_path(root, &root.join("site"))?;
    let mut out: BTreeMap<String, Asset> = BTreeMap::new();
    for observation in observations.values() {
        let base = Url::parse(&observation.capture.metadata.base_url)?;
        let mut urls = Vec::new();
        for resource in &observation.capture.resources {
            urls.push((
                base.join(&resource.url)?,
                format!("resource:{}", resource.initiator),
            ));
        }
        for sheet in &observation.capture.stylesheets {
            if let Some(href) = &sheet.href {
                urls.push((base.join(href)?, "stylesheet".into()));
            }
            let sheet_base = Url::parse(&sheet.base_url)?;
            for rule in &sheet.rules {
                urls.extend(
                    mirror::refs(rule, &sheet_base)
                        .into_iter()
                        .map(|u| (u, "css_reference".into())),
                );
            }
        }
        for node in &observation.capture.nodes {
            if let evidence::Content::Element {
                tag, attributes, ..
            } = &node.content
            {
                for attr in attributes
                    .iter()
                    .filter(|a| matches!(a.name.as_str(), "src" | "poster"))
                {
                    if let Ok(url) = base.join(&attr.value) {
                        urls.push((url, format!("attribute:{tag}:{}", attr.name)));
                    }
                }
            }
        }
        for (mut url, usage) in urls {
            url.set_fragment(None);
            let key = url.to_string();
            out.entry(key.clone())
                .or_insert_with(|| Asset {
                    url: key,
                    usages: BTreeSet::new(),
                    status: "unresolved".into(),
                    local_path: None,
                    local_sha256: None,
                    bytes: None,
                })
                .usages
                .insert(usage);
            ensure!(out.len() <= 20_000, "project asset count limit exceeded");
        }
    }
    let origins: Vec<_> = observations
        .values()
        .map(|o| Url::parse(&o.capture.url))
        .collect::<std::result::Result<_, _>>()?;
    let mut paths: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (key, asset) in &mut out {
        let url = Url::parse(key)?;
        if asset.usages.iter().all(|s| {
            matches!(
                s.as_str(),
                "resource:script" | "resource:fetch" | "resource:xmlhttprequest"
            )
        }) {
            asset.status = "runtime_observation".into();
            continue;
        }
        if !matches!(url.scheme(), "http" | "https") {
            asset.status = "inline_or_unsupported".into();
            continue;
        }
        if !origins.iter().any(|origin| origin.origin() == url.origin()) {
            asset.status = "external".into();
            continue;
        }
        let path = mirror::output_path(&site, &url, false)?;
        let relative = path.strip_prefix(root)?.to_string_lossy().into_owned();
        paths.entry(relative).or_default().push(key.clone());
    }
    let mut total = 0;
    for (relative, urls) in paths {
        // The mirror drops query strings. Even a single query-bearing URL lacks
        // an authoritative URL-to-file receipt, so never certify it as mapped.
        let ambiguous = urls.len() > 1
            || urls
                .iter()
                .any(|u| Url::parse(u).is_ok_and(|u| u.query().is_some()));
        let path = util::checked_path(root, &root.join(&relative))?;
        let bytes = if !ambiguous && path.is_file() {
            let file = std::fs::File::open(&path)?;
            let len = file.metadata()?.len();
            ensure!(
                len <= mirror::MAX_BODY && total + len <= ASSET_BYTES,
                "project local asset byte budget exceeded"
            );
            let mut bytes = Vec::new();
            file.take(mirror::MAX_BODY + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 == len,
                "local asset changed while reading"
            );
            total += len;
            Some((fingerprint(&bytes), len))
        } else {
            None
        };
        for url in urls {
            let asset = out.get_mut(&url).context("asset missing")?;
            if ambiguous {
                asset.status = "ambiguous_mirror_path".into();
            } else if let Some((digest, len)) = &bytes {
                asset.status = "local_candidate".into();
                asset.local_path = Some(relative.clone());
                asset.local_sha256 = Some(digest.clone());
                asset.bytes = Some(*len);
            }
        }
    }
    Ok(out)
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: u64,
}
struct ByteCounter(u64);
impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > self.0 {
            return Err(std::io::Error::other("project source byte budget exceeded"));
        }
        self.0 -= bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len() as u64 + bytes.len() as u64 > self.limit {
            return Err(std::io::Error::other("project output byte budget exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Build and atomically save `reconstruction/project.json`, without network or AI calls.
/// Incomplete or changed evidence leaves any previous project untouched.
pub fn build(root: &Path) -> Result<Project> {
    let project = derive(root)?;
    let root = root.canonicalize()?;
    let path = util::checked_path(&root, &root.join("reconstruction/project.json"))?;
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        limit: OUTPUT_BYTES,
    };
    serde_json::to_writer(&mut writer, &project)?;
    if path.exists() {
        util::atomic_replace(&path, &writer.bytes)?;
    } else {
        let mut file =
            tempfile::NamedTempFile::new_in(path.parent().context("project parent missing")?)?;
        file.write_all(&writer.bytes)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(&path)?;
    }
    Ok(project)
}

/// Derive and verify a project without modifying source captures or saved reports.
pub fn derive(root: &Path) -> Result<Project> {
    let root = root.canonicalize()?;
    let inventory = evidence::inventory(&root)?;
    ensure!(
        inventory.complete == inventory.captures.len(),
        "source evidence is incomplete"
    );
    let mut total = 0;
    for item in &inventory.captures {
        let path = util::checked_path(&root, &root.join(&item.snapshot))?;
        total += path.metadata()?.len();
        ensure!(total <= INPUT_BYTES, "project source byte budget exceeded");
    }
    let mut observations = BTreeMap::new();
    let mut routes: BTreeMap<String, Route> = BTreeMap::new();
    let mut styles = BTreeMap::new();
    for item in &inventory.captures {
        let (capture, digest) = evidence::load_with_digest(&root, Path::new(&item.snapshot))?;
        ensure!(
            digest == item.sha256,
            "source changed while building project"
        );
        capture.require_complete()?;
        let url = route_url(&capture.url)?;
        let route = fingerprint(url.as_str().as_bytes());
        let id = hash(&(&item.snapshot, &digest))?;
        routes
            .entry(route.clone())
            .or_insert_with(|| Route {
                url: url.to_string(),
                pathname: url.path().into(),
                query: url.query().map(str::to_owned),
                observations: Vec::new(),
            })
            .observations
            .push(id.clone());
        let mut style_ids = Vec::new();
        for style in &capture.styles {
            let id = hash(style)?;
            styles.entry(id.clone()).or_insert_with(|| style.clone());
            style_ids.push(id);
        }
        observations.insert(
            id,
            Observation {
                snapshot: item.snapshot.clone(),
                sha256: digest,
                route,
                capture,
                style_ids,
            },
        );
    }
    let trees = observations
        .iter()
        .map(|(id, o)| Ok((id.clone(), tree(o)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    let components = components(&observations, &trees)?;
    let layouts = layouts(&observations, &trees)?;
    let navigation = navigation(&observations, &routes)?;
    let assets = collect_assets(&root, &observations)?;
    let input_sha256 = input_hash(&observations, &assets)?;
    let project=Project{schema_version:1,algorithm:"mochi-source-model-v1".into(),input_sha256,routes,observations,styles,components,layouts,navigation,assets,
        notes:vec!["Literal URLs only; query order is preserved; no inferred dynamic routes or router nesting.".into(),
        "Structural candidates retain full source references and style variants; behavior is unverified.".into(),
        "CSSOM, fonts and custom properties remain in observations; token names and hooks are not invented.".into(),
        "Local asset hashes describe candidate bytes, not proof of origin URL identity. CSS URL discovery uses the mirror's conservative parser; declarations remain authoritative.".into(),
        format!("At most {MAX_COMPONENTS} non-overlapping component candidates; all other nodes remain inline evidence.")]};
    project.validate()?;
    // Recheck saved evidence and candidate bytes immediately before publication.
    let current = evidence::inventory(&root)?;
    ensure!(
        hash(&current)? == hash(&inventory)?,
        "source inventory changed while building project"
    );
    ensure!(
        hash(&collect_assets(&root, &project.observations)?)? == hash(&project.assets)?,
        "local assets changed while building project"
    );
    Ok(project)
}

fn input_hash(
    observations: &BTreeMap<String, Observation>,
    assets: &BTreeMap<String, Asset>,
) -> Result<String> {
    let mut inputs = observations
        .values()
        .map(|o| Ok((&o.snapshot, &o.sha256, hash(&o.capture)?)))
        .collect::<Result<Vec<_>>>()?;
    inputs.sort();
    hash(&(inputs, assets))
}

impl Project {
    /// Validate evidence and derived references before a model is consumed.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.algorithm == "mochi-source-model-v1",
            "unsupported project model"
        );
        ensure!(
            !self.observations.is_empty() && self.observations.len() <= 1000,
            "invalid observation count"
        );
        ensure!(
            self.routes.len() <= self.observations.len()
                && self.components.len() <= MAX_COMPONENTS
                && self.assets.len() <= 20_000,
            "project count limit exceeded"
        );
        let mut nodes = 0;
        let mut input_budget = ByteCounter(INPUT_BYTES);
        let mut expected_styles = BTreeSet::new();
        for (id, observation) in &self.observations {
            observation.capture.require_complete()?;
            serde_json::to_writer(&mut input_budget, &observation.capture)?;
            nodes += observation.capture.nodes.len();
            ensure!(nodes <= 250_000, "project total node budget exceeded");
            ensure!(
                observation.snapshot.starts_with("reconstruction/pages/")
                    && observation.snapshot.ends_with(".json")
                    && observation.snapshot.len() <= 256
                    && !observation.snapshot.contains(['\\', ':'])
                    && Path::new(&observation.snapshot)
                        .components()
                        .all(|c| matches!(c, std::path::Component::Normal(_)))
                    && observation.sha256.len() == 64
                    && observation.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid source identity"
            );
            ensure!(
                *id == hash(&(&observation.snapshot, &observation.sha256))?,
                "invalid observation identity"
            );
            let url = route_url(&observation.capture.url)?;
            ensure!(
                observation.route == fingerprint(url.as_str().as_bytes()),
                "invalid observation route"
            );
            let route = self
                .routes
                .get(&observation.route)
                .context("missing route")?;
            ensure!(
                route.url == url.as_str()
                    && route.pathname == url.path()
                    && route.query.as_deref() == url.query()
                    && route.observations.contains(id),
                "invalid route observation"
            );
            ensure!(
                observation.style_ids.len() == observation.capture.styles.len(),
                "invalid style map"
            );
            for (style, id) in observation
                .capture
                .styles
                .iter()
                .zip(&observation.style_ids)
            {
                expected_styles.insert(id);
                ensure!(
                    *id == hash(style)? && self.styles.get(id) == Some(style),
                    "invalid shared style"
                );
            }
        }
        ensure!(
            expected_styles.len() == self.styles.len(),
            "unreferenced shared styles"
        );
        for (id, route) in &self.routes {
            ensure!(!route.observations.is_empty(), "empty route");
            ensure!(
                route.observations.iter().collect::<BTreeSet<_>>().len()
                    == route.observations.len(),
                "duplicate route observations"
            );
            for observation in &route.observations {
                ensure!(
                    self.observations
                        .get(observation)
                        .is_some_and(|o| &o.route == id),
                    "dangling route observation"
                );
            }
        }
        let trees = self
            .observations
            .iter()
            .map(|(id, o)| Ok((id.clone(), tree(o)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        // Regenerate bounded structural decisions, including prop values and
        // non-overlap, so edited or invalid model references cannot be consumed.
        ensure!(
            hash(&self.components)? == hash(&components(&self.observations, &trees)?)?,
            "invalid component model"
        );
        ensure!(
            hash(&self.layouts)? == hash(&layouts(&self.observations, &trees)?)?,
            "invalid layout model"
        );
        ensure!(
            hash(&self.navigation)? == hash(&navigation(&self.observations, &self.routes)?)?,
            "invalid navigation model"
        );
        for (url, asset) in &self.assets {
            ensure!(
                url == &asset.url && Url::parse(url).is_ok(),
                "invalid asset URL"
            );
            ensure!(
                !asset.usages.is_empty()
                    && matches!(
                        asset.status.as_str(),
                        "local_candidate"
                            | "unresolved"
                            | "external"
                            | "ambiguous_mirror_path"
                            | "runtime_observation"
                            | "inline_or_unsupported"
                    ),
                "invalid asset status"
            );
            ensure!(
                (asset.status == "local_candidate") == asset.local_path.is_some(),
                "asset path/status mismatch"
            );
            if let Some(path) = &asset.local_path {
                ensure!(
                    Path::new(path).starts_with("site")
                        && Path::new(path).components().count() > 1
                        && !path.contains(['\\', ':'])
                        && !Path::new(path).is_absolute()
                        && Path::new(path)
                            .components()
                            .all(|c| matches!(c, std::path::Component::Normal(_))),
                    "invalid local asset path"
                );
                ensure!(
                    asset.status == "local_candidate"
                        && asset.local_sha256.as_ref().is_some_and(
                            |s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
                        )
                        && asset.bytes.is_some_and(|n| n <= mirror::MAX_BODY),
                    "invalid local asset candidate"
                );
            } else {
                ensure!(
                    asset.local_sha256.is_none() && asset.bytes.is_none(),
                    "unmapped asset has local bytes"
                );
            }
        }
        ensure!(
            self.input_sha256 == input_hash(&self.observations, &self.assets)?,
            "invalid project input fingerprint"
        );
        Ok(())
    }
}
