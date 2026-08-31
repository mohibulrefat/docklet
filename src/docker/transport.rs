//! Connecting to a Docker endpoint.
//!
//! This is the seam that keeps the unix-socket-only decision reversible.
//! `http` is written against the boxed stream returned here rather than against
//! `UnixStream`, so adding `tcp://`, `ssh://` or TLS later means a new arm in
//! `connect` and nothing else.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use super::endpoint::Endpoint;
use super::DockerError;

/// How long a single request may wait to connect, and to read a response.
///
/// Streaming endpoints (logs, stats) will need to opt out of the read timeout;
/// that arrives with the streaming reader.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// A connected byte stream. Boxed so the transport can vary without `http`
/// knowing which kind it holds.
pub trait Stream: Read + Write + Send {}

impl<T: Read + Write + Send> Stream for T {}

/// Open a connection to the endpoint.
pub fn connect(endpoint: &Endpoint) -> Result<Box<dyn Stream>, DockerError> {
    match endpoint {
        Endpoint::Unix(path) => {
            let stream = UnixStream::connect(path).map_err(|e| connect_error(e, endpoint))?;
            stream
                .set_read_timeout(Some(TIMEOUT))
                .map_err(|e| connect_error(e, endpoint))?;
            stream
                .set_write_timeout(Some(TIMEOUT))
                .map_err(|e| connect_error(e, endpoint))?;
            Ok(Box::new(stream))
        }
    }
}

/// Turn a connect failure into an error that says what to do about it.
fn connect_error(err: std::io::Error, endpoint: &Endpoint) -> DockerError {
    use std::io::ErrorKind::*;

    match err.kind() {
        NotFound | ConnectionRefused => DockerError::Unreachable(format!(
            "Cannot reach Docker at {endpoint}. Is the Docker daemon running?"
        )),
        PermissionDenied => DockerError::PermissionDenied(format!(
            "Permission denied opening {endpoint}. \
             Adding your user to the 'docker' group grants access."
        )),
        TimedOut => DockerError::Timeout,
        _ => DockerError::Unreachable(format!("Cannot reach Docker at {endpoint}: {err}")),
    }
}
