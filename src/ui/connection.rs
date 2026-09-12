//! The app-wide Docker connection indicator, shown in the window's footer.
//!
//! Every page's refresh reports here when it succeeds or fails, so the
//! footer reflects whatever any page most recently learned about Docker —
//! not just what a one-time startup check saw.

use std::cell::Cell;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::{Align, Label};

use crate::docker::{Docker, DockerError};

const LOG_DOMAIN: &str = "docklet";

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Checking,
    Connected,
    Unavailable,
}

/// The footer label reporting whether Docker is reachable.
pub struct ConnectionStatus {
    label: Label,
    state: Cell<State>,
}

impl ConnectionStatus {
    /// Build the indicator and kick off the initial ping + version check on
    /// a worker thread, so the window opens before Docker answers.
    pub fn new() -> Rc<Self> {
        let label = Label::new(Some("Checking Docker…"));
        label.add_css_class("dim-label");
        label.set_halign(Align::Start);
        label.set_margin_top(4);
        label.set_margin_bottom(4);
        label.set_margin_start(8);
        label.set_margin_end(8);

        let status = Rc::new(ConnectionStatus {
            label,
            state: Cell::new(State::Checking),
        });
        status.check();
        status
    }

    pub fn widget(&self) -> &Label {
        &self.label
    }

    /// Run the ping + version check and update the footer with the result.
    fn check(self: &Rc<Self>) {
        let status = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let outcome = gio::spawn_blocking(connect_and_describe).await;

            let Some(status) = status.upgrade() else {
                return;
            };
            match outcome {
                Ok(Ok(description)) => {
                    glib::g_info!(LOG_DOMAIN, "connected to {description}");
                    status.set_connected(&description);
                }
                Ok(Err(e)) => {
                    glib::g_warning!(LOG_DOMAIN, "docker unavailable: {e}");
                    status.set_unavailable(&e.to_string());
                }
                Err(_) => status.set_unavailable("Could not check Docker."),
            }
        });
    }

    fn set_connected(&self, description: &str) {
        self.state.set(State::Connected);
        self.label.remove_css_class("docklet-status-error");
        self.label.set_text(description);
    }

    fn set_unavailable(&self, message: &str) {
        self.state.set(State::Unavailable);
        self.label.add_css_class("docklet-status-error");
        self.label
            .set_text(&format!("Docker unavailable — {message}"));
    }

    /// A page's refresh (or other Docker request) failed. Only a
    /// connection-level failure changes the footer — an API error (a 404, a
    /// conflict) means Docker was reached and answered fine.
    pub fn report_error(&self, error: &DockerError) {
        if error.is_connection_error() {
            self.set_unavailable(&error.to_string());
        }
    }

    /// A page's refresh succeeded. If the footer was showing "unavailable",
    /// re-run the full check to restore the descriptive version text a bare
    /// "connected" would lose; otherwise there is nothing to do, so this
    /// never adds a Docker round trip to routine, already-healthy refreshes.
    pub fn report_ok(self: &Rc<Self>) {
        if self.state.get() == State::Unavailable {
            self.check();
        }
    }
}

fn connect_and_describe() -> Result<String, DockerError> {
    let docker = Docker::connect()?;
    docker.ping()?;
    Ok(docker.version()?.to_string())
}
