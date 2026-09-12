//! The main window: a header bar, a page stack, and a status footer.
//!
//! The pages are placeholders for now; each is replaced by the milestone that
//! implements it.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, Button, HeaderBar, Orientation, Separator, Stack,
    StackSwitcher, ToggleButton,
};

use super::compose::ComposePage;
use super::connection::ConnectionStatus;
use super::containers::{self, ContainersPage};
use super::images::ImagesPage;
use super::networks::NetworksPage;
use super::volumes::VolumesPage;

const DEFAULT_WIDTH: i32 = 900;
const DEFAULT_HEIGHT: i32 = 600;
/// How often auto-refresh polls, once the user turns it on. It is never
/// consulted while auto-refresh is off, which is the default.
const AUTO_REFRESH_SECONDS: u32 = 5;

/// Build the main window.
pub fn build(app: &Application) -> ApplicationWindow {
    // Installed once, up front, rather than left to whichever page happens
    // to construct first: every page's rows (and the stale/status banners)
    // depend on these classes existing.
    containers::install_style();

    let connection = ConnectionStatus::new();

    let containers = ContainersPage::new(connection.clone());
    let compose = ComposePage::new(connection.clone());
    let images = ImagesPage::new(connection.clone());
    let volumes = VolumesPage::new(connection.clone());
    let networks = NetworksPage::new(connection.clone());

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

    let content = gtk::Box::new(Orientation::Vertical, 0);
    content.append(&stack);
    content.append(&Separator::new(Orientation::Horizontal));
    content.append(connection.widget());

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
