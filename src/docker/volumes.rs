//! The volumes API.

use std::collections::HashMap;

use serde::Deserialize;

use super::containers::null_as_default;
use super::{Docker, DockerError};

/// A volume as reported by `/volumes`.
#[derive(Debug, Clone, Deserialize)]
pub struct Volume {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Driver", default)]
    pub driver: String,
    #[serde(rename = "Mountpoint", default)]
    pub mountpoint: String,
    #[serde(rename = "Scope", default)]
    pub scope: String,
    /// RFC 3339, displayed verbatim.
    #[serde(rename = "CreatedAt", default)]
    pub created_at: String,
    #[serde(rename = "Labels", default, deserialize_with = "null_as_default")]
    pub labels: HashMap<String, String>,
    #[serde(rename = "Options", default, deserialize_with = "null_as_default")]
    pub options: HashMap<String, String>,
}

impl Volume {
    /// Driver options as `key=value`, comma separated and sorted.
    pub fn options_display(&self) -> String {
        let mut pairs: Vec<String> = self
            .options
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        pairs.sort();
        pairs.join(", ")
    }
}

/// `/volumes` answers with an object, not a bare array as the other list
/// endpoints do.
#[derive(Deserialize)]
struct VolumeList {
    #[serde(rename = "Volumes", default, deserialize_with = "null_as_default")]
    volumes: Vec<Volume>,
}

impl Docker {
    /// List volumes.
    pub fn volumes(&self) -> Result<Vec<Volume>, DockerError> {
        let listed: VolumeList = self.get_json("/volumes")?;
        Ok(listed.volumes)
    }

    /// Create a volume. An empty driver means Docker's default, `local`.
    pub fn create_volume(&self, name: &str, driver: &str) -> Result<(), DockerError> {
        let driver = if driver.trim().is_empty() {
            "local"
        } else {
            driver.trim()
        };
        let body = serde_json::json!({ "Name": name.trim(), "Driver": driver });
        self.post_json("/volumes/create", &body)?;
        Ok(())
    }

    /// Remove a volume. One still mounted by a container is a 409.
    pub fn remove_volume(&self, name: &str, force: bool) -> Result<(), DockerError> {
        let path = if force {
            format!("/volumes/{name}?force=true")
        } else {
            format!("/volumes/{name}")
        };
        self.delete(&path)
    }

    /// Inspect one volume.
    pub fn inspect_volume(&self, name: &str) -> Result<Volume, DockerError> {
        self.get_json(&format!("/volumes/{name}"))
    }

    /// Names of containers mounting a volume.
    ///
    /// The volume endpoints do not report their consumers, so this is derived
    /// by scanning the container list — one extra request per detail view.
    pub fn volume_users(&self, name: &str) -> Result<Vec<String>, DockerError> {
        Ok(self
            .containers(true)?
            .into_iter()
            .filter(|container| container.mounts_volume(name))
            .map(|container| container.name().to_string())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `/volumes` response.
    const LIST: &str = r#"{
        "Volumes": [
            {"CreatedAt": "2026-08-11T23:46:53+06:00", "Driver": "local", "Labels": null,
             "Mountpoint": "/var/lib/docker/volumes/xlabo_rebuild_mysql/_data",
             "Name": "xlabo_rebuild_mysql", "Options": null, "Scope": "local"},
            {"Name": "tmpfs-vol", "Driver": "local", "Mountpoint": "/x",
             "Options": {"type": "tmpfs", "device": "tmpfs"}, "Scope": "local"}
        ],
        "Warnings": null
    }"#;

    fn parsed() -> Vec<Volume> {
        let listed: VolumeList = serde_json::from_str(LIST).unwrap();
        listed.volumes
    }

    #[test]
    fn unwraps_the_volumes_object() {
        // Unlike containers and images, this endpoint nests its array.
        let volumes = parsed();
        assert_eq!(volumes.len(), 2);
        assert_eq!(volumes[0].name, "xlabo_rebuild_mysql");
        assert_eq!(volumes[0].driver, "local");
        assert_eq!(
            volumes[0].mountpoint,
            "/var/lib/docker/volumes/xlabo_rebuild_mysql/_data"
        );
    }

    #[test]
    fn treats_null_labels_and_options_as_empty() {
        let volumes = parsed();
        assert!(volumes[0].labels.is_empty());
        assert!(volumes[0].options.is_empty());
        assert_eq!(volumes[0].options_display(), "");
    }

    #[test]
    fn sorts_driver_options_for_display() {
        // A HashMap iterates arbitrarily; the display must not shuffle.
        assert_eq!(parsed()[1].options_display(), "device=tmpfs, type=tmpfs");
    }

    #[test]
    fn tolerates_a_response_with_no_volumes() {
        let listed: VolumeList = serde_json::from_str(r#"{"Volumes":null}"#).unwrap();
        assert!(listed.volumes.is_empty());
    }
}
