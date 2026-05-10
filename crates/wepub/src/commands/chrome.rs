use anyhow::{Context, Result, bail};
use wepub_core::chrome::{ChromeStore, PublishOptions, PublishType};

use crate::cli::{ChromeArgs, PublishTypeArg};

pub async fn run(args: ChromeArgs) -> Result<()> {
    let mut store = build_store(&args)?;
    if let Some(url) = args.cws_root_url {
        store = store.with_root_url(url);
    }
    if let Some(url) = args.cws_token_url {
        store = store.with_token_url(url);
    }

    let zip = tokio::fs::read(&args.zip)
        .await
        .with_context(|| format!("failed to read archive from {}", args.zip.display()))?;

    let options = PublishOptions {
        publish_type: args.publish_type.into(),
        skip_review: args.skip_review,
        deploy_percentage: args.deploy_percentage,
        ..PublishOptions::default()
    };

    store
        .publish(zip, options)
        .await
        .context("Chrome Web Store publish failed")?;
    Ok(())
}

fn build_store(args: &ChromeArgs) -> Result<ChromeStore> {
    let client_set = (
        args.client_id.as_deref(),
        args.client_secret.as_deref(),
        args.refresh_token.as_deref(),
    );
    let any_client_field =
        client_set.0.is_some() || client_set.1.is_some() || client_set.2.is_some();

    match (any_client_field, args.access_token.as_deref()) {
        (true, Some(_)) => bail!(
            "--access-token cannot be combined with --client-id / --client-secret / --refresh-token; \
             choose one authentication mode"
        ),
        (false, None) => bail!(
            "missing Chrome Web Store credentials: pass either --access-token or the trio \
             --client-id / --client-secret / --refresh-token"
        ),
        (true, None) => {
            let (Some(client_id), Some(client_secret), Some(refresh_token)) = client_set else {
                bail!(
                    "--client-id, --client-secret and --refresh-token must all be provided together"
                );
            };
            Ok(ChromeStore::from_client_credentials(
                args.publisher_id.clone(),
                args.item_id.clone(),
                client_id.to_string(),
                client_secret.to_string(),
                refresh_token.to_string(),
            )?)
        }
        (false, Some(access_token)) => Ok(ChromeStore::from_access_token(
            args.publisher_id.clone(),
            args.item_id.clone(),
            access_token.to_string(),
        )?),
    }
}

impl From<PublishTypeArg> for PublishType {
    fn from(value: PublishTypeArg) -> Self {
        match value {
            PublishTypeArg::Default => PublishType::Default,
            PublishTypeArg::Staged => PublishType::Staged,
        }
    }
}
