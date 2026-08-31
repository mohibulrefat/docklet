//! The images API, and the formatting its list needs.

use serde::Deserialize;

use async_channel::Sender;

use super::containers::null_as_default;
use super::stream::{StreamEvent, StreamHandle};
use super::{Docker, DockerError};

/// What Docker calls an untagged repository or tag.
const NONE: &str = "<none>";

/// An image as reported by `/images/json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Image {
    /// Digest-prefixed, e.g. `sha256:a8fe9a…`.
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "RepoTags", default, deserialize_with = "null_as_default")]
    pub repo_tags: Vec<String>,
    #[serde(rename = "Size", default)]
    pub size: u64,
    /// Unix seconds.
    #[serde(rename = "Created", default)]
    pub created: i64,
}

impl Image {
    /// The first `repository:tag`, or `<none>:<none>` when untagged.
    fn reference(&self) -> &str {
        self.repo_tags.first().map(String::as_str).unwrap_or(NONE)
    }

    pub fn repository(&self) -> &str {
        split_reference(self.reference()).0
    }

    pub fn tag(&self) -> &str {
        split_reference(self.reference()).1
    }

    /// The 12 hex characters Docker displays, without the `sha256:` prefix.
    pub fn short_id(&self) -> &str {
        super::short_id(self.id.strip_prefix("sha256:").unwrap_or(&self.id))
    }

    /// An image with no usable tag — the `<none>` rows in `docker images`.
    pub fn is_dangling(&self) -> bool {
        self.repository() == NONE
    }

    pub fn size_display(&self) -> String {
        human_size(self.size)
    }

    pub fn created_display(&self, now: i64) -> String {
        relative_age(self.created, now)
    }
}

/// Split `repository:tag`.
///
/// The last colon only introduces a tag if no `/` follows it, so a registry
/// port such as `localhost:5000/app` is not mistaken for one.
fn split_reference(reference: &str) -> (&str, &str) {
    match reference.rsplit_once(':') {
        Some((repository, tag)) if !tag.contains('/') && !repository.is_empty() => {
            (repository, tag)
        }
        _ => (reference, NONE),
    }
}

/// Format a byte count the way Docker does — decimal units, three significant
/// figures.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }

    if unit == 0 {
        return format!("{bytes} B");
    }
    let decimals = if value >= 100.0 {
        0
    } else if value >= 10.0 {
        1
    } else {
        2
    };
    format!("{value:.decimals$} {}", UNITS[unit])
}

/// Describe how long ago a unix timestamp was.
pub fn relative_age(then: i64, now: i64) -> String {
    let seconds = now - then;
    if seconds < 0 {
        // Clock skew between us and the daemon; not worth a special case.
        return "just now".to_string();
    }

    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 365 * DAY;

    // The brackets follow Docker's own vocabulary, so ages here read the same
    // as they do in `docker images`.
    let (count, unit) = match seconds {
        s if s < MINUTE => return "just now".to_string(),
        s if s < HOUR => (s / MINUTE, "minute"),
        s if s < DAY => (s / HOUR, "hour"),
        s if s < WEEK => (s / DAY, "day"),
        s if s < 2 * MONTH => (s / WEEK, "week"),
        s if s < YEAR => (s / MONTH, "month"),
        s => (s / YEAR, "year"),
    };

    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// Seconds since the unix epoch, for relative ages.
pub fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

/// What a pull reports as it runs.
#[derive(Debug, PartialEq)]
pub enum PullEvent {
    /// A human-readable step, with the current layer's progress if known.
    Progress {
        message: String,
        fraction: Option<f64>,
    },
    /// The pull failed. Docker reports these *inside* a 200 response.
    Failed(String),
}

/// One line of Docker's pull stream.
#[derive(Deserialize)]
struct PullLine {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    id: Option<String>,
    /// Present only on failure — and the HTTP status is still 200.
    #[serde(default)]
    error: Option<String>,
    #[serde(rename = "progressDetail", default)]
    progress: Option<ProgressDetail>,
}

#[derive(Deserialize)]
struct ProgressDetail {
    #[serde(default)]
    current: Option<u64>,
    #[serde(default)]
    total: Option<u64>,
}

/// Turns the pull stream's newline-delimited JSON into events.
///
/// Lines can be split across reads, so partial input is buffered rather than
/// dropped.
#[derive(Default)]
pub struct PullDecoder {
    buffer: String,
}

impl PullDecoder {
    /// Feed raw bytes, returning whatever complete lines they yielded.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<PullEvent> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));

        let mut events = Vec::new();
        while let Some(newline) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=newline).collect();
            if let Some(event) = parse_pull_line(line.trim()) {
                events.push(event);
            }
        }
        events
    }
}

/// Interpret one JSON line of the pull stream.
fn parse_pull_line(line: &str) -> Option<PullEvent> {
    if line.is_empty() {
        return None;
    }

    // A line we cannot parse is not worth failing a pull over.
    let parsed: PullLine = serde_json::from_str(line).ok()?;

    if let Some(error) = parsed.error {
        return Some(PullEvent::Failed(error));
    }

    let status = parsed.status?;
    let message = match parsed.id {
        Some(id) if !id.is_empty() => format!("{status} {id}"),
        _ => status,
    };

    let fraction = parsed.progress.and_then(|p| match (p.current, p.total) {
        (Some(current), Some(total)) if total > 0 => Some((current as f64 / total as f64).min(1.0)),
        _ => None,
    });

    Some(PullEvent::Progress { message, fraction })
}

/// Split a pull reference into the name and tag Docker's API wants.
///
/// An unqualified reference means `latest`, as it does everywhere else.
fn split_for_pull(reference: &str) -> (&str, &str) {
    match split_reference(reference) {
        (name, tag) if tag != NONE => (name, tag),
        _ => (reference, "latest"),
    }
}

impl Docker {
    /// Pull an image, reporting progress until it finishes or fails.
    ///
    /// The stream ends by closing the channel; a `Failed` event arriving first
    /// is the only way to know a pull did not succeed, because Docker answers
    /// `200 OK` even when it is about to fail.
    pub fn pull_image(&self, reference: &str, sender: Sender<PullEvent>) -> StreamHandle {
        let (name, tag) = split_for_pull(reference.trim());
        let path = format!("/images/create?fromImage={name}&tag={tag}");
        let mut decoder = PullDecoder::default();

        self.stream("POST", &path, move |event| match event {
            StreamEvent::Data(data) => {
                for event in decoder.feed(data) {
                    let failed = matches!(event, PullEvent::Failed(_));
                    if sender.send_blocking(event).is_err() || failed {
                        return false;
                    }
                }
                true
            }
            StreamEvent::Failed(e) => {
                let _ = sender.send_blocking(PullEvent::Failed(e.to_string()));
                false
            }
        })
    }

    /// List images, including untagged ones.
    pub fn images(&self) -> Result<Vec<Image>, DockerError> {
        self.get_json("/images/json?all=0")
    }

    /// Remove an image.
    ///
    /// An image a container still references is a 409 unless `force` is set.
    pub fn remove_image(&self, id: &str, force: bool) -> Result<(), DockerError> {
        let path = if force {
            format!("/images/{id}?force=true")
        } else {
            format!("/images/{id}")
        };
        self.delete(&path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(tags: &str, size: u64) -> Image {
        serde_json::from_str(&format!(
            r#"{{"Id":"sha256:a8fe9a209bd10b89dfd92f0d5beb30c17b98c65303bc620bdb9ab072c7bda259",
                 "RepoTags":{tags},"Size":{size},"Created":1787480758}}"#
        ))
        .expect("fixture should parse")
    }

    #[test]
    fn splits_repository_and_tag() {
        let image = image(r#"["nginx:alpine"]"#, 0);
        assert_eq!(image.repository(), "nginx");
        assert_eq!(image.tag(), "alpine");
    }

    #[test]
    fn keeps_a_registry_port_with_the_repository() {
        // The colon in the port must not be read as a tag separator.
        assert_eq!(
            split_reference("localhost:5000/app:v2"),
            ("localhost:5000/app", "v2")
        );
        assert_eq!(
            split_reference("localhost:5000/app"),
            ("localhost:5000/app", NONE)
        );
    }

    #[test]
    fn handles_a_namespaced_repository() {
        let image = image(r#"["minio/minio:latest"]"#, 0);
        assert_eq!(image.repository(), "minio/minio");
        assert_eq!(image.tag(), "latest");
    }

    #[test]
    fn reports_an_untagged_image_as_dangling() {
        let image = image(r#"["<none>:<none>"]"#, 0);
        assert!(image.is_dangling());
        assert_eq!(image.repository(), NONE);
        assert_eq!(image.tag(), NONE);
    }

    #[test]
    fn treats_null_repo_tags_as_dangling() {
        let image = image("null", 0);
        assert!(image.is_dangling());
    }

    #[test]
    fn strips_the_digest_prefix_from_the_id() {
        assert_eq!(image(r#"["a:b"]"#, 0).short_id(), "a8fe9a209bd1");
    }

    #[test]
    fn formats_sizes_like_docker() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1_193_603), "1.19 MB");
        assert_eq!(human_size(56_200_000), "56.2 MB");
        assert_eq!(human_size(412_000_000), "412 MB");
        assert_eq!(human_size(2_500_000_000), "2.50 GB");
    }

    #[test]
    fn switches_units_at_a_thousand() {
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1000), "1.00 kB");
    }

    #[test]
    fn describes_recent_times_loosely() {
        let now = 1_800_000_000;
        assert_eq!(relative_age(now, now), "just now");
        assert_eq!(relative_age(now - 30, now), "just now");
        assert_eq!(relative_age(now - 60, now), "1 minute ago");
        assert_eq!(relative_age(now - 3600, now), "1 hour ago");
        assert_eq!(relative_age(now - 4 * 86400, now), "4 days ago");
    }

    #[test]
    fn uses_weeks_like_docker_does() {
        // `docker images` shows "2 weeks ago" where a naive formatter would
        // say "19 days ago".
        let now = 1_800_000_000;
        assert_eq!(relative_age(now - 6 * 86400, now), "6 days ago");
        assert_eq!(relative_age(now - 7 * 86400, now), "1 week ago");
        assert_eq!(relative_age(now - 19 * 86400, now), "2 weeks ago");
        assert_eq!(relative_age(now - 70 * 86400, now), "2 months ago");
    }

    #[test]
    fn pluralises_ages() {
        let now = 1_800_000_000;
        assert_eq!(relative_age(now - 120, now), "2 minutes ago");
        assert_eq!(relative_age(now - 2 * 30 * 86400, now), "2 months ago");
        assert_eq!(relative_age(now - 400 * 86400, now), "1 year ago");
    }

    #[test]
    fn defaults_an_untagged_pull_to_latest() {
        assert_eq!(split_for_pull("alpine"), ("alpine", "latest"));
        assert_eq!(split_for_pull("alpine:3.19"), ("alpine", "3.19"));
        assert_eq!(split_for_pull("minio/minio"), ("minio/minio", "latest"));
        assert_eq!(
            split_for_pull("localhost:5000/app"),
            ("localhost:5000/app", "latest")
        );
    }

    #[test]
    fn reports_pull_progress() {
        let mut decoder = PullDecoder::default();
        let events = decoder.feed(
            br#"{"status":"Pulling fs layer","progressDetail":{},"id":"17a39c0ba978"}
"#,
        );
        assert_eq!(
            events,
            vec![PullEvent::Progress {
                message: "Pulling fs layer 17a39c0ba978".to_string(),
                fraction: None,
            }]
        );
    }

    #[test]
    fn computes_a_fraction_when_docker_gives_counts() {
        let mut decoder = PullDecoder::default();
        let events = decoder.feed(
            br#"{"status":"Downloading","progressDetail":{"current":50,"total":200},"id":"abc"}
"#,
        );
        match &events[0] {
            PullEvent::Progress { fraction, .. } => assert_eq!(*fraction, Some(0.25)),
            other => panic!("expected progress, got {other:?}"),
        }
    }

    #[test]
    fn detects_a_failure_carried_inside_a_successful_response() {
        // Docker answers 200 and then reports the failure in the body; missing
        // this would leave a failed pull looking like it worked.
        let mut decoder = PullDecoder::default();
        let events = decoder.feed(
            br#"{"errorDetail":{"message":"manifest unknown"},"error":"manifest unknown"}
"#,
        );
        assert_eq!(
            events,
            vec![PullEvent::Failed("manifest unknown".to_string())]
        );
    }

    #[test]
    fn buffers_a_line_split_across_reads() {
        let mut decoder = PullDecoder::default();
        assert!(decoder.feed(br#"{"status":"Down"#).is_empty());
        let events = decoder.feed(b"loading\"}\n");
        assert_eq!(
            events,
            vec![PullEvent::Progress {
                message: "Downloading".to_string(),
                fraction: None,
            }]
        );
    }

    #[test]
    fn reads_several_lines_from_one_read() {
        let mut decoder = PullDecoder::default();
        let events = decoder.feed(b"{\"status\":\"one\"}\n{\"status\":\"two\"}\n");
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn ignores_blank_and_unparseable_lines() {
        let mut decoder = PullDecoder::default();
        assert!(decoder.feed(b"\n\r\n").is_empty());
        assert!(decoder.feed(b"not json\n").is_empty());
    }

    #[test]
    fn tolerates_a_timestamp_in_the_future() {
        // Clock skew between Docklet and the daemon must not print nonsense.
        let now = 1_800_000_000;
        assert_eq!(relative_age(now + 500, now), "just now");
    }
}
