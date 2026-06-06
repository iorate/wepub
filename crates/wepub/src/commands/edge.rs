use anyhow::Result;
use wepub_core::edge::{Client, Credentials, PublishOptions};

use crate::cli::EdgeArgs;
use crate::commands::common::{read_binary_input, resolve_text_input};

pub(crate) async fn run(args: EdgeArgs) -> Result<()> {
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

    client.publish(zip, options).await?;

    Ok(())
}
