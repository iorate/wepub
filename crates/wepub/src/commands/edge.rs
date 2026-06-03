use anyhow::{Context, Result};
use wepub_core::edge::{Client, Credentials, Progress, PublishOptions};

use crate::cli::EdgeArgs;
use crate::commands::common::{read_binary_input, resolve_text_input};

pub(crate) async fn run(args: EdgeArgs, no_progress: bool) -> Result<()> {
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

    let on_progress: fn(Progress) = if no_progress { |_| {} } else { report };

    client
        .publish(zip, options, on_progress)
        .await
        .context("Edge Add-ons")?;

    Ok(())
}

fn report(progress: Progress) {
    match progress {
        Progress::StartUpload => eprintln!("Uploading the package archive..."),
        Progress::AwaitUpload => eprintln!("Waiting for the upload to be processed..."),
        Progress::StartSubmit => eprintln!("Submitting the draft..."),
        Progress::AwaitSubmit => eprintln!("Waiting for the submission to be processed..."),
        _ => {}
    }
}
