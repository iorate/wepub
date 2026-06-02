use anyhow::{Context, Result};
use wepub_core::edge::{Client, Credentials, Progress, PublishOptions};

use crate::cli::EdgeArgs;
use crate::commands::common::{read_binary_input, resolve_text_input};

pub async fn run(args: EdgeArgs, quiet: bool) -> Result<()> {
    let client = Client::new(
        args.product_id,
        Credentials {
            client_id: args.client_id,
            api_key: args.api_key,
        },
    )?;

    let zip = read_binary_input(&args.zip, "package").await?;

    let notes = resolve_text_input(args.notes, args.notes_file.as_deref(), "notes").await?;
    let options = PublishOptions { notes };

    client
        .publish(zip, options, |progress| report(progress, quiet))
        .await
        .context("Edge Add-ons")?;
    Ok(())
}

fn report(progress: Progress, quiet: bool) {
    if quiet {
        return;
    }
    match progress {
        Progress::Uploading => eprintln!("Uploading the package archive..."),
        Progress::PollingUpload => eprintln!("Waiting for the upload to be processed..."),
        Progress::Publishing => eprintln!("Publishing the draft..."),
        Progress::PollingPublish => eprintln!("Waiting for the draft to be published..."),
        Progress::Succeeded => eprintln!("Published to Edge Add-ons."),
        _ => {}
    }
}
