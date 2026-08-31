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
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
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

    #[test]
    fn falls_back_to_the_short_id_when_unnamed() {
        let container: Container = serde_json::from_str(
            r#"{"Id":"deadbeefcafe0123","Image":"x","State":"created","Status":"Created"}"#,
        )
        .unwrap();
        assert_eq!(container.name(), "deadbeefcafe");
    }
}
