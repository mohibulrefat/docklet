//! The networks API.

use std::collections::HashMap;

use serde::Deserialize;

use super::containers::null_as_default;
use super::{Docker, DockerError};

/// Networks Docker creates itself and refuses to remove.
const PREDEFINED: [&str; 3] = ["bridge", "host", "none"];

/// A network as reported by `/networks`.
#[derive(Debug, Clone, Deserialize)]
pub struct Network {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Driver", default)]
    pub driver: String,
    #[serde(rename = "Scope", default)]
    pub scope: String,
    #[serde(rename = "Internal", default)]
    pub internal: bool,
    #[serde(rename = "Created", default)]
    pub created: String,
    #[serde(rename = "IPAM", default)]
    pub ipam: Ipam,
    /// Only `/networks/{id}` fills this in; the list leaves it empty.
    #[serde(rename = "Containers", default, deserialize_with = "null_as_default")]
    pub containers: HashMap<String, NetworkContainer>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Ipam {
    #[serde(rename = "Config", default, deserialize_with = "null_as_default")]
    pub config: Vec<IpamConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IpamConfig {
    #[serde(rename = "Subnet", default)]
    pub subnet: String,
    #[serde(rename = "Gateway", default)]
    pub gateway: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkContainer {
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "IPv4Address", default)]
    pub ipv4_address: String,
}

impl Network {
    /// Whether Docker created this network itself.
    ///
    /// These cannot be removed, so the UI disables the action rather than
    /// letting the user discover it through a 403.
    pub fn is_predefined(&self) -> bool {
        PREDEFINED.contains(&self.name.as_str())
    }

    /// Subnets, comma separated.
    pub fn subnets(&self) -> String {
        join(self.ipam.config.iter().map(|c| c.subnet.as_str()))
    }

    /// Gateways, comma separated.
    pub fn gateways(&self) -> String {
        join(self.ipam.config.iter().map(|c| c.gateway.as_str()))
    }

    /// Connected containers as `name (address)`, sorted by name.
    pub fn connected(&self) -> String {
        let mut rows: Vec<String> = self
            .containers
            .values()
            .map(|c| {
                if c.ipv4_address.is_empty() {
                    c.name.clone()
                } else {
                    format!("{} ({})", c.name, c.ipv4_address)
                }
            })
            .collect();
        rows.sort();
        rows.join(", ")
    }
}

/// Join non-empty values with commas.
fn join<'a>(values: impl Iterator<Item = &'a str>) -> String {
    values
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

impl Docker {
    /// List networks.
    pub fn networks(&self) -> Result<Vec<Network>, DockerError> {
        self.get_json("/networks")
    }

    /// Create a network. An empty driver means Docker's default, `bridge`.
    pub fn create_network(&self, name: &str, driver: &str) -> Result<(), DockerError> {
        let driver = if driver.trim().is_empty() {
            "bridge"
        } else {
            driver.trim()
        };
        let body = serde_json::json!({ "Name": name.trim(), "Driver": driver });
        self.post_json("/networks/create", &body)?;
        Ok(())
    }

    /// Remove a network.
    pub fn remove_network(&self, id: &str) -> Result<(), DockerError> {
        self.delete(&format!("/networks/{id}"))
    }

    /// Inspect one network, which is what reports its connected containers.
    pub fn inspect_network(&self, id: &str) -> Result<Network, DockerError> {
        self.get_json(&format!("/networks/{id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `/networks/{id}` response.
    const NETWORK: &str = r#"{
        "Name": "ubl-backend-services_ubl-network",
        "Id": "48025e55c98e299318a3a17143d691580a6ba5eac5372b25f585114b7cfac190",
        "Created": "2026-08-23T16:26:28.22397475+06:00",
        "Scope": "local", "Driver": "bridge", "Internal": false,
        "IPAM": {"Driver": "default", "Options": null,
                 "Config": [{"Subnet": "172.19.0.0/16", "Gateway": "172.19.0.1"}]},
        "Containers": {
            "abc": {"Name": "redis", "IPv4Address": "172.19.0.3/16"},
            "def": {"Name": "backend", "IPv4Address": "172.19.0.2/16"}
        }
    }"#;

    fn parsed() -> Network {
        serde_json::from_str(NETWORK).expect("fixture should parse")
    }

    #[test]
    fn parses_a_network() {
        let network = parsed();
        assert_eq!(network.driver, "bridge");
        assert_eq!(network.scope, "local");
        assert!(!network.internal);
    }

    #[test]
    fn reads_subnet_and_gateway_from_ipam() {
        let network = parsed();
        assert_eq!(network.subnets(), "172.19.0.0/16");
        assert_eq!(network.gateways(), "172.19.0.1");
    }

    #[test]
    fn lists_connected_containers_sorted() {
        // The map iterates arbitrarily; the display must be stable.
        assert_eq!(
            parsed().connected(),
            "backend (172.19.0.2/16), redis (172.19.0.3/16)"
        );
    }

    #[test]
    fn recognises_the_networks_docker_owns() {
        for name in ["bridge", "host", "none"] {
            let network: Network =
                serde_json::from_str(&format!(r#"{{"Id":"x","Name":"{name}"}}"#)).unwrap();
            assert!(network.is_predefined(), "{name} should be predefined");
        }
        let mine: Network = serde_json::from_str(r#"{"Id":"x","Name":"my-net"}"#).unwrap();
        assert!(!mine.is_predefined());
    }

    #[test]
    fn tolerates_a_network_with_no_ipam_or_containers() {
        let network: Network = serde_json::from_str(
            r#"{"Id":"x","Name":"none","Driver":"null","IPAM":{"Config":null},"Containers":null}"#,
        )
        .unwrap();
        assert_eq!(network.subnets(), "");
        assert_eq!(network.gateways(), "");
        assert_eq!(network.connected(), "");
    }

    #[test]
    fn omits_an_empty_gateway() {
        let network: Network = serde_json::from_str(
            r#"{"Id":"x","Name":"n","IPAM":{"Config":[{"Subnet":"10.0.0.0/8","Gateway":""}]}}"#,
        )
        .unwrap();
        assert_eq!(network.subnets(), "10.0.0.0/8");
        assert_eq!(network.gateways(), "");
    }
}
