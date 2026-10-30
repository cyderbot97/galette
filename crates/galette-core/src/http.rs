//! HTTP client shared by the online services.

use std::thread;
use std::time::Duration;

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

/// Attempts before giving up on a busy server.
const ATTEMPTS: u32 = 5;

/// Downloads `url`. A 404 means "not in the database" and gives `None`.
///
/// A busy server (503, or 429 "too many requests", e.g. MusicBrainz's limit of
/// one request per second) is asked again after a pause: the one it gives in
/// `Retry-After`, otherwise 1, 2, 4 then 8 seconds.
pub(crate) fn get_bytes(agent: &ureq::Agent, url: &str) -> Result<Option<Vec<u8>>> {
    for attempt in 0.. {
        let mut response = agent.get(url).call()?;
        match response.status().as_u16() {
            200 => return Ok(Some(response.body_mut().read_to_vec()?)),
            404 => return Ok(None),
            503 | 429 if attempt + 1 < ATTEMPTS => {
                let wait = retry_after(&response).unwrap_or(1 << attempt);
                thread::sleep(Duration::from_secs(wait));
            }
            status => return Err(Error::Network(format!("{url} answered HTTP {status}"))),
        }
    }
    unreachable!("the loop only ends by returning")
}

/// The pause asked by the server, in seconds (capped: never wait a minute).
fn retry_after<B>(response: &ureq::http::Response<B>) -> Option<u64> {
    let seconds = response.headers().get("retry-after")?.to_str().ok()?.trim().parse::<u64>().ok()?;
    Some(seconds.min(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// Serves the given status lines in turn, one connection each.
    fn serve(answers: &'static [&'static str]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        thread::spawn(move || {
            for answer in answers {
                let (mut stream, _) = listener.accept().unwrap();
                // Read the request up to its blank line.
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap() > 2 {
                    line.clear();
                }
                write!(stream, "{answer}\r\nRetry-After: 0\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
            }
        });
        url
    }

    #[test]
    fn retries_a_busy_server() {
        let url = serve(&["HTTP/1.1 503 Service Unavailable", "HTTP/1.1 429 Too Many Requests", "HTTP/1.1 200 OK"]);
        assert_eq!(get_bytes(&agent(), &url).unwrap(), Some(b"ok".to_vec()));
    }

    #[test]
    fn gives_up_on_a_server_that_stays_busy() {
        let url = serve(&["HTTP/1.1 503 Service Unavailable"; ATTEMPTS as usize]);
        assert!(get_bytes(&agent(), &url).is_err());
    }
}
