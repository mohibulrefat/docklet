//! The containers API.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};

use super::{Docker, DockerError};

/// A container as reported by `/containers/json`.
///
/// Docker returns a great deal more per container; this is what Docklet shows.
#[derive(Debug, Clone, Deserialize)]
pub struct Container {
    #[serde(rename = "Id")]
    pub id: String,
    /// Every name Docker knows this container by, each with a leading `/`.
    #[serde(rename = "Names", default, deserialize_with = "null_as_default")]
    pub names: Vec<String>,
    #[serde(rename = "Image")]
    pub image: String,
    /// One of `running`, `exited`, `created`, `paused`, `restarting`, `dead`.
    #[serde(rename = "State")]
    pub state: String,
    /// Human-readable detail, e.g. `Exited (0) 4 days ago`.
    #[serde(rename = "Status")]
    pub status: String,
    #[serde(rename = "Labels", default, deserialize_with = "null_as_default")]
    pub labels: HashMap<String, String>,
    /// Present on `/containers/json`, and how volume usage is discovered.
    #[serde(rename = "Mounts", default, deserialize_with = "null_as_default")]
    pub mounts: Vec<Mount>,
}

impl Container {
    /// The container's display name, without Docker's leading slash.
    ///
    /// A container can carry several names; Docker lists the primary one first.
    /// Unnamed containers fall back to the short id, as `docker ps` does.
    pub fn name(&self) -> &str {
        match self.names.first() {
            Some(name) => name.strip_prefix('/').unwrap_or(name),
            None => self.short_id(),
        }
    }

    /// Whether this container mounts a named volume.
    pub fn mounts_volume(&self, volume: &str) -> bool {
        self.mounts
            .iter()
            .any(|mount| mount.kind == "volume" && mount.name == volume)
    }

    /// The first 12 characters of the id — what Docker itself displays.
    pub fn short_id(&self) -> &str {
        short_id(&self.id)
    }
}

/// Shorten a container id to the 12 characters Docker displays.
pub fn short_id(id: &str) -> &str {
    let end = id.char_indices().nth(12).map_or(id.len(), |(i, _)| i);
    &id[..end]
}

/// Treat an explicit `null` as an empty value.
///
/// Docker sends `null` rather than `{}` or `[]` for some absent collections.
pub(super) fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::deserialize(deserializer)?.unwrap_or_default())
}

impl Docker {
    /// List containers. With `all`, stopped ones are included too.
    pub fn containers(&self, all: bool) -> Result<Vec<Container>, DockerError> {
        let path = if all {
            "/containers/json?all=1"
        } else {
            "/containers/json"
        };
        self.get_json(path)
    }
}

/// The parts of `/containers/{id}/json` Docklet displays.
///
/// The full inspect payload is enormous and mostly unused. Only fields that are
/// actually rendered appear here — state and status already come from the list,
/// so they are deliberately absent rather than duplicated.
#[derive(Debug, Clone, Deserialize)]
pub struct Inspect {
    /// RFC 3339, displayed verbatim rather than reformatted.
    #[serde(rename = "Created", default)]
    pub created: String,
    #[serde(rename = "Config", default)]
    pub config: InspectConfig,
    #[serde(rename = "HostConfig", default)]
    pub host_config: InspectHostConfig,
    #[serde(rename = "NetworkSettings", default)]
    pub network_settings: NetworkSettings,
    #[serde(rename = "Mounts", default, deserialize_with = "null_as_default")]
    pub mounts: Vec<Mount>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InspectConfig {
    #[serde(rename = "Cmd", default, deserialize_with = "null_as_default")]
    pub cmd: Vec<String>,
    /// Whether the container has a TTY, which decides whether its log stream
    /// is framed or raw.
    #[serde(rename = "Tty", default)]
    pub tty: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InspectHostConfig {
    #[serde(rename = "RestartPolicy", default)]
    pub restart_policy: RestartPolicy,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RestartPolicy {
    #[serde(rename = "Name", default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NetworkSettings {
    #[serde(rename = "Networks", default, deserialize_with = "null_as_default")]
    pub networks: HashMap<String, NetworkEndpoint>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NetworkEndpoint {
    #[serde(rename = "IPAddress", default)]
    pub ip_address: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Mount {
    /// `volume` or `bind`; only named volumes carry a `Name`.
    #[serde(rename = "Type", default)]
    pub kind: String,
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "Source", default)]
    pub source: String,
    #[serde(rename = "Destination", default)]
    pub destination: String,
}

impl Inspect {
    /// The command line, joined for display.
    pub fn command(&self) -> String {
        self.config.cmd.join(" ")
    }

    /// Networks as `name (ip)`, comma separated.
    pub fn networks(&self) -> String {
        let mut names: Vec<String> = self
            .network_settings
            .networks
            .iter()
            .map(|(name, endpoint)| {
                if endpoint.ip_address.is_empty() {
                    name.clone()
                } else {
                    format!("{name} ({})", endpoint.ip_address)
                }
            })
            .collect();
        names.sort();
        names.join(", ")
    }

    /// Mounts as `source -> destination`, one per line.
    pub fn mount_list(&self) -> String {
        self.mounts
            .iter()
            .map(|m| format!("{} \u{2192} {}", m.source, m.destination))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Docker {
    /// Inspect one container.
    pub fn inspect_container(&self, id: &str) -> Result<Inspect, DockerError> {
        self.get_json(&format!("/containers/{id}/json"))
    }

    /// Start a stopped container.
    pub fn start_container(&self, id: &str) -> Result<(), DockerError> {
        self.post(&format!("/containers/{id}/start"))
    }

    /// Stop a running container.
    ///
    /// Docker sends SIGTERM and waits before killing, so this can legitimately
    /// take several seconds.
    pub fn stop_container(&self, id: &str) -> Result<(), DockerError> {
        self.post(&format!("/containers/{id}/stop"))
    }

    /// Restart a container, running or not.
    pub fn restart_container(&self, id: &str) -> Result<(), DockerError> {
        self.post(&format!("/containers/{id}/restart"))
    }

    /// Remove a container.
    ///
    /// Removing a running container is a 409 unless `force` is set. Named
    /// volumes are always left alone — Docker only removes anonymous ones, and
    /// only when asked, which Docklet never does.
    pub fn remove_container(&self, id: &str, force: bool) -> Result<(), DockerError> {
        let path = if force {
            format!("/containers/{id}?force=true")
        } else {
            format!("/containers/{id}")
        };
        self.delete(&path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `/containers/json?all=1` response.
    const LIST: &str = r#"[
        {
            "Id": "95ed5f9807720835f7df9b7d428c5ffb1470f2e1d9ae17ea9d642a30b55f58cc",
            "Names": ["/ubl-backend"],
            "Image": "ubl-backend-services-backend",
            "ImageID": "sha256:a8fe9a209bd10b89dfd92f0d5beb30c17b98c65303bc620bdb9ab072c7bda259",
            "Command": "uvicorn main:app",
            "Created": 1787480789,
            "Ports": [],
            "Labels": {"com.docker.compose.project": "ubl-backend-services",
                       "com.docker.compose.service": "backend"},
            "State": "exited",
            "Status": "Exited (0) 4 days ago"
        },
        {
            "Id": "abc123",
            "Names": ["/redis", "/app_redis_1"],
            "Image": "redis:7.2-alpine",
            "Labels": null,
            "State": "running",
            "Status": "Up 2 hours"
        }
    ]"#;

    fn parsed() -> Vec<Container> {
        serde_json::from_str(LIST).expect("fixture should parse")
    }

    #[test]
    fn parses_a_container_list() {
        let containers = parsed();
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0].image, "ubl-backend-services-backend");
        assert_eq!(containers[0].state, "exited");
        assert_eq!(containers[0].status, "Exited (0) 4 days ago");
    }

    #[test]
    fn strips_the_leading_slash_from_names() {
        assert_eq!(parsed()[0].name(), "ubl-backend");
    }

    #[test]
    fn uses_the_first_of_several_names() {
        assert_eq!(parsed()[1].name(), "redis");
    }

    #[test]
    fn reads_labels() {
        let containers = parsed();
        assert_eq!(
            containers[0]
                .labels
                .get("com.docker.compose.project")
                .map(String::as_str),
            Some("ubl-backend-services")
        );
    }

    #[test]
    fn treats_null_labels_as_empty() {
        assert!(parsed()[1].labels.is_empty());
    }

    #[test]
    fn shortens_a_full_length_id() {
        assert_eq!(parsed()[0].short_id(), "95ed5f980772");
    }

    #[test]
    fn leaves_an_already_short_id_alone() {
        assert_eq!(parsed()[1].short_id(), "abc123");
    }

    /// Trimmed from a real `/containers/{id}/json` response.
    const INSPECT: &str = r#"{
        "Created": "2026-08-31T09:50:08.400881773Z",
        "State": {"Status": "running", "Running": true, "ExitCode": 0},
        "Config": {"Image": "alpine:latest", "Tty": false,
                   "Cmd": ["sh", "-c", "echo hi"],
                   "Env": ["PATH=/usr/bin"]},
        "HostConfig": {"RestartPolicy": {"Name": "unless-stopped",
                                         "MaximumRetryCount": 0}},
        "NetworkSettings": {"Networks": {"bridge": {"IPAddress": "172.17.0.2"}}},
        "Mounts": [{"Source": "/host/data", "Destination": "/data"}]
    }"#;

    #[test]
    fn finds_containers_mounting_a_named_volume() {
        let container: Container = serde_json::from_str(
            r#"{"Id":"a","Names":["/db"],"Image":"i","State":"running","Status":"Up",
                 "Mounts":[{"Type":"bind","Source":"/host","Destination":"/app"},
                           {"Type":"volume","Name":"pgdata","Destination":"/var/lib"}]}"#,
        )
        .unwrap();
        assert!(container.mounts_volume("pgdata"));
        // A bind mount is not a named volume, and must not match by path.
        assert!(!container.mounts_volume("/host"));
        assert!(!container.mounts_volume("other"));
    }

    #[test]
    fn parses_the_inspect_subset() {
        let inspect: Inspect = serde_json::from_str(INSPECT).unwrap();
        assert_eq!(inspect.created, "2026-08-31T09:50:08.400881773Z");
        assert_eq!(inspect.host_config.restart_policy.name, "unless-stopped");
    }

    #[test]
    fn joins_the_command_for_display() {
        let inspect: Inspect = serde_json::from_str(INSPECT).unwrap();
        assert_eq!(inspect.command(), "sh -c echo hi");
    }

    #[test]
    fn shows_networks_with_their_addresses() {
        let inspect: Inspect = serde_json::from_str(INSPECT).unwrap();
        assert_eq!(inspect.networks(), "bridge (172.17.0.2)");
    }

    #[test]
    fn omits_the_address_when_a_network_has_none() {
        let inspect: Inspect =
            serde_json::from_str(r#"{"NetworkSettings":{"Networks":{"host":{"IPAddress":""}}}}"#)
                .unwrap();
        assert_eq!(inspect.networks(), "host");
    }

    #[test]
    fn lists_mounts_as_source_to_destination() {
        let inspect: Inspect = serde_json::from_str(INSPECT).unwrap();
        assert_eq!(inspect.mount_list(), "/host/data \u{2192} /data");
    }

    #[test]
    fn tolerates_an_inspect_with_nothing_set() {
        // Every field defaults, so a sparse or older payload still renders.
        let inspect: Inspect = serde_json::from_str("{}").unwrap();
        assert_eq!(inspect.created, "");
        assert_eq!(inspect.command(), "");
        assert_eq!(inspect.networks(), "");
        assert_eq!(inspect.mount_list(), "");
    }

    #[test]
    fn falls_back_to_the_short_id_when_unnamed() {
        let container: Container = serde_json::from_str(
            r#"{"Id":"deadbeefcafe0123","Image":"x","State":"created","Status":"Created"}"#,
        )
        .unwrap();
        assert_eq!(container.name(), "deadbeefcafe");
    }
}
