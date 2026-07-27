use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, bail};
use wepub_core::firefox::{self, Application, Channel, Compatibility};

use crate::cli::{FirefoxApplicationArg, FirefoxArgs, FirefoxChannelArg};

use super::input::{read_binary_input, resolve_text_input};

pub(crate) async fn run(args: FirefoxArgs) -> Result<()> {
    if is_stdin_path(args.approval_notes_file.as_deref())
        && is_stdin_path(args.release_notes_file.as_deref())
    {
        bail!("--approval-notes-file and --release-notes-file cannot both read from stdin (\"-\")");
    }

    let package = read_binary_input(&args.package, "package").await?;

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

    firefox::publish()
        .addon_id(args.addon_id)
        .api_key(args.api_key)
        .api_secret(args.api_secret)
        .package(package)
        .channel(args.channel.into())
        .maybe_compatibility(build_compatibility(&args.compatibility))
        .maybe_approval_notes(approval_notes)
        .maybe_release_notes(release_notes)
        .maybe_source(source)
        .maybe_root_url(args.internal_root_url)
        .call()
        .await?;

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
