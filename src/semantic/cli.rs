//! `mochi semantic`: analyze one captured page and write the report and, with
//! `--emit`, the Base UI based TSX project fragment. Never changes the default
//! export output; the output directory must not exist.
use super::{analyze_page, emit, report};
use anyhow::{Context, Result, bail, ensure};
use clap::Args as ClapArgs;
use std::path::PathBuf;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// A capture directory (the one holding `manifest.json`, `site/` and `reconstruction/`).
    pub capture: PathBuf,
    /// Output directory (must not exist): `report.json`, `report.md` and, with --emit, `project/`.
    #[arg(short, long)]
    pub out: PathBuf,
    /// Page index in the capture manifest.
    #[arg(long, default_value_t = 0)]
    pub page: usize,
    /// Viewport of the page evidence to use, e.g. `1440x900` (default: the widest captured).
    #[arg(long)]
    pub viewport: Option<String>,
    /// Do not read the original script bundles (DOM and ARIA evidence only).
    #[arg(long)]
    pub no_js: bool,
    /// Also write the TSX project fragment (`components/ui`, compositions, data).
    #[arg(long)]
    pub emit: bool,
    /// Assemble a complete TanStack Start project from every captured route (routes, root
    /// document, head, components, data, assets and fonts with provenance, provisional styles).
    #[arg(long, conflicts_with_all = ["emit", "no_js"])]
    pub assemble: bool,
    /// With --assemble: do not run the tokens pipeline (the project then has no stylesheet).
    #[arg(long, requires = "assemble")]
    pub no_tokens: bool,
    /// With --assemble: assemble at most this many routes.
    #[arg(long, requires = "assemble")]
    pub routes: Option<usize>,
}

fn page_evidence(args: &Args) -> Result<PathBuf> {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(args.capture.join("manifest.json"))
            .context("reading capture manifest.json")?,
    )?;
    let pages = manifest["pages"]
        .as_array()
        .context("manifest has no pages")?;
    let page = pages
        .get(args.page)
        .with_context(|| format!("no page {} in the manifest", args.page))?;
    let evidence: Vec<&str> = page["source_evidence"]
        .as_array()
        .context("page has no source_evidence")?
        .iter()
        .filter_map(|e| e.as_str())
        .collect();
    let size = |p: &str| -> Option<(u32, u32)> {
        let stem = p.rsplit('/').next()?.strip_suffix(".json")?;
        let (w, h) = stem.rsplit_once('-')?.1.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    };
    let chosen = match &args.viewport {
        Some(v) => evidence
            .iter()
            .find(|p| p.ends_with(&format!("-{v}.json")))
            .with_context(|| format!("no evidence for viewport {v}: {evidence:?}"))?,
        None => evidence
            .iter()
            .max_by_key(|p| size(p).map(|s| s.0))
            .context("page has no evidence files")?,
    };
    Ok(PathBuf::from(chosen))
}

fn ensure_safe(path: &str) -> Result<()> {
    use std::path::{Component, Path};
    if path.is_empty()
        || !Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        bail!("refusing to write {path}");
    }
    Ok(())
}

fn write_project(args: &Args) -> Result<()> {
    let project = super::project::assemble(
        &args.capture,
        &super::project::Options {
            viewport: args.viewport.as_deref(),
            routes: args.routes,
            tokens: !args.no_tokens,
        },
    )?;
    // Validate every path before touching the disk: a legitimate name such as `v1..2.png` is
    // fine, a parent or absolute component is not, and a refusal must not leave a half-written tree.
    for path in project.files.keys() {
        ensure_safe(path)?;
    }
    std::fs::create_dir_all(&args.out)?;
    let root = args.out.join("project");
    for (path, bytes) in &project.files {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap())?;
        std::fs::write(target, bytes)?;
    }
    let r = &project.report;
    println!(
        "Assembled {} route(s), {} files, {} assets ({} not copied), {} gap entries. Project: {}. Report: {}",
        r.routes.len(),
        r.files,
        r.assets.copied,
        r.assets.skipped.len(),
        r.gaps.len(),
        root.display(),
        root.join(".mochi/semantic/project.report.md").display()
    );
    Ok(())
}

pub fn run(args: Args) -> Result<()> {
    ensure!(
        !args.out.exists(),
        "output directory {} already exists",
        args.out.display()
    );
    if args.assemble {
        return write_project(&args);
    }
    let page = page_evidence(&args)?;
    let analysis = analyze_page(&args.capture, &page, !args.no_js)?;
    let emitted = if args.emit {
        emit::emit(&analysis, &emit::EmitOptions::default())
    } else {
        Default::default()
    };
    let rep = report::build(&analysis, emitted);
    let md = report::markdown(&analysis, &rep);
    for path in rep.emitted.files.keys() {
        ensure_safe(path)?;
    }
    std::fs::create_dir_all(&args.out)?;
    std::fs::write(
        args.out.join("report.json"),
        serde_json::to_vec_pretty(&rep)?,
    )?;
    std::fs::write(args.out.join("report.md"), md)?;
    for (path, text) in &rep.emitted.files {
        let target = args.out.join("project").join(path);
        std::fs::create_dir_all(target.parent().unwrap())?;
        std::fs::write(target, text)?;
    }
    let s = &rep.summary;
    println!(
        "{} controls: {} bound to a primitive ({} not Button), {} native links, {} unrecognized; {} patterns; {} components emitted. Report: {}",
        s.controls,
        s.recognized_controls,
        s.non_button_primitive_controls,
        s.native_links,
        s.unrecognized_controls,
        s.patterns,
        s.emitted_components,
        args.out.join("report.md").display()
    );
    Ok(())
}
