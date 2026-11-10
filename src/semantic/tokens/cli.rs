//! `mochi tokens`: tokenize the styles of one capture into a central styles file.
//! Never changes the default export; the output directory must not exist.

use super::pipeline::{self, Output};
use super::plan::Options;
use anyhow::Result;
use clap::Args as ClapArgs;
use std::path::PathBuf;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// A capture directory (the one holding `manifest.json`, `site/` and `reconstruction/`).
    pub capture: PathBuf,
    /// Output directory (must not exist): `src/styles/tokens.css`, `src/styles/rewritten/`,
    /// `tokens.manifest.json`, `tokens.report.{json,md}` and `tokens.equality-harness.js`.
    #[arg(short, long)]
    pub out: PathBuf,
    /// Declarations a value must appear in before it gets a new token.
    #[arg(long, default_value_t = 2)]
    pub min_uses: usize,
    /// Keep px and rem lengths apart (default: merge them when the root font size is constant).
    #[arg(long)]
    pub no_merge_rem_px: bool,
    /// Write the output even when the static equality check found differences (the report
    /// status is then `static_differences_found`).
    #[arg(long)]
    pub allow_differences: bool,
}

pub fn summary(output: &Output) -> String {
    let r = &output.report;
    let s = &r.summary;
    let (mut compared, mut equal, mut different) = (0, 0, 0);
    for c in &r.rewrite_equality {
        compared += c.compared;
        equal += c.equal;
        different += c.different;
    }
    format!(
        "{} tokens ({} site-named, {} derived); {} site variables hoisted, {} left in place; {} replacements; {} eligible values left unnamed.\nStatic equality over {} snapshots: {} of {} property comparisons equal, {} different (status: {}).",
        s.tokens,
        s.by_origin.get("site").copied().unwrap_or(0),
        s.by_origin.get("derived").copied().unwrap_or(0),
        s.hoisted_site_variables,
        s.site_variables_kept_in_place,
        s.replacements,
        s.unnamed_eligible_values,
        r.rewrite_equality.len(),
        equal,
        compared,
        different,
        r.status
    )
}

pub fn run(args: Args) -> Result<()> {
    anyhow::ensure!(
        !args.out.exists(),
        "{} already exists; choose a new directory",
        args.out.display()
    );
    let options = Options {
        min_uses: args.min_uses,
        merge_rem_px: !args.no_merge_rem_px,
    };
    let output = pipeline::run(&args.capture, &options)?;
    let differences: usize = output
        .report
        .rewrite_equality
        .iter()
        .map(|c| c.different)
        .sum();
    let vacuous = output.report.status == "static_check_vacuous";
    if vacuous && !args.allow_differences {
        anyhow::bail!(
            "the static equality check compared nothing for at least one snapshot, so it proves nothing; nothing was written (pass --allow-differences to write anyway)"
        );
    }
    if differences > 0 && !args.allow_differences {
        for c in &output.report.rewrite_equality {
            for d in c.samples.iter().take(5) {
                eprintln!(
                    "{} {} {:?} {}: {} -> {}",
                    d.snapshot, d.element, d.pseudo, d.property, d.before, d.after
                );
            }
        }
        anyhow::bail!(
            "{differences} property comparisons differ after the rewrite; nothing was written (pass --allow-differences to write the output and report anyway)"
        );
    }
    pipeline::write_output(&output, &args.out)?;
    println!("{}", summary(&output));
    println!("wrote {}", args.out.display());
    Ok(())
}
