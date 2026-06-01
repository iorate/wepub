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
    /// Show additional debug logs.
    #[arg(short = 'v', long = "verbose", global = true)]
    pub verbose: bool,

    /// Show only warnings and errors.
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

    /// Publisher ID (UUID).
    #[arg(long, env = "WEPUB_CHROME_PUBLISHER_ID")]
    pub publisher_id: String,

    /// Item ID (32-character string).
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

    /// Pre-fetched OAuth access token.
    #[arg(long, env = "WEPUB_CHROME_ACCESS_TOKEN")]
    pub access_token: Option<String>,

    /// Whether to publish immediately on approval or stage for later publishing.
    #[arg(long, value_enum)]
    pub publish_type: Option<ChromePublishTypeArg>,

    /// Initial deploy percentage (0-100). Omit to use the Developer Dashboard default.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=100))]
    pub deploy_percentage: Option<u8>,

    /// Attempt to skip item review.
    #[arg(long)]
    pub skip_review: Option<bool>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ChromePublishTypeArg {
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
    /// Path to the add-on archive (zip).
    #[arg(value_name = "ZIP")]
    pub zip: PathBuf,

    /// Add-on ID (slug or GUID).
    #[arg(long, env = "WEPUB_FIREFOX_ADDON_ID")]
    pub addon_id: String,

    /// API key (JWT issuer).
    #[arg(long, env = "WEPUB_FIREFOX_API_KEY")]
    pub api_key: String,

    /// API secret (JWT secret).
    #[arg(long, env = "WEPUB_FIREFOX_API_SECRET")]
    pub api_secret: String,

    /// Version channel. Determines visibility on the site.
    #[arg(long, value_enum)]
    pub channel: FirefoxChannelArg,

    /// Compatible applications, comma-separated (e.g. "firefox,android").
    #[arg(long, value_delimiter = ',')]
    pub compatibility: Vec<FirefoxApplicationArg>,

    /// Information for Mozilla reviewers. Mutually exclusive with --approval-notes-file.
    #[arg(long)]
    pub approval_notes: Option<String>,

    /// Path to a file containing approval notes. Use "-" for stdin.
    #[arg(long, value_name = "PATH")]
    pub approval_notes_file: Option<PathBuf>,

    /// Release notes. Mutually exclusive with --release-notes-file.
    #[arg(long)]
    pub release_notes: Option<String>,

    /// Path to a file containing release notes. Use "-" for stdin.
    #[arg(long, value_name = "PATH")]
    pub release_notes_file: Option<PathBuf>,

    /// Locale code for the release notes (e.g. "en-US", "ja").
    #[arg(
        long,
        value_name = "LANG",
        default_value = "en-US",
        requires = "release_notes_input"
    )]
    pub release_notes_lang: String,

    /// Path to a source archive to attach to the version.
    #[arg(long, value_name = "PATH")]
    pub source: Option<PathBuf>,

    #[arg(long, hide = true)]
    pub internal_root_url: Option<Url>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FirefoxChannelArg {
    Listed,
    Unlisted,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FirefoxApplicationArg {
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
    /// Path to the add-on archive (zip).
    #[arg(value_name = "ZIP")]
    pub zip: PathBuf,

    /// Product ID (GUID).
    #[arg(long, env = "WEPUB_EDGE_PRODUCT_ID")]
    pub product_id: String,

    /// Client ID.
    #[arg(long, env = "WEPUB_EDGE_CLIENT_ID")]
    pub client_id: String,

    /// API key.
    #[arg(long, env = "WEPUB_EDGE_API_KEY")]
    pub api_key: String,

    /// Notes for certification. Mutually exclusive with --notes-file.
    #[arg(long)]
    pub notes: Option<String>,

    /// Path to a file containing notes for certification. Use "-" for stdin.
    #[arg(long, value_name = "PATH")]
    pub notes_file: Option<PathBuf>,
}
