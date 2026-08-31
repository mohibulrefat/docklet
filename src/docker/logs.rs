//! Container logs, and the framing Docker wraps them in.
//!
//! A container created without a TTY has its stdout and stderr multiplexed into
//! one stream, each write prefixed by an 8-byte header:
//!
//! ```text
//! byte 0    stream: 0 stdin, 1 stdout, 2 stderr
//! bytes 1-3 zero padding
//! bytes 4-7 payload length, big endian
//! ```
//!
//! With a TTY there is no framing at all — the bytes are the output. Both cases
//! occur in practice, so both are handled.

use async_channel::Sender;

use super::stream::{StreamEvent, StreamHandle};
use super::{Docker, DockerError};

/// Header length for one multiplexed frame.
const HEADER: usize = 8;

/// Decodes a log stream, framed or raw.
///
/// Feeding is incremental: a frame split across reads is held until the rest
/// arrives, which is what lets the same decoder serve a one-shot fetch and a
/// live follow.
pub struct LogDecoder {
    tty: bool,
    buffer: Vec<u8>,
}

impl LogDecoder {
    pub fn new(tty: bool) -> Self {
        LogDecoder {
            tty,
            buffer: Vec::new(),
        }
    }

    /// Feed raw bytes, returning whatever complete output they yielded.
    pub fn feed(&mut self, bytes: &[u8]) -> String {
        if self.tty {
            // No framing: the bytes are the output.
            return String::from_utf8_lossy(bytes).into_owned();
        }

        self.buffer.extend_from_slice(bytes);
        let mut out = String::new();

        while self.buffer.len() >= HEADER {
            let length = u32::from_be_bytes([
                self.buffer[4],
                self.buffer[5],
                self.buffer[6],
                self.buffer[7],
            ]) as usize;

            let end = HEADER + length;
            if self.buffer.len() < end {
                // The rest of this frame has not arrived yet.
                break;
            }

            out.push_str(&String::from_utf8_lossy(&self.buffer[HEADER..end]));
            self.buffer.drain(..end);
        }

        out
    }
}

/// What a followed log stream delivers.
pub enum LogEvent {
    Text(String),
    Failed(String),
}

impl Docker {
    /// Follow a container's logs until the handle is dropped.
    ///
    /// Decoding lives in the worker: the decoder holds partial frames between
    /// reads, so it must be the same one for the life of the stream.
    pub fn follow_logs(
        &self,
        id: &str,
        tty: bool,
        tail: usize,
        sender: Sender<LogEvent>,
    ) -> StreamHandle {
        let mut decoder = LogDecoder::new(tty);
        let path = format!("/containers/{id}/logs?stdout=1&stderr=1&tail={tail}&follow=1");

        self.stream(&path, move |event| match event {
            StreamEvent::Data(data) => {
                let text = decoder.feed(data);
                if text.is_empty() {
                    // A partial frame; wait for the rest.
                    return true;
                }
                // A closed receiver means the view is gone: stop reading.
                sender.send_blocking(LogEvent::Text(text)).is_ok()
            }
            StreamEvent::Failed(e) => {
                let _ = sender.send_blocking(LogEvent::Failed(e.to_string()));
                false
            }
        })
    }

    /// Fetch the tail of a container's logs.
    ///
    /// `tty` comes from inspecting the container and decides how the response
    /// is framed; passing the wrong value yields binary noise, not an error.
    pub fn container_logs(&self, id: &str, tty: bool, tail: usize) -> Result<String, DockerError> {
        let body = self.get(&format!(
            "/containers/{id}/logs?stdout=1&stderr=1&tail={tail}"
        ))?;
        Ok(LogDecoder::new(tty).feed(&body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build one frame the way Docker does.
    fn frame(stream: u8, payload: &str) -> Vec<u8> {
        let mut out = vec![stream, 0, 0, 0];
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(payload.as_bytes());
        out
    }

    #[test]
    fn decodes_a_single_frame() {
        let mut decoder = LogDecoder::new(false);
        assert_eq!(decoder.feed(&frame(1, "tick 1\n")), "tick 1\n");
    }

    #[test]
    fn decodes_stdout_and_stderr_in_order() {
        let mut decoder = LogDecoder::new(false);
        let mut stream = frame(2, "err 1\n");
        stream.extend(frame(1, "tick 1\n"));
        assert_eq!(decoder.feed(&stream), "err 1\ntick 1\n");
    }

    #[test]
    fn decodes_real_captured_bytes() {
        // Captured from `docker logs` on this machine: stderr len 7, then
        // stdout len 8.
        let raw: Vec<u8> = vec![
            0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, b'e', b'r', b'r', b' ', b'1', b'7',
            b'\n', 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, b't', b'i', b'c', b'k', b' ',
            b'1', b'8', b'\n',
        ];
        let mut decoder = LogDecoder::new(false);
        assert_eq!(decoder.feed(&raw), "err 17\ntick 18\n");
    }

    #[test]
    fn holds_a_frame_split_mid_payload() {
        let mut decoder = LogDecoder::new(false);
        let full = frame(1, "hello world\n");
        let (head, tail) = full.split_at(12);

        assert_eq!(
            decoder.feed(head),
            "",
            "incomplete frame must yield nothing"
        );
        assert_eq!(decoder.feed(tail), "hello world\n");
    }

    #[test]
    fn holds_a_frame_split_mid_header() {
        let mut decoder = LogDecoder::new(false);
        let full = frame(1, "abc");
        let (head, tail) = full.split_at(3);

        assert_eq!(decoder.feed(head), "", "partial header must yield nothing");
        assert_eq!(decoder.feed(tail), "abc");
    }

    #[test]
    fn decodes_one_byte_at_a_time() {
        // The worst case a live stream can produce.
        let mut decoder = LogDecoder::new(false);
        let full = frame(1, "drip\n");
        let mut out = String::new();
        for byte in full {
            out.push_str(&decoder.feed(&[byte]));
        }
        assert_eq!(out, "drip\n");
    }

    #[test]
    fn passes_tty_output_through_unframed() {
        let mut decoder = LogDecoder::new(true);
        assert_eq!(decoder.feed(b"tty-line 62\r\n"), "tty-line 62\r\n");
    }

    #[test]
    fn yields_nothing_for_an_empty_stream() {
        assert_eq!(LogDecoder::new(false).feed(&[]), "");
        assert_eq!(LogDecoder::new(true).feed(&[]), "");
    }

    #[test]
    fn decodes_an_empty_frame() {
        let mut decoder = LogDecoder::new(false);
        let mut stream = frame(1, "");
        stream.extend(frame(1, "after\n"));
        assert_eq!(decoder.feed(&stream), "after\n");
    }

    #[test]
    fn survives_invalid_utf8() {
        let mut decoder = LogDecoder::new(false);
        let mut raw = vec![1, 0, 0, 0, 0, 0, 0, 2];
        raw.extend_from_slice(&[0xff, 0xfe]);
        // Lossy rather than an error: a log viewer must not fail on binary.
        assert!(!decoder.feed(&raw).is_empty());
    }
}
