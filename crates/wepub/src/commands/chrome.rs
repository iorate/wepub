use anyhow::{Context, Result, bail};
use wepub_core::chrome::{Client, Credentials, Progress, PublishOptions, PublishType};

use crate::cli::{ChromeArgs, ChromePublishTypeArg};

pub async fn run(args: ChromeArgs, quiet: bool) -> Result<()> {
    let client = build_client(&args)?;

    let zip = tokio::fs::read(&args.zip)
        .await
        .with_context(|| format!("failed to read archive from {}", args.zip.display()))?;

    let options = PublishOptions {
        publish_type: args.publish_type.map(Into::into),
        skip_review: args.skip_review,
        deploy_percentage: args.deploy_percentage,
    };

    client
        .publish(zip, options, |progress| report(progress, quiet))
        .await
        .context("Chrome Web Store")?;
    Ok(())
}

fn report(progress: Progress, quiet: bool) {
    if quiet {
        return;
    }
    match progress {
        Progress::Uploading => eprintln!("Uploading to Chrome Web Store..."),
        Progress::PollingUpload => eprintln!("Waiting for the upload to be processed..."),
        Progress::Publishing => eprintln!("Publishing..."),
        Progress::Succeeded => eprintln!("Published to Chrome Web Store."),
    }
}

fn build_client(args: &ChromeArgs) -> Result<Client> {
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
            Ok(Client::new(
                args.publisher_id.clone(),
                args.item_id.clone(),
                Credentials::RefreshToken {
                    client_id: client_id.to_string(),
                    client_secret: client_secret.to_string(),
                    refresh_token: refresh_token.to_string(),
                },
            )?)
        }
        (false, Some(access_token)) => Ok(Client::new(
            args.publisher_id.clone(),
            args.item_id.clone(),
            Credentials::AccessToken(access_token.to_string()),
        )?),
    }
}

impl From<ChromePublishTypeArg> for PublishType {
    fn from(value: ChromePublishTypeArg) -> Self {
        match value {
            ChromePublishTypeArg::Default => PublishType::DefaultPublish,
            ChromePublishTypeArg::Staged => PublishType::StagedPublish,
        }
    }
}
