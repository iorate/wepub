mod cli;
mod commands;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Commands};

#[tokio::main]
async fn main() -> ExitCode {
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet);

    match dispatch(cli.command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn dispatch(command: Commands) -> Result<()> {
    match command {
        Commands::Firefox(args) => commands::firefox::run(args).await,
        Commands::Chrome(args) => commands::chrome::run(args).await,
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
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!("wepub={default_level},wepub_core={default_level}"))
    });

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();
}
