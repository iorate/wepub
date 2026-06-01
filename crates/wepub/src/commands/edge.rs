use std::path::Path;

use anyhow::{Context, Result};
use wepub_core::edge::{Client, Credentials, Progress, PublishOptions};

use crate::cli::EdgeArgs;
use crate::commands::common::read_text_input;

pub async fn run(args: EdgeArgs, quiet: bool) -> Result<()> {
    let zip = tokio::fs::read(&args.zip)
        .await
        .with_context(|| format!("failed to read archive from {}", args.zip.display()))?;

    let notes = load_notes(args.notes, args.notes_file.as_deref()).await?;
    let options = PublishOptions { notes };

    let client = Client::new(
        args.product_id,
        Credentials {
            client_id: args.client_id,
            api_key: args.api_key,
        },
    )?;

    client
        .publish(zip, options, |progress| report(progress, quiet))
        .await
        .context("Edge Add-ons")?;
    Ok(())
}

async fn load_notes(notes: Option<String>, notes_file: Option<&Path>) -> Result<Option<String>> {
    match (notes, notes_file) {
        (Some(text), _) => Ok(Some(text)),
        (_, Some(path)) => {
            Ok(Some(read_text_input(path).await.with_context(|| {
                format!("failed to read notes from {}", path.display())
            })?))
        }
        _ => Ok(None),
    }
}

fn report(progress: Progress, quiet: bool) {
    if quiet {
        return;
    }
    match progress {
        Progress::Uploading => eprintln!("Uploading to Edge Add-ons..."),
        Progress::PollingUpload => eprintln!("Waiting for the upload to be processed..."),
        Progress::Publishing => eprintln!("Publishing..."),
        Progress::PollingPublish => eprintln!("Waiting for the submission to be published..."),
        Progress::Succeeded => eprintln!("Published to Edge Add-ons."),
    }
}
