use anyhow::{Result, bail};
use wepub_core::chrome::{self, Credentials, PublishType};

use crate::cli::{ChromeArgs, ChromePublishTypeArg};
use crate::commands::common::read_binary_input;

pub(crate) async fn run(args: ChromeArgs) -> Result<()> {
    let credentials = build_credentials(
        args.client_id,
        args.client_secret,
        args.refresh_token,
        args.access_token,
    )?;

    let zip = read_binary_input(&args.zip, "package").await?;

    let mut publish = chrome::publish(args.publisher_id, args.item_id, credentials, zip);
    if let Some(publish_type) = args.publish_type {
        publish = publish.publish_type(publish_type.into());
    }
    if let Some(deploy_percentage) = args.deploy_percentage {
        publish = publish.deploy_percentage(deploy_percentage);
    }
    if let Some(skip_review) = args.skip_review {
        publish = publish.skip_review(skip_review);
    }

    publish.await?;

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

fn build_credentials(
    client_id: Option<String>,
    client_secret: Option<String>,
    refresh_token: Option<String>,
    access_token: Option<String>,
) -> Result<Credentials> {
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
            Ok(Credentials::RefreshToken {
                client_id,
                client_secret,
                refresh_token,
            })
        }
        (false, Some(access_token)) => Ok(Credentials::AccessToken(access_token)),
    }
}
