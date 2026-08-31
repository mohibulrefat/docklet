//! Connecting to a Docker endpoint.
//!
//! This is the seam that keeps the unix-socket-only decision reversible.
//! `http` is written against [`Connection`] rather than a socket type, so
//! adding `tcp://`, `ssh://` or TLS later means a new arm in `connect` and a
//! second variant inside `Connection` — nothing above this module changes.

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use super::endpoint::Endpoint;
use super::DockerError;

/// How long a single request may wait to connect, and to read a response.
///
/// Streaming endpoints clear this: a followed log is idle for as long as the
/// container is quiet, which is not a timeout.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// An open connection to Docker.
pub struct Connection {
    socket: UnixStream,
}

impl Connection {
    /// A second handle to the same connection.
    ///
    /// Used to shut a stream down from another thread while its reader is
    /// blocked, which is the only way to interrupt a blocking read without
    /// polling for it.
    pub fn try_clone(&self) -> Result<Connection, DockerError> {
        let socket = self
            .socket
            .try_clone()
            .map_err(|e| DockerError::Protocol(e.to_string()))?;
        Ok(Connection { socket })
    }

    /// Stop a blocked read on this connection.
    pub fn shutdown_read(&self) {
        // Already-closed is the normal case when a stream ended on its own.
        let _ = self.socket.shutdown(Shutdown::Read);
    }

    /// Set or clear the read timeout. `None` waits indefinitely.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), DockerError> {
        self.socket
            .set_read_timeout(timeout)
            .map_err(|e| DockerError::Protocol(e.to_string()))
    }
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.socket.read(buf)
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.socket.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.socket.flush()
    }
}

/// Open a connection to the endpoint.
pub fn connect(endpoint: &Endpoint) -> Result<Connection, DockerError> {
    match endpoint {
        Endpoint::Unix(path) => {
            let socket = UnixStream::connect(path).map_err(|e| connect_error(e, endpoint))?;
            socket
                .set_read_timeout(Some(TIMEOUT))
                .map_err(|e| connect_error(e, endpoint))?;
            socket
                .set_write_timeout(Some(TIMEOUT))
                .map_err(|e| connect_error(e, endpoint))?;
            Ok(Connection { socket })
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
