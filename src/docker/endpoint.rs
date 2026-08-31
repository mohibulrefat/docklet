//! Docker endpoint resolution.
//!
//! This is the only module that knows where Docker lives. Everything above the
//! `docker` module receives an already-resolved endpoint and never learns the
//! socket path, the context name, or how either was determined.
//!
//! Resolution mirrors the Docker CLI, in order:
//!
//! 1. `DOCKER_HOST`
//! 2. the context named by `DOCKER_CONTEXT`, else by `currentContext` in
//!    `$DOCKER_CONFIG/config.json` (default `~/.docker/config.json`)
//! 3. `/var/run/docker.sock`

use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::DockerError;

/// The default socket, used when nothing else selects an endpoint.
const DEFAULT_SOCKET: &str = "/var/run/docker.sock";

/// The context name Docker reserves for "no stored context" — it means the
/// built-in default rather than a context stored on disk.
const DEFAULT_CONTEXT: &str = "default";

/// A resolved Docker endpoint.
///
/// Only unix sockets are supported today. Adding `tcp://`, `ssh://` or TLS
/// means a variant here and an arm in `transport::connect`; nothing else moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Unix(PathBuf),
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Endpoint::Unix(path) => write!(f, "unix://{}", path.display()),
        }
    }
}

/// Resolve the endpoint to talk to, following the Docker CLI's precedence.
pub fn resolve() -> Result<Endpoint, DockerError> {
    if let Some(host) = non_empty(env::var("DOCKER_HOST").ok()) {
        return parse_host(&host);
    }

    let config_dir = config_dir();
    let context = non_empty(env::var("DOCKER_CONTEXT").ok())
        .or_else(|| current_context(&config_dir))
        .unwrap_or_else(|| DEFAULT_CONTEXT.to_string());

    if context != DEFAULT_CONTEXT {
        if let Some(host) = context_host(&config_dir, &context) {
            return parse_host(&host);
        }
    }

    Ok(Endpoint::Unix(PathBuf::from(DEFAULT_SOCKET)))
}

/// Parse a Docker host string such as `unix:///var/run/docker.sock`.
///
/// Non-unix schemes are rejected by name, so a user on a `tcp://` context is
/// told what is unsupported rather than that a socket could not be found.
fn parse_host(host: &str) -> Result<Endpoint, DockerError> {
    let Some((scheme, rest)) = host.split_once("://") else {
        return Err(DockerError::UnsupportedEndpoint(format!(
            "{host} is not a valid Docker host (expected a scheme such as unix://)"
        )));
    };

    match scheme {
        "unix" => {
            if rest.is_empty() {
                Err(DockerError::UnsupportedEndpoint(format!(
                    "{host} does not name a socket path"
                )))
            } else {
                Ok(Endpoint::Unix(PathBuf::from(rest)))
            }
        }
        other => Err(DockerError::UnsupportedEndpoint(format!(
            "{other}:// endpoints are not supported yet; Docklet connects over a unix socket"
        ))),
    }
}

/// `$DOCKER_CONFIG`, else `~/.docker`.
fn config_dir() -> PathBuf {
    if let Some(dir) = non_empty(env::var("DOCKER_CONFIG").ok()) {
        return PathBuf::from(dir);
    }
    match non_empty(env::var("HOME").ok()) {
        Some(home) => PathBuf::from(home).join(".docker"),
        None => PathBuf::from(".docker"),
    }
}

#[derive(Deserialize)]
struct Config {
    #[serde(rename = "currentContext")]
    current_context: Option<String>,
}

fn current_context(config_dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(config_dir.join("config.json")).ok()?;
    let config: Config = serde_json::from_str(&raw).ok()?;
    non_empty(config.current_context)
}

#[derive(Deserialize)]
struct Meta {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Endpoints")]
    endpoints: Option<Endpoints>,
}

#[derive(Deserialize)]
struct Endpoints {
    docker: Option<DockerEndpoint>,
}

#[derive(Deserialize)]
struct DockerEndpoint {
    #[serde(rename = "Host")]
    host: Option<String>,
}

/// Find the host for a stored context.
///
/// Each context lives in `contexts/meta/<sha256 of name>/meta.json`, so the
/// directory name cannot be derived without hashing — we scan and match on the
/// `Name` field instead.
fn context_host(config_dir: &Path, wanted: &str) -> Option<String> {
    let entries = fs::read_dir(config_dir.join("contexts").join("meta")).ok()?;
    for entry in entries.flatten() {
        let Ok(raw) = fs::read_to_string(entry.path().join("meta.json")) else {
            continue;
        };
        if let Some(host) = host_from_meta(&raw, wanted) {
            return Some(host);
        }
    }
    None
}

/// Extract the docker host from one `meta.json`, if it describes `wanted`.
fn host_from_meta(raw: &str, wanted: &str) -> Option<String> {
    let meta: Meta = serde_json::from_str(raw).ok()?;
    if meta.name != wanted {
        return None;
    }
    non_empty(meta.endpoints?.docker?.host)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unix_host() {
        assert_eq!(
            parse_host("unix:///var/run/docker.sock").unwrap(),
            Endpoint::Unix(PathBuf::from("/var/run/docker.sock"))
        );
    }

    #[test]
    fn parses_unix_host_outside_var_run() {
        assert_eq!(
            parse_host("unix:///home/u/.docker/desktop/docker.sock").unwrap(),
            Endpoint::Unix(PathBuf::from("/home/u/.docker/desktop/docker.sock"))
        );
    }

    #[test]
    fn rejects_tcp_by_name() {
        let err = parse_host("tcp://192.168.1.5:2375").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("tcp://"), "{msg}");
    }

    #[test]
    fn rejects_ssh_by_name() {
        let err = parse_host("ssh://user@host").unwrap_err();
        assert!(err.to_string().contains("ssh://"));
    }

    #[test]
    fn rejects_host_without_scheme() {
        assert!(parse_host("/var/run/docker.sock").is_err());
    }

    #[test]
    fn rejects_unix_without_path() {
        assert!(parse_host("unix://").is_err());
    }

    #[test]
    fn reads_host_from_matching_meta() {
        let raw = r#"{
            "Name": "desktop-linux",
            "Metadata": {"Description": "Docker Desktop"},
            "Endpoints": {"docker": {"Host": "unix:///home/u/.docker/desktop/docker.sock",
                                     "SkipTLSVerify": false}}
        }"#;
        assert_eq!(
            host_from_meta(raw, "desktop-linux").as_deref(),
            Some("unix:///home/u/.docker/desktop/docker.sock")
        );
    }

    #[test]
    fn ignores_meta_for_another_context() {
        let raw = r#"{"Name": "remote", "Endpoints": {"docker": {"Host": "tcp://h:2376"}}}"#;
        assert_eq!(host_from_meta(raw, "desktop-linux"), None);
    }

    #[test]
    fn tolerates_meta_without_docker_endpoint() {
        let raw = r#"{"Name": "odd", "Endpoints": {}}"#;
        assert_eq!(host_from_meta(raw, "odd"), None);
    }

    #[test]
    fn tolerates_malformed_meta() {
        assert_eq!(host_from_meta("not json", "anything"), None);
    }

    #[test]
    fn endpoint_displays_as_a_url() {
        let endpoint = Endpoint::Unix(PathBuf::from("/var/run/docker.sock"));
        assert_eq!(endpoint.to_string(), "unix:///var/run/docker.sock");
    }
}
