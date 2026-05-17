use std::path::PathBuf;

use anyhow::Result;
use tokio::io::AsyncReadExt;

// `path == "-"` reads stdin once. Callers that accept stdin for two
// different inputs must reject the dual-`-` combination themselves;
// stdin is a single stream.
pub(crate) async fn read_text_input(path: &PathBuf) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut buf = String::new();
        tokio::io::stdin().read_to_string(&mut buf).await?;
        Ok(buf)
    } else {
        Ok(tokio::fs::read_to_string(path).await?)
    }
}
