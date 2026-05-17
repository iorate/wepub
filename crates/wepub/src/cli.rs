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
    /// Upload a zip and submit a new version to Chrome Web Store.
    Chrome(ChromeArgs),
    /// Upload a zip and submit a new version to Firefox Add-ons.
    Firefox(FirefoxArgs),
    /// Upload a zip and submit a new version to Edge Add-ons.
    Edge(EdgeArgs),
}

#[derive(Debug, Args)]
pub struct ChromeArgs {
    /// Path to the extension archive (zip).
    #[arg(value_name = "ZIP")]
    pub zip: PathBuf,

    /// Publisher ID under which the item is registered.
    #[arg(long, env = "WEPUB_CHROME_PUBLISHER_ID")]
    pub publisher_id: String,

    /// Item ID (the 32-character extension ID).
    #[arg(long, env = "WEPUB_CHROME_ITEM_ID")]
    pub item_id: String,

    /// OAuth client ID. Use together with --client-secret and --refresh-token.
    #[arg(long, env = "WEPUB_CHROME_CLIENT_ID")]
    pub client_id: Option<String>,

    /// OAuth client secret. Use together with --client-id and --refresh-token.
    #[arg(long, env = "WEPUB_CHROME_CLIENT_SECRET")]
    pub client_secret: Option<String>,

    /// OAuth refresh token. Use together with --client-id and --client-secret.
    #[arg(long, env = "WEPUB_CHROME_REFRESH_TOKEN")]
    pub refresh_token: Option<String>,

    /// Pre-fetched OAuth access token (escape hatch for WIF / gcloud).
    #[arg(long, env = "WEPUB_CHROME_ACCESS_TOKEN")]
    pub access_token: Option<String>,

    /// Publish type.
    #[arg(long, value_enum, default_value_t = PublishTypeArg::Default)]
    pub publish_type: PublishTypeArg,

    /// Bypass the standard review queue (only honoured for changes Google deems eligible).
    #[arg(long)]
    pub skip_review: bool,

    /// Initial deploy percentage (0-100). Omit to use the Developer Dashboard default.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=100))]
    pub deploy_percentage: Option<u8>,

    /// Override the Chrome Web Store API root URL (for testing).
    #[arg(long, env = "WEPUB_CHROME_TEST_ROOT_URL")]
    pub test_root_url: Option<Url>,

    /// Override the OAuth token endpoint URL (for testing).
    #[arg(long, env = "WEPUB_CHROME_TEST_TOKEN_URL")]
    pub test_token_url: Option<Url>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PublishTypeArg {
    Default,
    Staged,
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
pub struct FirefoxArgs {
    /// Path to the extension archive (zip).
    #[arg(value_name = "ZIP")]
    pub zip: PathBuf,

    /// Add-on ID (e.g. "myaddon@example.com").
    #[arg(long, env = "WEPUB_FIREFOX_ADDON_ID")]
    pub addon_id: String,

    /// Distribution channel.
    #[arg(long, value_enum, default_value_t = ChannelArg::Listed)]
    pub channel: ChannelArg,

    /// Firefox Add-ons API key (JWT issuer).
    #[arg(long, env = "WEPUB_FIREFOX_API_KEY")]
    pub api_key: String,

    /// Firefox Add-ons API secret (JWT secret).
    #[arg(long, env = "WEPUB_FIREFOX_API_SECRET")]
    pub api_secret: String,

    /// Override the Firefox Add-ons API root URL (for local addons-server etc.).
    #[arg(long, env = "WEPUB_FIREFOX_TEST_ROOT_URL")]
    pub test_root_url: Option<Url>,

    /// Compatible applications, comma-separated (e.g. "firefox,android").
    #[arg(long, value_delimiter = ',')]
    pub compatibility: Vec<ApplicationArg>,

    /// Release notes (en-US). Mutually exclusive with --release-notes-file.
    #[arg(long)]
    pub release_notes: Option<String>,

    /// Path to a file containing en-US release notes. Use "-" for stdin.
    #[arg(long, value_name = "PATH")]
    pub release_notes_file: Option<PathBuf>,

    /// Approval notes for Firefox Add-ons reviewers. Mutually exclusive with --approval-notes-file.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum ApplicationArg {
    Firefox,
    Android,
}

#[derive(Debug, Args)]
#[command(group(
    clap::ArgGroup::new("edge_notes_input")
        .multiple(false)
        .args(["notes", "notes_file"]),
))]
pub struct EdgeArgs {
    /// Path to the extension archive (zip).
    #[arg(value_name = "ZIP")]
    pub zip: PathBuf,

    /// Edge Add-ons product ID (GUID).
    #[arg(long, env = "WEPUB_EDGE_PRODUCT_ID")]
    pub product_id: String,

    /// Partner Center Client ID.
    #[arg(long, env = "WEPUB_EDGE_CLIENT_ID")]
    pub client_id: String,

    /// Partner Center API key.
    #[arg(long, env = "WEPUB_EDGE_API_KEY")]
    pub api_key: String,

    /// Notes for the Edge Add-ons certification team. Mutually exclusive with --notes-file.
    #[arg(long)]
    pub notes: Option<String>,

    /// Path to a file containing certification notes. Use "-" for stdin.
    #[arg(long, value_name = "PATH")]
    pub notes_file: Option<PathBuf>,

    /// Override the Edge Add-ons API root URL (for testing).
    #[arg(long, env = "WEPUB_EDGE_TEST_ROOT_URL")]
    pub test_root_url: Option<Url>,
}
