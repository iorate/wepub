use thiserror::Error;

pub type Result<T> = std::result::Result<T, WepubError>;

#[derive(Debug, Error)]
pub enum WepubError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("API error (status {status}): {body}")]
    Api { status: u16, body: String },

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("AMO validation failed: {0}")]
    Validation(String),

    #[error("CWS upload failed: {0}")]
    Upload(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("internal error: {0}")]
    Internal(String),
}
