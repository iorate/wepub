use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use url::Url;

#[derive(Debug, Parser)]
#[command(
    name = "wepub",
    version,
    about = "Publish browser extensions to web stores"
)]
pub struct Cli {
    /// Show debug-level logs.
    #[arg(short = 'v', long = "verbose", global = true)]
    pub verbose: bool,

    /// Suppress non-warning logs.
    #[arg(short = 'q', long = "quiet", global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Firefox (AMO) commands.
    Firefox {
        #[command(subcommand)]
        command: FirefoxCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum FirefoxCommands {
    /// Upload a zip and create a new version on AMO.
    Publish(FirefoxPublishArgs),
}

#[derive(Debug, Args)]
#[command(group(
    clap::ArgGroup::new("release_notes_input")
        .multiple(false)
        .args(["release_notes", "release_notes_file"]),
))]
#[command(group(
    clap::ArgGroup::new("approval_notes_input")
        .multiple(false)
        .args(["approval_notes", "approval_notes_file"]),
))]
pub struct FirefoxPublishArgs {
    /// Path to the extension archive (zip).
    #[arg(value_name = "ZIP")]
    pub zip: PathBuf,

    /// Add-on ID (e.g. "myaddon@example.com").
    #[arg(long, env = "WEPUB_FIREFOX_ADDON_ID")]
    pub addon_id: String,

    /// Distribution channel.
    #[arg(long, value_enum, default_value_t = ChannelArg::Listed)]
    pub channel: ChannelArg,

    /// AMO API key (JWT issuer).
    #[arg(long, env = "WEPUB_FIREFOX_API_KEY")]
    pub api_key: String,

    /// AMO API secret (JWT secret).
    #[arg(long, env = "WEPUB_FIREFOX_API_SECRET")]
    pub api_secret: String,

    /// Override the AMO API base URL (for local addons-server etc.).
    #[arg(long, env = "WEPUB_FIREFOX_AMO_BASE_URL")]
    pub amo_base_url: Option<Url>,

    /// Compatible applications, comma-separated (e.g. "firefox,android").
    #[arg(long, value_delimiter = ',')]
    pub compatibility: Vec<ApplicationArg>,

    /// Release notes (en-US). Mutually exclusive with --release-notes-file.
    #[arg(long)]
    pub release_notes: Option<String>,

    /// Path to a file containing en-US release notes. Use "-" for stdin.
    #[arg(long, value_name = "PATH")]
    pub release_notes_file: Option<PathBuf>,

    /// Approval notes for AMO reviewers. Mutually exclusive with --approval-notes-file.
    #[arg(long)]
    pub approval_notes: Option<String>,

    /// Path to a file containing approval notes. Use "-" for stdin.
    #[arg(long, value_name = "PATH")]
    pub approval_notes_file: Option<PathBuf>,

    /// Path to a source archive to attach to the version.
    #[arg(long, value_name = "PATH")]
    pub source: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ChannelArg {
    Listed,
    Unlisted,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ApplicationArg {
    Firefox,
    Android,
}
