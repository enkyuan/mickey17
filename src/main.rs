use anyhow::Result;
use clap::{Parser, Subcommand};
use mochi::{analysis, crawl, edit, evidence, project, serve, trackers, util};
use std::path::PathBuf;
#[derive(Parser)]
#[command(
    name = "mochi",
    version,
    about = "Mirror websites with Chromium; edit and serve them with Rust"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Render a website in Chromium and mirror its HTML and assets.
    Clone {
        url: String,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 200)]
        max_pages: usize,
        #[arg(long)]
        extract: bool,
        #[arg(long)]
        keep_trackers: bool,
        #[arg(long)]
        ignore_robots: bool,
        /// Capture rendered DOM and report structural reuse candidates.
        #[arg(long)]
        analyze: bool,
        /// Concurrent asset downloads (1-16); page visits remain sequential.
        #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16))]
        jobs: u8,
        /// Revalidate and reuse public asset bodies in this shared cache directory.
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// Preserve richer text, CSS, fonts, and viewport screenshots for future source export.
        #[arg(long)]
        source_evidence: bool,
        /// Explicit source capture sizes; defaults to 375x812,768x1024,1440x900.
        #[arg(long, value_delimiter = ',', requires = "source_evidence")]
        source_viewports: Vec<evidence::Viewport>,
    },
    /// Recompute structural analysis from a clone captured with --analyze, without network access.
    Analyze { site: PathBuf },
    /// Validate saved source evidence and screenshot integrity offline; fail if incomplete.
    CheckEvidence { site: PathBuf },
    /// Derive an offline source model from complete evidence; does not emit an app.
    ModelProject { site: PathBuf },
    /// Preview a clone on a loopback HTTP server.
    Serve {
        site: PathBuf,
        #[arg(short, long, default_value_t = 8000)]
        port: u16,
    },
    /// Replace text while preserving URLs and React flight lengths.
    Replace {
        site: PathBuf,
        old: String,
        new: String,
        #[arg(long)]
        exact_case: bool,
        #[arg(long)]
        substring: bool,
    },
    /// Point absolute URLs at a new origin.
    Rehost {
        site: PathBuf,
        old_origin: String,
        new_origin: String,
    },
    /// Remove recognized analytics tags and script loaders.
    StripTrackers { site: PathBuf },
}
fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Clone {
            url,
            out,
            max_pages,
            extract,
            keep_trackers,
            ignore_robots,
            analyze,
            jobs,
            cache_dir,
            source_evidence,
            source_viewports,
        } => {
            let out = out.unwrap_or_else(|| {
                url::Url::parse(&if url.contains("://") {
                    url.clone()
                } else {
                    format!("https://{url}")
                })
                .ok()
                .and_then(|u| {
                    u.host_str()
                        .map(|s| PathBuf::from(s.trim_start_matches("www.")))
                })
                .unwrap_or_else(|| "clone".into())
            });
            let options = crawl::Options {
                max_pages,
                extract,
                keep_trackers,
                ignore_robots,
            };
            let pipeline = crawl::PipelineOptions {
                analyze,
                jobs: jobs.into(),
                cache_dir,
            };
            let report = if source_evidence {
                let source = if source_viewports.is_empty() {
                    evidence::CaptureOptions::default()
                } else {
                    evidence::CaptureOptions {
                        viewports: source_viewports,
                    }
                };
                crawl::clone_with_evidence(&url, &out, &options, &pipeline, &source)?
            } else {
                crawl::clone_with_options(&url, &out, &options, &pipeline)?
            };
            println!(
                "Saved {} pages and {} assets to {} ({} failures; see manifest.json)",
                report.saved_pages,
                report.assets_fetched,
                out.display(),
                report.failures.len()
            );
        }
        Command::Analyze { site } => {
            let report = analysis::analyze(&site)?;
            println!(
                "Analyzed {} nodes: {} templates, {} component candidates (analysis/report.json)",
                report.nodes_analyzed,
                report.templates.len(),
                report.component_candidates.len()
            );
        }
        Command::CheckEvidence { site } => {
            let report = evidence::inspect(&site)?;
            println!(
                "Source evidence: {}/{} captures complete (reconstruction/report.json)",
                report.complete,
                report.captures.len()
            );
            anyhow::ensure!(
                report.complete == report.captures.len(),
                "source evidence is incomplete; inspect reconstruction/report.json"
            );
        }
        Command::ModelProject { site } => {
            let project = project::build(&site)?;
            println!(
                "Modeled {} literal routes, {} observations, {} component candidates, {} shared shells (reconstruction/project.json)",
                project.routes.len(),
                project.observations.len(),
                project.components.len(),
                project.layouts.len()
            );
        }
        Command::Serve { site, port } => {
            tokio::runtime::Runtime::new()?.block_on(serve::serve(site, port))?
        }
        Command::Replace {
            site,
            old,
            new,
            exact_case,
            substring,
        } => {
            let r = edit::Replacer::new(&old, &new, !exact_case, !substring)?;
            println!(
                "Changed {} files",
                edit::replace(&util::site_dir(&site), &r)?
            );
        }
        Command::Rehost {
            site,
            old_origin,
            new_origin,
        } => println!(
            "Changed {} files",
            edit::rehost(&util::site_dir(&site), &old_origin, &new_origin)?
        ),
        Command::StripTrackers { site } => {
            println!("Changed {} files", trackers::strip(&util::site_dir(&site))?)
        }
    }
    Ok(())
}
