use anyhow::Result;
use clap::{Parser, Subcommand};
use mochi::{
    analysis, authored, crawl, edit, evidence, export, interpret, lift, parity, project, serve,
    trackers, util,
};
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
    /// Lift closed functions from a recovered controller into a typed TypeScript module
    /// with per-function provenance. Refuses functions that capture outside state.
    Lift {
        spec: PathBuf,
        /// New TypeScript module path; must not exist.
        #[arg(long)]
        out: PathBuf,
        /// New provenance manifest path; must not exist.
        #[arg(long)]
        manifest: PathBuf,
    },
    /// List function bindings in a recovered controller and whether each can be lifted.
    LiftSurvey {
        source: PathBuf,
        #[arg(long, requires = "end_line")]
        start_line: Option<usize>,
        #[arg(long, requires = "start_line")]
        end_line: Option<usize>,
    },
    /// Prepare evidence for a reviewed semantic source specification, without model calls.
    PlanSource {
        site: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Save bounded Jev request payloads locally; sends nothing to a provider.
    PlanInterpretation {
        site: PathBuf,
        #[arg(long, default_value_t = 64)]
        max_candidates: usize,
        #[arg(long, default_value_t = 8)]
        max_text_props: usize,
        #[arg(long, default_value_t = 3)]
        examples: usize,
        #[arg(long, default_value_t = 4)]
        batch_candidates: usize,
    },
    /// Replay semantic proposals offline, or explicitly enable budgeted live TypeSafe calls.
    Interpret {
        site: PathBuf,
        #[arg(long, conflicts_with = "replay")]
        live: bool,
        #[arg(long, conflicts_with = "live")]
        replay: bool,
        #[arg(long, value_enum, default_value = "typesafe")]
        provider: InterpreterProvider,
        #[arg(long, requires = "live")]
        max_requests: Option<u32>,
        #[arg(long, requires = "live")]
        max_input_tokens: Option<u64>,
        #[arg(long, requires = "live")]
        max_cost_usd: Option<String>,
        #[arg(long, requires = "live")]
        input_price_per_million_usd: Option<String>,
        #[arg(long, requires = "live")]
        deadline_seconds: Option<u64>,
        #[arg(long, requires = "live")]
        max_retries: Option<u8>,
    },
    /// Generate a new editable TanStack Start project from a saved source model.
    Export {
        site: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, value_enum, default_value = "tanstack-start")]
        framework: Framework,
        /// Generate typed source from a reviewed semantic specification, without recovered bundles.
        #[arg(long, conflicts_with_all = ["interpretation", "recover_client_routes"])]
        authored_spec: Option<PathBuf>,
        /// Explicitly export structural compatibility source, which can contain recovered controllers.
        #[arg(long, conflicts_with = "authored_spec")]
        compatibility: bool,
        /// Attach saved semantic proposals as review-required comments; never calls AI.
        #[arg(long)]
        interpretation: bool,
        /// Recover a captured public client root for this literal path as editable code.
        /// The generated app executes recovered code; unsupported modules are refused.
        #[arg(long = "recover-client-route")]
        recover_client_routes: Vec<String>,
    },
    /// Hash editable app inputs before collecting candidate parity measurements.
    FingerprintProject { project: PathBuf },
    /// Compare fixed browser states; fail on stale input, gaps or measured differences.
    CheckParity {
        project: PathBuf,
        #[arg(long)]
        contract: PathBuf,
        #[arg(long)]
        reference: PathBuf,
        #[arg(long)]
        candidate: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
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
#[derive(Clone, clap::ValueEnum)]
enum Framework {
    TanstackStart,
}
#[derive(Clone, clap::ValueEnum)]
enum InterpreterProvider {
    Typesafe,
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
        Command::FingerprintProject { project } => {
            println!("{}", parity::project_fingerprint(&project)?);
        }
        Command::CheckParity {
            project,
            contract,
            reference,
            candidate,
            out,
        } => {
            parity::check(&project, &contract, &reference, &candidate, &out)?;
            println!(
                "Matched declared parity contract; scope and input hashes: {}",
                out.display()
            );
        }
        Command::Lift {
            spec,
            out,
            manifest,
        } => {
            let report = lift::lift(&spec, &out, &manifest)?;
            println!(
                "Lifted {} functions to {}; provenance in {}; {} functions in the requested region remain unlifted.",
                report.functions.len(),
                out.display(),
                manifest.display(),
                report.unlifted.len()
            );
        }
        Command::LiftSurvey {
            source,
            start_line,
            end_line,
        } => {
            let region = start_line
                .zip(end_line)
                .map(|(start_line, end_line)| lift::Region {
                    start_line,
                    end_line,
                });
            println!(
                "{}",
                serde_json::to_string_pretty(&lift::survey(&source, region)?)?
            );
        }
        Command::PlanSource { site, out } => {
            authored::plan(&site, &out)?;
            println!(
                "Saved semantic evidence plan to {}. Authored decisions and qualification remain required.",
                out.display()
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
        Command::PlanInterpretation {
            site,
            max_candidates,
            max_text_props,
            examples,
            batch_candidates,
        } => {
            let plan = interpret::save_plan(
                &site,
                interpret::PlanConfig {
                    max_candidates,
                    max_text_props,
                    examples,
                    batch_candidates,
                },
            )?;
            println!(
                "Planned {} bounded requests for {} ({} skipped items). No provider calls made; inspect reconstruction/interpretation/plan.json before enabling live interpretation.",
                plan.entries.len(),
                interpret::MODEL,
                plan.skipped.len()
            );
        }
        Command::Interpret {
            site,
            live,
            replay: _,
            provider: InterpreterProvider::Typesafe,
            max_requests,
            max_input_tokens,
            max_cost_usd,
            input_price_per_million_usd,
            deadline_seconds,
            max_retries,
        } => {
            use anyhow::Context;
            let outcome = if live {
                let limits = interpret::Limits {
                    max_requests: max_requests.context("--max-requests is required for --live")?,
                    max_input_tokens: max_input_tokens
                        .context("--max-input-tokens is required for --live")?,
                    max_cost_microusd: interpret::usd_micros(
                        &max_cost_usd.context("--max-cost-usd is required for --live")?,
                    )?,
                    input_price_microusd_per_million: interpret::usd_micros(
                        &input_price_per_million_usd.context(
                            "--input-price-per-million-usd is required for live cost accounting",
                        )?,
                    )?,
                    deadline_seconds: deadline_seconds
                        .context("--deadline-seconds is required for --live")?,
                    max_retries: max_retries.unwrap_or(0),
                };
                limits.validate()?;
                let key=std::env::var("TYPESAFE_API_KEY").map_err(|_| anyhow::anyhow!("configure TYPESAFE_API_KEY locally for --live; never pass the key as a CLI argument"))?;
                interpret::interpret_live(&site, &interpret::TypeSafe::new(&key)?, limits)?
            } else {
                interpret::replay(&site)?
            };
            let proposals = outcome
                .report
                .decisions
                .iter()
                .filter(|d| d.status == interpret::Status::ProposalNeedsReview)
                .count();
            let abstentions = outcome
                .report
                .decisions
                .iter()
                .filter(|d| d.status == interpret::Status::Abstained)
                .count();
            println!(
                "Saved {proposals} review-required semantic proposals and {abstentions} abstentions; {} provider attempts, {} cache hits. Calibration unqualified; no changes applied. Journal: {}",
                outcome.summary.attempts,
                outcome.summary.cache_hits,
                outcome.journal.display()
            );
        }
        Command::Export {
            site,
            out,
            framework: Framework::TanstackStart,
            authored_spec,
            compatibility,
            interpretation,
            recover_client_routes,
        } => {
            if let Some(specification) = authored_spec {
                let report = authored::generate(&site, &specification, &out)?;
                println!(
                    "Generated authored candidate: {} routes, {} typed components, no recovered modules. Verification not run; see .mochi/authored-report.json and EDITING.md.",
                    report.routes, report.components
                );
                return Ok(());
            }
            anyhow::ensure!(
                compatibility,
                "Editable source requires --authored-spec <spec.json>. Run plan-source to prepare evidence for semantic authoring. Structural replay is available only with explicit --compatibility; it is not a semantic reconstruction."
            );
            let report = if !recover_client_routes.is_empty() {
                export::tanstack_start_with_clients(
                    &site,
                    &out,
                    interpretation,
                    &recover_client_routes.into_iter().collect(),
                )?
            } else if interpretation {
                export::tanstack_start_with_interpretation(&site, &out)?
            } else {
                export::tanstack_start(&site, &out)?
            };
            println!(
                "Generated compatibility source: {} routes, {} structural components and {} shared layouts in {}. Runtime verification not run; see .mochi/export.json and EDITING.md.",
                report.routes,
                report.components,
                report.layouts,
                out.display()
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
