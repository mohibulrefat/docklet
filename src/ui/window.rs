//! The main window: a header bar, a page stack, and a status footer.
//!
//! The pages are placeholders for now; each is replaced by the milestone that
//! implements it.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use gtk::{
    Align, Application, ApplicationWindow, Button, HeaderBar, Label, Orientation, Separator, Stack,
    StackSwitcher, ToggleButton,
};

use super::compose::ComposePage;
use super::containers::ContainersPage;
use super::images::ImagesPage;
use super::networks::NetworksPage;
use super::volumes::VolumesPage;
use crate::docker::Docker;

const LOG_DOMAIN: &str = "docklet";
const DEFAULT_WIDTH: i32 = 900;
const DEFAULT_HEIGHT: i32 = 600;
/// How often auto-refresh polls, once the user turns it on. It is never
/// consulted while auto-refresh is off, which is the default.
const AUTO_REFRESH_SECONDS: u32 = 5;

/// Build the main window.
pub fn build(app: &Application) -> ApplicationWindow {
    let containers = ContainersPage::new();
    let compose = ComposePage::new();
    let images = ImagesPage::new();
    let volumes = VolumesPage::new();
    let networks = NetworksPage::new();

    let stack = Stack::builder().vexpand(true).build();
    stack.add_titled(containers.widget(), Some("containers"), "Containers");
    stack.add_titled(images.widget(), Some("images"), "Images");
    stack.add_titled(volumes.widget(), Some("volumes"), "Volumes");
    stack.add_titled(networks.widget(), Some("networks"), "Networks");
    stack.add_titled(compose.widget(), Some("compose"), "Compose");

    // One shared path for refreshing "whichever page is currently showing" —
    // both the manual button and auto-refresh's timer call this, so there is
    // exactly one place that knows how to route a refresh to a page.
    let refresh_visible_page: Rc<dyn Fn()> = Rc::new({
        let stack = stack.clone();
        move || match stack.visible_child_name().as_deref() {
            Some("images") => images.refresh(),
            Some("volumes") => volumes.refresh(),
            Some("networks") => networks.refresh(),
            Some("compose") => compose.refresh(),
            _ => containers.refresh(),
        }
    });

    let refresh = Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Refresh"));
    refresh.connect_clicked({
        let refresh_visible_page = refresh_visible_page.clone();
        move |_| refresh_visible_page()
    });

    let auto_refresh = ToggleButton::builder()
        .icon_name("alarm-symbolic")
        .tooltip_text(
            "Auto-refresh the current page every 5 seconds. Off by default — \
             manual refresh always works regardless of this setting.",
        )
        .build();

    let header = HeaderBar::new();
    header.set_title_widget(Some(&StackSwitcher::builder().stack(&stack).build()));
    header.pack_start(&refresh);
    header.pack_start(&auto_refresh);

    let status = status_bar();
    check_docker(&status);

    let content = gtk::Box::new(Orientation::Vertical, 0);
    content.append(&stack);
    content.append(&Separator::new(Orientation::Horizontal));
    content.append(&status);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Docklet")
        .default_width(DEFAULT_WIDTH)
        .default_height(DEFAULT_HEIGHT)
        .child(&content)
        .build();
    window.set_titlebar(Some(&header));

    connect_auto_refresh(&auto_refresh, &window, refresh_visible_page);

    window
}

/// Wire the auto-refresh toggle to a `glib::timeout_add_seconds_local`.
///
/// Off is the default and the common case, and off means no timer exists at
/// all — not a timer that wakes up and does nothing. Turning it on adds one
/// source; turning it off, or the window losing focus or being unmapped,
/// removes it. The window regaining focus while the toggle is still on adds
/// it back, so a backgrounded Docklet spends nothing while it is not the
/// window someone is looking at.
fn connect_auto_refresh(toggle: &ToggleButton, window: &ApplicationWindow, refresh: Rc<dyn Fn()>) {
    let source: Rc<std::cell::RefCell<Option<glib::SourceId>>> =
        Rc::new(std::cell::RefCell::new(None));

    let start = {
        let source = source.clone();
        let refresh = refresh.clone();
        move || {
            if source.borrow().is_some() {
                return; // Already running.
            }
            let refresh = refresh.clone();
            let id = glib::timeout_add_seconds_local(AUTO_REFRESH_SECONDS, move || {
                refresh();
                glib::ControlFlow::Continue
            });
            source.replace(Some(id));
        }
    };

    let stop = {
        let source = source.clone();
        move || {
            if let Some(id) = source.take() {
                id.remove();
            }
        }
    };

    toggle.connect_toggled({
        let window = window.clone();
        let start = start.clone();
        let stop = stop.clone();
        move |button| {
            if button.is_active() && window.is_active() {
                start();
            } else {
                stop();
            }
        }
    });

    window.connect_is_active_notify({
        let toggle = toggle.clone();
        let start = start.clone();
        let stop = stop.clone();
        move |window| {
            if !toggle.is_active() {
                return;
            }
            if window.is_active() {
                start();
            } else {
                stop();
            }
        }
    });

    // Belt and braces alongside is-active: a window can be unmapped (e.g.
    // minimized on some compositors) without necessarily losing is-active.
    window.connect_unmap({
        let stop = stop.clone();
        move |_| stop()
    });
    window.connect_map({
        let toggle = toggle.clone();
        move |window| {
            if toggle.is_active() && window.is_active() {
                start();
            }
        }
    });
}

/// Report Docker's status in the footer.
///
/// The check runs on a worker thread so the window opens immediately: a daemon
/// that is down takes as long to discover as one that is up, and neither should
/// delay the first frame.
fn check_docker(status: &Label) {
    status.set_text("Checking Docker…");

    let status = status.clone();
    glib::spawn_future_local(async move {
        match gio::spawn_blocking(docker_status).await {
            Ok(text) => status.set_text(&text),
            // Only reachable if the worker panicked.
            Err(_) => status.set_text("Could not check Docker."),
        }
    });
}

/// Ask Docker who it is. Runs on a worker thread; returns text to display.
///
/// The message is built here rather than in the widget code so the UI never has
/// to interpret a `DockerError` — or know that endpoints exist.
fn docker_status() -> String {
    match connect_and_describe() {
        Ok(description) => {
            glib::g_info!(LOG_DOMAIN, "connected to {description}");
            description
        }
        Err(e) => {
            glib::g_warning!(LOG_DOMAIN, "docker unavailable: {e}");
            e.to_string()
        }
    }
}

fn connect_and_describe() -> Result<String, crate::docker::DockerError> {
    let docker = Docker::connect()?;
    docker.ping()?;
    Ok(docker.version()?.to_string())
}

/// The footer that reports Docker's status.
///
/// Created empty; `check_docker` sets the text before the window is shown.
fn status_bar() -> Label {
    let label = Label::new(None);
    label.add_css_class("dim-label");
    label.set_halign(Align::Start);
    label.set_margin_top(4);
    label.set_margin_bottom(4);
    label.set_margin_start(8);
    label.set_margin_end(8);
    label
}
