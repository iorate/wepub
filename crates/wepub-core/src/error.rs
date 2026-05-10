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

    #[error("AMO validation failed for upload {uuid}: {body}")]
    Validation { uuid: String, body: String },

    #[error("CWS upload failed for item {item_id}: {body}")]
    Upload { item_id: String, body: String },

    #[error("publish failed for item {item_id}: {body}")]
    Publish { item_id: String, body: String },

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("internal error: {0}")]
    Internal(String),
}
