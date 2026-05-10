use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokio::io::AsyncReadExt;
use wepub_core::firefox::{Application, Channel, Compatibility, FirefoxStore, PublishOptions};

use crate::cli::{ApplicationArg, ChannelArg, FirefoxArgs};

const RELEASE_NOTES_LOCALE: &str = "en-US";

pub async fn run(args: FirefoxArgs) -> Result<()> {
    if is_stdin_path(args.release_notes_file.as_deref())
        && is_stdin_path(args.approval_notes_file.as_deref())
    {
        bail!(
            "--release-notes-file and --approval-notes-file cannot both read from stdin (\"-\"); \
             stdin is a single stream"
        );
    }

    let zip = tokio::fs::read(&args.zip)
        .await
        .with_context(|| format!("failed to read archive from {}", args.zip.display()))?;

    let release_notes = load_release_notes(&args).await?;
    let approval_notes = load_approval_notes(&args).await?;
    let source = match &args.source {
        Some(path) => Some(
            tokio::fs::read(path)
                .await
                .with_context(|| format!("failed to read source from {}", path.display()))?,
        ),
        None => None,
    };

    let mut store =
        FirefoxStore::from_jwt_credentials(args.addon_id, args.api_key, args.api_secret)
            .context("failed to construct FirefoxStore")?;
    if let Some(base_url) = args.amo_base_url {
        store = store.with_base_url(base_url);
    }

    let options = PublishOptions {
        channel: args.channel.into(),
        compatibility: build_compatibility(&args.compatibility),
        release_notes,
        approval_notes,
        source,
        ..PublishOptions::default()
    };

    store
        .publish(zip, options)
        .await
        .context("AMO publish failed")?;
    Ok(())
}

fn is_stdin_path(path: Option<&Path>) -> bool {
    path.is_some_and(|p| p.as_os_str() == "-")
}

async fn load_release_notes(args: &FirefoxArgs) -> Result<HashMap<String, String>> {
    let text =
        match (&args.release_notes, &args.release_notes_file) {
            (Some(text), _) => Some(text.clone()),
            (_, Some(path)) => Some(read_text_input(path).await.with_context(|| {
                format!("failed to read release notes from {}", path.display())
            })?),
            _ => None,
        };
    Ok(text.map_or_else(HashMap::new, |t| {
        HashMap::from([(RELEASE_NOTES_LOCALE.to_string(), t)])
    }))
}

async fn load_approval_notes(args: &FirefoxArgs) -> Result<Option<String>> {
    match (&args.approval_notes, &args.approval_notes_file) {
        (Some(text), _) => Ok(Some(text.clone())),
        (_, Some(path)) => Ok(Some(read_text_input(path).await.with_context(|| {
            format!("failed to read approval notes from {}", path.display())
        })?)),
        _ => Ok(None),
    }
}

async fn read_text_input(path: &PathBuf) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut buf = String::new();
        tokio::io::stdin().read_to_string(&mut buf).await?;
        Ok(buf)
    } else {
        Ok(tokio::fs::read_to_string(path).await?)
    }
}

fn build_compatibility(apps: &[ApplicationArg]) -> Option<Compatibility> {
    if apps.is_empty() {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<Application> = apps
        .iter()
        .copied()
        .filter(|app| seen.insert(*app))
        .map(Into::into)
        .collect();
    Some(Compatibility::Apps(unique))
}

impl From<ChannelArg> for Channel {
    fn from(value: ChannelArg) -> Self {
        match value {
            ChannelArg::Listed => Channel::Listed,
            ChannelArg::Unlisted => Channel::Unlisted,
        }
    }
}

impl From<ApplicationArg> for Application {
    fn from(value: ApplicationArg) -> Self {
        match value {
            ApplicationArg::Firefox => Application::Firefox,
            ApplicationArg::Android => Application::Android,
        }
    }
}
