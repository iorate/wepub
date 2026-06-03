use anyhow::{Context, Result, bail};
use wepub_core::chrome::{Client, Credentials, Progress, PublishOptions, PublishType};

use crate::cli::{ChromeArgs, ChromePublishTypeArg};
use crate::commands::common::read_binary_input;

pub async fn run(args: ChromeArgs, no_progress: bool) -> Result<()> {
    let client = build_client(
        args.publisher_id,
        args.item_id,
        args.client_id,
        args.client_secret,
        args.refresh_token,
        args.access_token,
    )?;

    let zip = read_binary_input(&args.zip, "package").await?;

    let options = PublishOptions {
        publish_type: args.publish_type.map(Into::into),
        deploy_percentage: args.deploy_percentage,
        skip_review: args.skip_review,
    };

    let on_progress: fn(Progress) = if no_progress { |_| {} } else { report };

    client
        .publish(zip, options, on_progress)
        .await
        .context("Chrome Web Store")?;

    Ok(())
}

impl From<ChromePublishTypeArg> for PublishType {
    fn from(value: ChromePublishTypeArg) -> Self {
        match value {
            ChromePublishTypeArg::Default => PublishType::DefaultPublish,
            ChromePublishTypeArg::Staged => PublishType::StagedPublish,
        }
    }
}

fn build_client(
    publisher_id: String,
    item_id: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    refresh_token: Option<String>,
    access_token: Option<String>,
) -> Result<Client> {
    let any_refresh_token_arg =
        client_id.is_some() || client_secret.is_some() || refresh_token.is_some();

    match (any_refresh_token_arg, access_token) {
        (true, Some(_)) => bail!(
            "--client-id / --client-secret / --refresh-token cannot be combined with --access-token"
        ),
        (false, None) => bail!(
            "missing credentials: pass either the trio \
             --client-id / --client-secret / --refresh-token or --access-token"
        ),
        (true, None) => {
            let (Some(client_id), Some(client_secret), Some(refresh_token)) =
                (client_id, client_secret, refresh_token)
            else {
                bail!(
                    "--client-id / --client-secret / --refresh-token must all be provided together"
                );
            };
            Ok(Client::new(
                publisher_id,
                item_id,
                Credentials::RefreshToken {
                    client_id,
                    client_secret,
                    refresh_token,
                },
            )?)
        }
        (false, Some(access_token)) => Ok(Client::new(
            publisher_id,
            item_id,
            Credentials::AccessToken(access_token),
        )?),
    }
}

fn report(progress: Progress) {
    match progress {
        Progress::StartUpload => eprintln!("Uploading the package archive..."),
        Progress::AwaitUpload => eprintln!("Waiting for the upload to be processed..."),
        Progress::Submit => eprintln!("Submitting the draft..."),
        _ => {}
    }
}
