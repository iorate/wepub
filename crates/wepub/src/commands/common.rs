use std::path::Path;

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;

pub(crate) async fn read_binary_input(path: &Path, label: &str) -> Result<Vec<u8>> {
    tokio::fs::read(path)
        .await
        .with_context(|| format!("failed to read {label} from {}", path.display()))
}

pub(crate) async fn resolve_text_input(
    inline: Option<String>,
    file: Option<&Path>,
    label: &str,
) -> Result<Option<String>> {
    match (inline, file) {
        (Some(text), _) => Ok(Some(text)),
        (_, Some(path)) => {
            Ok(Some(read_text_input(path).await.with_context(|| {
                format!("failed to read {label} from {}", path.display())
            })?))
        }
        _ => Ok(None),
    }
}

async fn read_text_input(path: &Path) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut buf = String::new();
        tokio::io::stdin().read_to_string(&mut buf).await?;
        Ok(buf)
    } else {
        Ok(tokio::fs::read_to_string(path).await?)
    }
}
