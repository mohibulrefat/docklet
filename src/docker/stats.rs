//! Container resource statistics — deliberately demand-driven.
//!
//! Docklet opens exactly one `/stats` stream at a time, for whichever
//! container's detail pane is open, and closes it the moment that pane
//! closes. There is no stats column in the list, no history, no persistence:
//! that would mean one open connection and one thread per running container,
//! continuously, which is precisely the "heavy monitoring subsystem" the
//! project's brief rules out. Cost stays constant regardless of how many
//! containers exist.

use async_channel::Sender;
use serde::Deserialize;

use super::stream::{StreamEvent, StreamHandle};
use super::{Docker, DockerError};

/// One JSON line of `/containers/{id}/stats?stream=1`.
///
/// Docker's payload is much larger; only the fields Docklet renders are kept.
#[derive(Debug, Clone, Deserialize)]
pub struct StatsSample {
    #[serde(rename = "cpu_stats", default)]
    pub cpu_stats: CpuStats,
    #[serde(rename = "precpu_stats", default)]
    pub precpu_stats: CpuStats,
    #[serde(rename = "memory_stats", default)]
    pub memory_stats: MemoryStats,
    #[serde(rename = "networks", default)]
    pub networks: std::collections::HashMap<String, NetworkStats>,
    /// RFC 3339. Used to turn cumulative network counters into a rate.
    #[serde(rename = "read", default)]
    pub read: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CpuStats {
    #[serde(rename = "cpu_usage", default)]
    pub cpu_usage: CpuUsage,
    /// Absent on a container's very first sample, not merely zero — Docker has
    /// not taken a second reading yet.
    #[serde(rename = "system_cpu_usage")]
    pub system_cpu_usage: Option<u64>,
    #[serde(rename = "online_cpus", default)]
    pub online_cpus: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CpuUsage {
    #[serde(rename = "total_usage", default)]
    pub total_usage: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemoryStats {
    #[serde(rename = "usage", default)]
    pub usage: u64,
    #[serde(rename = "limit", default)]
    pub limit: u64,
    #[serde(rename = "stats", default)]
    pub stats: std::collections::HashMap<String, u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NetworkStats {
    #[serde(rename = "rx_bytes", default)]
    pub rx_bytes: u64,
    #[serde(rename = "tx_bytes", default)]
    pub tx_bytes: u64,
}

/// What a stats stream delivers.
pub enum StatsEvent {
    Sample(StatsSample),
    Failed(String),
}

impl Docker {
    /// Stream a container's stats until the handle is dropped.
    pub fn follow_stats(&self, id: &str, sender: Sender<StatsEvent>) -> StreamHandle {
        let mut buffer = Vec::new();
        let path = format!("/containers/{id}/stats?stream=1");

        self.stream("GET", &path, move |event| match event {
            StreamEvent::Data(data) => {
                buffer.extend_from_slice(data);
                // Each JSON object is one line; a partial line waits for more.
                while let Some(newline) = buffer.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buffer.drain(..=newline).collect();
                    match serde_json::from_slice::<StatsSample>(&line) {
                        Ok(sample) => {
                            if sender.send_blocking(StatsEvent::Sample(sample)).is_err() {
                                return false;
                            }
                        }
                        // A line that fails to parse is not worth ending the
                        // stream over; the next one usually parses fine.
                        Err(_) => continue,
                    }
                }
                true
            }
            StreamEvent::Failed(e) => {
                let _ = sender.send_blocking(StatsEvent::Failed(e.to_string()));
                false
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `/containers/{id}/stats?stream=1` response.
    const SAMPLE: &str = r#"{
        "read": "2026-08-31T15:37:24.782923897Z",
        "cpu_stats": {"cpu_usage": {"total_usage": 76337000}, "system_cpu_usage": 224145870000000, "online_cpus": 8},
        "precpu_stats": {"cpu_usage": {"total_usage": 60000000}, "system_cpu_usage": 224137970000000, "online_cpus": 8},
        "memory_stats": {"usage": 2211840, "limit": 67108864,
                         "stats": {"inactive_file": 102400, "active_file": 51200}},
        "networks": {"eth0": {"rx_bytes": 6352, "tx_bytes": 126}}
    }"#;

    /// A container's first sample: `precpu_stats.system_cpu_usage` is absent.
    const FIRST_SAMPLE: &str = r#"{
        "read": "2026-08-31T15:37:20.000000000Z",
        "cpu_stats": {"cpu_usage": {"total_usage": 0}, "system_cpu_usage": 224315250000000, "online_cpus": 8},
        "precpu_stats": {"cpu_usage": {"total_usage": 0}},
        "memory_stats": {"usage": 1000000, "limit": 67108864, "stats": {}},
        "networks": {}
    }"#;

    #[test]
    fn parses_a_real_sample() {
        let sample: StatsSample = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(sample.cpu_stats.cpu_usage.total_usage, 76337000);
        assert_eq!(sample.cpu_stats.online_cpus, 8);
        assert_eq!(sample.memory_stats.usage, 2211840);
        assert_eq!(sample.memory_stats.limit, 67108864);
        assert_eq!(sample.networks["eth0"].rx_bytes, 6352);
    }

    #[test]
    fn treats_a_missing_precpu_system_usage_as_none_not_zero() {
        // The distinction matters: a real 0 would compute a real (if boring)
        // percentage, but a missing value must not be treated as one.
        let sample: StatsSample = serde_json::from_str(FIRST_SAMPLE).unwrap();
        assert_eq!(sample.precpu_stats.system_cpu_usage, None);
        assert_eq!(sample.cpu_stats.system_cpu_usage, Some(224315250000000));
    }

    #[test]
    fn parses_a_sample_with_no_network_interfaces() {
        let sample: StatsSample = serde_json::from_str(FIRST_SAMPLE).unwrap();
        assert!(sample.networks.is_empty());
    }
}
