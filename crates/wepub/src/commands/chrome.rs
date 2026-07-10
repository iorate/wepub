use anyhow::{Result, bail};
use wepub_core::chrome::{self, PublishType};

use crate::cli::{ChromeArgs, ChromePublishTypeArg};
use crate::commands::common::read_binary_input;

pub(crate) async fn run(args: ChromeArgs) -> Result<()> {
    let auth = build_auth(
        args.client_id,
        args.client_secret,
        args.refresh_token,
        args.access_token,
    )?;

    let package = read_binary_input(&args.package, "package").await?;

    let access_token = match auth {
        Auth::AccessToken(access_token) => access_token,
        Auth::RefreshToken {
            client_id,
            client_secret,
            refresh_token,
        } => {
            chrome::fetch_access_token()
                .client_id(client_id)
                .client_secret(client_secret)
                .refresh_token(refresh_token)
                .call()
                .await?
        }
    };

    chrome::publish()
        .publisher_id(args.publisher_id)
        .item_id(args.item_id)
        .access_token(access_token)
        .package(package)
        .maybe_publish_type(args.publish_type.map(Into::into))
        .maybe_deploy_percentage(args.deploy_percentage)
        .maybe_skip_review(args.skip_review)
        .call()
        .await?;

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

enum Auth {
    RefreshToken {
        client_id: String,
        client_secret: String,
        refresh_token: String,
    },
    AccessToken(String),
}

fn build_auth(
    client_id: Option<String>,
    client_secret: Option<String>,
    refresh_token: Option<String>,
    access_token: Option<String>,
) -> Result<Auth> {
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
            Ok(Auth::RefreshToken {
                client_id,
                client_secret,
                refresh_token,
            })
        }
        (false, Some(access_token)) => Ok(Auth::AccessToken(access_token)),
    }
}
