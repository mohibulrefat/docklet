//! The main window: a header bar, a page stack, and a status footer.
//!
//! The pages are placeholders for now; each is replaced by the milestone that
//! implements it.

use gtk::prelude::*;
use gtk::{gio, glib};
use gtk::{
    Align, Application, ApplicationWindow, Button, HeaderBar, Label, Orientation, Separator, Stack,
    StackSwitcher,
};

use std::rc::Rc;

use super::containers::ContainersPage;
use crate::docker::Docker;

const LOG_DOMAIN: &str = "docklet";
const DEFAULT_WIDTH: i32 = 900;
const DEFAULT_HEIGHT: i32 = 600;

/// Build the main window.
pub fn build(app: &Application) -> ApplicationWindow {
    let containers = Rc::new(ContainersPage::new());

    let stack = Stack::builder().vexpand(true).build();
    stack.add_titled(containers.widget(), Some("containers"), "Containers");
    stack.add_titled(&placeholder("Images"), Some("images"), "Images");
    stack.add_titled(&placeholder("Volumes"), Some("volumes"), "Volumes");
    stack.add_titled(&placeholder("Networks"), Some("networks"), "Networks");

    let refresh = Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Refresh"));
    refresh.connect_clicked({
        let containers = containers.clone();
        move |_| containers.refresh()
    });

    let header = HeaderBar::new();
    header.set_title_widget(Some(&StackSwitcher::builder().stack(&stack).build()));
    header.pack_start(&refresh);

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
    window
}

/// A page that has not been implemented yet.
fn placeholder(name: &str) -> Label {
    let label = Label::new(Some(name));
    label.add_css_class("dim-label");
    label.set_vexpand(true);
    label
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
