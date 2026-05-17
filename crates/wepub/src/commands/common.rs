use std::path::Path;

use anyhow::Result;
use tokio::io::AsyncReadExt;

pub(crate) async fn read_text_input(path: &Path) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut buf = String::new();
        tokio::io::stdin().read_to_string(&mut buf).await?;
        Ok(buf)
    } else {
        Ok(tokio::fs::read_to_string(path).await?)
    }
}
