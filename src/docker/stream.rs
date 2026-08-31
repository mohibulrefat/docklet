//! Long-lived streams, and stopping them.
//!
//! Docker's follow, stats and pull endpoints all hold a connection open and
//! deliver data until the caller goes away. They share one problem: a worker
//! blocked in `read` cannot notice a cancel flag. Shutting the socket down from
//! another thread unblocks that read immediately, which is why [`StreamHandle`]
//! keeps a second handle to the connection rather than polling with a timeout.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use super::transport::{self, Connection};
use super::{http, Docker, DockerError};

/// What a stream hands to its callback.
pub enum StreamEvent<'a> {
    /// A piece of body.
    Data(&'a [u8]),
    /// The stream could not be opened, or died mid-flight.
    Failed(DockerError),
}

/// A running stream. Dropping it stops the stream.
pub struct StreamHandle {
    cancelled: Arc<AtomicBool>,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl StreamHandle {
    /// Stop the stream and unblock its worker.
    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Ok(connection) = self.connection.lock() {
            if let Some(connection) = connection.as_ref() {
                connection.shutdown_read();
            }
        }
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        // Closing a detail pane or the window must not leave a thread reading.
        self.stop();
    }
}

impl Docker {
    /// Stream a GET, handing each piece of body to `on_data` on a worker thread.
    ///
    /// `on_data` returns false to stop. It is called off the main thread, so it
    /// must not touch widgets — send the data somewhere instead.
    pub fn stream<F>(&self, path: &str, mut on_event: F) -> StreamHandle
    where
        F: FnMut(StreamEvent) -> bool + Send + 'static,
    {
        let cancelled = Arc::new(AtomicBool::new(false));
        let connection: Arc<Mutex<Option<Connection>>> = Arc::new(Mutex::new(None));

        let handle = StreamHandle {
            cancelled: cancelled.clone(),
            connection: connection.clone(),
        };

        let endpoint = self.endpoint.clone();
        let path = path.to_string();

        thread::spawn(move || {
            let mut body = match open(&endpoint, &path, &connection) {
                Ok(body) => body,
                Err(e) => {
                    on_event(StreamEvent::Failed(e));
                    return;
                }
            };

            while !cancelled.load(Ordering::Relaxed) {
                match body.next() {
                    Ok(Some(data)) => {
                        if !on_event(StreamEvent::Data(&data)) {
                            break;
                        }
                    }
                    // The stream ended on its own.
                    Ok(None) => break,
                    Err(e) => {
                        // A cancel shuts the socket down, which surfaces here
                        // as a read error; that is expected, not a failure.
                        if !cancelled.load(Ordering::Relaxed) {
                            on_event(StreamEvent::Failed(e));
                        }
                        break;
                    }
                }
            }
        });

        handle
    }
}

/// Connect, publish a shutdown handle, and read past the response head.
fn open(
    endpoint: &super::Endpoint,
    path: &str,
    connection: &Arc<Mutex<Option<Connection>>>,
) -> Result<http::Streaming, DockerError> {
    let socket = transport::connect(endpoint)?;

    // A followed stream is idle whenever the container is quiet, so the
    // request timeout must not apply to it.
    socket.set_read_timeout(None)?;

    if let Ok(mut slot) = connection.lock() {
        *slot = Some(socket.try_clone()?);
    }

    let (status, mut body) = http::open_stream(socket, "GET", path)?;
    if status >= 400 {
        // The failure body carries Docker's own message; read it rather than
        // showing the user a URL.
        let mut raw = Vec::new();
        while let Ok(Some(chunk)) = body.next() {
            raw.extend_from_slice(&chunk);
            if raw.len() > 4096 {
                break;
            }
        }
        return Err(DockerError::Api {
            status,
            message: super::api_message(&raw),
        });
    }
    Ok(body)
}
