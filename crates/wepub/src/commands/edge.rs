use std::path::PathBuf;

use anyhow::{Context, Result};
use wepub_core::edge::{Client, PublishOptions};

use crate::cli::EdgeArgs;
use crate::commands::common::read_text_input;

pub async fn run(args: EdgeArgs) -> Result<()> {
    let zip = tokio::fs::read(&args.zip)
        .await
        .with_context(|| format!("failed to read archive from {}", args.zip.display()))?;

    let notes = load_notes(args.notes, args.notes_file).await?;

    let client = Client::new(args.product_id, args.client_id, args.api_key)?;

    let options = PublishOptions {
        notes,
        ..PublishOptions::new()
    };

    client
        .publish(zip, options)
        .await
        .context("Edge Add-ons publish failed")?;
    Ok(())
}

async fn load_notes(notes: Option<String>, notes_file: Option<PathBuf>) -> Result<Option<String>> {
    match (notes, notes_file) {
        (Some(text), _) => Ok(Some(text)),
        (_, Some(path)) => {
            Ok(Some(read_text_input(&path).await.with_context(|| {
                format!("failed to read notes from {}", path.display())
            })?))
        }
        _ => Ok(None),
    }
}
