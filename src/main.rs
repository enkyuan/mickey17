use anyhow::Result;
use clap::{Parser, Subcommand};
use perfectclone::{crawl, edit, serve, trackers, util};
use std::path::PathBuf;
#[derive(Parser)]
#[command(
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
fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Clone {
            url,
            out,
            max_pages,
            extract,
            keep_trackers,
            ignore_robots,
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
            let report = crawl::clone_site(
                &url,
                &out,
                &crawl::Options {
                    max_pages,
                    extract,
                    keep_trackers,
                    ignore_robots,
                },
            )?;
            println!(
                "Saved {} pages and {} assets to {} ({} failures; see manifest.json)",
                report.saved_pages,
                report.assets_fetched,
                out.display(),
                report.failures.len()
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
