//! The Docker Engine API client.
//!
//! Layering: `endpoint` decides where Docker is, `transport` connects to it,
//! `http` speaks HTTP/1.1 over that connection, and the API modules above turn
//! bytes into typed values. Nothing in here imports GTK, which is what keeps it
//! testable without a UI — or a daemon.

mod containers;
mod endpoint;
mod http;
mod transport;

use std::fmt;

use serde::Deserialize;

pub use containers::Container;
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

    /// Check that the daemon is alive.
    ///
    /// `/_ping` is the cheapest endpoint Docker offers — it answers `OK` and
    /// touches no state.
    pub fn ping(&self) -> Result<(), DockerError> {
        self.get("/_ping")?;
        Ok(())
    }

    /// Ask which Docker and API version we are talking to.
    pub fn version(&self) -> Result<Version, DockerError> {
        self.get_json("/version")
    }
}

/// The parts of `/version` Docklet displays.
///
/// Docker returns a great deal more; deserializing only these two keeps the
/// model honest about what is actually used.
#[derive(Debug, Clone, Deserialize)]
pub struct Version {
    #[serde(rename = "Version")]
    pub version: String,
    #[serde(rename = "ApiVersion")]
    pub api_version: String,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Docker {} (API {})", self.version, self.api_version)
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
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    /// A throwaway unix socket serving one canned response, then closing.
    ///
    /// This exercises the whole hand-rolled path — connect, write the request,
    /// parse the status line, headers and body — against a real socket, which
    /// is the part no amount of parser unit testing covers.
    ///
    /// The socket is removed when the guard drops, so a failing assertion still
    /// cleans up.
    struct Server {
        docker: Docker,
        path: PathBuf,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn serve(response: String) -> Server {
        // Unique per test: these run on threads within one process.
        let path = std::env::temp_dir().join(format!(
            "docklet-{}-{:?}.sock",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);

        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(response.as_bytes());
            }
        });

        Server {
            docker: Docker {
                endpoint: Endpoint::Unix(path.clone()),
            },
            path,
        }
    }

    /// Frame a body with a computed `Content-Length`, so the fixture cannot
    /// drift out of step with the body it describes.
    fn with_length(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    /// The same, as a single chunk, to exercise the chunked path Docker uses.
    fn chunked(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n\
             Content-Type: application/json\r\n\
             Transfer-Encoding: chunked\r\n\r\n\
             {:x}\r\n{body}\r\n0\r\n\r\n",
            body.len()
        )
    }

    #[test]
    fn pings_over_a_real_socket() {
        let server = serve(with_length("200 OK", "OK"));
        assert!(server.docker.ping().is_ok());
    }

    #[test]
    fn reads_version_over_a_real_socket() {
        let server = serve(chunked(
            "200 OK",
            r#"{"Version":"29.7.2","ApiVersion":"1.52"}"#,
        ));

        let version = server.docker.version().expect("version should parse");
        assert_eq!(version.to_string(), "Docker 29.7.2 (API 1.52)");
    }

    #[test]
    fn surfaces_an_api_error_status() {
        let server = serve(with_length(
            "404 Not Found",
            r#"{"message":"page not found"}"#,
        ));

        match server.docker.ping() {
            Err(DockerError::Api { status, message }) => {
                assert_eq!(status, 404);
                assert_eq!(message, "page not found");
            }
            other => panic!("expected an API error, got {other:?}"),
        }
    }

    #[test]
    fn reports_an_absent_socket_as_unreachable() {
        let docker = Docker {
            endpoint: Endpoint::Unix(PathBuf::from("/nonexistent/docker.sock")),
        };
        match docker.ping() {
            Err(DockerError::Unreachable(msg)) => assert!(msg.contains("daemon running"), "{msg}"),
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

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

    #[test]
    fn reads_version_from_a_full_docker_payload() {
        // Trimmed from a real /version response — the extra keys must be ignored.
        let body = r#"{
            "Platform": {"Name": "Docker Engine - Community"},
            "Components": [{"Name": "Engine", "Version": "29.7.2"}],
            "Version": "29.7.2",
            "ApiVersion": "1.52",
            "MinAPIVersion": "1.24",
            "GitCommit": "a7dcaa6",
            "Os": "linux",
            "Arch": "amd64"
        }"#;

        let version: Version = serde_json::from_str(body).unwrap();
        assert_eq!(version.version, "29.7.2");
        assert_eq!(version.api_version, "1.52");
    }

    #[test]
    fn version_displays_for_the_status_line() {
        let version = Version {
            version: "29.7.2".to_string(),
            api_version: "1.52".to_string(),
        };
        assert_eq!(version.to_string(), "Docker 29.7.2 (API 1.52)");
    }
}
