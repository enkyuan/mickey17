//! Stand-alone entry point for the token pipeline until `mochi tokens` is registered in
//! `src/main.rs`: `mochi-tokens <capture-dir> --out <new-dir>`.
use clap::Parser;
use mochi::semantic::tokens::cli::{Args, run};

#[derive(Debug, Parser)]
#[command(
    name = "mochi-tokens",
    about = "Tokenize captured styles into one central styles file"
)]
struct Cli {
    #[command(flatten)]
    args: Args,
}

fn main() -> anyhow::Result<()> {
    run(Cli::parse().args)
}
