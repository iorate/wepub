use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use wepub_core::firefox::{
    Application, Channel, Client, Compatibility, Credentials, Progress, PublishOptions,
};

use crate::cli::{FirefoxApplicationArg, FirefoxArgs, FirefoxChannelArg};
use crate::commands::common::{read_binary_input, resolve_text_input};

pub async fn run(args: FirefoxArgs, quiet: bool) -> Result<()> {
    if is_stdin_path(args.approval_notes_file.as_deref())
        && is_stdin_path(args.release_notes_file.as_deref())
    {
        bail!("--approval-notes-file and --release-notes-file cannot both read from stdin (\"-\")");
    }

    let mut client = Client::new(
        args.addon_id,
        Credentials {
            api_key: args.api_key,
            api_secret: args.api_secret,
        },
    )?;
    if let Some(root_url) = args.internal_root_url {
        client = client.with_root_url(root_url.as_str())?;
    }

    let zip = read_binary_input(&args.zip, "package").await?;

    let channel: Channel = args.channel.into();

    let compatibility = build_compatibility(&args.compatibility);
    let approval_notes = resolve_text_input(
        args.approval_notes,
        args.approval_notes_file.as_deref(),
        "approval notes",
    )
    .await?;
    let release_notes = resolve_text_input(
        args.release_notes,
        args.release_notes_file.as_deref(),
        "release notes",
    )
    .await?
    .map(|text| HashMap::from([(args.release_notes_lang, text)]));
    let source = match &args.source {
        Some(path) => Some(read_binary_input(path, "source").await?),
        None => None,
    };
    let options = PublishOptions {
        compatibility,
        approval_notes,
        release_notes,
        source,
    };

    client
        .publish(zip, channel, options, |progress| report(progress, quiet))
        .await
        .context("Firefox Add-ons")?;
    Ok(())
}

fn is_stdin_path(path: Option<&Path>) -> bool {
    path.is_some_and(|p| p.as_os_str() == "-")
}

fn build_compatibility(apps: &[FirefoxApplicationArg]) -> Option<Compatibility> {
    if apps.is_empty() {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<Application> = apps
        .iter()
        .copied()
        .map(Into::into)
        .filter(|app| seen.insert(*app))
        .collect();
    Some(Compatibility::Shorthand(unique))
}

impl From<FirefoxChannelArg> for Channel {
    fn from(value: FirefoxChannelArg) -> Self {
        match value {
            FirefoxChannelArg::Listed => Channel::Listed,
            FirefoxChannelArg::Unlisted => Channel::Unlisted,
        }
    }
}

impl From<FirefoxApplicationArg> for Application {
    fn from(value: FirefoxApplicationArg) -> Self {
        match value {
            FirefoxApplicationArg::Firefox => Application::Firefox,
            FirefoxApplicationArg::Android => Application::Android,
        }
    }
}

fn report(progress: Progress, quiet: bool) {
    if quiet {
        return;
    }
    match progress {
        Progress::Uploading => eprintln!("Uploading to Firefox Add-ons..."),
        Progress::PollingValidation => eprintln!("Waiting for validation..."),
        Progress::CreatingVersion => eprintln!("Creating the new version..."),
        Progress::UploadingSource => eprintln!("Uploading the source archive..."),
        Progress::Succeeded => eprintln!("Published to Firefox Add-ons."),
    }
}
