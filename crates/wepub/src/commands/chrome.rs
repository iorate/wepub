use anyhow::{Context, Result, bail};
use wepub_core::chrome::{Client, Credentials, Progress, PublishOptions, PublishType};

use crate::cli::{ChromeArgs, ChromePublishTypeArg};

pub async fn run(args: ChromeArgs, quiet: bool) -> Result<()> {
    let zip = tokio::fs::read(&args.zip)
        .await
        .with_context(|| format!("failed to read archive from {}", args.zip.display()))?;

    let options = PublishOptions {
        publish_type: args.publish_type.map(Into::into),
        deploy_percentage: args.deploy_percentage,
        skip_review: args.skip_review,
    };

    let client = build_client(args)?;

    client
        .publish(zip, options, |progress| report(progress, quiet))
        .await
        .context("Chrome Web Store")?;
    Ok(())
}

fn build_client(args: ChromeArgs) -> Result<Client> {
    let any_refresh_token_arg =
        args.client_id.is_some() || args.client_secret.is_some() || args.refresh_token.is_some();

    match (any_refresh_token_arg, args.access_token) {
        (true, Some(_)) => bail!(
            "--access-token cannot be combined with --client-id / --client-secret / --refresh-token"
        ),
        (false, None) => bail!(
            "missing credentials: pass either --access-token or the trio \
             --client-id / --client-secret / --refresh-token"
        ),
        (true, None) => {
            let (Some(client_id), Some(client_secret), Some(refresh_token)) =
                (args.client_id, args.client_secret, args.refresh_token)
            else {
                bail!(
                    "--client-id / --client-secret / --refresh-token must all be provided together"
                );
            };
            Ok(Client::new(
                args.publisher_id,
                args.item_id,
                Credentials::RefreshToken {
                    client_id,
                    client_secret,
                    refresh_token,
                },
            )?)
        }
        (false, Some(access_token)) => Ok(Client::new(
            args.publisher_id,
            args.item_id,
            Credentials::AccessToken(access_token),
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
