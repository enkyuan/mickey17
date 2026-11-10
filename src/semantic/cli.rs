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

pub fn run(args: Args) -> Result<()> {
    ensure!(
        !args.out.exists(),
        "output directory {} already exists",
        args.out.display()
    );
    let page = page_evidence(&args)?;
    let analysis = analyze_page(&args.capture, &page, !args.no_js)?;
    let emitted = if args.emit {
        emit::emit(&analysis, &emit::EmitOptions::default())
    } else {
        Default::default()
    };
    let rep = report::build(&analysis, emitted);
    let md = report::markdown(&analysis, &rep);
    std::fs::create_dir_all(&args.out)?;
    std::fs::write(
        args.out.join("report.json"),
        serde_json::to_vec_pretty(&rep)?,
    )?;
    std::fs::write(args.out.join("report.md"), md)?;
    for (path, text) in &rep.emitted.files {
        if path.contains("..") || path.starts_with('/') {
            bail!("refusing to write {path}");
        }
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
