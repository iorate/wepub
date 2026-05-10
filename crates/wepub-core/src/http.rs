use std::time::Duration;

use crate::Result;

const USER_AGENT: &str = concat!("wepub/", env!("CARGO_PKG_VERSION"));
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5 * 60);

pub(crate) fn build_client() -> Result<reqwest::Client> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()?;
    Ok(client)
}
