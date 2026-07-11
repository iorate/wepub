use std::time::Duration;

use isahc::HttpClient;
use isahc::config::{Configurable, RedirectPolicy};
use isahc::http::header;

use crate::{Result, WepubError};

const USER_AGENT: &str = concat!("wepub/", env!("CARGO_PKG_VERSION"));
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const STALL_SPEED_LIMIT: u32 = 1; // bytes/sec
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn build_client() -> Result<HttpClient> {
    let client = HttpClient::builder()
        .default_header(header::USER_AGENT, USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .low_speed_timeout(STALL_SPEED_LIMIT, STALL_TIMEOUT)
        .expect_continue(false)
        .redirect_policy(RedirectPolicy::Limit(10))
        .build()
        .map_err(WepubError::http)?;
    Ok(client)
}
