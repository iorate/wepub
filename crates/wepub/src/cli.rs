use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use url::Url;

#[derive(Debug, Parser)]
#[command(
    name = "wepub",
    version,
    about = "Publish browser extensions to web stores"
)]
pub(crate) struct Cli {
    #[command(flatten)]
    #[command(next_help_heading = "Logging")]
    pub(crate) verbosity: Verbosity<InfoLevel>,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    /// Publish to Chrome Web Store.
    Chrome(ChromeArgs),
    /// Publish to Firefox Add-ons.
    Firefox(FirefoxArgs),
    /// Publish to Edge Add-ons.
    Edge(EdgeArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ChromeArgs {
    /// Path to the extension archive (zip).
    #[arg(value_name = "PACKAGE")]
    pub(crate) package: PathBuf,

    /// Publisher ID (UUID).
    #[arg(long, env = "WEPUB_CHROME_PUBLISHER_ID")]
    pub(crate) publisher_id: String,

    /// Item ID (32-character string).
    #[arg(long, env = "WEPUB_CHROME_ITEM_ID")]
    pub(crate) item_id: String,

    /// OAuth client ID. Use together with --client-secret and --refresh-token.
    #[arg(long, env = "WEPUB_CHROME_CLIENT_ID")]
    pub(crate) client_id: Option<String>,

    /// OAuth client secret. Use together with --client-id and --refresh-token.
    #[arg(long, env = "WEPUB_CHROME_CLIENT_SECRET", hide_env_values = true)]
    pub(crate) client_secret: Option<String>,

    /// OAuth refresh token. Use together with --client-id and --client-secret.
    #[arg(long, env = "WEPUB_CHROME_REFRESH_TOKEN", hide_env_values = true)]
    pub(crate) refresh_token: Option<String>,

    /// Pre-fetched OAuth access token.
    #[arg(long, env = "WEPUB_CHROME_ACCESS_TOKEN", hide_env_values = true)]
    pub(crate) access_token: Option<String>,

    /// Whether to publish immediately on approval or stage for later publishing.
    #[arg(long, value_enum)]
    pub(crate) publish_type: Option<ChromePublishTypeArg>,

    /// Initial deploy percentage (0-100). Omit to use the Developer Dashboard default.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=100))]
    pub(crate) deploy_percentage: Option<u8>,

    /// Attempt to skip item review.
    #[arg(long)]
    pub(crate) skip_review: Option<bool>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ChromePublishTypeArg {
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
pub(crate) struct FirefoxArgs {
    /// Path to the add-on archive (zip).
    #[arg(value_name = "PACKAGE")]
    pub(crate) package: PathBuf,

    /// Add-on ID (slug or GUID).
    #[arg(long, env = "WEPUB_FIREFOX_ADDON_ID")]
    pub(crate) addon_id: String,

    /// API key (JWT issuer).
    #[arg(long, env = "WEPUB_FIREFOX_API_KEY")]
    pub(crate) api_key: String,

    /// API secret (JWT secret).
    #[arg(long, env = "WEPUB_FIREFOX_API_SECRET", hide_env_values = true)]
    pub(crate) api_secret: String,

    /// Version channel. Determines visibility on the site.
    #[arg(long, value_enum)]
    pub(crate) channel: FirefoxChannelArg,

    /// Compatible applications, comma-separated (e.g. "firefox,android").
    #[arg(long, value_delimiter = ',')]
    pub(crate) compatibility: Vec<FirefoxApplicationArg>,

    /// Information for Mozilla reviewers. Mutually exclusive with --approval-notes-file.
    #[arg(long)]
    pub(crate) approval_notes: Option<String>,

    /// Path to a file containing approval notes. Use "-" for stdin.
    #[arg(long, value_name = "PATH")]
    pub(crate) approval_notes_file: Option<PathBuf>,

    /// Release notes. Mutually exclusive with --release-notes-file.
    #[arg(long)]
    pub(crate) release_notes: Option<String>,

    /// Path to a file containing release notes. Use "-" for stdin.
    #[arg(long, value_name = "PATH")]
    pub(crate) release_notes_file: Option<PathBuf>,

    /// Locale code for the release notes (e.g. "en-US", "ja").
    #[arg(
        long,
        value_name = "LANG",
        default_value = "en-US",
        requires = "release_notes_input"
    )]
    pub(crate) release_notes_lang: String,

    /// Path to a source archive to attach to the version.
    #[arg(long, value_name = "PATH")]
    pub(crate) source: Option<PathBuf>,

    #[arg(long, hide = true, env = "WEPUB_FIREFOX_INTERNAL_ROOT_URL")]
    pub(crate) internal_root_url: Option<Url>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FirefoxChannelArg {
    Listed,
    Unlisted,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum FirefoxApplicationArg {
    Firefox,
    Android,
}

#[derive(Debug, Args)]
#[command(group(
    clap::ArgGroup::new("edge_notes_input")
        .multiple(false)
        .args(["notes", "notes_file"]),
))]
pub(crate) struct EdgeArgs {
    /// Path to the add-on archive (zip).
    #[arg(value_name = "PACKAGE")]
    pub(crate) package: PathBuf,

    /// Product ID (GUID).
    #[arg(long, env = "WEPUB_EDGE_PRODUCT_ID")]
    pub(crate) product_id: String,

    /// Client ID.
    #[arg(long, env = "WEPUB_EDGE_CLIENT_ID")]
    pub(crate) client_id: String,

    /// API key.
    #[arg(long, env = "WEPUB_EDGE_API_KEY", hide_env_values = true)]
    pub(crate) api_key: String,

    /// Notes for certification. Mutually exclusive with --notes-file.
    #[arg(long)]
    pub(crate) notes: Option<String>,

    /// Path to a file containing notes for certification. Use "-" for stdin.
    #[arg(long, value_name = "PATH")]
    pub(crate) notes_file: Option<PathBuf>,
}
