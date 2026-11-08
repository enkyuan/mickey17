//! Semantic source synthesis from a bounded, evidence-bound program. No model
//! calls, bundle loading, or guessed backend behavior. Custom algorithms are
//! explicit reviewed typed modules with a validated import graph.
mod assets;
mod emit;
mod names;
mod schema;
mod source;
mod validate;
use crate::{cache::fingerprint, project, util};
use anyhow::{Context, Result, ensure};
pub use schema::*;
use serde::Serialize;
use std::{collections::BTreeMap, io::Read, path::Path};

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub source_quality: String,
    pub input_sha256: String,
    pub specification_sha256: String,
    pub routes: usize,
    pub components: usize,
    pub component_paths: BTreeMap<String, String>,
    pub recovered_modules: usize,
    pub source_modules: BTreeMap<String, String>,
    pub source_assets: BTreeMap<String, String>,
    pub gaps: Vec<String>,
    pub verification: String,
}
pub(crate) fn read(path: &Path, max: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let file = std::fs::File::open(path)
        .with_context(|| format!("read authored input {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= max,
        "authored input exceeds byte budget"
    );
    file.take(max + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= max,
        "authored input changed or exceeds byte budget"
    );
    Ok(bytes)
}
/// Saves evidence for semantic synthesis. This does not invent component roles,
/// UI state, token meanings, or a complete authored specification.
pub fn plan(root: &Path, out: &Path) -> Result<()> {
    let model = project::derive(root)?;
    let mut controls = Vec::new();
    let mut headings = Vec::new();
    for (id, observation) in &model.observations {
        for (node, value) in observation.capture.nodes.iter().enumerate() {
            if let crate::evidence::Content::Element {
                tag, attributes, ..
            } = &value.content
            {
                if matches!(
                    tag.as_str(),
                    "button" | "input" | "select" | "textarea" | "dialog" | "nav"
                ) {
                    controls.push(serde_json::json!({"source":{"observation":id,"node":node},"tag":tag,"attributes":attributes.iter().filter(|a|matches!(a.name.as_str(),"role"|"aria-label"|"aria-controls"|"aria-expanded"|"type"|"name")).collect::<Vec<_>>()}));
                }
                if matches!(tag.as_str(), "h1" | "h2" | "h3") {
                    headings.push(
                        serde_json::json!({"source":{"observation":id,"node":node},"tag":tag}),
                    );
                }
            }
        }
    }
    let text = serde_json::to_vec_pretty(
        &serde_json::json!({"schema_version":1,"input_sha256":model.input_sha256,"status":"semantic_decisions_required","routes":model.routes,"structural_candidates":model.components,"headings":headings,"controls":controls,"instructions":"Produce an authored Specification. Bind components to source references; infer state only from observed interactions or inspected source. Specify typed props, direct Base UI parts, scoped tokens and local fonts. Record unsupported behavior in gaps. Never mark this evidence plan as an authored app."}),
    )?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)?;
    file.write_all(&text)?;
    Ok(())
}

pub fn generate(root: &Path, specification: &Path, out: &Path) -> Result<Report> {
    let root = root.canonicalize()?;
    let bytes = read(specification, 8 * 1024 * 1024)?;
    let spec: Specification =
        serde_json::from_slice(&bytes).context("parse authored specification")?;
    let model = project::derive(&root)?;
    validate::specification(&spec, &model)?;
    let source_assets = assets::load(specification, &spec, &model)?;
    let sources = source::load(specification, &spec, &model)?;
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .context("authored output parent must already exist")?;
    let destination = util::checked_path(
        &parent,
        &parent.join(out.file_name().context("output directory name missing")?),
    )?;
    ensure!(
        destination
            .symlink_metadata()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "authored output already exists"
    );
    ensure!(
        !destination.starts_with(&root),
        "authored output cannot be inside capture"
    );
    let stage = tempfile::Builder::new()
        .prefix(".mochi-authored-")
        .tempdir_in(parent)?;
    let report = emit::project(
        &root,
        stage.path(),
        &spec,
        &model,
        &fingerprint(&bytes),
        &sources,
        &source_assets,
    )?;
    ensure!(
        read(specification, 8 * 1024 * 1024)? == bytes,
        "authored specification changed during export"
    );
    ensure!(
        project::derive(&root)?.input_sha256 == model.input_sha256,
        "authored evidence changed during export"
    );
    ensure!(
        source::load(specification, &spec, &model)? == sources,
        "owned source changed during export"
    );
    ensure!(
        assets::load(specification, &spec, &model)? == source_assets,
        "owned assets changed during export"
    );
    crate::export::publish(stage.path(), &destination)?;
    Ok(report)
}
