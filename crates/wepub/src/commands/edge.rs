use anyhow::Result;
use wepub_core::edge::{self, Credentials};

use crate::cli::EdgeArgs;
use crate::commands::common::{read_binary_input, resolve_text_input};

pub(crate) async fn run(args: EdgeArgs) -> Result<()> {
    let credentials = Credentials {
        client_id: args.client_id,
        api_key: args.api_key,
    };

    let zip = read_binary_input(&args.zip, "package").await?;

    let mut publish = edge::publish(args.product_id, credentials, zip);
    if let Some(notes) = resolve_text_input(args.notes, args.notes_file.as_deref(), "notes").await?
    {
        publish = publish.notes(notes);
    }

    publish.await?;

    Ok(())
}
