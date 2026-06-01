mod cli;
mod commands;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Commands};

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(err) = dotenvy::dotenv()
        && !err.not_found()
    {
        eprintln!("warning: failed to load .env: {err}");
    }

    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet);
    match dispatch(cli.command, cli.quiet).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_tracing(verbose: bool, quiet: bool) {
    let default_level = if quiet {
        "warn"
    } else if verbose {
        "debug"
    } else {
        "info"
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("wepub_core={default_level}")));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();
}

async fn dispatch(command: Commands, quiet: bool) -> Result<()> {
    match command {
        Commands::Chrome(args) => commands::chrome::run(args, quiet).await,
        Commands::Firefox(args) => commands::firefox::run(args, quiet).await,
        Commands::Edge(args) => commands::edge::run(args, quiet).await,
    }
}
