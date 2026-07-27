use anyhow::Result;
use wepub_core::edge;

use crate::cli::EdgeArgs;

use super::input::{read_binary_input, resolve_text_input};

pub(crate) async fn run(args: EdgeArgs) -> Result<()> {
    let package = read_binary_input(&args.package, "package").await?;

    let notes = resolve_text_input(args.notes, args.notes_file.as_deref(), "notes").await?;

    edge::publish()
        .product_id(args.product_id)
        .client_id(args.client_id)
        .api_key(args.api_key)
        .package(package)
        .maybe_notes(notes)
        .call()
        .await?;

    Ok(())
}
