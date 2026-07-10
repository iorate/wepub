use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, bail};
use wepub_core::firefox::{self, Application, Channel, Compatibility, Credentials};

use crate::cli::{FirefoxApplicationArg, FirefoxArgs, FirefoxChannelArg};
use crate::commands::common::{read_binary_input, resolve_text_input};

pub(crate) async fn run(args: FirefoxArgs) -> Result<()> {
    if is_stdin_path(args.approval_notes_file.as_deref())
        && is_stdin_path(args.release_notes_file.as_deref())
    {
        bail!("--approval-notes-file and --release-notes-file cannot both read from stdin (\"-\")");
    }

    let credentials = Credentials {
        api_key: args.api_key,
        api_secret: args.api_secret,
    };

    let zip = read_binary_input(&args.zip, "package").await?;

    let channel: Channel = args.channel.into();

    let mut publish = firefox::publish(args.addon_id, credentials, zip, channel);
    if let Some(root_url) = args.internal_root_url {
        publish = publish.root_url(root_url);
    }
    if let Some(compatibility) = build_compatibility(&args.compatibility) {
        publish = publish.compatibility(compatibility);
    }
    if let Some(approval_notes) = resolve_text_input(
        args.approval_notes,
        args.approval_notes_file.as_deref(),
        "approval notes",
    )
    .await?
    {
        publish = publish.approval_notes(approval_notes);
    }
    if let Some(release_notes) = resolve_text_input(
        args.release_notes,
        args.release_notes_file.as_deref(),
        "release notes",
    )
    .await?
    .map(|text| HashMap::from([(args.release_notes_lang, text)]))
    {
        publish = publish.release_notes(release_notes);
    }
    if let Some(path) = &args.source {
        publish = publish.source(read_binary_input(path, "source").await?);
    }

    publish.await?;

    Ok(())
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
