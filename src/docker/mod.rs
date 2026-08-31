//! The Docker Engine API client.
//!
//! Layering: `endpoint` decides where Docker is, `transport` connects to it,
//! `http` speaks HTTP/1.1 over that connection, and the API modules above turn
//! bytes into typed values. Nothing in here imports GTK, which is what keeps it
//! testable without a UI — or a daemon.

mod endpoint;
mod http;
mod transport;

use std::fmt;

use serde::Deserialize;

pub use endpoint::Endpoint;

/// Everything that can go wrong talking to Docker.
///
/// The variants exist to carry different advice, not to classify failures for
/// their own sake: an unreachable daemon and a permission problem need
/// different things from the user.
#[derive(Debug)]
pub enum DockerError {
    /// The daemon could not be reached — usually not running.
    Unreachable(String),
    /// The socket exists but is not readable by this user.
    PermissionDenied(String),
    /// The endpoint uses a scheme Docklet does not support yet.
    UnsupportedEndpoint(String),
    /// Docker answered with an error status.
    Api { status: u16, message: String },
    /// The response was not valid HTTP, or ended early.
    Protocol(String),
    /// The response body was not the JSON we expected.
    Decode(String),
    /// The request took too long.
    Timeout,
}

impl fmt::Display for DockerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DockerError::Unreachable(msg)
            | DockerError::PermissionDenied(msg)
            | DockerError::UnsupportedEndpoint(msg) => f.write_str(msg),
            DockerError::Api { status, message } => {
                write!(f, "Docker returned {status}: {message}")
            }
            DockerError::Protocol(msg) => write!(f, "Unexpected response from Docker: {msg}"),
            DockerError::Decode(msg) => write!(f, "Could not read Docker's response: {msg}"),
            DockerError::Timeout => f.write_str("Docker did not respond in time."),
        }
    }
}

impl std::error::Error for DockerError {}

/// A handle to the Docker Engine API.
///
/// Holding one does not hold a connection: each request dials the endpoint and
/// closes it afterwards. Constructing a `Docker` therefore succeeds even when
/// the daemon is down — the failure surfaces on first use, which is what lets
/// the window open instantly and report status asynchronously.
#[derive(Clone)]
pub struct Docker {
    endpoint: Endpoint,
}

impl Docker {
    /// Resolve the endpoint Docker should be reached at.
    pub fn connect() -> Result<Self, DockerError> {
        Ok(Docker {
            endpoint: endpoint::resolve()?,
        })
    }

    /// Perform a request and return the raw body, mapping error statuses.
    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<Vec<u8>, DockerError> {
        let stream = transport::connect(&self.endpoint)?;
        let response = http::request(stream, method, path, body)?;

        if response.status >= 400 {
            return Err(DockerError::Api {
                status: response.status,
                message: api_message(&response.body),
            });
        }
        Ok(response.body)
    }

    /// GET a path and return the raw body.
    pub fn get(&self, path: &str) -> Result<Vec<u8>, DockerError> {
        self.request("GET", path, None)
    }

    /// GET a path and deserialize the JSON body.
    pub fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, DockerError> {
        let body = self.get(path)?;
        serde_json::from_slice(&body).map_err(|e| DockerError::Decode(e.to_string()))
    }
}

/// Docker reports errors as `{"message": "..."}`; fall back to the raw body.
fn api_message(body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct Message {
        message: String,
    }

    match serde_json::from_slice::<Message>(body) {
        Ok(parsed) => parsed.message,
        Err(_) => String::from_utf8_lossy(body).trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_docker_error_message() {
        let body = br#"{"message":"No such container: abc"}"#;
        assert_eq!(api_message(body), "No such container: abc");
    }

    #[test]
    fn falls_back_to_the_raw_body() {
        assert_eq!(api_message(b"plain failure\n"), "plain failure");
    }

    #[test]
    fn api_errors_mention_status_and_message() {
        let err = DockerError::Api {
            status: 409,
            message: "container is running".to_string(),
        };
        assert_eq!(err.to_string(), "Docker returned 409: container is running");
    }

    #[test]
    fn advice_errors_display_their_message_verbatim() {
        let err = DockerError::Unreachable("Is the Docker daemon running?".to_string());
        assert_eq!(err.to_string(), "Is the Docker daemon running?");
    }
}
