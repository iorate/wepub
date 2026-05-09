use crate::Result;

const USER_AGENT: &str = concat!("wepub/", env!("CARGO_PKG_VERSION"));

pub(crate) fn build_client() -> Result<reqwest::Client> {
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;
    Ok(client)
}
