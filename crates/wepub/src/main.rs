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

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!(
            "wepub_core={}",
            cli.verbosity.tracing_level_filter()
        ))
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();

    match dispatch(cli.command, cli.no_progress).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn dispatch(command: Commands, no_progress: bool) -> Result<()> {
    match command {
        Commands::Chrome(args) => commands::chrome::run(args, no_progress).await,
        Commands::Firefox(args) => commands::firefox::run(args, no_progress).await,
        Commands::Edge(args) => commands::edge::run(args, no_progress).await,
    }
}
