//! HTTP client shared by the online services.

use crate::{Error, Result};

const USER_AGENT: &str = concat!(
    "Galette/",
    env!("CARGO_PKG_VERSION"),
    " ( ",
    env!("CARGO_PKG_REPOSITORY"),
    " )"
);

/// An agent that identifies Galette and returns HTTP errors as responses.
pub(crate) fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .http_status_as_error(false)
        .build()
        .into()
}

/// Downloads `url`. A 404 means "not in the database" and gives `None`.
pub(crate) fn get_bytes(agent: &ureq::Agent, url: &str) -> Result<Option<Vec<u8>>> {
    let mut response = agent.get(url).call()?;
    match response.status().as_u16() {
        200 => Ok(Some(response.body_mut().read_to_vec()?)),
        404 => Ok(None),
        status => Err(Error::Network(format!("{url} answered HTTP {status}"))),
    }
}
